//! Win32 backend: Monitor Configuration API (dxva2) for DDC/CI, and the
//! CCD API (QueryDisplayConfig) for names and connector types.

use std::collections::HashMap;
use std::ptr;
use std::thread::sleep;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use windows_sys::Win32::Devices::Display::*;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::core::BOOL;

use super::MonitorInfo;
use crate::config::MonitorId;

fn wide_to_string(w: &[u16]) -> String {
    let end = w.iter().position(|&c| c == 0).unwrap_or(w.len());
    String::from_utf16_lossy(&w[..end])
}

/// Info about one active display target, keyed by its GDI source name.
#[derive(Clone)]
struct Target {
    friendly_name: String,
    device_path: String,
    edid_mfg: String,
    connector: String,
}

fn tech_name(tech: i32) -> &'static str {
    match tech {
        DISPLAYCONFIG_OUTPUT_TECHNOLOGY_HD15 => "VGA",
        DISPLAYCONFIG_OUTPUT_TECHNOLOGY_DVI => "DVI",
        DISPLAYCONFIG_OUTPUT_TECHNOLOGY_HDMI => "HDMI",
        DISPLAYCONFIG_OUTPUT_TECHNOLOGY_DISPLAYPORT_EXTERNAL => "DisplayPort",
        DISPLAYCONFIG_OUTPUT_TECHNOLOGY_DISPLAYPORT_EMBEDDED => "eDP",
        DISPLAYCONFIG_OUTPUT_TECHNOLOGY_DISPLAYPORT_USB_TUNNEL => "USB-C (DP)",
        DISPLAYCONFIG_OUTPUT_TECHNOLOGY_INTERNAL | DISPLAYCONFIG_OUTPUT_TECHNOLOGY_LVDS => {
            "Internal"
        }
        DISPLAYCONFIG_OUTPUT_TECHNOLOGY_MIRACAST => "Miracast",
        _ => "Other",
    }
}

/// EDID packs the PnP manufacturer id as three 5-bit letters, big-endian.
fn decode_pnp(id: u16) -> String {
    let id = id.swap_bytes();
    [(id >> 10) & 0x1f, (id >> 5) & 0x1f, id & 0x1f]
        .iter()
        .map(|&c| (b'A' - 1 + c as u8) as char)
        .collect()
}

fn targets() -> HashMap<String, Vec<Target>> {
    let mut map: HashMap<String, Vec<Target>> = HashMap::new();
    unsafe {
        let (mut npaths, mut nmodes) = (0u32, 0u32);
        if GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut npaths, &mut nmodes)
            != ERROR_SUCCESS
        {
            return map;
        }
        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); npaths as usize];
        let mut modes: Vec<DISPLAYCONFIG_MODE_INFO> = vec![std::mem::zeroed(); nmodes as usize];
        if QueryDisplayConfig(
            QDC_ONLY_ACTIVE_PATHS,
            &mut npaths,
            paths.as_mut_ptr(),
            &mut nmodes,
            modes.as_mut_ptr(),
            ptr::null_mut(),
        ) != ERROR_SUCCESS
        {
            return map;
        }
        paths.truncate(npaths as usize);

        for path in &paths {
            let mut source = DISPLAYCONFIG_SOURCE_DEVICE_NAME::default();
            source.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
            source.header.size = std::mem::size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32;
            source.header.adapterId = path.sourceInfo.adapterId;
            source.header.id = path.sourceInfo.id;
            if DisplayConfigGetDeviceInfo(&mut source.header) != 0 {
                continue;
            }

            let mut target = DISPLAYCONFIG_TARGET_DEVICE_NAME::default();
            target.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME;
            target.header.size = std::mem::size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32;
            target.header.adapterId = path.targetInfo.adapterId;
            target.header.id = path.targetInfo.id;
            if DisplayConfigGetDeviceInfo(&mut target.header) != 0 {
                continue;
            }

            map.entry(wide_to_string(&source.viewGdiDeviceName))
                .or_default()
                .push(Target {
                    friendly_name: wide_to_string(&target.monitorFriendlyDeviceName),
                    device_path: wide_to_string(&target.monitorDevicePath),
                    edid_mfg: decode_pnp(target.edidManufactureId),
                    connector: format!(
                        "{} {}",
                        tech_name(target.outputTechnology),
                        target.connectorInstance
                    ),
                });
        }
    }
    map
}

struct Physical {
    handle: HANDLE,
    id: MonitorId,
    connector: String,
    mode: Option<super::DisplayMode>,
}

