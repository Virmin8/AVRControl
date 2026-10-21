//! Audio keep-alive: plays an inaudible signal to the receiver's audio
//! output so the HDMI audio link never goes idle. Without it the PC stops
//! sending audio when nothing plays, and the receiver drops the first
//! second or so of the next sound while it locks onto the stream again.
//!
//! The signal is a ±1 LSB (16-bit) square wave at 1 Hz rather than digital
//! silence, because some drivers and receivers treat all-zero audio as no
//! audio at all.
//!
//! Linux plays through `pacat` (PulseAudio or PipeWire's pulse server),
//! Windows through WASAPI.

use std::sync::Mutex;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::config::AudioConfig;

const RATE: u32 = 48_000;
/// How long to wait before trying again when the device is missing or the
/// stream failed (e.g. the receiver is off and its HDMI output is gone).
const RETRY: Duration = Duration::from_secs(5);

/// An audio output, as offered in the settings window.
#[derive(Debug, Clone, PartialEq)]
pub struct Device {
    /// What goes in the config (sink name on Linux, endpoint name on Windows).
    pub id: String,
    /// Human-readable name.
    pub label: String,
}

struct Running {
    settings: AudioConfig,
    stop: mpsc::Sender<()>,
    thread: JoinHandle<()>,
}

/// What should be playing: the settings, and whether the receiver is on
/// (None when unknown, e.g. unreachable).
struct Wanted {
    settings: AudioConfig,
    receiver_on: Option<bool>,
}

static WANTED: Mutex<Option<Wanted>> = Mutex::new(None);
static RUNNING: Mutex<Option<Running>> = Mutex::new(None);

/// Starts, stops or retargets the keep-alive to match the settings.
pub fn apply(settings: &AudioConfig) {
    let mut wanted = WANTED.lock().unwrap();
    let receiver_on = wanted.as_ref().and_then(|w| w.receiver_on);
    *wanted = Some(Wanted {
        settings: settings.clone(),
        receiver_on,
    });
    reconcile(wanted.as_ref().unwrap());
}

/// Tells the keep-alive whether the receiver is on, so it can pause while
/// it's in standby.
pub fn set_receiver_power(on: Option<bool>) {
    let mut wanted = WANTED.lock().unwrap();
    let Some(w) = wanted.as_mut() else {
        return;
    };
    if w.receiver_on != on {
        w.receiver_on = on;
        reconcile(w);
    }
}

fn reconcile(wanted: &Wanted) {
    let settings = &wanted.settings;
    let play =
        settings.keep_alive && !(settings.pause_in_standby && wanted.receiver_on == Some(false));
    let mut guard = RUNNING.lock().unwrap();
    let current = guard.as_ref().map(|r| &r.settings);
    if (play && current == Some(settings)) || (!play && current.is_none()) {
        return;
    }
    if let Some(r) = guard.take() {
        let _ = r.stop.send(());
        let _ = r.thread.join();
    }
    if !play {
        return;
    }
    let (stop, stop_rx) = mpsc::channel();
    let device = settings.device.clone();
    let thread = thread::Builder::new()
        .name("audio-keepalive".into())
        .spawn(move || imp::run(&device, &stop_rx))
        .expect("spawning the audio keep-alive thread");
    *guard = Some(Running {
        settings: settings.clone(),
        stop,
        thread,
    });
}

/// Audio outputs that can be kept alive.
pub fn devices() -> anyhow::Result<Vec<Device>> {
    imp::devices()
}

/// Sample `n` of the keep-alive signal: +1 for half a second, then -1.
fn sample(n: u64) -> i16 {
    if (n / (RATE as u64 / 2)).is_multiple_of(2) {
        1
    } else {
        -1
    }
}

/// Waits up to `timeout`; true if the keep-alive should stop.
fn stopped(stop: &mpsc::Receiver<()>, timeout: Duration) -> bool {
    !matches!(stop.recv_timeout(timeout), Err(RecvTimeoutError::Timeout))
}

