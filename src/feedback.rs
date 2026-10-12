//! User feedback: desktop notifications and the volume on-screen display.

use std::io::Write;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::time::{Duration, Instant};

use notify_rust::Notification;

use crate::config::{APP_ID, APP_NAME, Config};

pub struct Feedback {
    notifications: bool,
    volume_osd: bool,
    max_volume: u8,
    half_steps: bool,
    #[cfg(target_os = "linux")]
    last_id: Option<u32>,
    osd_child: Option<(Child, ChildStdin)>,
    /// When the overlay was last sent a value.
    osd_written: Instant,
}

/// Stop reusing an overlay this long after its last value: it closes itself
/// at `osd::HIDE_AFTER`, and a value sent while it closes would be lost.
const OSD_REUSE: Duration = Duration::from_millis(1500);

fn notification_icon() -> String {
    #[cfg(target_os = "linux")]
    {
        let installed = [
            "/usr/share/icons/hicolor/256x256/apps",
            "/usr/local/share/icons/hicolor/256x256/apps",
        ]
        .iter()
        .any(|dir| {
            std::path::Path::new(dir)
                .join(format!("{APP_ID}.png"))
                .exists()
        });
        if installed {
            return APP_ID.to_string();
        }
    }
    let _ = APP_ID;
    "audio-card".to_string()
}

impl Feedback {
    pub fn new(cfg: &Config) -> Self {
        let mut f = Self {
            notifications: true,
            volume_osd: true,
            max_volume: 100,
            half_steps: false,
            #[cfg(target_os = "linux")]
            last_id: None,
            osd_child: None,
            osd_written: Instant::now(),
        };
        f.apply_config(cfg);
        f
    }

    pub fn apply_config(&mut self, cfg: &Config) {
        self.notifications = cfg.general.notifications;
        self.volume_osd = cfg.general.volume_osd;
        self.max_volume = cfg.receiver.max_volume.max(1);
        self.half_steps = cfg.receiver.volume_half_steps;
    }

    /// Informational message; suppressed when notifications are off.
    pub fn info(&mut self, msg: &str) {
        if self.notifications {
            self.show(msg);
        }
    }

    /// Errors are always shown, otherwise a failing hotkey is silent.
    pub fn error(&mut self, msg: &str) {
        eprintln!("error: {msg}");
        self.show(msg);
    }

    fn show(&mut self, msg: &str) {
        let mut n = Notification::new();
        n.summary(APP_NAME)
            .body(msg)
            .appname(APP_NAME)
            .icon(&notification_icon())
            .timeout(2500);
        // Replace the previous bubble instead of stacking a new one per keypress.
        #[cfg(target_os = "linux")]
        {
            if let Some(id) = self.last_id {
                n.id(id);
            }
            match n.show() {
                Ok(handle) => self.last_id = Some(handle.id()),
                Err(e) => eprintln!("notification failed: {e}"),
            }
        }
        #[cfg(not(target_os = "linux"))]
        if let Err(e) = n.show() {
            eprintln!("notification failed: {e}");
        }
    }

    /// Shows the volume indicator for a raw `MVL` value.
    pub fn volume(&mut self, raw: u8, muted: bool) {
        if !self.volume_osd {
            return;
        }
        let (display, text) = if self.half_steps {
            let v = raw as f32 / 2.0;
            (v, format!("{v:.1}"))
        } else {
            (raw as f32, raw.to_string())
        };
        let text = if muted { "muted".to_string() } else { text };
        let percent = if muted {
            0
        } else {
            (display / self.max_volume as f32 * 100.0)
                .round()
                .clamp(0.0, 100.0) as i32
        };
        self.overlay_volume(percent, &text);
    }

    /// Our own overlay window, run as a child process (`avrcontrol osd`)
    /// that reads "<percent> <text>" lines from stdin and closes itself after
    /// a short idle period.
    fn overlay_volume(&mut self, percent: i32, text: &str) {
        let line = format!("{percent} {text}\n");
        let fresh = self.osd_written.elapsed() < OSD_REUSE;
        self.osd_written = Instant::now();
        if let Some((child, stdin)) = &mut self.osd_child {
            let alive = matches!(child.try_wait(), Ok(None));
            if alive
                && fresh
                && stdin
                    .write_all(line.as_bytes())
                    .and_then(|_| stdin.flush())
                    .is_ok()
            {
                return;
            }
        }
        // An overlay that's about to close (or already has) is replaced, not
        // reused.
        if let Some((mut child, _)) = self.osd_child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let Ok(exe) = std::env::current_exe() else {
            return;
        };
        match Command::new(exe).arg("osd").stdin(Stdio::piped()).spawn() {
            Ok(mut child) => {
                let mut stdin = child.stdin.take().unwrap();
                let _ = stdin.write_all(line.as_bytes());
                let _ = stdin.flush();
                self.osd_child = Some((child, stdin));
            }
            Err(e) => eprintln!("could not start volume overlay: {e}"),
        }
    }
}
