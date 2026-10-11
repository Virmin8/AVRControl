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