/// Current mode of a GDI display such as "\\.\DISPLAY1".
fn gdi_mode(gdi: &str, info: &MONITORINFOEXW) -> Option<super::DisplayMode> {
    let name: Vec<u16> = gdi.encode_utf16().chain(std::iter::once(0)).collect();
    let mut dm: DEVMODEW = unsafe { std::mem::zeroed() };
    dm.dmSize = std::mem::size_of::<DEVMODEW>() as u16;
    if unsafe { EnumDisplaySettingsW(name.as_ptr(), ENUM_CURRENT_SETTINGS, &mut dm) } == 0 {
        return None;
    }
    let rc = info.monitorInfo.rcMonitor;
    Some(super::DisplayMode {
        width: dm.dmPelsWidth,
        height: dm.dmPelsHeight,
        refresh_hz: dm.dmDisplayFrequency as f32,
        position: Some((rc.left, rc.top)),
        // MONITORINFOF_PRIMARY
        primary: info.monitorInfo.dwFlags & 1 != 0,
        enabled: true,
        ..Default::default()
    })
}

/// Owns the physical monitor handles for one enumeration pass.
struct Physicals(Vec<PHYSICAL_MONITOR>, Vec<Physical>);

impl Drop for Physicals {
    fn drop(&mut self) {
        if !self.0.is_empty() {
            unsafe {
                DestroyPhysicalMonitors(self.0.len() as u32, self.0.as_ptr());
            }
        }
    }
}

unsafe extern "system" fn enum_proc(hmon: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
    let list = unsafe { &mut *(data as *mut Vec<HMONITOR>) };
    list.push(hmon);
    1
}

fn physicals() -> Physicals {
    let targets = targets();
    let mut hmons: Vec<HMONITOR> = Vec::new();
    let mut raw = Vec::new();
    let mut out = Vec::new();
    unsafe {
        EnumDisplayMonitors(
            ptr::null_mut(),
            ptr::null(),
            Some(enum_proc),
            &mut hmons as *mut _ as LPARAM,
        );
        for hmon in hmons {
            let mut info = MONITORINFOEXW::default();
            info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
            if GetMonitorInfoW(hmon, &mut info as *mut _ as *mut MONITORINFO) == 0 {
                continue;
            }
            let gdi = wide_to_string(&info.szDevice);
            let mut count = 0u32;
            if GetNumberOfPhysicalMonitorsFromHMONITOR(hmon, &mut count) == 0 || count == 0 {
                continue;
            }
            let mut phys = vec![PHYSICAL_MONITOR::default(); count as usize];
            if GetPhysicalMonitorsFromHMONITOR(hmon, count, phys.as_mut_ptr()) == 0 {
                continue;
            }
            let tlist = targets.get(&gdi).cloned().unwrap_or_default();
            let mode = gdi_mode(&gdi, &info);
            for (i, p) in phys.iter().enumerate() {
                let t = tlist.get(i);
                let model = t
                    .map(|t| t.friendly_name.clone())
                    .filter(|n| !n.is_empty())
                    .unwrap_or_else(|| {
                        // PHYSICAL_MONITOR is packed: copy the field out before borrowing it.
                        let desc = p.szPhysicalMonitorDescription;
                        wide_to_string(&desc)
                    });
                // "\\?\DISPLAY#DELD0F4#5&2a3b&0&UID4352#{guid}" -> "DELD0F4#5&2a3b&0&UID4352"
                let serial = t
                    .map(|t| {
                        let parts: Vec<&str> = t.device_path.split('#').collect();
                        parts.get(1..3).map(|p| p.join("#")).unwrap_or_default()
                    })
                    .unwrap_or_default();
                out.push(Physical {
                    handle: p.hPhysicalMonitor,
                    id: MonitorId {
                        mfg: t.map(|t| t.edid_mfg.clone()).unwrap_or_default(),
                        model,
                        serial,
                    },
                    connector: t
                        .map(|t| t.connector.clone())
                        .unwrap_or_else(|| gdi.clone()),
                    mode: mode.clone(),
                });
            }
            raw.extend(phys);
        }
    }
    Physicals(raw, out)
}

/// Reads a VCP feature as (current, max). Monitors now and then miss a
/// request, so a failed read is retried once.
fn read_vcp(handle: HANDLE, code: u8) -> Option<(u32, u32)> {
    for attempt in 0..2 {
        if attempt > 0 {
            sleep(Duration::from_millis(100));
        }
        let (mut kind, mut cur, mut max) = (0, 0u32, 0u32);
        let ok =
            unsafe { GetVCPFeatureAndVCPFeatureReply(handle, code, &mut kind, &mut cur, &mut max) };
        if ok != 0 {
            return Some((cur, max));
        }
    }
    None
}

fn vcp_continuous(handle: HANDLE, code: u8) -> Option<(u16, u16)> {
    read_vcp(handle, code).map(|(cur, max)| (cur as u16, max as u16))
}

fn signal(handle: HANDLE) -> Option<super::Signal> {
    super::Signal::from_vcp(vcp_continuous(handle, 0xac)?, vcp_continuous(handle, 0xae)?)
}

