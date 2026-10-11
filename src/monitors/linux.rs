//! ddcutil backend. ddcutil only probes the DDC channels of real display
//! connectors, unlike raw /dev/i2c-* scanning which also hits GPU SMU and
//! motherboard SMBus devices.

use std::collections::HashMap;
use std::process::Command;
use std::sync::Mutex;
use std::thread;

use anyhow::{Context, Result, bail};

use super::MonitorInfo;
use crate::config::MonitorId;

/// Monitor -> I2C bus, so switching inputs doesn't need a full re-detect.
static BUS_CACHE: Mutex<Option<HashMap<MonitorId, (u32, String)>>> = Mutex::new(None);

fn ddcutil(args: &[&str]) -> Result<String> {
    let out = Command::new("ddcutil")
        .args(args)
        .output()
        .context("could not run ddcutil (is it installed?)")?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let msg = if stderr.trim().is_empty() {
            stdout.trim()
        } else {
            stderr.trim()
        };
        bail!("ddcutil {}: {}", args.join(" "), msg);
    }
    Ok(stdout)
}

struct Detected {
    bus: Option<u32>,
    connector: String,
    id: MonitorId,
    ddc: bool,
}

/// Parses `ddcutil detect --terse`:
///
/// ```text
/// Display 1
///    I2C bus:          /dev/i2c-6
///    DRM connector:    card1-DP-1
///    Monitor:          DEL:DELL S3220DGF:93SJM73
/// Invalid display
///    ...
/// ```
fn parse_detect(text: &str) -> Vec<Detected> {
    let mut out: Vec<Detected> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if !line.starts_with(' ') && !trimmed.is_empty() {
            let ddc = trimmed.starts_with("Display");
            if ddc
                || trimmed.starts_with("Invalid display")
                || trimmed.starts_with("Phantom display")
            {
                out.push(Detected {
                    bus: None,
                    connector: String::new(),
                    id: MonitorId::default(),
                    ddc,
                });
            }
            continue;
        }
        let Some(cur) = out.last_mut() else { continue };
        let Some((key, value)) = trimmed.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "I2C bus" => cur.bus = value.rsplit('-').next().and_then(|n| n.parse().ok()),
            "DRM connector" | "DRM_connector" => {
                // "card1-DP-1" -> "DP-1"
                cur.connector = value
                    .split_once('-')
                    .map(|(_, c)| c)
                    .unwrap_or(value)
                    .to_string();
            }
            "Monitor" => {
                let mut parts = value.splitn(3, ':');
                cur.id.mfg = parts.next().unwrap_or("").to_string();
                cur.id.model = parts.next().unwrap_or("").to_string();
                cur.id.serial = parts.next().unwrap_or("").to_string();
            }
            _ => {}
        }
    }
    out.retain(|d| !d.id.model.is_empty() || d.bus.is_some());
    out
}

/// Pulls the VCP 0x60 values out of `ddcutil capabilities`:
///
/// ```text
///    Feature: 60 (Input Source)
///       Values:
///          0f: DisplayPort-1
///          11: HDMI-1
///    Feature: 62 (...)
/// ```
fn parse_input_caps(text: &str) -> Vec<u8> {
    let mut inputs = Vec::new();
    let mut in_feature = false;
    for line in text.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("Feature:") {
            in_feature = rest.trim().starts_with("60");
            continue;
        }
        if !in_feature {
            continue;
        }
        if let Some((code, _)) = t.split_once(':')
            && let Ok(v) = u8::from_str_radix(code.trim(), 16)
        {
            inputs.push(v);
        }
    }
    inputs
}

/// Parses `getvcp 60 --terse` ("VCP 60 SNC x0f"). Some monitors put junk in
/// the high byte, so only the low byte is used.
fn parse_getvcp(text: &str) -> Option<u8> {
    let token = text.split_whitespace().last()?;
    let hex = token.trim_start_matches('x');
    u16::from_str_radix(hex, 16).ok().map(|v| (v & 0xff) as u8)
}

