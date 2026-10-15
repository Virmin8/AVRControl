//! Runs shortcuts against the receiver and monitors. Lives on its own
//! thread and is driven by `Event`s from hotkeys, the tray and IPC.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver as Rx, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::config::{Action, Config, Shortcut};
use crate::eiscp::Receiver;
use crate::feedback::Feedback;
use crate::monitors;

/// How often an idle connection is poked so the receiver doesn't drop it.
const KEEPALIVE: Duration = Duration::from_secs(30);
/// Quiet period after an action before the tray status is refreshed.
const STATUS_DELAY: Duration = Duration::from_millis(700);

#[derive(Debug, Clone)]
pub enum Event {
    Trigger(String),
    Reload,
    Reconnect,
    Quit,
    /// A question from the settings window, answered as JSON. Goes through
    /// the controller so only one connection to the receiver is open (some
    /// models drop the older connection when a second client connects).
    Query {
        kind: QueryKind,
        host: String,
        port: u16,
        reply: std::sync::mpsc::Sender<Result<String, String>>,
    },
}

#[derive(Debug, Clone)]
pub enum QueryKind {
    Status,
    Probe,
    /// Send one raw command; answered with the receiver's reply.
    Command(String),
}

/// Callbacks into the platform layer (tray + hotkeys).
pub struct Hooks {
    pub on_reload: Box<dyn Fn(&Config) + Send>,
    pub on_quit: Box<dyn Fn() + Send>,
    /// New tray tooltip text (receiver status summary).
    pub on_status: Arc<dyn Fn(String) + Send + Sync>,
    /// Tray entry for an available update (its label), or None to hide it.
    pub on_update: Arc<dyn Fn(Option<String>) + Send + Sync>,
}

pub struct Controller {
    cfg: Config,
    /// True while the startup shortcut runs: monitors already on their
    /// target input are left alone instead of being switched again.
    starting: bool,
    receiver: Receiver,
    feedback: Feedback,
    hooks: Hooks,
    /// Last known reachability, so we notify on changes rather than repeatedly.
    online: Option<bool>,
    /// Receiver model, looked up once per connection for the tooltip.
    model: Option<String>,
    /// An action ran since the tray status was last refreshed.
    status_stale: bool,
    /// Tooltip parts: (receiver lines, monitor lines). The monitor part is
    /// filled in by a background thread so DDC reads never delay hotkeys.
    tooltip: Arc<Mutex<(String, String)>>,
    monitors_busy: Arc<AtomicBool>,
}

fn parse_volume(reply: &str) -> Option<u8> {
    let hex = reply.strip_prefix("MVL")?;
    u8::from_str_radix(hex.get(..2)?, 16).ok()
}

impl Controller {
    pub fn new(cfg: Config, hooks: Hooks) -> Self {
        Self {
            starting: false,
            receiver: {
                let mut r = Receiver::new(&cfg.receiver.host, cfg.receiver.port);
                r.set_features(&cfg.effective_features());
                r
            },
            feedback: Feedback::new(&cfg),
            cfg,
            hooks,
            online: None,
            model: None,
            status_stale: true,
            tooltip: Default::default(),
            monitors_busy: Default::default(),
        }
    }

    pub fn run(mut self, events: Rx<Event>) {
        if self.has_receiver() {
            self.connect();
            let startup = self.cfg.general.startup_shortcut.clone();
            if !startup.is_empty() {
                self.starting = true;
                self.trigger(&startup);
                self.starting = false;
            }
        } else {
            self.feedback
                .info("No receiver configured yet. Open Settings from the tray icon.");
        }

        loop {
            // After an action, refresh the status once things go quiet (not
            // between steps of a held volume key).
            let wait = if self.status_stale {
                STATUS_DELAY
            } else {
                KEEPALIVE
            };
            match events.recv_timeout(wait) {
                Ok(Event::Trigger(id)) => {
                    self.trigger(&id);
                    crate::repeat::executed();
                    self.status_stale = true;
                }
                Ok(Event::Query {
                    kind,
                    host,
                    port,
                    reply,
                }) => {
                    let _ =
                        reply.send(self.answer(kind, &host, port).map_err(|e| format!("{e:#}")));
                }
                Ok(Event::Reload) => self.reload(),
                Ok(Event::Reconnect) => self.connect(),
                Ok(Event::Quit) | Err(RecvTimeoutError::Disconnected) => break,
                Err(RecvTimeoutError::Timeout) => self.keepalive(),
            }
        }
        self.receiver.disconnect();
        (self.hooks.on_quit)();
    }

