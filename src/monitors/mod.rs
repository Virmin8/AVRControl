//! Monitor discovery and input switching over DDC/CI (VCP feature 0x60).
//!
//! Linux talks to `ddcutil`; Windows uses the Win32 monitor configuration API.

use crate::config::MonitorId;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux as backend;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as backend;

#[derive(Debug, Clone, Default)]
pub struct MonitorInfo {
    pub id: MonitorId,
    /// GPU-side connector the monitor is plugged into, e.g. "DP-1" or "HDMI-A-1".
    pub connector: String,
    /// False for displays that don't answer DDC/CI (often AV receivers or TVs).
    pub ddc: bool,
    /// VCP 0x60 value the monitor is currently showing.
    pub current_input: Option<u8>,
    /// Input sources the monitor advertises in its capabilities string.
    pub inputs: Vec<u8>,
    /// What the desktop is driving it with (resolution, refresh, ...).
    pub mode: Option<DisplayMode>,
    /// The signal on the input it's showing, as the monitor reports it.
    pub signal: Option<Signal>,
    /// (current, max) of VCP 0x10 / 0x12.
    pub brightness: Option<(u16, u16)>,
    pub contrast: Option<(u16, u16)>,
    pub error: Option<String>,
}

/// The current video mode and output settings of a display, from the
/// compositor / OS (not the monitor itself).
#[derive(Debug, Clone, Default)]
pub struct DisplayMode {
    pub width: u32,
    pub height: u32,
    pub refresh_hz: f32,
    pub scale: Option<f32>,
    pub position: Option<(i32, i32)>,
    pub rotation: Option<String>,
    pub primary: bool,
    pub enabled: bool,
    pub hdr: Option<bool>,
    pub wide_color: Option<bool>,
    pub vrr: Option<String>,
    pub bits_per_color: Option<u32>,
    /// Physical size of the panel in millimetres.
    pub size_mm: Option<(u32, u32)>,
}

impl DisplayMode {
    pub fn resolution(&self) -> String {
        format!("{} × {}", self.width, self.height)
    }

    pub fn refresh(&self) -> String {
        if self.refresh_hz <= 0.0 {
            return "–".into();
        }
        format!("{:.2} Hz", self.refresh_hz).replace(".00 Hz", " Hz")
    }

    /// Diagonal in inches, from the physical size.
    pub fn diagonal_inches(&self) -> Option<f32> {
        let (w, h) = self.size_mm?;
        if w == 0 || h == 0 {
            return None;
        }
        Some(((w * w + h * h) as f32).sqrt() / 25.4)
    }
}

/// Timing of the signal a monitor is showing, from VCP 0xAC (horizontal
/// frequency) and 0xAE (vertical frequency). Unlike `DisplayMode`, which is
/// what this PC sends, it follows the monitor's current input.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Signal {
    pub h_freq_hz: u32,
    pub v_freq_hz: f32,
}

/// Visible line counts of common modes, for guessing the resolution.
const COMMON_HEIGHTS: [u32; 9] = [2160, 1600, 1440, 1200, 1080, 1024, 768, 720, 480];

impl Signal {
    /// From the raw (current, max) words of VCP 0xAC and 0xAE.
    pub fn from_vcp(ac: (u16, u16), ae: (u16, u16)) -> Option<Signal> {
        // 0xAC is 24 bits in Hz: the high byte comes back as "max". 0xAE is
        // in 0.01 Hz. 0 means no signal, FFFF that the monitor can't tell.
        let h_freq_hz = (((ac.1 & 0xff) as u32) << 16) | ac.0 as u32;
        if h_freq_hz == 0 || ae.0 == 0 || ae.0 == 0xffff {
            return None;
        }
        Some(Signal {
            h_freq_hz,
            v_freq_hz: ae.0 as f32 / 100.0,
        })
    }

    /// Visible lines per frame, guessed from the total: a frame carries a
    /// few percent of blanking lines on top of the visible ones. Monitors
    /// don't report the resolution itself.
    pub fn lines(&self) -> Option<u32> {
        let total = self.h_freq_hz as f32 / self.v_freq_hz;
        COMMON_HEIGHTS
            .into_iter()
            .find(|&h| (1.0..=1.10).contains(&(total / h as f32)))
    }

    /// e.g. "1440p · 163.90 Hz"
    pub fn describe(&self) -> String {
        let hz = format!("{:.2} Hz", self.v_freq_hz).replace(".00 Hz", " Hz");
        match self.lines() {
            Some(lines) => format!("{lines}p · {hz}"),
            None => hz,
        }
    }
}

/// Lists all connected displays. With `details`, also reads every monitor's
/// current input and supported inputs, which takes a second or two each.
pub fn scan(details: bool) -> anyhow::Result<Vec<MonitorInfo>> {
    let mut list = backend::scan(details)?;
    if details {
        let modes = backend::display_modes();
        for m in &mut list {
            if m.mode.is_none() {
                m.mode = modes.get(&m.connector).cloned();
            }
        }
    }
    Ok(list)
}

