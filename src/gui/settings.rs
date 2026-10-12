//! The settings window (`avrcontrol settings`). Edits the config file and
//! asks the running background instance to reload it on save.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, RichText};

use crate::config::{Action, Config, MonitorId, MonitorTarget, Shortcut};
use crate::eiscp::{self, Capabilities, DiscoveredReceiver, ReceiverStatus};
use crate::gui::theme::{self, icon};
use crate::hotkey::{self, Hotkey};
use crate::monitors::{self, MonitorInfo};
use crate::{autostart, ipc};

/// A background job whose result is polled from the UI thread.
struct Task<T> {
    rx: Option<mpsc::Receiver<Result<T, String>>>,
    result: Option<Result<T, String>>,
}

impl<T: Send + 'static> Task<T> {
    fn new() -> Self {
        Self {
            rx: None,
            result: None,
        }
    }

    fn start(
        &mut self,
        ctx: &egui::Context,
        f: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
    ) {
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(f().map_err(|e| format!("{e:#}")));
            ctx.request_repaint();
        });
        self.rx = Some(rx);
    }

    fn running(&self) -> bool {
        self.rx.is_some()
    }

    /// Picks up a finished result; returns true on the frame it arrives.
    fn poll(&mut self) -> bool {
        if let Some(rx) = &self.rx
            && let Ok(r) = rx.try_recv()
        {
            self.result = Some(r);
            self.rx = None;
            return true;
        }
        false
    }

    fn ok(&self) -> Option<&T> {
        self.result.as_ref().and_then(|r| r.as_ref().ok())
    }
}

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Status,
    Receiver,
    Monitors,
    Shortcuts,
    General,
}

struct SettingsApp {
    cfg: Config,
    saved: Config,
    /// False until the config file has been written once (first run).
    on_disk: bool,
    tab: Tab,
    selected: usize,
    recording: bool,
    status: Option<(String, bool, Instant)>,
    monitors: Task<Vec<MonitorInfo>>,
    receiver: Task<ReceiverStatus>,
    discovery: Task<Vec<DiscoveredReceiver>>,
    monitor_test: Task<()>,
    probe: Task<Capabilities>,
    /// "Check now" on the General tab.
    update_check: Task<String>,
    /// When the receiver status was last requested (auto-refresh on the Receiver tab).
    status_at: Instant,
    /// Receiver tab: setup (features, inputs, volume scale) instead of controls.
    receiver_setup: bool,
    monitors_at: Instant,
    /// A control-panel button press in flight.
    command: Task<Option<String>>,
    /// "Sent X -> reply" for the control panel.
    last_command: String,
    volume_target: u8,
    /// Volume picked on the Status page slider, not sent yet (one command
    /// is in flight at a time).
    volume_want: Option<f32>,
    /// Volume sent from the slider, shown until the receiver reports it, so
    /// the slider doesn't jump back while the status catches up.
    volume_pending: Option<(f32, Instant)>,
    /// Wheel movement over the slider not yet turned into a volume step.
    slider_scroll: f32,
    raw_command: String,
    /// Detection runs automatically once per session when nothing is stored yet.
    auto_probed: bool,
    autostart: bool,
    daemon_running: bool,
    daemon_checked: Instant,
}

/// (command group, label, [(code, option label)])
type ChoiceRow<'a> = (&'a str, &'a str, &'a [(&'a str, &'a str)]);

/// "Label  [opt|opt|opt]" grid row; the current value is highlighted and a
/// click queues `<group><code>`. Long lists become a dropdown.
fn choice_row(
    ui: &mut egui::Ui,
    label: &str,
    current: Option<&str>,
    options: &[(&str, &str)],
    group: &str,
    cmd: &mut Option<String>,
) {
    if options.is_empty() {
        return;
    }
    ui.label(RichText::new(label).color(theme::TEXT_SECONDARY));
    if options.len() <= 4 {
        let names: Vec<&str> = options.iter().map(|o| o.1).collect();
        let selected = options.iter().position(|o| Some(o.0) == current);
        if let Some(i) = theme::segmented(ui, group, &names, selected) {
            *cmd = Some(format!("{group}{}", options[i].0));
        }
    } else {
        let text = options
            .iter()
            .find(|o| Some(o.0) == current)
            .map(|o| o.1)
            .unwrap_or("–");
        egui::ComboBox::from_id_salt(("ctl", group))
            .selected_text(text)
            .width(170.0)
            .show_ui(ui, |ui| {
                for (code, name) in options {
                    if ui.selectable_label(current == Some(*code), *name).clicked() {
                        *cmd = Some(format!("{group}{code}"));
                    }
                }
            });
    }
    ui.end_row();
}

/// A `ChoiceRow` narrowed to the options the model accepts.
type SupportedRow<'a> = (&'a str, &'a str, Vec<(&'a str, &'a str)>);

/// A card of `choice_row`s, left out entirely when the model supports none
/// of them.
fn settings_card(
    ui: &mut egui::Ui,
    icon: &str,
    title: &str,
    cfg: &Config,
    st: &ReceiverStatus,
    rows: &[ChoiceRow],
    cmd: &mut Option<String>,
) {
    let rows: Vec<SupportedRow> = rows
        .iter()
        .filter(|(group, _, _)| cfg.supports(group))
        .map(|(group, label, options)| {
            let options = options
                .iter()
                .copied()
                .filter(|(code, _)| cfg.supports_value(group, code))
                .collect();
            (*group, *label, options)
        })
        .collect();
    if rows.is_empty() {
        return;
    }
    theme::card(ui, icon, title, None, |ui| {
        controls_grid(ui, title, |ui| {
            for (group, label, options) in &rows {
                let current = if *group == "HDO" {
                    st.hdmi_output.as_deref()
                } else {
                    st.extra.get(*group).map(String::as_str)
                };
                choice_row(ui, label, current, options, group, cmd);
            }
        });
    });
    ui.add_space(12.0);
}

/// Two-column "label  control" grid used by every controls card.
fn controls_grid(ui: &mut egui::Ui, id: &str, add: impl FnOnce(&mut egui::Ui)) {
    egui::Grid::new(("controls", id))
        .num_columns(2)
        .min_col_width(130.0)
        .spacing([16.0, 10.0])
        .show(ui, add);
}

/// The receiver's inputs as hardware-style keys, filling `width` in equal
/// rows. A click on another input queues `SLI<code>`.
fn input_keys(
    ui: &mut egui::Ui,
    cfg: &Config,
    current: Option<&str>,
    width: f32,
    busy: bool,
    cmd: &mut Option<String>,
) {
    if !cfg.supports("SLI") {
        ui.label(
            RichText::new("This receiver can't switch inputs over the network.")
                .color(theme::TEXT_MUTED),
        );
        return;
    }
    let inputs: Vec<(&str, String)> = eiscp::INPUTS
        .iter()
        .filter(|(c, _)| {
            !cfg.receiver.hidden_inputs.iter().any(|h| h == c) && cfg.supports_value("SLI", c)
        })
        .map(|(c, _)| (*c, cfg.input_label(c)))
        .collect();
    let spacing = 8.0;
    let per_row = (((width + spacing) / (100.0 + spacing)).floor() as usize).max(1);
    let key_w = (width - spacing * (per_row - 1) as f32) / per_row as f32;
    for row in inputs.chunks(per_row) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = spacing;
            for (code, label) in row {
                let on = current == Some(*code);
                if theme::input_key(ui, label, on, key_w).clicked() && !busy && !on {
                    *cmd = Some(format!("SLI{code}"));
                }
            }
        });
        ui.add_space(spacing - ui.spacing().item_spacing.y);
    }
}

/// A dropdown over one or more (code, name) tables.
fn code_combo(ui: &mut egui::Ui, id: &str, value: &mut String, tables: &[&[(&str, &str)]]) {
    let selected = tables
        .iter()
        .flat_map(|t| t.iter())
        .find(|(c, _)| *c == value.as_str())
        .map(|(c, n)| format!("{n} ({c})"))
        .unwrap_or_else(|| value.clone());
    egui::ComboBox::from_id_salt(id)
        .selected_text(selected)
        .width(240.0)
        .height(400.0)
        .show_ui(ui, |ui| {
            for (i, table) in tables.iter().enumerate() {
                if i > 0 {
                    ui.separator();
                }
                for (code, name) in *table {
                    ui.selectable_value(value, code.to_string(), format!("{name} ({code})"));
                }
            }
        });
}

/// "[x] label [dropdown]": None while unticked.
fn optional_code(
    ui: &mut egui::Ui,
    id: &str,
    label: &str,
    value: &mut Option<String>,
    table: &[(&str, &str)],
) {
    ui.horizontal(|ui| {
        let mut enabled = value.is_some();
        if ui.checkbox(&mut enabled, label).changed() {
            *value = enabled.then(|| table[0].0.to_string());
        }
        if let Some(v) = value {
            code_combo(ui, id, v, &[table]);
        }
    });
}

fn nonempty(s: &str) -> String {
    if s.trim().is_empty() {
        "–".to_string()
    } else {
        s.to_string()
    }
}

/// One monitor's full details for the Monitors tab. Every card shows the
/// same rows in the same order so cards side by side line up.
fn monitor_card(ui: &mut egui::Ui, m: &MonitorInfo) {
    let dash = || "–".to_string();
    let subtitle = [
        m.id.mfg.clone(),
        if m.id.serial.is_empty() {
            String::new()
        } else {
            format!("S/N {}", m.id.serial)
        },
    ]
    .into_iter()
    .filter(|s| !s.is_empty())
    .collect::<Vec<_>>()
    .join("  ·  ");
    let primary = m.mode.as_ref().is_some_and(|d| d.primary);
    theme::card_with(
        ui,
        icon::MONITOR,
        &m.id.model,
        Some(&subtitle),
        |ui| {
            if primary {
                theme::status_pill(ui, "Primary", theme::ACCENT_STRONG, theme::ACCENT_BORDER);
            }
        },
        |ui| {
            let d = m.mode.clone().unwrap_or_default();
            let has_mode = m.mode.is_some();
            let on_off = |b: Option<bool>| {
                b.map(|b| if b { "On" } else { "Off" }.to_string())
                    .unwrap_or_else(dash)
            };
            let pair = |v: Option<(u16, u16)>| {
                v.map(|(c, max)| format!("{c} / {max}"))
                    .unwrap_or_else(dash)
            };

            theme::kv(ui, "Connector", m.connector.clone());
            theme::kv(
                ui,
                "Current input",
                m.current_input
                    .map(monitors::input_name)
                    .unwrap_or_else(|| "No answer".into()),
            );
            theme::kv(ui, "Input signal", signal_text(m));
            if has_mode && !d.enabled {
                theme::kv_colored(ui, "PC output", "Disabled", theme::TEXT_MUTED);
            }
            theme::kv(
                ui,
                "PC resolution",
                if has_mode { d.resolution() } else { dash() },
            );
            theme::kv(
                ui,
                "PC refresh rate",
                if has_mode { d.refresh() } else { dash() },
            );
            theme::kv(
                ui,
                "Scale",
                d.scale
                    .map(|s| format!("{:.0}%", s * 100.0))
                    .unwrap_or_else(dash),
            );
            theme::kv(
                ui,
                "Position",
                d.position
                    .map(|(x, y)| format!("{x}, {y}"))
                    .unwrap_or_else(dash),
            );
            theme::kv(ui, "Rotation", d.rotation.clone().unwrap_or_else(dash));
            theme::kv(
                ui,
                "Colour depth",
                d.bits_per_color
                    .map(|b| format!("up to {b}-bit"))
                    .unwrap_or_else(dash),
            );
            theme::kv(ui, "HDR", on_off(d.hdr));
            theme::kv(ui, "Wide colour gamut", on_off(d.wide_color));
            theme::kv(ui, "Adaptive sync", d.vrr.clone().unwrap_or_else(dash));
            theme::kv(
                ui,
                "Screen size",
                match (d.diagonal_inches(), d.size_mm) {
                    (Some(inch), Some((w, h))) => format!("{inch:.1}\"  ({w} × {h} mm)"),
                    _ => dash(),
                },
            );
            theme::kv(ui, "Brightness", pair(m.brightness));
            theme::kv(ui, "Contrast", pair(m.contrast));
            let inputs: Vec<String> = m.inputs.iter().map(|&c| monitors::input_name(c)).collect();
            theme::kv(
                ui,
                "Inputs",
                if inputs.is_empty() {
                    dash()
                } else {
                    inputs.join(", ")
                },
            );
            if let Some(e) = &m.error {
                ui.colored_label(theme::WARNING, e);
            }
        },
    );
}