    fn has_receiver(&self) -> bool {
        !self.cfg.receiver.host.trim().is_empty()
    }

    fn set_online(&mut self, online: bool, detail: &str) {
        if self.online != Some(online) {
            if online {
                self.feedback.info(&format!(
                    "Connected to receiver at {}",
                    self.receiver.host()
                ));
            } else {
                self.feedback
                    .error(&format!("Receiver unreachable: {detail}"));
            }
        }
        self.online = Some(online);
    }

    fn connect(&mut self) {
        match self.receiver.connect() {
            Ok(()) => self.set_online(true, ""),
            Err(e) => self.set_online(false, &format!("{e:#}")),
        }
    }

    fn answer(&mut self, kind: QueryKind, host: &str, port: u16) -> anyhow::Result<String> {
        let same = host.trim() == self.cfg.receiver.host.trim() && port == self.cfg.receiver.port;
        let mut temp;
        let receiver = if same {
            &mut self.receiver
        } else {
            // Settings is testing an address that hasn't been saved yet.
            temp = Receiver::new(host, port);
            &mut temp
        };
        let json = match kind {
            QueryKind::Status => {
                let mut status = receiver.status()?;
                status.model = if same && self.model.is_some() {
                    self.model.clone()
                } else {
                    crate::eiscp::identify(host, port).map(|r| r.model)
                };
                if same {
                    self.model = status.model.clone();
                    self.set_online(true, "");
                }
                serde_json::to_string(&status)?
            }
            QueryKind::Probe => serde_json::to_string(&receiver.probe()?)?,
            QueryKind::Command(cmd) => {
                let reply = receiver.send(&cmd)?;
                if same {
                    self.status_stale = true;
                }
                serde_json::to_string(&reply)?
            }
        };
        Ok(json)
    }

    fn publish(tooltip: &Mutex<(String, String)>, hook: &(dyn Fn(String) + Send + Sync)) {
        let t = tooltip.lock().unwrap();
        let text = if t.1.is_empty() {
            t.0.clone()
        } else {
            format!("{}\n\nMonitors\n{}", t.0, t.1)
        };
        hook(text);
    }

    /// Updates the receiver part of the tooltip and kicks off a monitor refresh.
    fn set_receiver_status(&mut self, text: String) {
        self.tooltip.lock().unwrap().0 = text;
        Self::publish(&self.tooltip, &*self.hooks.on_status);
        if self.monitors_busy.swap(true, Ordering::SeqCst) {
            return;
        }
        let tooltip = self.tooltip.clone();
        let hook = self.hooks.on_status.clone();
        let busy = self.monitors_busy.clone();
        thread::spawn(move || {
            let summary = monitors::summary();
            tooltip.lock().unwrap().1 = summary;
            Self::publish(&tooltip, &*hook);
            busy.store(false, Ordering::SeqCst);
        });
    }

    /// Runs on idle: refreshes the tray status, which also keeps the
    /// connection from being dropped by the receiver.
    fn keepalive(&mut self) {
        self.status_stale = false;
        if !self.has_receiver() {
            self.set_receiver_status("No receiver configured".into());
            return;
        }
        if !self.receiver.is_connected() && self.online != Some(true) {
            self.set_receiver_status(format!("Receiver unreachable ({})", self.receiver.host()));
            return;
        }
        match self.receiver.status() {
            Ok(status) => {
                if self.model.is_none() {
                    self.model =
                        crate::eiscp::identify(&self.cfg.receiver.host, self.cfg.receiver.port)
                            .map(|r| r.model);
                }
                let title = match &self.model {
                    Some(m) => format!("Onkyo {m}"),
                    None => "Onkyo receiver".into(),
                };
                self.set_receiver_status(format!("{title}\n{}", status.summary(&self.cfg)));
            }
            Err(_) => {
                // Quietly drop it; the next command reconnects on demand.
                self.receiver.disconnect();
                self.set_receiver_status("Receiver not responding".into());
            }
        }
    }

    fn reload(&mut self) {
        let cfg = match Config::load() {
            Ok(c) => c,
            Err(e) => {
                self.feedback
                    .error(&format!("Could not load settings: {e:#}"));
                return;
            }
        };
        let receiver_changed = cfg.receiver.host != self.cfg.receiver.host
            || cfg.receiver.port != self.cfg.receiver.port;
        self.feedback.apply_config(&cfg);
        crate::audio::apply(&cfg.audio);
        (self.hooks.on_reload)(&cfg);
        self.cfg = cfg;
        self.receiver.set_features(&self.cfg.effective_features());
        if receiver_changed {
            self.receiver.disconnect();
            self.receiver = Receiver::new(&self.cfg.receiver.host, self.cfg.receiver.port);
            self.receiver.set_features(&self.cfg.effective_features());
            self.online = None;
            self.model = None;
            self.status_stale = true;
            if self.has_receiver() {
                self.connect();
            }
        }
        self.feedback.info("Settings reloaded");
    }