fn detect() -> Result<Vec<Detected>> {
    // ddcutil exits non-zero when it finds displays without DDC/CI, but still
    // prints the list, so accept output regardless of the exit code.
    let out = Command::new("ddcutil")
        .args(["detect", "--terse"])
        .output()
        .context("could not run ddcutil (is it installed?)")?;
    let found = parse_detect(&String::from_utf8_lossy(&out.stdout));
    if found.is_empty() && !out.status.success() {
        bail!(
            "ddcutil detect: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let mut cache = BUS_CACHE.lock().unwrap();
    let map = cache.get_or_insert_with(HashMap::new);
    map.clear();
    for d in found.iter().filter(|d| d.ddc) {
        if let Some(bus) = d.bus {
            map.insert(d.id.clone(), (bus, d.connector.clone()));
        }
    }
    Ok(found)
}

pub fn scan(details: bool) -> Result<Vec<MonitorInfo>> {
    let found = detect()?;
    let handles: Vec<_> = found
        .into_iter()
        .map(|d| {
            thread::spawn(move || {
                let mut info = MonitorInfo {
                    id: d.id,
                    connector: d.connector,
                    ddc: d.ddc,
                    ..Default::default()
                };
                if details
                    && d.ddc
                    && let Some(bus) = d.bus
                {
                    let bus = bus.to_string();
                    match ddcutil(&["--bus", &bus, "getvcp", "60", "--terse"]) {
                        Ok(out) => info.current_input = parse_getvcp(&out),
                        Err(e) => info.error = Some(e.to_string()),
                    }
                    match ddcutil(&["--bus", &bus, "capabilities"]) {
                        Ok(out) => info.inputs = parse_input_caps(&out),
                        Err(e) => {
                            info.error.get_or_insert(e.to_string());
                        }
                    }
                    if let Ok(out) = ddcutil(&["--bus", &bus, "getvcp", "10", "12", "--terse"]) {
                        let values = parse_continuous(&out);
                        info.brightness = values.get(&0x10).copied();
                        info.contrast = values.get(&0x12).copied();
                    }
                }
                info
            })
        })
        .collect();
    Ok(handles.into_iter().filter_map(|h| h.join().ok()).collect())
}

/// Parses continuous values from `getvcp ... --terse` ("VCP 10 C 75 100").
fn parse_continuous(text: &str) -> HashMap<u8, (u16, u16)> {
    text.lines()
        .filter_map(|line| {
            let t: Vec<&str> = line.split_whitespace().collect();
            match t.as_slice() {
                ["VCP", code, "C", cur, max, ..] => Some((
                    u8::from_str_radix(code, 16).ok()?,
                    (cur.parse().ok()?, max.parse().ok()?),
                )),
                _ => None,
            }
        })
        .collect()
}

/// Current mode of every output, keyed by connector ("DP-1"). Uses
/// kscreen-doctor (KDE Plasma); empty elsewhere.
pub fn display_modes() -> HashMap<String, super::DisplayMode> {
    let Ok(out) = Command::new("kscreen-doctor").arg("-j").output() else {
        return HashMap::new();
    };
    parse_kscreen(&String::from_utf8_lossy(&out.stdout))
}

fn parse_kscreen(json: &str) -> HashMap<String, super::DisplayMode> {
    use serde_json::Value;
    let Ok(root) = serde_json::from_str::<Value>(json) else {
        return HashMap::new();
    };
    let mut map = HashMap::new();
    for o in root["outputs"].as_array().into_iter().flatten() {
        if o["connected"] != Value::Bool(true) {
            continue;
        }
        let Some(name) = o["name"].as_str() else {
            continue;
        };
        let current = o["currentModeId"].as_str().unwrap_or("");
        let mode = o["modes"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|m| m["id"].as_str() == Some(current));
        let num = |v: &Value| v.as_f64();
        let size = |v: &Value| Some((num(&v["width"])? as u32, num(&v["height"])? as u32));
        let (width, height) = mode.and_then(|m| size(&m["size"])).unwrap_or_default();
        map.insert(
            name.to_string(),
            super::DisplayMode {
                width,
                height,
                refresh_hz: mode.and_then(|m| num(&m["refreshRate"])).unwrap_or(0.0) as f32,
                scale: num(&o["scale"]).map(|s| s as f32),
                position: Some((
                    num(&o["pos"]["x"]).unwrap_or(0.0) as i32,
                    num(&o["pos"]["y"]).unwrap_or(0.0) as i32,
                )),
                rotation: o["rotation"].as_u64().map(|r| {
                    match r {
                        1 => "Normal",
                        2 => "90°",
                        4 => "180°",
                        8 => "270°",
                        _ => "Flipped",
                    }
                    .to_string()
                }),
                primary: o["priority"].as_u64() == Some(1),
                enabled: o["enabled"].as_bool().unwrap_or(true),
                hdr: o["hdr"].as_bool(),
                wide_color: o["wcg"].as_bool(),
                vrr: o["vrrPolicy"].as_u64().map(|v| {
                    match v {
                        0 => "Never",
                        1 => "Always",
                        _ => "Automatic",
                    }
                    .to_string()
                }),
                bits_per_color: o["maxBpc"].as_u64().filter(|&b| b > 0).map(|b| b as u32),
                size_mm: size(&o["sizeMM"]),
            },
        );
    }
    map
}

fn cached_bus(id: &MonitorId) -> Option<u32> {
    BUS_CACHE
        .lock()
        .unwrap()
        .as_ref()?
        .get(id)
        .map(|(bus, _)| *bus)
}

/// Current input of every DDC/CI monitor, cheap enough to poll: reuses the
/// cached bus list and only reads VCP 0x60.
pub fn current_inputs() -> Result<Vec<(MonitorId, String, Option<u8>)>> {
    let cached = BUS_CACHE.lock().unwrap().clone();
    let map = match cached {
        Some(m) if !m.is_empty() => m,
        _ => {
            detect()?;
            BUS_CACHE.lock().unwrap().clone().unwrap_or_default()
        }
    };
    let handles: Vec<_> = map
        .into_iter()
        .map(|(id, (bus, connector))| {
            thread::spawn(move || {
                let input = ddcutil(&["--bus", &bus.to_string(), "getvcp", "60", "--terse"])
                    .ok()
                    .and_then(|out| parse_getvcp(&out));
                (id, connector, input)
            })
        })
        .collect();
    let mut list: Vec<_> = handles.into_iter().filter_map(|h| h.join().ok()).collect();
    list.sort_by(|a, b| a.1.cmp(&b.1));
    Ok(list)
}

pub fn set_input(id: &MonitorId, input: u8) -> Result<()> {
    let value = format!("0x{input:02x}");
    // --noverify: the monitor may already be switching away (and stop
    // answering on this link) by the time ddcutil would read the value back.
    let try_bus = |bus: u32| {
        ddcutil(&[
            "--bus",
            &bus.to_string(),
            "--noverify",
            "setvcp",
            "60",
            &value,
        ])
    };

    if let Some(bus) = cached_bus(id)
        && try_bus(bus).is_ok()
    {
        return Ok(());
    }
    // Unknown, or buses got renumbered (hotplug, driver reload): re-detect.
    detect()?;
    match cached_bus(id) {
        Some(bus) => try_bus(bus).map(|_| ()),
        None => bail!("monitor {} not found", id.label()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_output() {
        let text = "Invalid display\n   I2C bus:          /dev/i2c-5\n   DRM connector:    card1-HDMI-A-1\n   drm_connector_id: 472\n   Monitor:          ONK:DTR-50.7:\n\nDisplay 1\n   I2C bus:          /dev/i2c-6\n   DRM connector:    card1-DP-1\n   drm_connector_id: 449\n   Monitor:          DEL:DELL S3220DGF:93SJM73\n";
        let d = parse_detect(text);
        assert_eq!(d.len(), 2);
        assert!(!d[0].ddc);
        assert_eq!(d[0].connector, "HDMI-A-1");
        assert_eq!(d[0].id.model, "DTR-50.7");
        assert_eq!(d[1].bus, Some(6));
        assert_eq!(d[1].id.serial, "93SJM73");
        assert_eq!(d[1].connector, "DP-1");
    }

    #[test]
    fn kscreen_and_continuous() {
        let json = r#"{"outputs":[{"name":"DP-1","connected":true,"enabled":true,"currentModeId":"68",
            "modes":[{"id":"68","refreshRate":164.05599975585938,"size":{"width":2560,"height":1440}}],
            "scale":1,"pos":{"x":0,"y":0},"rotation":1,"priority":1,"hdr":false,"wcg":false,
            "vrrPolicy":2,"maxBpc":10,"sizeMM":{"width":697,"height":392}}]}"#;
        let m = &parse_kscreen(json)["DP-1"];
        assert_eq!((m.width, m.height), (2560, 1440));
        assert!(m.primary);
        assert_eq!(m.refresh(), "164.06 Hz");
        assert_eq!(m.vrr.as_deref(), Some("Automatic"));
        assert!((m.diagonal_inches().unwrap() - 31.5).abs() < 0.1);
        let v = parse_continuous("VCP 10 C 75 100\nVCP 12 C 50 100\n");
        assert_eq!(v[&0x10], (75, 100));
        assert_eq!(v[&0x12], (50, 100));
    }

    #[test]
    fn caps_and_getvcp() {
        let caps = "   Feature: 14 (Color preset)\n      Values:\n         05: 6500 K\n   Feature: 60 (Input Source)\n      Values:\n         0f: DisplayPort-1\n         11: HDMI-1\n         12: HDMI-2\n   Feature: 62 (Audio speaker volume)\n";
        assert_eq!(parse_input_caps(caps), vec![0x0f, 0x11, 0x12]);
        assert_eq!(parse_getvcp("VCP 60 SNC x0f\n"), Some(0x0f));
    }
}