/// What the monitor says it's receiving on its current input. "–" covers
/// both no signal and monitors that don't report it.
fn signal_text(m: &MonitorInfo) -> String {
    m.signal.map(|s| s.describe()).unwrap_or_else(|| "–".into())
}

fn err_color(ui: &egui::Ui) -> Color32 {
    ui.visuals().error_fg_color
}

impl SettingsApp {
    fn new(ctx: &egui::Context, tab: Tab) -> Self {
        let cfg = Config::load().unwrap_or_default();
        let mut app = Self {
            saved: cfg.clone(),
            on_disk: Config::exists(),
            cfg,
            tab,
            selected: 0,
            recording: false,
            status: None,
            monitors: Task::new(),
            receiver: Task::new(),
            discovery: Task::new(),
            monitor_test: Task::new(),
            probe: Task::new(),
            update_check: Task::new(),
            status_at: Instant::now(),
            receiver_setup: false,
            monitors_at: Instant::now(),
            command: Task::new(),
            last_command: String::new(),
            volume_target: 40,
            volume_want: None,
            volume_pending: None,
            slider_scroll: 0.0,
            raw_command: String::new(),
            auto_probed: false,
            autostart: autostart::is_enabled(),
            daemon_running: ipc::is_running(),
            daemon_checked: Instant::now(),
        };
        if !Config::exists() {
            app.set_status(
                "Welcome! Enter your receiver's IP address (or use Find), then Save.",
                false,
            );
        }
        app.scan_monitors(ctx);
        if !app.cfg.receiver.host.trim().is_empty() {
            app.refresh_receiver(ctx);
        }
        app
    }

    fn set_status(&mut self, msg: impl Into<String>, error: bool) {
        self.status = Some((msg.into(), error, Instant::now()));
    }

    fn dirty(&self) -> bool {
        self.cfg != self.saved || !self.on_disk
    }

    fn scan_monitors(&mut self, ctx: &egui::Context) {
        self.monitors_at = Instant::now();
        self.monitors.start(ctx, || monitors::scan(true));
    }

    fn start_probe(&mut self, ctx: &egui::Context) {
        let host = self.cfg.receiver.host.clone();
        let port = self.cfg.receiver.port;
        self.auto_probed = true;
        self.probe.start(ctx, move || {
            ipc::ask_receiver("probe", &host, port, "", Duration::from_secs(40), |r| {
                r.probe()
            })
        });
    }

    /// Stores detection results and saves them right away, so detection
    /// only ever runs once. If the receiver listed its inputs, only those
    /// are shown, with the receiver's names (user labels are kept).
    fn apply_probe(&mut self, caps: &Capabilities) {
        let rc = &mut self.cfg.receiver;
        rc.features = caps
            .features
            .iter()
            .map(|(g, ok, _)| (g.clone(), *ok))
            .collect();
        if let Some(model) = caps
            .model
            .clone()
            .or_else(|| self.receiver.ok().and_then(|s| s.model.clone()))
        {
            rc.model = model;
        }
        rc.features_source = caps.source.clone();
        // Models whose volume runs 0..200 count in half steps.
        if let Some(mvl) = crate::models::lookup(&rc.model).and_then(|t| t.get("MVL")) {
            rc.volume_half_steps =
                mvl.iter().any(|v| v == "0..200") && !mvl.iter().any(|v| v == "0..100");
        }
        let inputs: Vec<&(String, String)> = caps
            .inputs
            .iter()
            .filter(|(code, _)| eiscp::INPUTS.iter().any(|(c, _)| c == code))
            .collect();
        if !inputs.is_empty() {
            rc.hidden_inputs = eiscp::INPUTS
                .iter()
                .map(|(c, _)| c.to_string())
                .filter(|c| !inputs.iter().any(|(code, _)| code == c))
                .collect();
            for (code, name) in &inputs {
                let label = rc.input_names.entry(code.clone()).or_default();
                if label.is_empty() {
                    *label = name.clone();
                }
            }
        }

        // Write only the detection results into the file, leaving any other
        // unsaved edits for the Save button (on first run, save everything).
        let mut disk = if self.on_disk {
            Config::load().unwrap_or_else(|_| self.saved.clone())
        } else {
            self.cfg.clone()
        };
        for target in [&mut disk.receiver, &mut self.saved.receiver] {
            target.features = self.cfg.receiver.features.clone();
            target.model = self.cfg.receiver.model.clone();
            target.features_source = self.cfg.receiver.features_source.clone();
            target.volume_half_steps = self.cfg.receiver.volume_half_steps;
            target.hidden_inputs = self.cfg.receiver.hidden_inputs.clone();
            target.input_names = self.cfg.receiver.input_names.clone();
        }
        if !self.on_disk {
            self.saved = disk.clone();
        }
        match disk.save() {
            Ok(()) => {
                self.on_disk = true;
                if ipc::is_running() {
                    let _ = ipc::request("reload");
                }
                let supported = caps.features.iter().filter(|f| f.1).count();
                self.set_status(
                    format!(
                        "Receiver detected: {supported} features, {} inputs (from {}). Saved.",
                        inputs.len(),
                        caps.source
                    ),
                    false,
                );
            }
            Err(e) => self.set_status(format!("Could not save detection results: {e:#}"), true),
        }
    }

    fn send_command(&mut self, ctx: &egui::Context, cmd: String) {
        let host = self.cfg.receiver.host.clone();
        let port = self.cfg.receiver.port;
        self.last_command = format!("Sent {cmd}…");
        self.command.start(ctx, move || {
            ipc::ask_receiver("send", &host, port, &cmd, Duration::from_secs(10), |r| {
                r.send(&cmd)
            })
            .map(|reply| {
                Some(format!(
                    "Sent {cmd} -> {}",
                    reply.unwrap_or_else(|| "no reply".into())
                ))
            })
        });
    }

    fn refresh_receiver(&mut self, ctx: &egui::Context) {
        self.status_at = Instant::now();
        let host = self.cfg.receiver.host.clone();
        let port = self.cfg.receiver.port;
        self.receiver.start(ctx, move || {
            ipc::ask_receiver("status", &host, port, "", Duration::from_secs(15), |r| {
                r.status_with_model()
            })
        });
    }

    fn validate(&self) -> Result<(), String> {
        let mut ids = std::collections::HashSet::new();
        for s in &self.cfg.shortcuts {
            if s.id.is_empty()
                || !s
                    .id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            {
                return Err(format!(
                    "Shortcut \"{}\": the ID may only contain letters, digits, _ and -",
                    s.name
                ));
            }
            if !ids.insert(&s.id) {
                return Err(format!("Two shortcuts share the ID \"{}\"", s.id));
            }
            hotkey::validate(&s.hotkey).map_err(|e| format!("Shortcut \"{}\": {e}", s.name))?;
        }
        Ok(())
    }

    fn save(&mut self) {
        if let Err(e) = self.validate() {
            self.set_status(e, true);
            return;
        }
        // Normalise hotkey spelling ("ctrl+alt+p" -> "Ctrl+Alt+P").
        for s in &mut self.cfg.shortcuts {
            if let Some(h) = Hotkey::parse(&s.hotkey) {
                s.hotkey = h.to_string();
            }
        }
        if let Err(e) = self.cfg.save() {
            self.set_status(format!("Could not save: {e:#}"), true);
            return;
        }
        self.saved = self.cfg.clone();
        self.on_disk = true;
        if ipc::is_running() {
            match ipc::request("reload") {
                Ok(_) => self.set_status("Saved and applied.", false),
                Err(e) => self.set_status(
                    format!("Saved, but the background service didn't reload: {e:#}"),
                    true,
                ),
            }
        } else {
            self.set_status(
                "Saved. Start the background service (General tab) to use the hotkeys.",
                false,
            );
        }
    }

    // ------------------------------------------------------------------ tabs