    fn trigger(&mut self, id: &str) {
        let Some(shortcut) = self.cfg.shortcut(id).cloned() else {
            self.feedback.error(&format!("Unknown shortcut \"{id}\""));
            return;
        };
        if let Err(e) = self.execute(&shortcut) {
            let msg = format!("{}: {e:#}", shortcut.name);
            if self.receiver.is_connected() {
                self.feedback.error(&msg);
            } else {
                self.set_online(false, &msg);
            }
        }
    }

    /// Sends a command, recording reachability for the notifications.
    fn send(&mut self, cmd: &str) -> anyhow::Result<Option<String>> {
        let r = self.receiver.send(cmd)?;
        self.set_online(true, "");
        Ok(r)
    }

    fn execute(&mut self, shortcut: &Shortcut) -> anyhow::Result<()> {
        match &shortcut.action {
            Action::Switch { .. } => self.switch(shortcut),
            Action::ListeningMode { mode } => {
                let reply = self.send(&format!("LMD{mode}"))?.unwrap_or_default();
                // Stepping commands answer with the mode actually selected.
                let code = reply.strip_prefix("LMD").unwrap_or(mode);
                if code == "N/A" {
                    anyhow::bail!("listening mode not available for this signal");
                }
                self.feedback.info(&format!(
                    "Listening mode: {}",
                    crate::eiscp::listening_mode_name(code)
                ));
                Ok(())
            }
            Action::HdmiOutput { output } => {
                let reply = self.send(&format!("HDO{output}"))?.unwrap_or_default();
                let code = reply.strip_prefix("HDO").unwrap_or(output);
                self.feedback.info(&format!(
                    "HDMI output: {}",
                    crate::eiscp::hdmi_output_name(code)
                ));
                Ok(())
            }
            Action::Setting { group, value } => {
                let reply = self.send(&format!("{group}{value}"))?.unwrap_or_default();
                let info = crate::eiscp::setting_info(group);
                let label = info.map(|i| i.label).unwrap_or(group.as_str());
                let result = reply.strip_prefix(group.as_str()).unwrap_or("").to_string();
                if result == "N/A" {
                    anyhow::bail!("{label} isn't available right now");
                }
                if group == "MVL" {
                    if let Some(v) = parse_volume(&reply) {
                        self.feedback.volume(v, false);
                    }
                } else if info.is_none_or(|i| i.notify) && !result.is_empty() {
                    let text = self.describe_setting(group, &result);
                    self.feedback.info(&format!("{label}: {text}"));
                }
                Ok(())
            }
            Action::SetVolume { level } => {
                let raw = self.cfg.volume_raw(*level);
                let reply = self.send(&format!("MVL{raw:02X}"))?.unwrap_or_default();
                self.feedback
                    .volume(parse_volume(&reply).unwrap_or(raw), false);
                Ok(())
            }
            Action::PowerToggle => {
                let on = self.receiver.query("PWR")? == "01";
                self.send(if on { "PWR00" } else { "PWR01" })?;
                self.feedback
                    .info(if on { "Receiver off" } else { "Receiver on" });
                Ok(())
            }
            Action::PowerOn => {
                self.send("PWR01")?;
                self.feedback.info("Receiver on");
                Ok(())
            }
            Action::PowerOff => {
                self.send("PWR00")?;
                self.feedback.info("Receiver off");
                Ok(())
            }
            Action::VolumeUp | Action::VolumeDown => {
                let cmd = if shortcut.action == Action::VolumeUp {
                    "MVLUP"
                } else {
                    "MVLDOWN"
                };
                let reply = self.send(cmd)?.unwrap_or_default();
                let volume = match parse_volume(&reply) {
                    Some(v) => v,
                    None => {
                        u8::from_str_radix(self.receiver.query("MVL")?.get(..2).unwrap_or("0"), 16)?
                    }
                };
                self.feedback.volume(volume, false);
                Ok(())
            }
            Action::MuteToggle => {
                let reply = self.send("AMTTG")?.unwrap_or_default();
                let muted = reply == "AMT01";
                let volume = self
                    .receiver
                    .query("MVL")
                    .ok()
                    .and_then(|v| u8::from_str_radix(v.get(..2)?, 16).ok());
                match volume {
                    Some(v) => self.feedback.volume(v, muted),
                    None => self.feedback.info(if muted { "Muted" } else { "Unmuted" }),
                }
                Ok(())
            }
            Action::HdmiAudioToggle => {
                let on = self.receiver.query("HAO")? == "01";
                self.send(if on { "HAO00" } else { "HAO01" })?;
                self.feedback.info(if on {
                    "HDMI audio out: OFF"
                } else {
                    "HDMI audio out: ON"
                });
                Ok(())
            }
            Action::Commands { commands } => {
                for c in commands {
                    self.send(c)?;
                }
                self.feedback.info(&shortcut.name);
                Ok(())
            }
        }
    }

