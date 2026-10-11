// No console window when launched from Explorer / autostart on Windows.
#![cfg_attr(windows, windows_subsystem = "windows")]

mod autostart;
mod config;
mod controller;
mod daemon;
mod eiscp;
mod feedback;
mod gui;
mod hotkey;
mod ipc;
mod models;
mod monitors;
mod platform;
mod repeat;

use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{Result, bail};

use config::Config;

pub struct IconRgba {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// The app icon as raw RGBA, decoded once.
pub fn icon_rgba() -> &'static IconRgba {
    static ICON: OnceLock<IconRgba> = OnceLock::new();
    ICON.get_or_init(|| {
        let img = image::load_from_memory_with_format(
            include_bytes!("../assets/icon-64.png"),
            image::ImageFormat::Png,
        )
        .expect("embedded icon is a valid PNG")
        .into_rgba8();
        IconRgba {
            width: img.width(),
            height: img.height(),
            data: img.into_raw(),
        }
    })
}

const USAGE: &str = "\
AVR Control - Onkyo receiver and monitor input hotkeys

Usage: avrcontrol [COMMAND]

Commands:
  (none)            Start in the background, or open settings if already running
  daemon            Start the background service (tray icon + hotkeys)
  settings [TAB]    Open the settings window (receiver|monitors|shortcuts|general)
  trigger <ID>      Run a shortcut in the running instance
  list              List configured shortcuts
  monitors          List connected monitors and their inputs
  receiver          Show receiver status
  discover          Search the network for receivers
  features          Detect which commands the receiver supports
  reload            Make the running instance re-read its config
  quit              Stop the running instance
";

/// On Windows the binary is a GUI-subsystem app; reattach to the parent
/// console so CLI output is visible when run from a terminal.
fn attach_console() {
    #[cfg(windows)]
    unsafe {
        windows_sys::Win32::System::Console::AttachConsole(
            windows_sys::Win32::System::Console::ATTACH_PARENT_PROCESS,
        );
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !args.is_empty() && !matches!(args[0].as_str(), "daemon" | "settings" | "osd") {
        attach_console();
    }
    if let Err(e) = run(&args) {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

fn run(args: &[String]) -> Result<()> {
    let cmd = args.first().map(String::as_str).unwrap_or("");
    match cmd {
        "" => {
            if ipc::is_running() {
                ipc::request("settings")?;
                Ok(())
            } else {
                daemon::run()
            }
        }
        "daemon" => daemon::run(),
        "settings" => gui::settings::run(args.get(1).map(String::as_str)),
        "osd" => gui::osd::run(),
        "trigger" => {
            let Some(id) = args.get(1) else {
                bail!("usage: avrcontrol trigger <ID>")
            };
            ipc::request(&format!("trigger {id}"))?;
            Ok(())
        }
        "reload" | "quit" | "reconnect" => {
            ipc::request(cmd)?;
            Ok(())
        }
        "list" => {
            let cfg = Config::load()?;
            for s in &cfg.shortcuts {
                let hk = if s.hotkey.is_empty() { "-" } else { &s.hotkey };
                println!(
                    "{:<16} {:<24} {:<26} {}",
                    s.id,
                    hk,
                    s.name,
                    config::Action::kind_label(s.action.kind())
                );
            }
            Ok(())
        }
        "monitors" => {
            for m in monitors::scan(true)? {
                println!("{} [{}] on {}", m.id.label(), m.id.mfg, m.connector);
                if !m.ddc {
                    println!("    DDC/CI not supported");
                    continue;
                }
                let current = m
                    .current_input
                    .map(monitors::input_name)
                    .unwrap_or_else(|| "?".into());
                let inputs: Vec<String> = m
                    .inputs
                    .iter()
                    .map(|&c| format!("{} (0x{c:02x})", monitors::input_name(c)))
                    .collect();
                println!("    current input: {current}");
                println!("    inputs: {}", inputs.join(", "));
                if let Some(e) = m.error {
                    println!("    error: {e}");
                }
            }
            Ok(())
        }
        "receiver" => {
            let cfg = Config::load()?;
            let s: eiscp::ReceiverStatus = ipc::ask_receiver(
                "status",
                &cfg.receiver.host,
                cfg.receiver.port,
                "",
                Duration::from_secs(15),
                |r| r.status_with_model(),
            )?;
            println!("host:   {}", cfg.receiver.host);
            if let Some(m) = &s.model {
                println!("model:  {m}");
            }
            println!("power:  {}", if s.power_on { "on" } else { "standby" });
            if let Some(i) = s.input {
                println!("input:  {} ({i})", cfg.input_label(&i));
            }
            if let Some(v) = s.volume {
                println!(
                    "volume: {}{}",
                    cfg.volume_text(v),
                    if s.muted == Some(true) {
                        " (muted)"
                    } else {
                        ""
                    }
                );
            }
            if let Some(m) = &s.listening_mode {
                println!("mode:   {} ({m})", eiscp::listening_mode_name(m));
            }
            if let Some(o) = &s.hdmi_output {
                println!("hdmi:   {} ({o})", eiscp::hdmi_output_name(o));
            }
            if let Some(v) = &s.video {
                println!(
                    "video:  {} {} -> {} {}",
                    v.input_port, v.input_format, v.output_port, v.output_format
                );
            }
            if let Some(a) = &s.audio {
                println!("audio:  {a}");
            }
            Ok(())
        }
        "features" => {
            let cfg = Config::load()?;
            let caps: eiscp::Capabilities = ipc::ask_receiver(
                "probe",
                &cfg.receiver.host,
                cfg.receiver.port,
                "",
                Duration::from_secs(40),
                |r| r.probe(),
            )?;
            println!("source: {}\n", caps.source);
            for (group, ok, value) in &caps.features {
                let label = eiscp::FEATURES
                    .iter()
                    .find(|(g, _)| g == group)
                    .map(|(_, l)| *l)
                    .unwrap_or("");
                println!(
                    "{group}  {:<3}  {label:<34} {value}",
                    if *ok { "yes" } else { "no" }
                );
            }
            for (code, name) in &caps.inputs {
                println!("input {code}: {name}");
            }
            Ok(())
        }
        "discover" => {
            let found = eiscp::discover(Duration::from_secs(2))?;
            if found.is_empty() {
                println!("no receivers found");
            }
            for r in found {
                println!("{}  {}:{}  {}", r.model, r.host, r.port, r.identifier);
            }
            Ok(())
        }
        "-h" | "--help" | "help" => {
            print!("{USAGE}");
            Ok(())
        }
        "-V" | "--version" => {
            println!("avrcontrol {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        other => bail!("unknown command \"{other}\"\n\n{USAGE}"),
    }
}