    fn receiver_tab(&mut self, ui: &mut egui::Ui) {
        let has_host = !self.cfg.receiver.host.trim().is_empty();
        let mut test = false;
        let mut view = None;
        theme::page_header(
            ui,
            "Receiver",
            if self.receiver_setup {
                "Volume scale, supported features and inputs."
            } else {
                "Connection, and live controls sent straight to the receiver."
            },
            |ui| {
                if ui
                    .add_enabled(
                        has_host && !self.receiver.running(),
                        egui::Button::new(format!("{}  Test connection", icon::REFRESH))
                            .min_size(egui::vec2(0.0, 32.0)),
                    )
                    .clicked()
                {
                    test = true;
                }
                ui.add_space(6.0);
                let selected = Some(usize::from(self.receiver_setup));
                view = theme::segmented(ui, "receiver_view", &["Controls", "Setup"], selected);
            },
        );
        if let Some(i) = view {
            self.receiver_setup = i == 1;
        }
        if test {
            self.refresh_receiver(ui.ctx());
        }

        theme::card(
            ui,
            icon::LINK,
            "Connection",
            Some("Where AVR Control finds the receiver."),
            |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("IP address").color(theme::TEXT_SECONDARY));
                    ui.add(
                        egui::TextEdit::singleline(&mut self.cfg.receiver.host)
                            .hint_text("192.168.1.30")
                            .desired_width(180.0)
                            .margin(egui::vec2(8.0, 6.0)),
                    );
                    ui.add_space(12.0);
                    ui.label(RichText::new("Port").color(theme::TEXT_SECONDARY));
                    ui.add(egui::DragValue::new(&mut self.cfg.receiver.port).range(1..=65535));
                    ui.add_space(12.0);
                    if ui
                        .add_enabled(
                            !self.discovery.running(),
                            egui::Button::new(format!("{}  Find on network", icon::SEARCH))
                                .min_size(egui::vec2(0.0, 32.0)),
                        )
                        .clicked()
                    {
                        self.discovery
                            .start(ui.ctx(), || eiscp::discover(Duration::from_secs(2)));
                    }
                    if self.discovery.running()
                        || (self.receiver.running() && self.receiver.ok().is_none())
                    {
                        ui.add(egui::Spinner::new());
                    }
                    if self.probe.running() {
                        ui.add(egui::Spinner::new());
                        ui.label(
                            RichText::new("Detecting receiver features…").color(theme::TEXT_MUTED),
                        );
                    }
                });

                match &self.discovery.result {
                    Some(Ok(found)) if found.is_empty() => {
                        ui.colored_label(
                            theme::WARNING,
                            "No receivers answered. Is network control enabled on the receiver?",
                        );
                    }
                    Some(Ok(found)) => {
                        let mut chosen = None;
                        for r in found {
                            ui.horizontal(|ui| {
                                ui.label(format!("Onkyo {} at {}", r.model, r.host));
                                if theme::button(ui, "Use").clicked() {
                                    chosen = Some(r.clone());
                                }
                            });
                        }
                        if let Some(r) = chosen {
                            self.cfg.receiver.host = r.host;
                            self.cfg.receiver.port = r.port;
                            self.refresh_receiver(ui.ctx());
                        }
                    }
                    Some(Err(e)) => {
                        ui.colored_label(theme::DANGER, format!("Search failed: {e}"));
                    }
                    None => {}
                }
                if let Some(Err(e)) = &self.receiver.result {
                    ui.colored_label(theme::DANGER, format!("Can't reach receiver: {e}"));
                }
            },
        );

        if self.receiver_setup {
            ui.add_space(12.0);
            self.receiver_settings_ui(ui);
            return;
        }
        let Some(status) = self.receiver.ok().cloned() else {
            return;
        };
        ui.add_space(12.0);
        let mut cmd: Option<String> = None;
        self.controls(ui, &status, &mut cmd);
        // One command at a time; clicks while one is in flight are dropped.
        if let Some(c) = cmd
            && !self.command.running()
        {
            self.send_command(ui.ctx(), c);
        }
    }

    fn status_page(&mut self, ui: &mut egui::Ui) {
        let cfg = self.cfg.clone();
        let st = self.receiver.ok().cloned();
        // Only monitors whose input can be switched (DDC/CI).
        let monitors: Vec<MonitorInfo> = self
            .monitors
            .ok()
            .map(|l| l.iter().filter(|m| m.ddc).cloned().collect())
            .unwrap_or_default();
        let dash = || "–".to_string();

        self.front_panel(ui, st.as_ref());
        if let Some(Err(e)) = &self.receiver.result {
            ui.add_space(6.0);
            ui.colored_label(theme::DANGER, format!("Can't reach receiver: {e}"));
        }
        ui.add_space(18.0);

        let Some(st) = st else {
            self.monitor_switch_cards(ui, &monitors);
            return;
        };

        let mut cmd: Option<String> = None;
        self.input_and_volume(ui, &st, &mut cmd);
        if let Some(c) = cmd {
            self.send_command(ui.ctx(), c);
        }
        ui.add_space(22.0);

        let extra = |g: &str| st.extra.get(g).cloned();
        theme::columns(ui, 3, |cols| {
            theme::spec(&mut cols[0], "Video", |ui| {
                let v = st.video.clone().unwrap_or_default();
                theme::kv(ui, "Input port", nonempty(&v.input_port));
                theme::kv(ui, "Input resolution", nonempty(&v.input_resolution));
                theme::kv(ui, "Input colour", nonempty(&v.input_color));
                theme::kv(ui, "Output port", nonempty(&v.output_port));
                theme::kv(ui, "Output resolution", nonempty(&v.output_resolution));
                theme::kv(ui, "Output colour", nonempty(&v.output_color));
                theme::kv(ui, "Picture mode", nonempty(&v.picture_mode));
            });
            theme::spec(&mut cols[1], "Audio", |ui| {
                let a = st.audio_info.clone().unwrap_or_default();
                theme::kv_colored(
                    ui,
                    "Format",
                    nonempty(&a.format),
                    if a.format.is_empty() {
                        theme::TEXT
                    } else {
                        theme::ACCENT
                    },
                );
                theme::kv(ui, "Input port", nonempty(&a.input_port));
                theme::kv(ui, "Sample rate", nonempty(&a.sample_rate));
                theme::kv(ui, "Input channels", nonempty(&a.input_channels));
                theme::kv(ui, "Listening mode", nonempty(&a.listening_mode));
                theme::kv(ui, "Output channels", nonempty(&a.output_channels));
                theme::kv(ui, "Output sample rate", nonempty(&a.output_sample_rate));
            });
            theme::spec(&mut cols[2], "Receiver", |ui| {
                theme::kv_colored(
                    ui,
                    "Power",
                    if st.power_on { "On" } else { "Standby" },
                    if st.power_on {
                        theme::SUCCESS
                    } else {
                        theme::WARNING
                    },
                );
                theme::kv(
                    ui,
                    "Input",
                    st.input
                        .as_deref()
                        .map(|i| format!("{} ({i})", cfg.input_label(i)))
                        .unwrap_or_else(dash),
                );
                theme::kv(
                    ui,
                    "Mute",
                    if st.muted == Some(true) { "On" } else { "Off" },
                );
                theme::kv(
                    ui,
                    "HDMI output",
                    st.hdmi_output
                        .as_deref()
                        .map(eiscp::hdmi_output_name)
                        .unwrap_or_else(dash),
                );
                let hao = extra("HAO").map(|v| match v.as_str() {
                    "00" => "Off".to_string(),
                    "01" => "On".to_string(),
                    "02" => "Auto".to_string(),
                    o => o.to_string(),
                });
                theme::kv(ui, "HDMI audio out", hao.unwrap_or_else(dash));
                let sla = extra("SLA").map(|v| {
                    match v.as_str() {
                        "00" => "Auto",
                        "01" => "Multichannel",
                        "02" => "Analog",
                        "03" => "iLINK",
                        "04" => "HDMI",
                        "05" => "Coax/Optical",
                        "06" => "Balanced",
                        "07" => "ARC",
                        _ => "?",
                    }
                    .to_string()
                });
                theme::kv(ui, "Audio selector", sla.unwrap_or_else(dash));
                // Zones, only when the receiver has them.
                for (label, pwr, sel, vol, mute) in [
                    ("Zone 2", "ZPW", "SLZ", "ZVL", "ZMT"),
                    ("Zone 3", "PW3", "SL3", "VL3", "MT3"),
                ] {
                    if !st.extra.contains_key(pwr) {
                        continue;
                    }
                    if extra(pwr).as_deref() != Some("01") {
                        theme::kv_colored(ui, label, "Standby", theme::TEXT_MUTED);
                        continue;
                    }
                    let input = extra(sel)
                        .map(|c| {
                            if c == "80" {
                                "Same as main".to_string()
                            } else {
                                cfg.input_label(&c)
                            }
                        })
                        .unwrap_or_else(dash);
                    let volume = if extra(mute).as_deref() == Some("01") {
                        "Muted".to_string()
                    } else {
                        extra(vol)
                            .and_then(|v| u8::from_str_radix(&v, 16).ok())
                            .map(|v| v.to_string())
                            .unwrap_or_else(dash)
                    };
                    theme::kv(ui, label, format!("{input} · {volume}"));
                }
            });
        });
        ui.add_space(26.0);
        self.monitor_switch_cards(ui, &monitors);
    }

    /// The big front-panel display at the top of the Status page.
    fn front_panel(&self, ui: &mut egui::Ui, st: Option<&ReceiverStatus>) {
        use theme::VfdState;
        let cfg = &self.cfg;
        let no_indicators: Vec<(&str, bool)> = Vec::new();
        let host = cfg.receiver.host.trim();
        let (state, source, value, unit, left, right) = match (&self.receiver.result, st) {
            _ if host.is_empty() => (
                VfdState::Dim,
                "NOT SET UP".to_string(),
                String::new(),
                "",
                "Add the receiver on the Receiver tab".to_string(),
                String::new(),
            ),
            (_, Some(s)) if s.power_on => {
                let muted = s.muted == Some(true);
                (
                    VfdState::On,
                    s.input
                        .as_deref()
                        .map(|i| cfg.input_label(i).to_uppercase())
                        .unwrap_or_else(|| "–".into()),
                    if muted {
                        "MUTE".to_string()
                    } else {
                        s.volume.map(|v| cfg.volume_text(v)).unwrap_or_default()
                    },
                    if muted || s.volume.is_none() {
                        ""
                    } else {
                        "VOL"
                    },
                    s.listening_mode
                        .as_deref()
                        .map(eiscp::listening_mode_name)
                        .unwrap_or_default(),
                    s.hdmi_output
                        .as_deref()
                        .map(|o| format!("HDMI {}", eiscp::hdmi_output_name(o)))
                        .unwrap_or_default(),
                )
            }
            (_, Some(s)) => (
                VfdState::Dim,
                "STANDBY".to_string(),
                String::new(),
                "",
                s.input
                    .as_deref()
                    .map(|i| format!("Last: {}", cfg.input_label(i)))
                    .unwrap_or_default(),
                String::new(),
            ),
            (Some(Err(_)), _) => (
                VfdState::Error,
                "NO LINK".to_string(),
                String::new(),
                "",
                format!("Retrying {host}"),
                String::new(),
            ),
            _ => (
                VfdState::Dim,
                "CONNECTING".to_string(),
                String::new(),
                "",
                host.to_string(),
                String::new(),
            ),
        };

        let indicators = match st {
            Some(s) if s.power_on => {
                let v = s.video.clone().unwrap_or_default();
                let a = s.audio_info.clone().unwrap_or_default();
                let mode = s
                    .listening_mode
                    .as_deref()
                    .map(eiscp::listening_mode_name)
                    .unwrap_or_default();
                let audio = format!("{} {} {}", a.format, a.listening_mode, mode).to_lowercase();
                let res = format!("{} {}", v.input_resolution, v.output_resolution);
                let extra = |g: &str| s.extra.get(g).map(String::as_str);
                vec![
                    ("HDMI", v.input_port.to_uppercase().contains("HDMI")),
                    ("ATMOS", audio.contains("atmos")),
                    ("DTS:X", audio.contains("dts:x") || audio.contains("dts-x")),
                    (
                        "4K",
                        res.contains("3840") || res.contains("4096") || res.contains("4K"),
                    ),
                    (
                        "HDR",
                        format!("{} {}", v.input_color, v.output_color)
                            .to_uppercase()
                            .contains("HDR"),
                    ),
                    ("MUTE", s.muted == Some(true)),
                    ("ZONE 2", extra("ZPW") == Some("01")),
                    (
                        "SLEEP",
                        extra("SLP").is_some_and(|v| v != "00" && v != "OFF"),
                    ),
                ]
            }
            _ => no_indicators,
        };

        theme::vfd(
            ui,
            &theme::Vfd {
                state,
                source: &source,
                value: &value,
                unit,
                left: &left,
                right: &right,
                indicators: &indicators,
            },
        );
    }

    /// Input keys and the volume slider under the display.
    fn input_and_volume(
        &mut self,
        ui: &mut egui::Ui,
        st: &ReceiverStatus,
        cmd: &mut Option<String>,
    ) {
        let cfg = self.cfg.clone();
        let volume_width = 320.0;
        let gap = 18.0;
        let keys_width = (ui.available_width() - volume_width - gap).max(200.0);
        let busy = self.command.running();
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(keys_width);
                theme::section_label(ui, "Input");
                ui.add_space(2.0);
                input_keys(ui, &cfg, st.input.as_deref(), keys_width, busy, cmd);
            });
            ui.add_space(gap);
            ui.vertical(|ui| {
                ui.set_width(volume_width);
                theme::section_label(ui, "Volume");
                ui.add_space(2.0);
                if !cfg.supports("MVL") {
                    return;
                }
                let max = cfg.volume_max().max(1) as f32;
                let current = st
                    .volume
                    .map(|v| {
                        if cfg.receiver.volume_half_steps {
                            v as f32 / 2.0
                        } else {
                            v as f32
                        }
                    })
                    .unwrap_or(0.0);
                if let Some((level, at)) = self.volume_pending
                    && ((level - current).abs() < 0.01 || at.elapsed() > Duration::from_secs(8))
                {
                    self.volume_pending = None;
                }
                let shown = self
                    .volume_want
                    .or(self.volume_pending.map(|(level, _)| level))
                    .unwrap_or(current);
                let half = cfg.receiver.volume_half_steps;
                let step = if half { 0.5 } else { 1.0 };
                let text = |level: f32| {
                    if half {
                        format!("{level:.1}")
                    } else {
                        format!("{level:.0}")
                    }
                };
                egui::Frame::new()
                    .fill(theme::CARD)
                    .stroke(egui::Stroke::new(1.0, theme::BORDER))
                    .corner_radius(theme::RADIUS)
                    .inner_margin(egui::Margin::symmetric(14, 12))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        let adjusting = self.volume_want.is_some() || self.volume_pending.is_some();
                        ui.horizontal(|ui| {
                            let value = if st.muted == Some(true) && !adjusting {
                                "MUTE".to_string()
                            } else if st.volume.is_some() || adjusting {
                                text(shown)
                            } else {
                                "–".to_string()
                            };
                            ui.label(
                                RichText::new(value)
                                    .font(egui::FontId::new(22.0, theme::mono_bold()))
                                    .color(if st.power_on {
                                        theme::ACCENT
                                    } else {
                                        theme::TEXT_DIM
                                    }),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        RichText::new(format!("max {}", cfg.volume_max()))
                                            .font(egui::FontId::new(12.0, theme::mono()))
                                            .color(theme::TEXT_DIM),
                                    );
                                },
                            );
                        });
                        ui.add_space(2.0);
                        let resp = theme::volume_slider(ui, shown / max, st.power_on);
                        if !st.power_on {
                            return;
                        }
                        // Click or drag picks a level.
                        if (resp.is_pointer_button_down_on() || resp.dragged())
                            && let Some(pos) = resp.interact_pointer_pos()
                        {
                            let f =
                                ((pos.x - resp.rect.left()) / resp.rect.width()).clamp(0.0, 1.0);
                            let level = (f * max / step).round() * step;
                            if (level - shown).abs() > 0.01 {
                                self.volume_want = Some(level);
                            }
                        }
                        if resp.hovered() {
                            // Take the wheel from the page's scroll area, and
                            // step once per notch's worth of scrolling.
                            let delta = ui
                                .ctx()
                                .input_mut(|i| std::mem::take(&mut i.smooth_scroll_delta).y);
                            self.slider_scroll += delta;
                            const NOTCH: f32 = 24.0;
                            while self.slider_scroll.abs() >= NOTCH {
                                let dir = self.slider_scroll.signum();
                                let base = self.volume_want.unwrap_or(shown);
                                self.volume_want = Some((base + dir * step).clamp(0.0, max));
                                self.slider_scroll -= NOTCH * dir;
                            }
                        } else {
                            self.slider_scroll = 0.0;
                        }
                        // Send the latest pick whenever the previous command is done.
                        if !busy && let Some(level) = self.volume_want.take() {
                            let raw = if half { level * 2.0 } else { level }.round() as u8;
                            *cmd = Some(format!("MVL{raw:02X}"));
                            self.volume_pending = Some((level, Instant::now()));
                        }
                        if self.volume_want.is_some() {
                            ui.ctx().request_repaint();
                        }
                        ui.add_space(2.0);
                        ui.label(
                            RichText::new("Click, drag or scroll")
                                .size(11.5)
                                .color(theme::TEXT_DIM),
                        );
                    });
            });
        });
    }

    /// Compact monitor cards with one-click input switching (Status page).
    fn monitor_switch_cards(&mut self, ui: &mut egui::Ui, list: &[MonitorInfo]) {
        theme::section_label(ui, "Monitors");
        ui.add_space(4.0);
        if let Some(Err(e)) = &self.monitor_test.result {
            ui.colored_label(theme::DANGER, format!("Switch failed: {e}"));
        }
        if list.is_empty() {
            let text = match &self.monitors.result {
                _ if self.monitors.running() => "Scanning monitors…".to_string(),
                Some(Err(e)) => format!("Scan failed: {e}"),
                _ => "No monitors with DDC/CI input switching found. DDC/CI may need turning on in the monitor's own menu.".to_string(),
            };
            ui.label(RichText::new(text).color(theme::TEXT_MUTED));
            return;
        }
        let busy = self.monitor_test.running();
        let mut test: Option<(MonitorId, u8)> = None;
        let per_row = list.len().clamp(2, 3);
        for row in list.chunks(per_row) {
            theme::columns(ui, per_row, |cols| {
                for (ui, m) in cols.iter_mut().zip(row) {
                    let subtitle = format!("{}  ·  {}", m.id.mfg, m.connector);
                    theme::card(ui, icon::MONITOR, &m.id.model, Some(&subtitle), |ui| {
                        theme::kv(
                            ui,
                            "Current input",
                            m.current_input
                                .map(monitors::input_name)
                                .unwrap_or_else(|| "No answer".into()),
                        );
                        theme::kv(ui, "Input signal", signal_text(m));
                        if let Some(d) = &m.mode {
                            theme::kv(
                                ui,
                                "PC output",
                                format!("{} @ {}", d.resolution(), d.refresh()),
                            );
                        }
                        ui.add_space(6.0);
                        ui.label(RichText::new("Switch to").color(theme::TEXT_SECONDARY));
                        ui.horizontal_wrapped(|ui| {
                            for code in monitors::selectable_inputs(Some(m)) {
                                let current = m.current_input == Some(code);
                                if ui
                                    .add_enabled_ui(!busy, |ui| {
                                        theme::choice(ui, monitors::input_name(code), current)
                                    })
                                    .inner
                                    .clicked()
                                {
                                    test = Some((m.id.clone(), code));
                                }
                            }
                        });
                        if let Some(e) = &m.error {
                            ui.colored_label(theme::WARNING, e);
                        }
                    });
                }
            });
            ui.add_space(12.0);
        }
        if let Some((id, code)) = test {
            let ctx = ui.ctx().clone();
            self.monitor_test
                .start(&ctx, move || monitors::set_input(&id, code));
        }
    }

    /// Icon rail: page buttons, and a power button at the foot.
    fn rail(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(2.0);
            ui.label(
                RichText::new(icon::AUDIO_LINES)
                    .size(22.0)
                    .color(theme::ACCENT),
            );
            ui.add_space(14.0);
            for (tab, ic, label) in [
                (Tab::Status, icon::DASHBOARD, "Status"),
                (Tab::Receiver, icon::SLIDERS, "Receiver"),
                (Tab::Monitors, icon::MONITOR, "Monitors"),
                (Tab::Shortcuts, icon::KEYBOARD, "Shortcuts"),
                (Tab::General, icon::SETTINGS, "General"),
            ] {
                if theme::rail_item(ui, ic, label, self.tab == tab).clicked() {
                    self.tab = tab;
                    self.recording = false;
                }
                ui.add_space(2.0);
            }
        });

        let st = self.receiver.ok().cloned();
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
            let Some(st) = st else { return };
            if !self.cfg.supports("PWR") {
                return;
            }
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(42.0, 42.0), egui::Sense::click());
            let color = if st.power_on {
                theme::SUCCESS
            } else {
                theme::TEXT_DIM
            };
            if resp.hovered() {
                ui.painter()
                    .rect_filled(rect, theme::RADIUS, theme::CONTROL);
            }
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                icon::POWER,
                egui::FontId::new(19.0, egui::FontFamily::Proportional),
                color,
            );
            let resp = resp
                .on_hover_text(if st.power_on {
                    "Put the receiver in standby"
                } else {
                    "Turn the receiver on"
                })
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            if resp.clicked() && !self.command.running() {
                let cmd = if st.power_on { "PWR00" } else { "PWR01" };
                self.send_command(ui.ctx(), cmd.into());
            }
        });
    }

    /// Receiver connection state for the device line: (text, colour).
    fn connection(&self) -> (String, Color32) {
        if self.cfg.receiver.host.trim().is_empty() {
            return ("Not set up".into(), theme::WARNING);
        }
        match (&self.receiver.result, self.receiver.ok()) {
            (_, Some(s)) if s.power_on => {
                let input = s
                    .input
                    .as_deref()
                    .map(|i| format!(" · {}", self.cfg.input_label(i)))
                    .unwrap_or_default();
                (format!("On{input}"), theme::SUCCESS)
            }
            (_, Some(_)) => ("Standby".into(), theme::WARNING),
            (Some(Err(_)), _) => ("Unreachable".into(), theme::DANGER),
            _ => ("Connecting…".into(), theme::TEXT_MUTED),
        }
    }

    /// The line above every page: which receiver, whether it's connected
    /// and on, and save state.
    fn device_bar(&mut self, ui: &mut egui::Ui) {
        let mut refresh = false;
        ui.horizontal(|ui| {
            ui.set_min_height(30.0);
            let model = self
                .receiver
                .ok()
                .and_then(|s| s.model.clone())
                .filter(|m| !m.is_empty())
                .unwrap_or_else(|| self.cfg.receiver.model.clone());
            let name = if model.is_empty() {
                "Onkyo receiver".to_string()
            } else {
                format!("Onkyo {model}")
            };
            let host = self.cfg.receiver.host.trim();
            let text = if host.is_empty() {
                name
            } else {
                format!("{name}  ·  {host}")
            };
            ui.label(
                RichText::new(text.to_uppercase())
                    .font(egui::FontId::new(11.5, theme::bold()))
                    .color(theme::TEXT_DIM)
                    .extra_letter_spacing(1.4),
            );
            ui.add_space(10.0);
            let (state, color) = self.connection();
            ui.label(
                RichText::new(format!("●  {state}"))
                    .font(egui::FontId::new(12.5, theme::bold()))
                    .color(color),
            );
            let failing = self
                .monitors
                .ok()
                .map(|l| {
                    l.iter()
                        .filter(|m| m.ddc && m.current_input.is_none())
                        .count()
                })
                .unwrap_or(0);
            if failing > 0 {
                ui.add_space(6.0);
                theme::status_pill(
                    ui,
                    &format!("{failing} monitor(s) not answering"),
                    theme::WARNING,
                    theme::WARNING_BORDER,
                );
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add(egui::Button::new(icon::REFRESH).min_size(egui::vec2(30.0, 28.0)))
                    .on_hover_text("Refresh now")
                    .clicked()
                {
                    refresh = true;
                }
                ui.add_space(6.0);
                if self.dirty() {
                    if theme::button(ui, format!("{}  Revert", icon::UNDO)).clicked() {
                        self.cfg = self.saved.clone();
                        self.selected = self
                            .selected
                            .min(self.cfg.shortcuts.len().saturating_sub(1));
                    }
                    if theme::primary_button(ui, format!("{}  Save", icon::SAVE)).clicked() {
                        self.save();
                    }
                    ui.label(RichText::new("Unsaved changes").color(theme::WARNING));
                } else {
                    ui.label(
                        RichText::new(format!("{}  All changes saved", icon::CHECK))
                            .size(12.5)
                            .color(theme::TEXT_DIM),
                    );
                }
                if let Some((msg, error, at)) = &self.status
                    && at.elapsed() < Duration::from_secs(12)
                {
                    let color = if *error {
                        theme::DANGER
                    } else {
                        theme::SUCCESS
                    };
                    ui.add(egui::Label::new(RichText::new(msg).color(color).size(12.5)).truncate());
                }
            });
        });
        if refresh {
            self.refresh_receiver(ui.ctx());
            self.scan_monitors(ui.ctx());
        }
    }

    fn controls(&mut self, ui: &mut egui::Ui, st: &ReceiverStatus, cmd: &mut Option<String>) {
        let cfg = self.cfg.clone();
        let extra = |g: &str| st.extra.get(g).map(String::as_str);
        // Only the options this model accepts (all of them if it's unknown).
        let opts = |group: &str,
                    list: &[(&'static str, &'static str)]|
         -> Vec<(&'static str, &'static str)> {
            list.iter()
                .copied()
                .filter(|(code, _)| cfg.supports_value(group, code))
                .collect()
        };
        let inputs = |group: &str| -> Vec<(&'static str, String)> {
            eiscp::INPUTS
                .iter()
                .filter(|(c, _)| {
                    !cfg.receiver.hidden_inputs.iter().any(|h| h == c)
                        && cfg.supports_value(group, c)
                })
                .map(|(c, _)| (*c, cfg.input_label(c)))
                .collect()
        };
        const ON_OFF: &[(&str, &str)] = &[("00", "Off"), ("01", "On")];

        if cfg.supports("SLI") {
            theme::card(ui, icon::CABLE, "Input", None, |ui| {
                let width = ui.available_width();
                input_keys(ui, &cfg, st.input.as_deref(), width, false, cmd);
            });
            ui.add_space(12.0);
        }

        let volume_target = &mut self.volume_target;
        theme::columns(ui, 2, |cols| {
            theme::card(&mut cols[0], icon::POWER, "Power and volume", None, |ui| {
                controls_grid(ui, "main", |ui| {
                    if cfg.supports("PWR") {
                        let power = Some(if st.power_on { "01" } else { "00" });
                        choice_row(
                            ui,
                            "Power",
                            power,
                            &[("01", "On"), ("00", "Standby")],
                            "PWR",
                            cmd,
                        );
                    }
                    if cfg.supports("MVL") {
                        ui.label(RichText::new("Volume").color(theme::TEXT_SECONDARY));
                        ui.horizontal(|ui| {
                            if ui
                                .add(egui::Button::new("−").min_size(egui::vec2(30.0, 28.0)))
                                .clicked()
                            {
                                *cmd = Some("MVLDOWN".into());
                            }
                            let text = st.volume.map(|v| cfg.volume_text(v)).unwrap_or("–".into());
                            ui.add_sized(
                                [44.0, 28.0],
                                egui::Label::new(
                                    RichText::new(text)
                                        .font(egui::FontId::new(16.0, theme::mono_bold()))
                                        .color(theme::ACCENT),
                                ),
                            );
                            if ui
                                .add(egui::Button::new("+").min_size(egui::vec2(30.0, 28.0)))
                                .clicked()
                            {
                                *cmd = Some("MVLUP".into());
                            }
                            if cfg.supports("AMT") {
                                ui.add_space(6.0);
                                if theme::choice(ui, "Mute", st.muted == Some(true)).clicked() {
                                    *cmd = Some("AMTTG".into());
                                }
                            }
                        });
                        ui.end_row();

                        ui.label(RichText::new("Set volume to").color(theme::TEXT_SECONDARY));
                        ui.horizontal(|ui| {
                            ui.add(egui::DragValue::new(volume_target).range(0..=cfg.volume_max()));
                            if theme::button(ui, "Set").clicked() {
                                let raw = cfg.volume_raw(*volume_target);
                                *cmd = Some(format!("MVL{raw:02X}"));
                            }
                        });
                        ui.end_row();
                    }
                    if cfg.supports("LMD") {
                        ui.label(RichText::new("Listening mode").color(theme::TEXT_SECONDARY));
                        let current = st
                            .listening_mode
                            .as_deref()
                            .map(eiscp::listening_mode_name)
                            .unwrap_or_else(|| "Choose…".into());
                        egui::ComboBox::from_id_salt("ctl_lmd")
                            .selected_text(current)
                            .width(220.0)
                            .height(400.0)
                            .show_ui(ui, |ui| {
                                for (code, name) in opts("LMD", eiscp::LISTENING_MODES) {
                                    let sel = st.listening_mode.as_deref() == Some(code);
                                    if ui.selectable_label(sel, name).clicked() {
                                        *cmd = Some(format!("LMD{code}"));
                                    }
                                }
                            });
                        ui.end_row();

                        let steps = opts(
                            "LMD",
                            &[
                                ("DOWN", "◀"),
                                ("UP", "▶"),
                                ("MOVIE", "Movie"),
                                ("MUSIC", "Music"),
                                ("GAME", "Game"),
                            ],
                        );
                        if !steps.is_empty() {
                            ui.label("");
                            ui.horizontal(|ui| {
                                for (code, label) in steps {
                                    if ui.button(label).clicked() {
                                        *cmd = Some(format!("LMD{code}"));
                                    }
                                }
                            });
                            ui.end_row();
                        }
                    }
                });
            });
            settings_card(
                &mut cols[1],
                icon::AUDIO_LINES,
                "Sound",
                &cfg,
                st,
                &[
                    (
                        "SLA",
                        "Audio selector",
                        &[
                            ("00", "Auto"),
                            ("01", "Multichannel"),
                            ("02", "Analog"),
                            ("03", "iLINK"),
                            ("04", "HDMI"),
                            ("05", "Coax/Optical"),
                            ("06", "Balanced"),
                            ("07", "ARC"),
                        ],
                    ),
                    (
                        "AEQ",
                        "AccuEQ",
                        &[("00", "Off"), ("01", "On"), ("02", "On (except front)")],
                    ),
                    ("MOT", "Music Optimizer", ON_OFF),
                    (
                        "LTN",
                        "Late night",
                        &[("00", "Off"), ("01", "Low"), ("02", "High"), ("03", "Auto")],
                    ),
                ],
                cmd,
            );
        });
        ui.add_space(12.0);

        theme::columns(ui, 2, |cols| {
            settings_card(
                &mut cols[0],
                icon::HDMI,
                "HDMI",
                &cfg,
                st,
                &[
                    (
                        "HDO",
                        "Output",
                        &[("01", "Main"), ("02", "Sub"), ("03", "Main + Sub")],
                    ),
                    (
                        "HAO",
                        "Audio out",
                        &[("00", "Off"), ("01", "On"), ("02", "Auto")],
                    ),
                    ("HAS", "Audio out (sub)", &[("00", "Off"), ("01", "On")]),
                    ("CEC", "HDMI CEC", ON_OFF),
                ],
                cmd,
            );
            settings_card(
                &mut cols[1],
                icon::SETTINGS,
                "System",
                &cfg,
                st,
                &[
                    (
                        "DIM",
                        "Display",
                        &[
                            ("00", "Bright"),
                            ("01", "Dim"),
                            ("02", "Dark"),
                            ("03", "Off"),
                        ],
                    ),
                    (
                        "SLP",
                        "Sleep timer",
                        &[
                            ("OFF", "Off"),
                            ("1E", "30 min"),
                            ("3C", "60 min"),
                            ("5A", "90 min"),
                        ],
                    ),
                    ("TGA", "12 V trigger A", ON_OFF),
                    ("TGB", "12 V trigger B", ON_OFF),
                    ("TGC", "12 V trigger C", ON_OFF),
                ],
                cmd,
            );
        });

        // Zones: one card each, side by side.
        let zones: Vec<(&str, &str, &str, &str, &str)> = [
            ("Zone 2", "ZPW", "ZVL", "ZMT", "SLZ"),
            ("Zone 3", "PW3", "VL3", "MT3", "SL3"),
        ]
        .into_iter()
        .filter(|z| cfg.supports(z.1))
        .collect();
        if !zones.is_empty() {
            theme::columns(ui, 2, |cols| {
                for (ui, (label, pwr, vol, mute, sel)) in cols.iter_mut().zip(&zones) {
                    theme::card(ui, icon::HOUSE, label, None, |ui| {
                        controls_grid(ui, label, |ui| {
                            choice_row(
                                ui,
                                "Power",
                                extra(pwr),
                                &[("01", "On"), ("00", "Standby")],
                                pwr,
                                cmd,
                            );
                            if cfg.supports(vol) {
                                ui.label(RichText::new("Volume").color(theme::TEXT_SECONDARY));
                                ui.horizontal(|ui| {
                                    if ui
                                        .add(
                                            egui::Button::new("−").min_size(egui::vec2(30.0, 28.0)),
                                        )
                                        .clicked()
                                    {
                                        *cmd = Some(format!("{vol}DOWN"));
                                    }
                                    let v = extra(vol)
                                        .and_then(|v| u8::from_str_radix(v, 16).ok())
                                        .map(|v| v.to_string())
                                        .unwrap_or("–".into());
                                    ui.add_sized(
                                        [44.0, 28.0],
                                        egui::Label::new(
                                            RichText::new(v)
                                                .font(egui::FontId::new(16.0, theme::mono_bold()))
                                                .color(theme::ACCENT),
                                        ),
                                    );
                                    if ui
                                        .add(
                                            egui::Button::new("+").min_size(egui::vec2(30.0, 28.0)),
                                        )
                                        .clicked()
                                    {
                                        *cmd = Some(format!("{vol}UP"));
                                    }
                                    if cfg.supports(mute) {
                                        ui.add_space(6.0);
                                        if theme::choice(ui, "Mute", extra(mute) == Some("01"))
                                            .clicked()
                                        {
                                            *cmd = Some(format!("{mute}TG"));
                                        }
                                    }
                                });
                                ui.end_row();
                            }
                            if cfg.supports(sel) {
                                ui.label(RichText::new("Input").color(theme::TEXT_SECONDARY));
                                let name = |c: &str| {
                                    if c == "80" {
                                        "Same as main".to_string()
                                    } else {
                                        cfg.input_label(c)
                                    }
                                };
                                let current =
                                    extra(sel).map(name).unwrap_or_else(|| "Input…".into());
                                egui::ComboBox::from_id_salt(("ctl_zone", *sel))
                                    .selected_text(current)
                                    .width(170.0)
                                    .show_ui(ui, |ui| {
                                        if cfg.supports_value(sel, "80")
                                            && ui
                                                .selectable_label(
                                                    extra(sel) == Some("80"),
                                                    "Same as main",
                                                )
                                                .clicked()
                                        {
                                            *cmd = Some(format!("{sel}80"));
                                        }
                                        for (code, label) in inputs(sel) {
                                            if ui
                                                .selectable_label(extra(sel) == Some(code), label)
                                                .clicked()
                                            {
                                                *cmd = Some(format!("{sel}{code}"));
                                            }
                                        }
                                    });
                                ui.end_row();
                            }
                        });
                    });
                }
            });
            ui.add_space(12.0);
        }

        theme::card(
            ui,
            icon::ZAP,
            "Raw command",
            Some("Any eISCP command, for anything not covered above."),
            |ui| {
                ui.horizontal(|ui| {
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut self.raw_command)
                            .hint_text("e.g. LMD0C")
                            .desired_width(160.0)
                            .margin(egui::vec2(8.0, 6.0))
                            .font(egui::FontId::new(13.0, theme::mono())),
                    );
                    let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if (theme::button(ui, "Send").clicked() || enter)
                        && !self.raw_command.trim().is_empty()
                    {
                        *cmd = Some(self.raw_command.trim().to_ascii_uppercase());
                    }
                    if !self.last_command.is_empty() {
                        ui.add_space(8.0);
                        ui.label(
                            RichText::new(&self.last_command)
                                .font(egui::FontId::new(12.5, theme::mono()))
                                .color(theme::TEXT_MUTED),
                        );
                    }
                });
            },
        );
    }

    /// The "Receiver settings…" window: one-time setup that rarely changes.
    /// The Receiver tab's Setup view: one-time setup that rarely changes.
    fn receiver_settings_ui(&mut self, ui: &mut egui::Ui) {
        theme::card(
            ui,
            icon::VOLUME,
            "Volume",
            Some("How the receiver counts volume."),
            |ui| {
                controls_grid(ui, "rs_volume", |ui| {
                    ui.label(RichText::new("Volume steps").color(theme::TEXT_SECONDARY));
                    let mut half = usize::from(self.cfg.receiver.volume_half_steps);
                    if let Some(i) =
                        theme::segmented(ui, "rs_steps", &["Whole (25)", "Half (25.5)"], Some(half))
                    {
                        half = i;
                        self.cfg.receiver.volume_half_steps = half == 1;
                    }
                    ui.end_row();

                    ui.label(RichText::new("Full-scale volume").color(theme::TEXT_SECONDARY))
                        .on_hover_text(
                            "The volume (as shown on the receiver) that fills the volume bar.",
                        );
                    ui.add(egui::DragValue::new(&mut self.cfg.receiver.max_volume).range(1..=200));
                    ui.end_row();
                });
                ui.add_space(4.0);
                ui.add(
                    egui::Label::new(
                        RichText::new(
                            "Use half steps only if the receiver's front panel shows them \
                                 (e.g. 25.5).",
                        )
                        .size(12.5)
                        .color(theme::TEXT_DIM),
                    )
                    .wrap(),
                );
            },
        );

        ui.add_space(12.0);

        // Features and inputs side by side.
        theme::columns(ui, 2, |cols| {
            let can = !self.cfg.receiver.host.trim().is_empty() && !self.probe.running();
            let mut detect = false;
            theme::card_with(
                &mut cols[0],
                icon::LAYERS,
                "Supported features",
                Some("Detected the first time the receiver is reached, and saved."),
                |ui| {
                    if ui
                        .add_enabled(can, egui::Button::new("Detect again"))
                        .clicked()
                    {
                        detect = true;
                    }
                    if self.probe.running() {
                        ui.add(egui::Spinner::new());
                    }
                },
                |ui| {
                    if let Some(t) = self.cfg.model_table() {
                        ui.add(
                            egui::Label::new(
                                RichText::new(format!(
                                    "Onkyo {} is in the protocol spreadsheet ({}): {} commands, \
                                     with the exact options per command.",
                                    self.cfg.receiver.model,
                                    crate::models::source(),
                                    t.len()
                                ))
                                .size(12.5)
                                .color(theme::TEXT_DIM),
                            )
                            .wrap(),
                        );
                    } else if !self.cfg.receiver.features_source.is_empty() {
                        ui.label(
                            RichText::new(format!("Source: {}", self.cfg.receiver.features_source))
                                .size(12.5)
                                .color(theme::TEXT_DIM),
                        );
                    }
                    if self.probe.running() {
                        ui.label("Asking the receiver (takes a few seconds)…");
                    }
                    if let Some(Err(e)) = &self.probe.result {
                        ui.colored_label(theme::DANGER, format!("Detection failed: {e}"));
                    }
                    ui.add_space(6.0);
                    let features = self.cfg.effective_features();
                    if features.is_empty() {
                        ui.label("Not detected yet.");
                        return;
                    }
                    egui::Grid::new("features")
                        .num_columns(4)
                        .spacing([18.0, 4.0])
                        .show(ui, |ui| {
                            let mut col = 0;
                            for (group, label) in eiscp::FEATURES {
                                let Some(ok) = features.get(*group) else {
                                    continue;
                                };
                                ui.label(
                                    RichText::new(*label)
                                        .size(12.5)
                                        .color(theme::TEXT_SECONDARY),
                                )
                                .on_hover_text(format!("{group}QSTN"));
                                ui.label(
                                    RichText::new(if *ok { "yes" } else { "no" })
                                        .font(egui::FontId::new(12.0, theme::mono()))
                                        .color(if *ok { theme::SUCCESS } else { theme::TEXT_DIM }),
                                );
                                col += 1;
                                if col % 2 == 0 {
                                    ui.end_row();
                                }
                            }
                        });
                },
            );
            if detect {
                self.start_probe(cols[0].ctx());
            }

            let current_input = self.receiver.ok().and_then(|s| s.input.clone());
            let reported = self.cfg.receiver.features.get("NRI") == Some(&true);
            theme::card(
                &mut cols[1],
                icon::CABLE,
                "Inputs",
                Some(if reported {
                    "Only the inputs the receiver reported are ticked. Untick any you don't want, and rename them if you like."
                } else {
                    "This receiver doesn't report its inputs, so all known inputs are listed. Rename the ones you use and untick the rest."
                }),
                |ui| {
                    egui::Grid::new("inputs")
                        .num_columns(4)
                        .spacing([16.0, 6.0])
                        .show(ui, |ui| {
                            for h in ["Show", "Code", "Default name", "Your label"] {
                                theme::section_label(ui, h);
                            }
                            ui.end_row();
                            for (code, default) in eiscp::INPUTS {
                                let mut shown =
                                    !self.cfg.receiver.hidden_inputs.iter().any(|c| c == code);
                                if ui.checkbox(&mut shown, "").changed() {
                                    if shown {
                                        self.cfg.receiver.hidden_inputs.retain(|c| c != code);
                                    } else {
                                        self.cfg.receiver.hidden_inputs.push(code.to_string());
                                    }
                                }
                                let active = current_input.as_deref() == Some(*code);
                                ui.label(
                                    RichText::new(*code)
                                        .font(egui::FontId::new(12.5, theme::mono()))
                                        .color(if active {
                                            theme::ACCENT
                                        } else {
                                            theme::TEXT_DIM
                                        }),
                                );
                                ui.label(if active {
                                    RichText::new(format!("{default}  (active)"))
                                        .color(theme::ACCENT)
                                } else {
                                    RichText::new(*default).color(theme::TEXT_SECONDARY)
                                });
                                let mut label = self
                                    .cfg
                                    .receiver
                                    .input_names
                                    .get(*code)
                                    .cloned()
                                    .unwrap_or_default();
                                if ui
                                    .add(
                                        egui::TextEdit::singleline(&mut label)
                                            .desired_width(120.0)
                                            .hint_text(*default),
                                    )
                                    .changed()
                                {
                                    if label.is_empty() {
                                        self.cfg.receiver.input_names.remove(*code);
                                    } else {
                                        self.cfg
                                            .receiver
                                            .input_names
                                            .insert(code.to_string(), label);
                                    }
                                }
                                ui.end_row();
                            }
                        });
                },
            );
        });
    }

    fn monitors_tab(&mut self, ui: &mut egui::Ui) {
        let mut rescan = false;
        let scanning = self.monitors.running();
        theme::page_header(
            ui,
            "Monitors",
            "Everything the connected displays report. Switch inputs from the Status page.",
            |ui| {
                if ui
                    .add_enabled(
                        !scanning,
                        egui::Button::new(format!("{}  Rescan", icon::REFRESH))
                            .min_size(egui::vec2(0.0, 32.0)),
                    )
                    .clicked()
                {
                    rescan = true;
                }
                if scanning {
                    ui.add(egui::Spinner::new());
                }
            },
        );
        if rescan {
            self.scan_monitors(ui.ctx());
        }
        if let Some(Err(e)) = &self.monitor_test.result {
            ui.colored_label(theme::DANGER, format!("Switch failed: {e}"));
        }

        let list = match &self.monitors.result {
            Some(Ok(list)) => list.clone(),
            Some(Err(e)) => {
                ui.colored_label(theme::DANGER, format!("Scan failed: {e}"));
                #[cfg(target_os = "linux")]
                ui.label(
                    "Make sure ddcutil is installed and you can access /dev/i2c-* (group \"i2c\").",
                );
                return;
            }
            None => {
                ui.label(RichText::new("Querying monitors over DDC/CI…").color(theme::TEXT_MUTED));
                return;
            }
        };

        // Only displays whose input can be switched over DDC/CI.
        let list: Vec<MonitorInfo> = list.into_iter().filter(|m| m.ddc).collect();
        if list.is_empty() {
            ui.label(
                RichText::new("No monitors with DDC/CI input switching found. DDC/CI may need turning on in the monitor's own menu.")
                    .color(theme::TEXT_MUTED),
            );
            return;
        }
        egui::ScrollArea::vertical().show(ui, |ui| {
            // Keeps cards clear of the floating scrollbar.
            egui::Frame::new()
                .inner_margin(egui::Margin {
                    right: 14,
                    ..Default::default()
                })
                .show(ui, |ui| {
                    let per_row = list.len().clamp(2, 3);
                    for row in list.chunks(per_row) {
                        theme::columns(ui, per_row, |cols| {
                            for (col, m) in cols.iter_mut().zip(row) {
                                monitor_card(col, m);
                            }
                        });
                        ui.add_space(12.0);
                    }
                });
        });
    }

    fn shortcuts_tab(&mut self, ui: &mut egui::Ui) {
        let mut add = false;
        theme::page_header(
            ui,
            "Shortcuts",
            "Hotkeys that switch the receiver and monitors, or control the receiver.",
            |ui| {
                if theme::primary_button(ui, format!("{}  New shortcut", icon::PLUS)).clicked() {
                    add = true;
                }
            },
        );
        if add {
            let mut n = self.cfg.shortcuts.len() + 1;
            while self.cfg.shortcut(&format!("shortcut{n}")).is_some() {
                n += 1;
            }
            self.cfg.shortcuts.push(Shortcut {
                id: format!("shortcut{n}"),
                name: format!("New shortcut {n}"),
                hotkey: String::new(),
                action: Action::from_kind("switch"),
            });
            self.selected = self.cfg.shortcuts.len() - 1;
        }

        let area = ui.available_rect_before_wrap();
        let height = area.height();
        let left = egui::Rect::from_min_size(area.min, egui::vec2(290.0, height));
        let right = egui::Rect::from_min_max(egui::pos2(left.right() + 12.0, area.top()), area.max);
        {
            ui.scope_builder(egui::UiBuilder::new().max_rect(left), |ui| {
                let has = self.selected < self.cfg.shortcuts.len();
                let can_down = has && self.selected + 1 < self.cfg.shortcuts.len();
                let can_up = has && self.selected > 0;
                let mut moved: i32 = 0;
                theme::card_with(
                    ui,
                    icon::KEYBOARD,
                    "All shortcuts",
                    None,
                    |ui| {
                        if ui
                            .add_enabled(can_down, egui::Button::new(icon::DOWN))
                            .on_hover_text("Move down")
                            .clicked()
                        {
                            moved = 1;
                        }
                        if ui
                            .add_enabled(can_up, egui::Button::new(icon::UP))
                            .on_hover_text("Move up")
                            .clicked()
                        {
                            moved = -1;
                        }
                    },
                    |ui| {
                        egui::ScrollArea::vertical()
                            .id_salt("shortcut_list")
                            .max_height(height - 110.0)
                            .show(ui, |ui| {
                                for i in 0..self.cfg.shortcuts.len() {
                                    let s = &self.cfg.shortcuts[i];
                                    let hk = if s.hotkey.is_empty() {
                                        "No hotkey".to_string()
                                    } else {
                                        s.hotkey.clone()
                                    };
                                    if theme::list_item(ui, &s.name, &hk, i == self.selected)
                                        .clicked()
                                    {
                                        self.selected = i;
                                        self.recording = false;
                                    }
                                }
                            });
                    },
                );
                if moved != 0 {
                    let other = (self.selected as i32 + moved) as usize;
                    self.cfg.shortcuts.swap(self.selected, other);
                    self.selected = other;
                }
            });
            ui.scope_builder(egui::UiBuilder::new().max_rect(right), |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("shortcut_editor")
                    .show(ui, |ui| {
                        if self.selected >= self.cfg.shortcuts.len() {
                            ui.label("Add a shortcut to get started.");
                            return;
                        }
                        let title = self.cfg.shortcuts[self.selected].name.clone();
                        let kind =
                            Action::kind_label(self.cfg.shortcuts[self.selected].action.kind());
                        let mut remove = false;
                        theme::card_with(
                            ui,
                            icon::SLIDERS,
                            &title,
                            Some(kind),
                            |ui| {
                                if ui
                                    .add(egui::Button::new(
                                        RichText::new(format!("{}  Delete", icon::TRASH))
                                            .color(theme::DANGER),
                                    ))
                                    .clicked()
                                {
                                    remove = true;
                                }
                            },
                            |ui| self.shortcut_editor(ui),
                        );
                        if remove {
                            self.cfg.shortcuts.remove(self.selected);
                            self.selected = self.selected.saturating_sub(1);
                        }
                    });
            });
        }
        ui.advance_cursor_after_rect(area);
    }

    fn shortcut_editor(&mut self, ui: &mut egui::Ui) {
        if self.selected >= self.cfg.shortcuts.len() {
            ui.label("Add a shortcut to get started.");
            return;
        }

        // Hotkey recording: grab the next key press with its modifiers.
        if self.recording {
            let pressed = ui.ctx().input(|i| {
                i.events.iter().find_map(|e| match e {
                    egui::Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        ..
                    } => Some((*key, *modifiers)),
                    _ => None,
                })
            });
            if let Some((key, mods)) = pressed {
                if key == egui::Key::Escape && !mods.any() {
                    self.recording = false;
                } else if let Some(h) = Hotkey::from_egui(key, mods) {
                    self.cfg.shortcuts[self.selected].hotkey = h.to_string();
                    self.recording = false;
                }
            }
        }

        let monitor_list: Vec<MonitorInfo> = self.monitors.ok().cloned().unwrap_or_default();
        let visible_inputs: Vec<(String, String)> = eiscp::INPUTS
            .iter()
            .filter(|(c, _)| !self.cfg.receiver.hidden_inputs.iter().any(|h| h == c))
            .map(|(c, _)| (c.to_string(), self.cfg.input_label(c)))
            .collect();
        let label_cfg = self.cfg.clone();
        let support_cfg = self.cfg.clone();
        let filtered = |group: &str,
                        list: &[(&'static str, &'static str)]|
         -> Vec<(&'static str, &'static str)> {
            list.iter()
                .copied()
                .filter(|(c, _)| support_cfg.supports_value(group, c))
                .collect()
        };
        let lmd_modes = filtered("LMD", eiscp::LISTENING_MODES);
        let lmd_steps = filtered("LMD", eiscp::LISTENING_MODE_STEPS);
        let hdo_options = filtered("HDO", eiscp::HDMI_OUTPUTS);
        let volume_max = self.cfg.volume_max();
        let input_label = move |code: &str| format!("{} ({code})", label_cfg.input_label(code));
        let dirty = self.dirty();
        let daemon_running = self.daemon_running;
        let mut run_now = false;
        #[cfg_attr(not(target_os = "linux"), allow(unused_mut))]
        let mut configure_portal = false;

        let s = &mut self.cfg.shortcuts[self.selected];
        egui::Grid::new("shortcut_fields")
            .num_columns(2)
            .spacing([12.0, 8.0])
            .show(ui, |ui| {
                ui.label("Name");
                ui.add(egui::TextEdit::singleline(&mut s.name).desired_width(260.0));
                ui.end_row();

                ui.label("ID").on_hover_text(
                    "Used by `avrcontrol trigger <id>` and the system shortcut settings.",
                );
                ui.add(
                    egui::TextEdit::singleline(&mut s.id)
                        .desired_width(160.0)
                        .font(egui::TextStyle::Monospace),
                );
                ui.end_row();

                ui.label("Hotkey");
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut s.hotkey)
                            .hint_text("e.g. Ctrl+Shift+Alt+F1")
                            .desired_width(200.0),
                    );
                    let label = if self.recording {
                        "Press keys… (Esc cancels)"
                    } else {
                        "Record"
                    };
                    if theme::choice(ui, label, self.recording).clicked() {
                        self.recording = !self.recording;
                    }
                    if ui.button("Clear").clicked() {
                        s.hotkey.clear();
                    }
                });
                ui.end_row();
                if let Err(e) = hotkey::validate(&s.hotkey) {
                    ui.label("");
                    ui.colored_label(err_color(ui), e);
                    ui.end_row();
                }

                ui.label("Action");
                let mut kind = s.action.kind();
                egui::ComboBox::from_id_salt("action_kind")
                    .selected_text(Action::kind_label(kind))
                    .width(280.0)
                    .show_ui(ui, |ui| {
                        for (k, label) in Action::KINDS {
                            let label = if support_cfg.supports_kind(k) {
                                label.to_string()
                            } else {
                                format!("{label} (not supported by receiver)")
                            };
                            ui.selectable_value(&mut kind, k, label);
                        }
                    });
                if kind != s.action.kind() {
                    s.action = Action::from_kind(kind);
                }
                ui.end_row();
            });

        ui.add_space(8.0);
        match &mut s.action {
            Action::Switch {
                receiver_input,
                power_on,
                hdmi_output,
                listening_mode,
                volume,
                extra_commands,
                monitors: targets,
            } => {
                theme::subheading(ui, icon::SPEAKER, "Receiver");
                ui.horizontal(|ui| {
                    let mut enabled = receiver_input.is_some();
                    if ui
                        .checkbox(&mut enabled, "Switch receiver input to")
                        .changed()
                    {
                        *receiver_input = enabled.then(|| {
                            visible_inputs
                                .first()
                                .map(|(c, _)| c.clone())
                                .unwrap_or("05".into())
                        });
                    }
                    if let Some(code) = receiver_input {
                        egui::ComboBox::from_id_salt("receiver_input")
                            .selected_text(input_label(code))
                            .width(220.0)
                            .show_ui(ui, |ui| {
                                for (c, label) in &visible_inputs {
                                    ui.selectable_value(code, c.clone(), format!("{label} ({c})"));
                                }
                            });
                    }
                });
                ui.checkbox(power_on, "Turn the receiver on first if it's in standby");
                optional_code(
                    ui,
                    "sw_hdo",
                    "Switch HDMI output to",
                    hdmi_output,
                    &hdo_options
                        .iter()
                        .copied()
                        .filter(|(c, _)| *c != "UP")
                        .collect::<Vec<_>>(),
                );
                optional_code(
                    ui,
                    "sw_lmd",
                    "Set listening mode to",
                    listening_mode,
                    &lmd_modes,
                );
                ui.horizontal(|ui| {
                    let mut enabled = volume.is_some();
                    if ui.checkbox(&mut enabled, "Set volume to").changed() {
                        *volume = enabled.then_some(30);
                    }
                    if let Some(v) = volume {
                        ui.add(egui::DragValue::new(v).range(0..=volume_max));
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Extra commands").on_hover_text(
                        "Raw eISCP commands sent afterwards, separated by spaces. \
                         E.g. HAO00 = HDMI audio out off, DIMDIM = cycle display dimmer.",
                    );
                    let mut text = extra_commands.join(" ");
                    if ui
                        .add(
                            egui::TextEdit::singleline(&mut text)
                                .desired_width(220.0)
                                .font(egui::TextStyle::Monospace),
                        )
                        .changed()
                    {
                        *extra_commands = text
                            .split_whitespace()
                            .map(|c| c.to_ascii_uppercase())
                            .collect();
                    }
                });

                ui.add_space(8.0);
                theme::subheading(ui, icon::MONITOR, "Monitors");
                if monitor_list.is_empty() && targets.is_empty() {
                    ui.label("No monitors detected yet. See the Monitors tab.");
                }
                // Detected DDC monitors, plus configured ones that aren't connected right now.
                let mut rows: Vec<(MonitorId, Option<MonitorInfo>)> = monitor_list
                    .iter()
                    .filter(|m| m.ddc)
                    .map(|m| (m.id.clone(), Some(m.clone())))
                    .collect();
                for t in targets.iter() {
                    if !rows.iter().any(|(id, _)| *id == t.monitor) {
                        rows.push((t.monitor.clone(), None));
                    }
                }
                egui::Grid::new("monitor_targets")
                    .num_columns(2)
                    .spacing([12.0, 6.0])
                    .show(ui, |ui| {
                        for (id, info) in &rows {
                            let label = match info {
                                Some(m) => format!("{} on {}", id.label(), m.connector),
                                None => format!("{} (not connected)", id.label()),
                            };
                            ui.label(label);
                            let pos = targets.iter().position(|t| t.monitor == *id);
                            let mut choice: Option<u8> = pos.map(|p| targets[p].input);
                            let text = choice
                                .map(monitors::input_name)
                                .unwrap_or_else(|| "Don't change".into());
                            egui::ComboBox::from_id_salt((
                                "mon_target",
                                id.model.as_str(),
                                id.serial.as_str(),
                            ))
                            .selected_text(text)
                            .width(180.0)
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut choice, None, "Don't change");
                                for code in monitors::selectable_inputs(info.as_ref()) {
                                    ui.selectable_value(
                                        &mut choice,
                                        Some(code),
                                        monitors::input_name(code),
                                    );
                                }
                            });
                            match (pos, choice) {
                                (Some(p), None) => {
                                    targets.remove(p);
                                }
                                (Some(p), Some(c)) => targets[p].input = c,
                                (None, Some(c)) => targets.push(MonitorTarget {
                                    monitor: id.clone(),
                                    input: c,
                                }),
                                (None, None) => {}
                            }
                            ui.end_row();
                        }
                    });
            }
            Action::Commands { commands } => {
                ui.label("One raw eISCP command per line (e.g. PWR01, SLI05, MVL28):");
                let mut text = commands.join("\n");
                if ui
                    .add(
                        egui::TextEdit::multiline(&mut text)
                            .desired_rows(5)
                            .font(egui::TextStyle::Monospace),
                    )
                    .changed()
                {
                    *commands = text
                        .lines()
                        .map(|l| l.trim().to_ascii_uppercase())
                        .filter(|l| !l.is_empty())
                        .collect();
                }
            }
            Action::ListeningMode { mode } => {
                ui.horizontal(|ui| {
                    ui.label("Mode");
                    code_combo(ui, "lmd_action", mode, &[&lmd_steps, &lmd_modes]);
                });
                ui.weak(
                    "The receiver rejects modes the current audio signal doesn't allow \
                     (e.g. Dolby modes with a stereo source).",
                );
            }
            Action::HdmiOutput { output } => {
                ui.horizontal(|ui| {
                    ui.label("Output");
                    code_combo(ui, "hdo_action", output, &[&hdo_options]);
                });
            }
            Action::SetVolume { level } => {
                ui.horizontal(|ui| {
                    ui.label("Volume");
                    ui.add(egui::DragValue::new(level).range(0..=volume_max));
                    ui.weak("as shown on the receiver");
                });
            }
            Action::Setting { group, value } => {
                // Options this model accepts for a setting (inputs for zone selectors).
                let options_for = |st: &eiscp::Setting| -> Vec<(String, String)> {
                    let mut opts: Vec<(String, String)> = st
                        .options
                        .iter()
                        .filter(|(c, _)| support_cfg.supports_value(st.group, c))
                        .map(|(c, l)| (c.to_string(), l.to_string()))
                        .collect();
                    if st.inputs {
                        opts.extend(
                            visible_inputs
                                .iter()
                                .filter(|(c, _)| support_cfg.supports_value(st.group, c))
                                .map(|(c, l)| (c.clone(), l.clone())),
                        );
                    }
                    opts
                };
                let available: Vec<&eiscp::Setting> = eiscp::SETTINGS
                    .iter()
                    .filter(|st| support_cfg.supports(st.group) && !options_for(st).is_empty())
                    .collect();
                let current = eiscp::setting_info(group);
                egui::Grid::new("setting_action")
                    .num_columns(2)
                    .spacing([12.0, 8.0])
                    .show(ui, |ui| {
                        ui.label("Setting");
                        let mut chosen = group.clone();
                        egui::ComboBox::from_id_salt("setting_group")
                            .selected_text(current.map(|c| c.label).unwrap_or(group.as_str()))
                            .width(260.0)
                            .height(420.0)
                            .show_ui(ui, |ui| {
                                for st in &available {
                                    ui.selectable_value(
                                        &mut chosen,
                                        st.group.to_string(),
                                        st.label,
                                    );
                                }
                            });
                        if chosen != *group {
                            if let Some(st) = eiscp::setting_info(&chosen) {
                                *value = options_for(st)
                                    .first()
                                    .map(|(c, _)| c.clone())
                                    .unwrap_or_default();
                            }
                            *group = chosen;
                        }
                        ui.end_row();

                        ui.label("Value");
                        let opts = eiscp::setting_info(group)
                            .map(options_for)
                            .unwrap_or_default();
                        let selected = opts
                            .iter()
                            .find(|(c, _)| c == value)
                            .map(|(_, l)| l.clone())
                            .unwrap_or_else(|| value.clone());
                        egui::ComboBox::from_id_salt("setting_value")
                            .selected_text(selected)
                            .width(260.0)
                            .show_ui(ui, |ui| {
                                for (code, label) in &opts {
                                    ui.selectable_value(value, code.clone(), label);
                                }
                            });
                        ui.end_row();
                    });
                if eiscp::setting_info(group).is_some_and(|st| st.repeat.contains(&value.as_str()))
                {
                    ui.label(
                        RichText::new("Holding the hotkey repeats this.").color(theme::TEXT_MUTED),
                    );
                }
                if available.len() < eiscp::SETTINGS.len() {
                    ui.label(
                        RichText::new(format!(
                            "{} more settings are hidden because this receiver doesn't support them.",
                            eiscp::SETTINGS.len() - available.len()
                        ))
                        .color(theme::TEXT_DIM),
                    );
                }
            }
            _ => {
                ui.label("No further options for this action.");
            }
        }

        ui.add_space(12.0);
        ui.separator();
        ui.horizontal(|ui| {
            let enabled = daemon_running && !dirty;
            let resp = ui.add_enabled(enabled, egui::Button::new("Run now"));
            if !daemon_running {
                resp.on_disabled_hover_text("The background service isn't running.");
            } else if dirty {
                resp.on_disabled_hover_text("Save first.");
            } else if resp.clicked() {
                run_now = true;
            }
            ui.weak(format!("CLI: avrcontrol trigger {}", s.id));
        });

        #[cfg(target_os = "linux")]
        {
            ui.add_space(8.0);
            ui.label(
                "On Linux, hotkeys are registered through the desktop's Global Shortcuts portal. \
                 The first time, your desktop asks you to confirm them. Afterwards the desktop owns \
                 the key bindings, so a changed hotkey here may need confirming or re-assigning there.",
            );
            if ui
                .add_enabled(
                    daemon_running,
                    egui::Button::new("Open system shortcut settings"),
                )
                .clicked()
            {
                configure_portal = true;
            }
        }

        if run_now {
            let id = self.cfg.shortcuts[self.selected].id.clone();
            if let Err(e) = ipc::request(&format!("trigger {id}")) {
                self.set_status(format!("{e:#}"), true);
            }
        }
        if configure_portal && let Err(e) = ipc::request("configure-shortcuts") {
            self.set_status(format!("{e:#}"), true);
        }
    }

    fn general_tab(&mut self, ui: &mut egui::Ui) {
        theme::page_header(
            ui,
            "General",
            "How AVR Control behaves on this computer.",
            |_| {},
        );
        egui::ScrollArea::vertical().show(ui, |ui| {
            theme::columns(ui, 2, |cols| {
                theme::card(&mut cols[0], icon::SETTINGS, "Behaviour", None, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Run at startup").color(theme::TEXT_SECONDARY));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let current = self.cfg.general.startup_shortcut.clone();
                            let text = self
                                .cfg
                                .shortcut(&current)
                                .map(|s| s.name.clone())
                                .unwrap_or_else(|| "Nothing".into());
                            let mut choice = current;
                            egui::ComboBox::from_id_salt("startup")
                                .selected_text(text)
                                .width(200.0)
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(&mut choice, String::new(), "Nothing");
                                    for s in &self.cfg.shortcuts {
                                        ui.selectable_value(&mut choice, s.id.clone(), &s.name);
                                    }
                                });
                            self.cfg.general.startup_shortcut = choice;
                        });
                    });
                    let startup_input = self.cfg.startup_input().map(|c| self.cfg.input_label(c));
                    ui.add_enabled_ui(startup_input.is_some(), |ui| {
                        let label = match &startup_input {
                            Some(input) => format!(
                                "Turn the receiver off when this PC shuts down, if it's still on {input}"
                            ),
                            None => "Turn the receiver off when this PC shuts down".into(),
                        };
                        ui.checkbox(&mut self.cfg.general.shutdown_power_off, label)
                            .on_hover_text(
                                "Left on if you've switched to another input (a console, a TV box), \
                                 and when restarting.",
                            )
                            .on_disabled_hover_text(
                                "Needs a startup action that switches the receiver's input.",
                            );
                    });
                    ui.add_space(6.0);
                    ui.checkbox(
                        &mut self.cfg.general.notifications,
                        "Show a notification for each action (errors are always shown)",
                    );
                    ui.checkbox(
                        &mut self.cfg.general.volume_osd,
                        "Show an on-screen volume indicator",
                    );
                    if ui
                        .checkbox(&mut self.autostart, "Start AVR Control when I log in")
                        .changed()
                        && let Err(e) = autostart::set_enabled(self.autostart)
                    {
                        self.set_status(format!("Could not change autostart: {e:#}"), true);
                        self.autostart = autostart::is_enabled();
                    }
                });

                theme::card(
                    &mut cols[1],
                    icon::ACTIVITY,
                    "Background service",
                    Some("Tray icon, hotkeys and the receiver connection."),
                    |ui| {
                        if self.daemon_running {
                            theme::kv_colored(ui, "Status", "Running", theme::SUCCESS);
                        } else {
                            theme::kv_colored(ui, "Status", "Not running", theme::WARNING);
                        }
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            if self.daemon_running {
                                if theme::button(ui, format!("{}  Reconnect", icon::REFRESH))
                                    .clicked()
                                {
                                    let _ = ipc::request("reconnect");
                                }
                                if theme::button(ui, format!("{}  Stop", icon::POWER)).clicked() {
                                    let _ = ipc::request("quit");
                                    self.daemon_checked = Instant::now() - Duration::from_secs(10);
                                }
                            } else if theme::primary_button(ui, format!("{}  Start", icon::PLAY))
                                .clicked()
                            {
                                match std::env::current_exe().and_then(|exe| {
                                    std::process::Command::new(exe).arg("daemon").spawn()
                                }) {
                                    Ok(_) => self.set_status("Started.", false),
                                    Err(e) => {
                                        self.set_status(format!("Could not start: {e}"), true)
                                    }
                                }
                                self.daemon_checked = Instant::now() - Duration::from_secs(10);
                            }
                        });
                    },
                );
            });
            ui.add_space(12.0);
            theme::card(ui, icon::INFO, "About", None, |ui| {
                theme::kv(ui, "Version", env!("CARGO_PKG_VERSION"));
                ui.horizontal(|ui| {
                    ui.checkbox(
                        &mut self.cfg.general.check_updates,
                        "Check for updates automatically",
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let checking = self.update_check.running();
                        let label = if checking { "Checking…" } else { "Check now" };
                        if ui
                            .add_enabled(!checking, egui::Button::new(label))
                            .clicked()
                        {
                            // Through the background service when it runs, so its
                            // tray entry appears too.
                            let daemon = self.daemon_running;
                            self.update_check.start(ui.ctx(), move || {
                                if daemon {
                                    ipc::request_with_timeout(
                                        "check-updates",
                                        Duration::from_secs(30),
                                    )
                                } else {
                                    crate::update::describe_latest()
                                }
                            });
                        }
                    });
                });
                match &self.update_check.result {
                    Some(Ok(text)) => {
                        ui.label(RichText::new(text).color(theme::TEXT_SECONDARY));
                    }
                    Some(Err(e)) => {
                        ui.label(
                            RichText::new(format!("Couldn't check: {e}")).color(theme::WARNING),
                        );
                    }
                    None => {}
                }
                theme::kv(ui, "Config file", Config::path().display().to_string());
                theme::kv(
                    ui,
                    "Receiver model table",
                    crate::models::source().to_string(),
                );
            });
        });
    }
}

