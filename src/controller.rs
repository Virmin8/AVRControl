//! Runs shortcuts against the receiver and monitors. Lives on its own
//! thread and is driven by `Event`s from hotkeys, the tray and IPC.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver as Rx, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::config::{Action, Config, Shortcut};
use crate::eiscp::{Push, Receiver, ReceiverStatus};
use crate::feedback::Feedback;
use crate::monitors;

/// How often the full status is read. This also keeps the connection from
/// being dropped by the receiver.
const KEEPALIVE: Duration = Duration::from_secs(30);
/// Quiet period after an action before the tray status is refreshed.
const STATUS_DELAY: Duration = Duration::from_millis(700);
/// How long a full status read stays good enough to answer the settings
/// window from. In between, updates the receiver sends keep it current.
const STATUS_FRESH: Duration = Duration::from_secs(10);
/// How often the monitors' inputs are read for the tray tooltip (DDC/CI
/// traffic), besides shortly after a shortcut switched them.
const MONITORS_REFRESH: Duration = Duration::from_secs(300);
/// How long after a monitor switch its input is read again: monitors don't
/// answer DDC/CI while they switch.
const MONITORS_SETTLE: Duration = Duration::from_secs(5);
/// How long to wait before asking again for a model name that couldn't be
/// found out.
const MODEL_RETRY: Duration = Duration::from_secs(300);
/// How long to wait for the network after waking from sleep.
const WAKE_WAIT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
pub enum Event {
    Trigger(String),
    /// An action without a shortcut, e.g. scrolling on the tray icon.
    Run(Action),
    Reload,
    Reconnect,
    Quit,
    /// A status update the receiver sent by itself, e.g. "MVL2A" when the
    /// volume knob was turned.
    Pushed(String),
    /// The receiver closed the connection.
    Disconnected,
    /// The PC is about to sleep; answered once the receiver is dealt with.
    Sleep(Sender<()>),
    /// The PC woke up from sleep.
    Resume,
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

/// Set while a tray action is queued, so fast scrolling can't build up a
/// backlog of volume steps.
static TRAY_PENDING: AtomicBool = AtomicBool::new(false);

/// Queues an action from the tray icon unless the previous one is still
/// waiting to run.
pub fn tray_action(tx: &Sender<Event>, action: Action) {
    if !TRAY_PENDING.swap(true, Ordering::SeqCst) && tx.send(Event::Run(action)).is_err() {
        TRAY_PENDING.store(false, Ordering::SeqCst);
    }
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

/// The receiver's state as last seen: from the last full read, kept up to
/// date by every message the receiver sends.
#[derive(Default)]
struct Live {
    status: ReceiverStatus,
    /// When the last full read finished; None until one has on this
    /// connection.
    read_at: Option<Instant>,
}

pub struct Controller {
    cfg: Config,
    /// True while the startup shortcut runs: monitors already on their
    /// target input are left alone instead of being switched again.
    starting: bool,
    receiver: Receiver,
    /// Our own event queue, for the receiver's status updates.
    tx: Sender<Event>,
    live: Arc<Mutex<Live>>,
    feedback: Feedback,
    hooks: Hooks,
    /// Last known reachability, so we notify on changes rather than repeatedly.
    online: Option<bool>,
    /// Receiver model for the tooltip and the settings window.
    model: Option<String>,
    /// When the model was last asked for over the network.
    model_asked: Option<Instant>,
    /// An action ran since the tray status was last refreshed.
    status_stale: bool,
    /// A status update arrived since the tooltip was last published.
    tooltip_stale: bool,
    /// Tooltip parts: (receiver lines, monitor lines). The monitor part is
    /// filled in by a background thread so DDC reads never delay hotkeys.
    tooltip: Arc<Mutex<(String, String)>>,
    monitors_busy: Arc<AtomicBool>,
    /// When the monitor part of the tooltip is due for a refresh.
    monitors_due: Instant,
    /// (volume, muted) as last seen here, so the volume indicator only shows
    /// for updates that change them.
    heard: (Option<u8>, Option<bool>),
}

fn parse_volume(reply: &str) -> Option<u8> {
    let hex = reply.strip_prefix("MVL")?;
    u8::from_str_radix(hex.get(..2)?, 16).ok()
}

impl Controller {
    pub fn new(cfg: Config, hooks: Hooks, tx: Sender<Event>) -> Self {
        let live: Arc<Mutex<Live>> = Default::default();
        let mut c = Self {
            starting: false,
            receiver: Receiver::new("", 0),
            tx,
            live,
            feedback: Feedback::new(&cfg),
            cfg,
            hooks,
            online: None,
            model: None,
            model_asked: None,
            status_stale: true,
            tooltip_stale: false,
            tooltip: Default::default(),
            monitors_busy: Default::default(),
            monitors_due: Instant::now(),
            heard: (None, None),
        };
        c.receiver = c.make_receiver();
        c
    }

    /// A receiver for the configured address whose messages update `live`
    /// and whose own status updates come back to us as events.
    fn make_receiver(&self) -> Receiver {
        let mut r = Receiver::new(&self.cfg.receiver.host, self.cfg.receiver.port);
        r.set_features(&self.cfg.effective_features());
        let live = self.live.clone();
        let tx = self.tx.clone();
        r.set_listener(Arc::new(move |push| match push {
            Push::Message { msg, solicited } => {
                live.lock().unwrap().status.apply(msg);
                if !solicited {
                    let _ = tx.send(Event::Pushed(msg.to_string()));
                }
            }
            Push::Closed => {
                live.lock().unwrap().read_at = None;
                let _ = tx.send(Event::Disconnected);
            }
        }));
        r
    }

    pub fn run(mut self, events: Rx<Event>) {
        if self.has_receiver() {
            self.connect();
            self.run_startup();
        } else {
            self.feedback
                .info("No receiver configured yet. Open Settings from the tray icon.");
        }

        loop {
            // After an action, refresh the status once things go quiet (not
            // between steps of a held volume key).
            let wait = if self.status_stale || self.tooltip_stale {
                STATUS_DELAY
            } else {
                KEEPALIVE
            };
            match events.recv_timeout(wait) {
                Ok(Event::Trigger(id)) => {
                    self.trigger(&id);
                    crate::repeat::executed();
                    self.status_stale = true;
                    self.heard_now();
                }
                Ok(Event::Run(action)) => {
                    TRAY_PENDING.store(false, Ordering::SeqCst);
                    let shortcut = Shortcut {
                        id: String::new(),
                        name: Action::kind_label(action.kind()).to_string(),
                        hotkey: String::new(),
                        action,
                    };
                    self.run_shortcut(&shortcut);
                    self.status_stale = true;
                    self.heard_now();
                }
                Ok(Event::Query {
                    kind,
                    host,
                    port,
                    reply,
                }) => {
                    let _ =
                        reply.send(self.answer(kind, &host, port).map_err(|e| format!("{e:#}")));
                    self.heard_now();
                }
                Ok(Event::Pushed(msg)) => self.pushed(&msg),
                // Reconnected by the next command or status read, at the
                // latest with the next keepalive; not straight away, so a
                // competing client on a one-connection model isn't kicked off.
                Ok(Event::Disconnected) => {}
                Ok(Event::Sleep(done)) => {
                    self.sleep();
                    let _ = done.send(());
                }
                Ok(Event::Resume) => self.resume(),
                Ok(Event::Reload) => self.reload(),
                Ok(Event::Reconnect) => self.connect(),
                Ok(Event::Quit) | Err(RecvTimeoutError::Disconnected) => break,
                Err(RecvTimeoutError::Timeout) => {
                    if self.tooltip_stale && !self.status_stale {
                        self.tooltip_stale = false;
                        self.publish_live();
                    } else {
                        self.keepalive();
                    }
                }
            }
        }
        self.receiver.disconnect();
        (self.hooks.on_quit)();
    }

    fn has_receiver(&self) -> bool {
        !self.cfg.receiver.host.trim().is_empty()
    }

    fn run_startup(&mut self) {
        let startup = self.cfg.general.startup_shortcut.clone();
        if !startup.is_empty() {
            self.starting = true;
            self.trigger(&startup);
            self.starting = false;
        }
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
        self.live.lock().unwrap().read_at = None;
        match self.receiver.connect() {
            Ok(()) => self.set_online(true, ""),
            Err(e) => self.set_online(false, &format!("{e:#}")),
        }
    }

    /// The receiver's model: from the settings when known, otherwise asked
    /// for over the network (not more than every few minutes if it fails).
    fn model(&mut self) -> Option<String> {
        if self.model.is_none() && !self.cfg.receiver.model.is_empty() {
            self.model = Some(self.cfg.receiver.model.clone());
        }
        if self.model.is_none() && self.model_asked.is_none_or(|t| t.elapsed() > MODEL_RETRY) {
            self.model_asked = Some(Instant::now());
            self.model = crate::eiscp::identify(&self.cfg.receiver.host, self.cfg.receiver.port)
                .map(|r| r.model);
        }
        self.model.clone()
    }

    /// Reads the full status, remembering it in `live`.
    fn read_status(&mut self) -> anyhow::Result<ReceiverStatus> {
        let mut status = self.receiver.status()?;
        status.model = self.model();
        let mut live = self.live.lock().unwrap();
        live.status = status.clone();
        live.read_at = Some(Instant::now());
        drop(live);
        self.heard = (status.volume, status.muted);
        crate::audio::set_receiver_power(Some(status.power_on));
        Ok(status)
    }

    fn answer(&mut self, kind: QueryKind, host: &str, port: u16) -> anyhow::Result<String> {
        let same = host.trim() == self.cfg.receiver.host.trim() && port == self.cfg.receiver.port;
        if same {
            let json = match kind {
                QueryKind::Status => {
                    let cached = {
                        let live = self.live.lock().unwrap();
                        live.read_at
                            .filter(|t| t.elapsed() < STATUS_FRESH && self.receiver.is_connected())
                            .map(|_| live.status.clone())
                    };
                    let status = match cached {
                        Some(mut s) => {
                            s.model = self.model();
                            s
                        }
                        None => self.read_status()?,
                    };
                    self.set_online(true, "");
                    serde_json::to_string(&status)?
                }
                QueryKind::Probe => serde_json::to_string(&self.receiver.probe()?)?,
                QueryKind::Command(cmd) => {
                    let reply = self.send(&cmd)?;
                    self.status_stale = true;
                    serde_json::to_string(&reply)?
                }
            };
            return Ok(json);
        }
        // Settings is testing an address that hasn't been saved yet.
        let mut receiver = Receiver::new(host, port);
        let json = match kind {
            QueryKind::Status => serde_json::to_string(&receiver.status_with_model()?)?,
            QueryKind::Probe => serde_json::to_string(&receiver.probe()?)?,
            QueryKind::Command(cmd) => serde_json::to_string(&receiver.send(&cmd)?)?,
        };
        receiver.disconnect();
        Ok(json)
    }

    /// Remembers the current volume and mute as seen, after our own changes.
    fn heard_now(&mut self) {
        let live = self.live.lock().unwrap();
        self.heard = (live.status.volume, live.status.muted);
    }

    /// A status update the receiver sent by itself (already applied to
    /// `live` by the listener).
    fn pushed(&mut self, msg: &str) {
        self.tooltip_stale = true;
        let status = self.live.lock().unwrap().status.clone();
        match msg.get(..3) {
            Some("MVL" | "AMT") => {
                // Receivers repeat the volume when they power on, for example.
                let now = (status.volume, status.muted);
                let changed = self.heard.0.is_some() && now != self.heard;
                self.heard = now;
                if changed
                    && self.cfg.general.volume_osd_external
                    && let Some(v) = status.volume
                {
                    self.feedback.volume(v, status.muted == Some(true));
                }
            }
            Some("PWR") => crate::audio::set_receiver_power(Some(status.power_on)),
            _ => {}
        }
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

    /// Updates the receiver part of the tooltip, and the monitor part when
    /// it's due.
    fn set_receiver_status(&mut self, text: String) {
        self.tooltip.lock().unwrap().0 = text;
        Self::publish(&self.tooltip, &*self.hooks.on_status);
        if Instant::now() < self.monitors_due || self.monitors_busy.swap(true, Ordering::SeqCst) {
            return;
        }
        self.monitors_due = Instant::now() + MONITORS_REFRESH;
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

    fn status_text(&mut self, status: &ReceiverStatus) -> String {
        let title = match self.model() {
            Some(m) => format!("Onkyo {m}"),
            None => "Onkyo receiver".into(),
        };
        format!("{title}\n{}", status.summary(&self.cfg))
    }

    /// Tooltip from what the receiver has told us, without asking it.
    fn publish_live(&mut self) {
        let status = self.live.lock().unwrap().status.clone();
        let text = self.status_text(&status);
        self.set_receiver_status(text);
    }

    /// Runs on idle: refreshes the tray status, which also keeps the
    /// connection from being dropped by the receiver.
    fn keepalive(&mut self) {
        self.status_stale = false;
        self.tooltip_stale = false;
        if !self.has_receiver() {
            self.set_receiver_status("No receiver configured".into());
            return;
        }
        if !self.receiver.is_connected() && self.online != Some(true) {
            self.set_receiver_status(format!("Receiver unreachable ({})", self.receiver.host()));
            return;
        }
        match self.read_status() {
            Ok(status) => {
                let text = self.status_text(&status);
                self.set_receiver_status(text);
            }
            Err(_) => {
                // Quietly drop it; the next command reconnects on demand.
                self.receiver.disconnect();
                crate::audio::set_receiver_power(None);
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
            self.receiver = self.make_receiver();
            *self.live.lock().unwrap() = Live::default();
            crate::audio::set_receiver_power(None);
            self.online = None;
            self.model = None;
            self.model_asked = None;
            self.status_stale = true;
            if self.has_receiver() {
                self.connect();
            }
        }
        self.feedback.info("Settings reloaded");
    }

    /// The PC is going to sleep: turn the receiver off if that's wanted and
    /// nothing else is using it, then let go of the connection, which won't
    /// survive the sleep anyway.
    fn sleep(&mut self) {
        if self.cfg.general.sleep_power_off && self.has_receiver() {
            match crate::shutdown::off_if_on_startup_input(&mut self.receiver, &self.cfg) {
                Ok(true) => eprintln!("sleep: receiver turned off"),
                Ok(false) => eprintln!("sleep: receiver left alone"),
                Err(e) => eprintln!("sleep: {e:#}"),
            }
        }
        self.receiver.disconnect();
        *self.live.lock().unwrap() = Live::default();
    }

    /// Back from sleep: reconnect, and run the startup shortcut again if
    /// that's wanted and the receiver isn't in use on another input.
    fn resume(&mut self) {
        self.status_stale = true;
        self.monitors_due = Instant::now() + MONITORS_SETTLE;
        // Without the startup action, reconnect lazily: waiting here would
        // hold up hotkeys while the network comes back.
        if !self.has_receiver() || !self.cfg.general.wake_startup {
            return;
        }
        // The network usually takes a few seconds to come back.
        let until = Instant::now() + WAKE_WAIT;
        while self.receiver.connect().is_err() && Instant::now() < until {
            thread::sleep(Duration::from_secs(2));
        }
        if !self.receiver.is_connected() {
            eprintln!("wake: receiver unreachable, startup action skipped");
            return;
        }
        let free = self.cfg.startup_input().is_none()
            || crate::shutdown::on_startup_input_or_off(&mut self.receiver, &self.cfg)
                .unwrap_or(false);
        if free {
            self.run_startup();
        } else {
            eprintln!("wake: receiver is in use on another input, left alone");
        }
    }

    fn trigger(&mut self, id: &str) {
        let Some(shortcut) = self.cfg.shortcut(id).cloned() else {
            self.feedback.error(&format!("Unknown shortcut \"{id}\""));
            return;
        };
        self.run_shortcut(&shortcut);
    }

    fn run_shortcut(&mut self, shortcut: &Shortcut) {
        if let Err(e) = self.execute(shortcut) {
            let msg = format!("{}: {e:#}", shortcut.name);
            if self.receiver.is_connected() {
                self.feedback.error(&msg);
            } else {
                self.set_online(false, &msg);
            }
        }
    }

    /// Sends a command, recording reachability for the notifications and
    /// keeping volume changes under the volume limit.
    fn send(&mut self, cmd: &str) -> anyhow::Result<Option<String>> {
        let cmd = cmd.trim().to_ascii_uppercase();
        let ceiling = self.cfg.volume_ceiling();
        let mut cmd = cmd.as_str();
        let clamped;
        if let (Some(max), Some(arg)) = (ceiling, cmd.strip_prefix("MVL")) {
            if matches!(arg, "UP" | "UP1") && self.current_volume()? >= max {
                // Already at the limit: answer as if the receiver had.
                return Ok(Some(format!("MVL{max:02X}")));
            }
            if arg.len() == 2
                && let Ok(v) = u8::from_str_radix(arg, 16)
                && v > max
            {
                clamped = format!("MVL{max:02X}");
                cmd = &clamped;
            }
        }
        let mut r = self.receiver.send(cmd)?;
        self.set_online(true, "");
        // A volume step can be bigger than one, so it may overshoot.
        if let Some(max) = ceiling
            && cmd.starts_with("MVLUP")
            && r.as_deref().and_then(parse_volume).is_some_and(|v| v > max)
        {
            r = self.receiver.send(&format!("MVL{max:02X}"))?;
        }
        Ok(r)
    }

    /// Raw volume: as last reported by the receiver, or asked for.
    fn current_volume(&mut self) -> anyhow::Result<u8> {
        if let Some(v) = self.live.lock().unwrap().status.volume
            && self.receiver.is_connected()
        {
            return Ok(v);
        }
        let reply = self.receiver.query("MVL")?;
        Ok(u8::from_str_radix(reply.get(..2).unwrap_or("0"), 16)?)
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
                    None => self.current_volume()?,
                };
                self.feedback.volume(volume, false);
                Ok(())
            }
            Action::MuteToggle => {
                let reply = self.send("AMTTG")?.unwrap_or_default();
                let muted = reply == "AMT01";
                let volume = self.current_volume().ok();
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
        if !targets.is_empty() {
            self.monitors_due = Instant::now() + MONITORS_SETTLE;
        }
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

#[cfg(test)]
mod tests {
    use super::parse_volume;

    #[test]
    fn volume_replies() {
        assert_eq!(parse_volume("MVL3A"), Some(0x3A));
        assert_eq!(parse_volume("MVLN/A"), None);
        assert_eq!(parse_volume("MVL3"), None);
        assert_eq!(parse_volume("AMT01"), None);
    }
}