#[cfg(target_os = "linux")]
mod imp {
    use std::io::Write;
    use std::process::{Child, Command, Stdio};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use anyhow::{Context, Result, bail};

    use super::{Device, RATE, RETRY, sample, stopped};

    /// How often to check that the chosen sink still exists. PipeWire and
    /// PulseAudio move a stream to another sink when its own goes away, so
    /// the stream is restarted once the receiver's sink is back.
    const SINK_CHECK: Duration = Duration::from_secs(10);

    pub fn devices() -> Result<Vec<Device>> {
        let out = Command::new("pactl")
            .args(["-f", "json", "list", "sinks"])
            .stderr(Stdio::null())
            .output()
            .context("running pactl (install libpulse / pulseaudio-utils)")?;
        if !out.status.success() {
            bail!("pactl failed; is PipeWire or PulseAudio running?");
        }
        let sinks: Vec<serde_json::Value> =
            serde_json::from_slice(&out.stdout).context("reading pactl's sink list")?;
        Ok(sinks
            .iter()
            .filter_map(|s| {
                let id = s.get("name")?.as_str()?.to_string();
                let label = s
                    .get("description")
                    .and_then(|d| d.as_str())
                    .filter(|d| !d.is_empty())
                    .unwrap_or(&id)
                    .to_string();
                Some(Device { id, label })
            })
            .collect())
    }

    /// Whether the sink exists. Unknown (pactl missing or failing) counts as
    /// yes, so pacat gets to report the real problem.
    fn sink_exists(name: &str) -> bool {
        devices().map_or(true, |list| list.iter().any(|d| d.id == name))
    }

    fn spawn_pacat(device: &str) -> std::io::Result<Child> {
        let mut cmd = Command::new("pacat");
        cmd.args([
            "--playback",
            "--raw",
            "--format=s16le",
            "--channels=2",
            "--client-name=AVR Control",
            "--stream-name=Receiver keep-alive",
            "--latency-msec=500",
        ])
        .arg(format!("--rate={RATE}"))
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
        if !device.is_empty() {
            cmd.arg(format!("--device={device}"));
        }
        cmd.spawn()
    }

    pub fn run(device: &str, stop: &mpsc::Receiver<()>) {
        let mut n: u64 = 0;
        // A tenth of a second of stereo s16 per write.
        let frames = RATE as usize / 10;
        let mut buf = vec![0u8; frames * 4];
        loop {
            if !device.is_empty() && !sink_exists(device) {
                if stopped(stop, RETRY) {
                    return;
                }
                continue;
            }
            let mut child = match spawn_pacat(device) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("audio keep-alive: could not run pacat: {e}");
                    if stopped(stop, RETRY * 6) {
                        return;
                    }
                    continue;
                }
            };
            let mut stdin = child.stdin.take().expect("piped stdin");
            let mut checked = Instant::now();
            // pacat reads in real time, so each write blocks until there is
            // room and the loop runs at the playback rate.
            let quit = loop {
                for frame in buf.as_chunks_mut::<4>().0 {
                    let s = sample(n).to_le_bytes();
                    frame[..2].copy_from_slice(&s);
                    frame[2..].copy_from_slice(&s);
                    n += 1;
                }
                if stdin.write_all(&buf).is_err() {
                    break false;
                }
                if stopped(stop, Duration::ZERO) {
                    break true;
                }
                if !device.is_empty() && checked.elapsed() > SINK_CHECK {
                    checked = Instant::now();
                    if !sink_exists(device) {
                        break false;
                    }
                }
            };
            drop(stdin);
            let _ = child.kill();
            let _ = child.wait();
            if quit || stopped(stop, RETRY) {
                return;
            }
        }
    }
}

#[cfg(windows)]
mod imp {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::mpsc;
    use std::time::Duration;

    use anyhow::{Context, Result, bail};
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