    /// Human-readable form of a setting's value as the receiver reports it.
    fn describe_setting(&self, group: &str, value: &str) -> String {
        if let Some(name) = crate::eiscp::setting_info(group)
            .and_then(|i| i.options.iter().find(|(c, _)| *c == value))
            .map(|(_, n)| n.to_string())
            .filter(|n| !matches!(n.as_str(), "Toggle" | "Next" | "Up" | "Down"))
        {
            return name;
        }
        match group {
            "SLZ" | "SL3" if value == "80" => "Same as main".into(),
            "SLZ" | "SL3" | "SLI" => self.cfg.input_label(value),
            "AMT" | "ZMT" | "MT3" | "PMB" | "MOT" | "CEC" | "HAS" | "TGA" | "TGB" | "TGC" => {
                if value == "01" {
                    "On".into()
                } else {
                    "Off".into()
                }
            }
            "ZVL" | "VL3" => u8::from_str_radix(value, 16)
                .map(|v| v.to_string())
                .unwrap_or_else(|_| value.into()),
            "SLP" if value != "OFF" => u32::from_str_radix(value, 16)
                .map(|m| format!("{m} min"))
                .unwrap_or_else(|_| value.into()),
            "AVS" => format!("{value} ms"),
            _ => crate::eiscp::setting_info(group)
                .and_then(|i| i.options.iter().find(|(c, _)| *c == value))
                .map(|(_, n)| n.to_string())
                .unwrap_or_else(|| value.to_string()),
        }
    }

    fn switch(&mut self, shortcut: &Shortcut) -> anyhow::Result<()> {
        let Action::Switch {
            receiver_input: input,
            power_on,
            hdmi_output,
            listening_mode,
            volume,
            extra_commands: extra,
            monitors: targets,
        } = &shortcut.action
        else {
            return Ok(());
        };
        // Monitors are switched on a separate thread so DDC/CI latency
        // overlaps with the network round-trips to the receiver.
        let mut targets = targets.to_vec();
        let starting = self.starting;
        let monitor_job = thread::spawn(move || {
            if starting && !targets.is_empty() {
                // At startup, leave monitors that are already on their input:
                // re-selecting it can blank the screen for a moment. If the
                // inputs can't be read, switch as usual.
                if let Ok(current) = monitors::current_inputs() {
                    targets.retain(|t| {
                        !current
                            .iter()
                            .any(|(id, _, input)| id == &t.monitor && *input == Some(t.input))
                    });
                }
            }
            targets
                .iter()
                .filter_map(|t| {
                    monitors::set_input(&t.monitor, t.input)
                        .err()
                        .map(|e| format!("{}: {e:#}", t.monitor.label()))
                })
                .collect::<Vec<_>>()
        });

        let receiver_result = (|| -> anyhow::Result<()> {
            let uses_receiver = input.is_some()
                || hdmi_output.is_some()
                || listening_mode.is_some()
                || volume.is_some()
                || !extra.is_empty();
            if !uses_receiver {
                return Ok(());
            }
            if *power_on && self.receiver.query("PWR")? != "01" {
                self.send("PWR01")?;
            }
            if let Some(code) = input {
                self.send(&format!("SLI{code}"))?;
            }
            if let Some(code) = hdmi_output {
                self.send(&format!("HDO{code}"))?;
            }
            // Set after the input: the receiver restores each input's last
            // listening mode when switching to it.
            if let Some(code) = listening_mode {
                self.send(&format!("LMD{code}"))?;
            }
            if let Some(level) = volume {
                let raw = self.cfg.volume_raw(*level);
                self.send(&format!("MVL{raw:02X}"))?;
            }
            for c in extra {
                self.send(c)?;
            }
            Ok(())
        })();

        let monitor_errors = monitor_job.join().unwrap_or_default();
        receiver_result?;
        if !monitor_errors.is_empty() {
            anyhow::bail!("monitor switch failed: {}", monitor_errors.join("; "));
        }
        self.feedback.info(&shortcut.name);
        Ok(())
    }
}