/// Modes are filled in by scan() directly on Windows.
pub fn display_modes() -> HashMap<String, super::DisplayMode> {
    HashMap::new()
}

fn current_input(handle: HANDLE) -> Option<u8> {
    read_vcp(handle, 0x60).map(|(cur, _)| (cur & 0xff) as u8)
}

/// Extracts the values listed for VCP 60 from an MCCS capabilities string,
/// e.g. "...vcp(02 04 60(0F 11 12) 62)...".
fn parse_caps_inputs(caps: &str) -> Vec<u8> {
    let upper = caps.to_ascii_uppercase();
    let Some(vcp) = upper.find("VCP(") else {
        return Vec::new();
    };
    let rest = &upper[vcp..];
    let Some(pos) = rest.find(" 60(").or_else(|| rest.find("(60(")) else {
        return Vec::new();
    };
    let list = &rest[pos + 4..];
    let end = list.find(')').unwrap_or(list.len());
    list[..end]
        .split_whitespace()
        .filter_map(|t| u8::from_str_radix(t, 16).ok())
        .collect()
}

fn capability_inputs(handle: HANDLE) -> Option<Vec<u8>> {
    unsafe {
        let mut len = 0u32;
        if GetCapabilitiesStringLength(handle, &mut len) == 0 || len == 0 {
            return None;
        }
        let mut buf = vec![0u8; len as usize];
        if CapabilitiesRequestAndCapabilitiesReply(handle, buf.as_mut_ptr(), len) == 0 {
            return None;
        }
        let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        Some(parse_caps_inputs(&String::from_utf8_lossy(&buf[..end])))
    }
}

pub fn scan(details: bool) -> Result<Vec<MonitorInfo>> {
    let phys = physicals();
    Ok(phys
        .1
        .iter()
        .map(|p| {
            let current = if details {
                current_input(p.handle)
            } else {
                None
            };
            let inputs = if details {
                capability_inputs(p.handle)
            } else {
                None
            };
            MonitorInfo {
                id: p.id.clone(),
                connector: p.connector.clone(),
                ddc: !details || current.is_some() || inputs.is_some(),
                current_input: current,
                inputs: inputs.unwrap_or_default(),
                mode: p.mode.clone(),
                signal: details.then(|| signal(p.handle)).flatten(),
                brightness: details.then(|| vcp_continuous(p.handle, 0x10)).flatten(),
                contrast: details.then(|| vcp_continuous(p.handle, 0x12)).flatten(),
                error: None,
            }
        })
        .collect())
}

/// Current input of every monitor (no capabilities request, so it's fast).
pub fn current_inputs() -> Result<Vec<(MonitorId, String, Option<u8>)>> {
    let phys = physicals();
    Ok(phys
        .1
        .iter()
        .map(|p| (p.id.clone(), p.connector.clone(), current_input(p.handle)))
        .collect())
}

/// How long to wait for a monitor to report the new input.
const SWITCH_TIMEOUT: Duration = Duration::from_secs(8);
/// A monitor still reporting another input this long after a write has
/// dropped it.
const RESEND_AFTER: Duration = Duration::from_millis(1500);
const MAX_SENDS: u32 = 3;

/// Switches input and waits until the monitor reports it. Monitors stop
/// answering DDC/CI for a few seconds while they switch, and acknowledge
/// writes that arrive meanwhile without acting on them, so a single write
/// isn't enough: read the input back and resend until it sticks.
pub fn set_input(id: &MonitorId, input: u8) -> Result<()> {
    let phys = physicals();
    let Some(p) = phys.1.iter().find(|p| &p.id == id) else {
        bail!("monitor {} not found", id.label());
    };
    // Monitors that can't report their input can only be switched blind.
    let verify = current_input(p.handle).is_some();
    let start = Instant::now();
    let mut sends = 0;
    let mut last_send = start;
    loop {
        if sends == 0 || last_send.elapsed() >= RESEND_AFTER {
            if sends == MAX_SENDS {
                bail!(
                    "monitor {} ignored the switch to {}",
                    id.label(),
                    super::input_name(input)
                );
            }
            if unsafe { SetVCPFeature(p.handle, 0x60, input as u32) } == 0 && !verify {
                bail!("monitor {} rejected the input change", id.label());
            }
            sends += 1;
            last_send = Instant::now();
        }
        if !verify || start.elapsed() > SWITCH_TIMEOUT {
            return Ok(());
        }
        sleep(Duration::from_millis(250));
        match current_input(p.handle) {
            Some(cur) if cur == input => return Ok(()),
            // Still on the old input: keep the resend timer running.
            Some(_) => {}
            // Busy switching: give it time before deciding to resend.
            None => last_send = Instant::now(),
        }
    }
}
