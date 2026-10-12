//! Turning the receiver off when the computer shuts down, if it's still on
//! the startup shortcut's input. On any other input something else (a
//! console, a TV box) is using it, so it's left alone.
//!
//! This runs apart from the controller. On Linux the desktop stops the
//! background service while logging out, before the shutdown itself
//! starts, so a separate watcher process outside the session handles it.

use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use crate::config::Config;
use crate::eiscp::Receiver;

/// Longest a shutdown is held up waiting for the receiver.
const MAX_WAIT: Duration = Duration::from_secs(4);

/// Reads the settings fresh and turns the receiver off if it's enabled and
/// still on the startup input. Gives up after `MAX_WAIT`.
pub fn power_off() {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(try_power_off());
    });
    match rx.recv_timeout(MAX_WAIT) {
        Ok(Ok(true)) => eprintln!("shutdown: receiver turned off"),
        Ok(Ok(false)) => eprintln!("shutdown: receiver left alone"),
        Ok(Err(e)) => eprintln!("shutdown: {e:#}"),
        Err(_) => eprintln!("shutdown: receiver did not answer in time"),
    }
}

fn try_power_off() -> anyhow::Result<bool> {
    let cfg = Config::load()?;
    if !cfg.general.shutdown_power_off || cfg.receiver.host.trim().is_empty() {
        return Ok(false);
    }
    let Some(input) = cfg.startup_input() else {
        return Ok(false);
    };
    let mut receiver = Receiver::new(&cfg.receiver.host, cfg.receiver.port);
    if receiver.query("PWR")? != "01" || !receiver.query("SLI")?.eq_ignore_ascii_case(input) {
        return Ok(false);
    }
    receiver.send("PWR00")?;
    Ok(true)
}

#[cfg(target_os = "linux")]
pub use linux::{start_watcher, watch};

#[cfg(target_os = "linux")]
mod linux {
    use std::path::PathBuf;
    use std::process::{Command, Stdio};

    use anyhow::{Result, anyhow};
    use zbus::blocking::proxy::SignalIterator;

    use crate::config::APP_NAME;

    const UNIT: &str = "avrcontrol-shutdown";

    /// Starts `avrcontrol shutdown-watch` as its own systemd user service,
    /// so it isn't stopped with the desktop session at logout. Does nothing
    /// if it's already running.
    pub fn start_watcher() {
        // An AppImage's mount goes away when it exits; run the file itself.
        let exe = match std::env::var_os("APPIMAGE") {
            Some(p) => PathBuf::from(p),
            None => match std::env::current_exe() {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("shutdown watcher: {e}");
                    return;
                }
            },
        };
        // After a package upgrade the old binary reads as "... (deleted)".
        let exe = match exe.to_str().and_then(|s| s.strip_suffix(" (deleted)")) {
            Some(s) => PathBuf::from(s),
            None => exe,
        };
        let result = Command::new("systemd-run")
            .args(["--user", "--quiet", "--collect"])
            .arg(format!("--unit={UNIT}"))
            .arg(format!("--description={APP_NAME} shutdown hook"))
            .arg(exe)
            .arg("shutdown-watch")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        if let Err(e) = result {
            eprintln!("shutdown watcher: systemd-run: {e}");
        }
    }

    /// Holds a logind "delay" inhibitor and turns the receiver off when a
    /// power-off starts. Runs until the system goes down.
    pub fn watch() -> Result<()> {
        let conn = zbus::blocking::Connection::system()?;
        let logind: zbus::blocking::Proxy = zbus::blocking::proxy::Builder::new(&conn)
            .destination("org.freedesktop.login1")?
            .path("/org/freedesktop/login1")?
            .interface("org.freedesktop.login1.Manager")?
            .cache_properties(zbus::proxy::CacheProperties::No)
            .build()?;
        let mut signals = logind.receive_signal("PrepareForShutdown")?;
        loop {
            let lock: zbus::zvariant::OwnedFd = logind.call(
                "Inhibit",
                &("shutdown", APP_NAME, "Turning off the receiver", "delay"),
            )?;
            // PrepareForShutdown(true) when it starts, (false) if cancelled.
            while !next(&mut signals)? {}
            if !is_reboot(&logind) {
                super::power_off();
            }
            drop(lock);
            // Still running: the shutdown was cancelled. Wait for logind to
            // say so, then take a new lock.
            while next(&mut signals)? {}
        }
    }

    fn next(signals: &mut SignalIterator) -> Result<bool> {
        let msg = signals
            .next()
            .ok_or_else(|| anyhow!("lost the system bus"))?;
        Ok(msg.body().deserialize::<bool>()?)
    }

    /// A reboot rather than a power-off; the receiver is left on since this
    /// PC will be back shortly. Needs systemd 255+; older versions count as
    /// power-off.
    fn is_reboot(logind: &zbus::blocking::Proxy) -> bool {
        use std::collections::HashMap;
        use zbus::zvariant::OwnedValue;

        let Ok(meta) = logind
            .get_property::<HashMap<String, OwnedValue>>("PreparingForShutdownWithMetadata")
        else {
            return false;
        };
        meta.get("type")
            .and_then(|v| <&str>::try_from(&**v).ok())
            .is_some_and(|t| matches!(t, "reboot" | "soft-reboot" | "kexec"))
    }
}