    use super::{Device, RETRY, sample, stopped};

    /// How often to check whether the default output changed (when keeping
    /// the default output alive), so the stream follows it.
    const DEFAULT_CHECK: Duration = Duration::from_secs(5);

    pub fn devices() -> Result<Vec<Device>> {
        let host = cpal::default_host();
        Ok(host
            .output_devices()
            .context("listing audio outputs")?
            .filter_map(|d| d.name().ok())
            .map(|name| Device {
                id: name.clone(),
                label: name,
            })
            .collect())
    }

    fn find(device: &str) -> Option<cpal::Device> {
        let host = cpal::default_host();
        if device.is_empty() {
            return host.default_output_device();
        }
        host.output_devices()
            .ok()?
            .find(|d| d.name().is_ok_and(|n| n == device))
    }

    fn default_name() -> Option<String> {
        cpal::default_host().default_output_device()?.name().ok()
    }

    /// Builds a playing stream; `failed` is set when it breaks (e.g. the
    /// device was unplugged or the receiver turned off).
    fn start(dev: &cpal::Device, failed: Arc<AtomicBool>) -> Result<cpal::Stream> {
        let supported = dev
            .default_output_config()
            .context("reading the output format")?;
        let format = supported.sample_format();
        let config: cpal::StreamConfig = supported.into();
        let channels = config.channels as usize;
        // Scale the signal's timing to the device rate.
        let step = 48_000.0 / config.sample_rate.0 as f64;
        let pos = Arc::new(AtomicU64::new(0));
        let next = move || {
            let n = pos.fetch_add(1, Ordering::Relaxed);
            sample((n as f64 * step) as u64)
        };
        let on_error = move |e: cpal::StreamError| {
            eprintln!("audio keep-alive: {e}");
            failed.store(true, Ordering::Relaxed);
        };
        let stream = match format {
            cpal::SampleFormat::F32 => dev.build_output_stream(
                &config,
                move |data: &mut [f32], _| {
                    for frame in data.chunks_mut(channels) {
                        frame.fill(next() as f32 / 32768.0);
                    }
                },
                on_error,
                None,
            )?,
            cpal::SampleFormat::I16 => dev.build_output_stream(
                &config,
                move |data: &mut [i16], _| {
                    for frame in data.chunks_mut(channels) {
                        frame.fill(next());
                    }
                },
                on_error,
                None,
            )?,
            cpal::SampleFormat::I32 => dev.build_output_stream(
                &config,
                move |data: &mut [i32], _| {
                    for frame in data.chunks_mut(channels) {
                        frame.fill((next() as i32) << 16);
                    }
                },
                on_error,
                None,
            )?,
            other => bail!("unsupported output format {other:?}"),
        };
        stream.play().context("starting playback")?;
        Ok(stream)
    }

    pub fn run(device: &str, stop: &mpsc::Receiver<()>) {
        loop {
            let Some(dev) = find(device) else {
                if stopped(stop, RETRY) {
                    return;
                }
                continue;
            };
            let name = dev.name().ok();
            let failed = Arc::new(AtomicBool::new(false));
            let stream = match start(&dev, failed.clone()) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("audio keep-alive: {e:#}");
                    if stopped(stop, RETRY) {
                        return;
                    }
                    continue;
                }
            };
            // The stream plays on its own thread; watch it from here.
            loop {
                if stopped(stop, DEFAULT_CHECK) {
                    return;
                }
                if failed.load(Ordering::Relaxed) || (device.is_empty() && default_name() != name) {
                    break;
                }
            }
            drop(stream);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signal_is_tiny_and_never_silent() {
        let half = RATE as u64 / 2;
        assert_eq!(sample(0), 1);
        assert_eq!(sample(half - 1), 1);
        assert_eq!(sample(half), -1);
        assert_eq!(sample(2 * half), 1);
        assert!((0..4 * half).all(|n| sample(n).abs() == 1));
    }
}
