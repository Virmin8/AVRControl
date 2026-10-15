//! The background instance: IPC server + controller thread + platform layer
//! (tray icon and global shortcuts).

use std::process::{Child, Command};
use std::sync::Mutex;
use std::sync::mpsc;
use std::thread;

use anyhow::{Result, bail};

use crate::config::Config;
use crate::controller::{Controller, Event, QueryKind};
use crate::{ipc, platform};

static SETTINGS: Mutex<Option<Child>> = Mutex::new(None);

/// Opens the settings window as a separate process, unless one is open.
pub fn open_settings() {
    let mut guard = SETTINGS.lock().unwrap();
    if let Some(child) = guard.as_mut()
        && matches!(child.try_wait(), Ok(None))
    {
        return;
    }
    match std::env::current_exe().and_then(|exe| Command::new(exe).arg("settings").spawn()) {
        Ok(child) => *guard = Some(child),
        Err(e) => eprintln!("could not open settings: {e}"),
    }
}

fn handle_request(req: &str, tx: &mpsc::Sender<Event>) -> Result<String> {
    let (cmd, arg) = req.split_once(' ').unwrap_or((req, ""));
    let event = match cmd {
        "ping" => return Ok("pong".into()),
        "settings" => {
            open_settings();
            return Ok("ok".into());
        }
        "configure-shortcuts" => {
            platform::configure_shortcuts()?;
            return Ok("ok".into());
        }
        "trigger" if !arg.is_empty() => {
            let id = arg.trim();
            if Config::load()?.shortcut(id).is_none() {
                bail!("no shortcut with ID \"{id}\" (see `avrcontrol list`)");
            }
            Event::Trigger(id.to_string())
        }
        "status" | "probe" | "send" => {
            // "status <host> <port>"
            let mut parts = arg.split_whitespace();
            let host = parts.next().unwrap_or("").to_string();
            let port = parts
                .next()
                .and_then(|p| p.parse().ok())
                .unwrap_or(crate::eiscp::DEFAULT_PORT);
            let kind = match cmd {
                "status" => QueryKind::Status,
                "probe" => QueryKind::Probe,
                // "send <host> <port> <COMMAND>"
                _ => QueryKind::Command(parts.next().unwrap_or("").to_string()),
            };
            let (reply_tx, reply_rx) = mpsc::channel();
            tx.send(Event::Query {
                kind,
                host,
                port,
                reply: reply_tx,
            })?;
            return reply_rx
                .recv_timeout(std::time::Duration::from_secs(30))
                .map_err(|_| anyhow::anyhow!("no answer from the background service"))?
                .map_err(|e| anyhow::anyhow!(e));
        }
        "check-updates" => return crate::update::check_now(),
        "reload" => Event::Reload,
        "reconnect" => Event::Reconnect,
        "quit" => Event::Quit,
        _ => bail!("unknown request \"{req}\""),
    };
    tx.send(event)?;
    Ok("ok".into())
}

pub fn run() -> Result<()> {
    let first_run = !Config::exists();
    let cfg = Config::load().unwrap_or_else(|e| {
        eprintln!("{e:#}; using defaults");
        Config::default()
    });

    let (tx, rx) = mpsc::channel::<Event>();
    let ipc_tx = tx.clone();
    ipc::serve(move |req| handle_request(req, &ipc_tx))?;

    if first_run || cfg.receiver.host.trim().is_empty() {
        open_settings();
    }

    crate::audio::apply(&cfg.audio);
    crate::update::clean_stale_download();
    let update_tx = tx.clone();
    let result = platform::run(cfg, tx, move |cfg, hooks| {
        crate::update::spawn(update_tx, hooks.on_update.clone());
        thread::spawn(move || Controller::new(cfg, hooks).run(rx));
    });
    ipc::cleanup();
    crate::update::restart_if_updated();
    result
}