impl eframe::App for SettingsApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if self.monitors.running() {
            let previous = self.monitors.ok().cloned().unwrap_or_default();
            if self.monitors.poll()
                && let Some(Ok(list)) = &mut self.monitors.result
            {
                monitors::carry_over(list, &previous);
            }
        }
        self.receiver.poll();
        self.discovery.poll();
        self.update_check.poll();
        self.monitor_test.poll();
        // Keep the status live while the Receiver tab is showing (the probe
        // uses its own connection, so wait for it to finish first).
        // Keep the status live (faster on the pages that show it).
        let interval = if matches!(self.tab, Tab::Status | Tab::Receiver) {
            5
        } else {
            15
        };
        if self.receiver.result.is_some()
            && !self.receiver.running()
            && !self.probe.running()
            && self.status_at.elapsed() > Duration::from_secs(interval)
        {
            self.refresh_receiver(ui.ctx());
        }
        if self.tab == Tab::Status
            && !self.monitors.running()
            && self.monitors_at.elapsed() > Duration::from_secs(20)
        {
            self.scan_monitors(ui.ctx());
        }
        if self.command.poll() {
            self.last_command = match self.command.result.take() {
                Some(Ok(Some(text))) => text,
                Some(Err(e)) => format!("Failed: {e}"),
                _ => String::new(),
            };
            // Show the effect right away.
            self.refresh_receiver(ui.ctx());
        }
        if self.probe.poll()
            && let Some(Ok(caps)) = self.probe.result.clone()
        {
            self.apply_probe(&caps);
        }
        if !self.auto_probed
            && self.cfg.receiver.features.is_empty()
            && self.receiver.ok().is_some()
            && !self.probe.running()
        {
            self.start_probe(ui.ctx());
        }
        if self.monitor_test.result.as_ref().is_some_and(|r| r.is_ok()) {
            // Re-read so "current input" reflects the switch.
            self.monitor_test.result = None;
            self.scan_monitors(ui.ctx());
        }
        if self.daemon_checked.elapsed() > Duration::from_secs(2) {
            self.daemon_running = ipc::is_running();
            self.daemon_checked = Instant::now();
        }
        ui.ctx().request_repaint_after(Duration::from_secs(2));

        egui::Panel::left("rail")
            .exact_size(64.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(theme::SIDEBAR)
                    .inner_margin(egui::Margin::symmetric(0, 16))
                    .stroke(egui::Stroke::new(1.0, theme::BORDER)),
            )
            .show(ui, |ui| self.rail(ui));

        egui::Panel::top("device_bar")
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(theme::BG)
                    .inner_margin(egui::Margin {
                        left: 26,
                        right: 26,
                        top: 14,
                        bottom: 0,
                    }),
            )
            .show(ui, |ui| self.device_bar(ui));

        egui::CentralPanel::no_frame()
            .frame(
                egui::Frame::new()
                    .fill(theme::BG)
                    .inner_margin(egui::Margin {
                        left: 26,
                        right: 26,
                        top: 12,
                        bottom: 20,
                    }),
            )
            .show(ui, |ui| match self.tab {
                // The right margin keeps content clear of the floating scrollbar.
                Tab::Status => {
                    egui::ScrollArea::vertical()
                        .id_salt("status_page")
                        .show(ui, |ui| {
                            egui::Frame::new()
                                .inner_margin(egui::Margin {
                                    right: 14,
                                    ..Default::default()
                                })
                                .show(ui, |ui| self.status_page(ui));
                        });
                }
                Tab::Receiver => {
                    egui::ScrollArea::vertical()
                        .id_salt("receiver_tab")
                        .show(ui, |ui| {
                            egui::Frame::new()
                                .inner_margin(egui::Margin {
                                    right: 14,
                                    ..Default::default()
                                })
                                .show(ui, |ui| self.receiver_tab(ui));
                        });
                }
                Tab::Monitors => self.monitors_tab(ui),
                Tab::Shortcuts => self.shortcuts_tab(ui),
                Tab::General => self.general_tab(ui),
            });
    }
}

pub fn run(tab: Option<&str>) -> anyhow::Result<()> {
    let tab = match tab {
        Some("monitors") => Tab::Monitors,
        Some("shortcuts") => Tab::Shortcuts,
        Some("general") => Tab::General,
        Some("receiver") => Tab::Receiver,
        _ => Tab::Status,
    };
    let img = crate::icon_rgba();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_app_id(crate::config::APP_ID)
            .with_title("AVR Control Settings")
            .with_inner_size([1240.0, 820.0])
            .with_min_inner_size([900.0, 560.0])
            .with_icon(egui::IconData {
                rgba: img.data.clone(),
                width: img.width,
                height: img.height,
            }),
        ..Default::default()
    };
    eframe::run_native(
        "avrcontrol",
        options,
        Box::new(move |cc| {
            theme::apply(&cc.egui_ctx);
            Ok(Box::new(SettingsApp::new(&cc.egui_ctx, tab)))
        }),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}