/// Monitors don't answer DDC/CI for a few seconds while switching input, so a
/// scan in that window would wrongly mark them as not supporting it. Keeps
/// what an earlier scan learned about such a monitor instead.
pub fn carry_over(list: &mut [MonitorInfo], previous: &[MonitorInfo]) {
    for m in list.iter_mut().filter(|m| !m.ddc) {
        if let Some(prev) = previous.iter().find(|p| p.ddc && p.id == m.id) {
            m.ddc = true;
            if m.inputs.is_empty() {
                m.inputs = prev.inputs.clone();
            }
        }
    }
}

/// Inputs each monitor advertised in its capabilities string. That never
/// changes, and asking for it is the slowest DDC/CI request there is.
static INPUT_CAPS: std::sync::Mutex<Option<std::collections::HashMap<MonitorId, Vec<u8>>>> =
    std::sync::Mutex::new(None);

/// The monitor's advertised inputs, read with `read` only the first time.
fn cached_inputs<E>(
    id: &MonitorId,
    read: impl FnOnce() -> Result<Vec<u8>, E>,
) -> Result<Vec<u8>, E> {
    if let Some(inputs) = INPUT_CAPS.lock().unwrap().as_ref().and_then(|m| m.get(id)) {
        return Ok(inputs.clone());
    }
    let inputs = read()?;
    if !inputs.is_empty() {
        INPUT_CAPS
            .lock()
            .unwrap()
            .get_or_insert_default()
            .insert(id.clone(), inputs.clone());
    }
    Ok(inputs)
}

/// (monitor, connector, current input) for each DDC/CI monitor.
pub fn current_inputs() -> anyhow::Result<Vec<(MonitorId, String, Option<u8>)>> {
    backend::current_inputs()
}

/// Tooltip lines like "DELL S3220DGF (DP-1): DisplayPort-1".
pub fn summary() -> String {
    match current_inputs() {
        Ok(list) if list.is_empty() => String::new(),
        Ok(list) => list
            .iter()
            .map(|(id, connector, input)| {
                let input = input.map(input_name).unwrap_or_else(|| "no answer".into());
                format!("{} ({connector}): {input}", id.model)
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Err(e) => format!("Monitors: {e}"),
    }
}

pub fn set_input(id: &MonitorId, input: u8) -> anyhow::Result<()> {
    backend::set_input(id, input)
}

/// Standard MCCS names for VCP 0x60 values.
pub fn input_name(code: u8) -> String {
    let name = match code {
        0x01 => "VGA-1",
        0x02 => "VGA-2",
        0x03 => "DVI-1",
        0x04 => "DVI-2",
        0x05 => "Composite-1",
        0x06 => "Composite-2",
        0x07 => "S-Video-1",
        0x08 => "S-Video-2",
        0x09 => "Tuner-1",
        0x0a => "Tuner-2",
        0x0b => "Tuner-3",
        0x0c => "Component-1",
        0x0d => "Component-2",
        0x0e => "Component-3",
        0x0f => "DisplayPort-1",
        0x10 => "DisplayPort-2",
        0x11 => "HDMI-1",
        0x12 => "HDMI-2",
        0x1b => "USB-C",
        _ => return format!("Input 0x{code:02x}"),
    };
    name.to_string()
}

/// Inputs to offer for a monitor: what it advertises, or the common ones if
/// its capabilities string couldn't be read.
pub fn selectable_inputs(info: Option<&MonitorInfo>) -> Vec<u8> {
    match info {
        Some(m) if !m.inputs.is_empty() => m.inputs.clone(),
        _ => vec![0x0f, 0x10, 0x11, 0x12, 0x03, 0x01, 0x1b],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signal_from_vcp() {
        // Read from a Dell S3220DGF (1440p @ 163.9 Hz) and S2721HN (1080p @ 75 Hz).
        let s = Signal::from_vcp((47192, 3), (16390, 65535)).unwrap();
        assert_eq!(s.h_freq_hz, 243_800);
        assert_eq!(s.describe(), "1440p · 163.90 Hz");
        let s = Signal::from_vcp((18364, 1), (7500, 65535)).unwrap();
        assert_eq!(s.describe(), "1080p · 75 Hz");
        // 4K60 (CTA timing, 2250 lines) and 720p60 (750 lines).
        assert_eq!(
            Signal::from_vcp((3928, 2), (6000, 0)).unwrap().lines(),
            Some(2160)
        );
        assert_eq!(
            Signal::from_vcp((45000, 0), (6000, 0)).unwrap().lines(),
            Some(720)
        );
        assert_eq!(Signal::from_vcp((0, 0), (0, 0)), None);
        assert_eq!(Signal::from_vcp((18364, 1), (0xffff, 0xffff)), None);
    }
}
