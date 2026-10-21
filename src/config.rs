//! Persistent configuration, stored as TOML in the platform config dir
//! (`~/.config/avrcontrol/config.toml` on Linux,
//! `%APPDATA%\avrcontrol\config\config.toml` on Windows).

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub const APP_ID: &str = "io.github.virmin8.AVRControl";
pub const APP_NAME: &str = "AVR Control";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    pub receiver: ReceiverConfig,
    pub general: GeneralConfig,
    pub audio: AudioConfig,
    pub shortcuts: Vec<Shortcut>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ReceiverConfig {
    /// IP address or hostname of the receiver. Empty = not configured yet.
    pub host: String,
    pub port: u16,
    /// Volume (as shown on the receiver) that corresponds to a full bar in the OSD.
    pub max_volume: u8,
    /// Highest volume (as shown on the receiver) that shortcuts, the tray
    /// and the settings window will set. 0 = no limit.
    pub volume_limit: u8,
    /// Some receivers count volume in 0.5 steps: `MVL` 0x33 (51) is shown as
    /// 25.5. Most (e.g. the DTR-50.7) show the raw value.
    pub volume_half_steps: bool,
    /// Model name reported by the receiver, e.g. "DTR-50.7".
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model: String,
    /// Where `features` came from (spreadsheet or probing).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub features_source: String,
    /// Feature probe results (command group -> supported), see eiscp::FEATURES.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub features: BTreeMap<String, bool>,
    /// User labels for receiver inputs, keyed by the two-digit SLI code.
    pub input_names: BTreeMap<String, String>,
    /// SLI codes hidden from input pickers.
    pub hidden_inputs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct GeneralConfig {
    /// Id of a shortcut to run once when the app starts. Empty = none.
    pub startup_shortcut: String,
    /// At shutdown, turn the receiver off if it is still on the startup
    /// shortcut's input (left alone on any other input, e.g. a console).
    pub shutdown_power_off: bool,
    /// Same as `shutdown_power_off`, when the PC goes to sleep.
    pub sleep_power_off: bool,
    /// After waking from sleep, run the startup shortcut again (unless the
    /// receiver is on and in use on another input).
    pub wake_startup: bool,
    pub notifications: bool,
    pub volume_osd: bool,
    /// Also show the volume indicator when the volume is changed on the
    /// receiver itself, its remote or another app.
    pub volume_osd_external: bool,
    /// Scrolling on the tray icon changes the volume; middle-click mutes.
    pub tray_volume: bool,
    /// Look for a new release on GitHub (at launch and every 48 hours).
    pub check_updates: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AudioConfig {
    /// Play an inaudible signal to the receiver's audio output so the HDMI
    /// audio link stays up (no cut-off start of sounds).
    pub keep_alive: bool,
    /// Output to keep alive: PulseAudio/PipeWire sink name on Linux, endpoint
    /// name on Windows. Empty = the default output.
    pub device: String,
    /// Stop while the receiver is in standby.
    pub pause_in_standby: bool,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            keep_alive: false,
            device: String::new(),
            pause_in_standby: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Shortcut {
    /// Stable identifier, used for the desktop portal and `avrcontrol trigger <id>`.
    pub id: String,
    pub name: String,
    /// e.g. "Ctrl+Shift+Alt+F1". Empty = only reachable from the tray menu / CLI.
    #[serde(default)]
    pub hotkey: String,
    pub action: Action,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Action {
    /// Switch the receiver and/or any number of monitors to an input.
    Switch {
        /// Two-digit SLI code (e.g. "05"), or None to leave the receiver alone.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        receiver_input: Option<String>,
        /// Turn the receiver on first if it is in standby.
        #[serde(default)]
        power_on: bool,
        /// HDMI output to use (HDO code: "01" main, "02" sub, "03" both).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hdmi_output: Option<String>,
        /// Listening mode to select (LMD code, e.g. "0C" All Ch Stereo).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        listening_mode: Option<String>,
        /// Volume to set, as shown on the receiver.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        volume: Option<u8>,
        /// Raw eISCP commands sent after the input change, e.g. "HAO00".
        #[serde(default)]
        extra_commands: Vec<String>,
        #[serde(default)]
        monitors: Vec<MonitorTarget>,
    },
    PowerToggle,
    PowerOn,
    PowerOff,
    VolumeUp,
    VolumeDown,
    MuteToggle,
    HdmiAudioToggle,
    /// Select or cycle the listening mode: an LMD code ("0C"), or "UP",
    /// "DOWN", "MOVIE", "MUSIC", "GAME" to step through modes.
    ListeningMode {
        mode: String,
    },
    /// Select the HDMI output: "01" main, "02" sub, "03" both, "UP" cycle.
    HdmiOutput {
        output: String,
    },
    /// Set the volume to a fixed level, as shown on the receiver.
    SetVolume {
        level: u8,
    },
    /// Any other receiver setting from eiscp::SETTINGS: sends `<group><value>`.
    Setting {
        group: String,
        value: String,
    },
    /// Send arbitrary raw eISCP commands in order.
    Commands {
        commands: Vec<String>,
    },
}

impl Action {
    pub const KINDS: &[(&str, &str)] = &[
        ("switch", "Switch inputs (receiver / monitors)"),
        ("power_toggle", "Receiver power toggle"),
        ("power_on", "Receiver power on"),
        ("power_off", "Receiver power off"),
        ("volume_up", "Volume up"),
        ("volume_down", "Volume down"),
        ("mute_toggle", "Mute toggle"),
        ("hdmi_audio_toggle", "HDMI audio out toggle"),
        ("listening_mode", "Listening mode"),
        ("hdmi_output", "HDMI output (main / sub / both)"),
        ("set_volume", "Set volume to a level"),
        ("setting", "Receiver setting (mute, AccuEQ, zones, menu, …)"),
        ("commands", "Raw eISCP commands"),
    ];

    pub fn kind(&self) -> &'static str {
        match self {
            Action::Switch { .. } => "switch",
            Action::PowerToggle => "power_toggle",
            Action::PowerOn => "power_on",
            Action::PowerOff => "power_off",
            Action::VolumeUp => "volume_up",
            Action::VolumeDown => "volume_down",
            Action::MuteToggle => "mute_toggle",
            Action::HdmiAudioToggle => "hdmi_audio_toggle",
            Action::ListeningMode { .. } => "listening_mode",
            Action::HdmiOutput { .. } => "hdmi_output",
            Action::SetVolume { .. } => "set_volume",
            Action::Setting { .. } => "setting",
            Action::Commands { .. } => "commands",
        }
    }

    /// Whether holding the hotkey should keep repeating the action.
    pub fn repeatable(&self) -> bool {
        match self {
            Action::VolumeUp | Action::VolumeDown => true,
            Action::Setting { group, value } => crate::eiscp::setting_info(group)
                .is_some_and(|s| s.repeat.contains(&value.as_str())),
            _ => false,
        }
    }

    pub fn from_kind(kind: &str) -> Action {
        match kind {
            "power_toggle" => Action::PowerToggle,
            "power_on" => Action::PowerOn,
            "power_off" => Action::PowerOff,
            "volume_up" => Action::VolumeUp,
            "volume_down" => Action::VolumeDown,
            "mute_toggle" => Action::MuteToggle,
            "hdmi_audio_toggle" => Action::HdmiAudioToggle,
            "listening_mode" => Action::ListeningMode { mode: "UP".into() },
            "hdmi_output" => Action::HdmiOutput {
                output: "01".into(),
            },
            "set_volume" => Action::SetVolume { level: 40 },
            "setting" => Action::Setting {
                group: "AMT".into(),
                value: "TG".into(),
            },
            "commands" => Action::Commands {
                commands: Vec::new(),
            },
            _ => Action::Switch {
                receiver_input: None,
                power_on: false,
                hdmi_output: None,
                listening_mode: None,
                volume: None,
                extra_commands: Vec::new(),
                monitors: Vec::new(),
            },
        }
    }

    /// The eISCP command group an action kind depends on, for feature checks.
    pub fn kind_feature(kind: &str) -> Option<&'static str> {
        match kind {
            "power_toggle" | "power_on" | "power_off" => Some("PWR"),
            "volume_up" | "volume_down" | "set_volume" => Some("MVL"),
            "listening_mode" => Some("LMD"),
            "hdmi_output" => Some("HDO"),
            "mute_toggle" => Some("AMT"),
            "hdmi_audio_toggle" => Some("HAO"),
            _ => None,
        }
    }

    pub fn kind_label(kind: &str) -> &'static str {
        Self::KINDS
            .iter()
            .find(|(k, _)| *k == kind)
            .map(|(_, l)| *l)
            .unwrap_or("?")
    }
}

/// Identifies a physical monitor independently of which port/bus it is on.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
pub struct MonitorId {
    #[serde(default)]
    pub mfg: String,
    pub model: String,
    #[serde(default)]
    pub serial: String,
}

impl MonitorId {
    pub fn label(&self) -> String {
        if self.serial.is_empty() {
            self.model.clone()
        } else {
            format!("{} ({})", self.model, self.serial)
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MonitorTarget {
    pub monitor: MonitorId,
    /// MCCS VCP 0x60 input source value, e.g. 0x0f = DisplayPort-1.
    pub input: u8,
}

impl Default for ReceiverConfig {
    fn default() -> Self {
        Self {
            host: String::new(),
            port: crate::eiscp::DEFAULT_PORT,
            max_volume: 100,
            volume_limit: 0,
            volume_half_steps: false,
            model: String::new(),
            features_source: String::new(),
            features: BTreeMap::new(),
            input_names: BTreeMap::new(),
            hidden_inputs: Vec::new(),
        }
    }
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            startup_shortcut: "pc".into(),
            shutdown_power_off: false,
            sleep_power_off: false,
            wake_startup: false,
            notifications: true,
            volume_osd: true,
            volume_osd_external: true,
            tray_volume: true,
            check_updates: true,
        }
    }
}

impl Default for Config {
    /// Mirrors the hotkeys of the original Python version.
    fn default() -> Self {
        let switch = |id: &str, name: &str, key: &str, input: &str, power_on: bool| Shortcut {
            id: id.into(),
            name: name.into(),
            hotkey: format!("Ctrl+Shift+Alt+{key}"),
            action: Action::Switch {
                receiver_input: Some(input.into()),
                power_on,
                hdmi_output: None,
                listening_mode: None,
                volume: None,
                extra_commands: vec!["HAO00".into()],
                monitors: Vec::new(),
            },
        };
        let simple = |id: &str, name: &str, key: &str, action: Action| Shortcut {
            id: id.into(),
            name: name.into(),
            hotkey: format!("Ctrl+Shift+Alt+{key}"),
            action,
        };
        Self {
            receiver: ReceiverConfig::default(),
            general: GeneralConfig::default(),
            audio: AudioConfig::default(),
            shortcuts: vec![
                simple("power", "Power toggle", "P", Action::PowerToggle),
                switch("pc", "Source: PC", "F1", "05", true),
                switch("bd", "Source: BD", "F2", "10", false),
                switch("game", "Source: Game", "F3", "02", false),
                simple(
                    "hdmi_audio",
                    "HDMI audio toggle",
                    "F5",
                    Action::HdmiAudioToggle,
                ),
                simple("volume_up", "Volume up", "Up", Action::VolumeUp),
                simple("volume_down", "Volume down", "Down", Action::VolumeDown),
            ],
        }
    }
}

impl Config {
    pub fn dir() -> PathBuf {
        directories::ProjectDirs::from("", "", "avrcontrol")
            .map(|d| d.config_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."))
    }

    pub fn path() -> PathBuf {
        Self::dir().join("config.toml")
    }

    pub fn exists() -> bool {
        Self::path().exists()
    }

    /// Loads the config, falling back to defaults if there is none yet.
    pub fn load() -> Result<Config> {
        let path = Self::path();
        if !path.exists() {
            return Ok(Config::default());
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path();
        std::fs::create_dir_all(Self::dir())?;
        let text = toml::to_string_pretty(self)?;
        // Write-then-rename so a crash never leaves a half-written config.
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    pub fn shortcut(&self, id: &str) -> Option<&Shortcut> {
        self.shortcuts.iter().find(|s| s.id == id)
    }

    /// Volume as the receiver displays it, from the raw `MVL` value.
    /// Raw `MVL` value for a volume as shown on the receiver.
    pub fn volume_raw(&self, level: u8) -> u8 {
        if self.receiver.volume_half_steps {
            level.saturating_mul(2)
        } else {
            level
        }
    }

    pub fn volume_text(&self, raw: u8) -> String {
        if self.receiver.volume_half_steps {
            format!("{:.1}", raw as f32 / 2.0)
        } else {
            raw.to_string()
        }
    }

    /// Raw `MVL` value of the volume limit, if one is set.
    pub fn volume_ceiling(&self) -> Option<u8> {
        (self.receiver.volume_limit > 0).then(|| self.volume_raw(self.receiver.volume_limit))
    }

    /// Command table for the receiver's model from the protocol spreadsheet.
    pub fn model_table(&self) -> Option<&'static crate::models::ModelTable> {
        crate::models::lookup(&self.receiver.model)
    }

    /// Feature map to use: from the spreadsheet when the model is listed,
    /// otherwise whatever probing found.
    pub fn effective_features(&self) -> BTreeMap<String, bool> {
        match self.model_table() {
            Some(t) => crate::eiscp::FEATURES
                .iter()
                .map(|(g, _)| (g.to_string(), t.contains_key(*g)))
                .collect(),
            None => self.receiver.features.clone(),
        }
    }

    /// Whether the receiver has the command group. Unknown (not detected yet)
    /// counts as supported.
    pub fn supports(&self, group: &str) -> bool {
        match self.model_table() {
            Some(t) => t.contains_key(group),
            None => self.receiver.features.get(group).copied().unwrap_or(true),
        }
    }

    /// Whether a specific value of a command works on this model (e.g.
    /// "DIM03"). Only the spreadsheet knows this; otherwise assume yes.
    pub fn supports_value(&self, group: &str, value: &str) -> bool {
        match self.model_table() {
            Some(t) => crate::models::table_supports(t, group, value),
            None => self.supports(group),
        }
    }

    /// Highest `MVL` value the model accepts, as shown on the receiver.
    pub fn volume_max(&self) -> u8 {
        let raw_max = self
            .model_table()
            .and_then(|t| t.get("MVL"))
            .and_then(|vals| {
                vals.iter()
                    .filter_map(|v| v.split_once("..")?.1.parse::<u32>().ok())
                    .max()
            })
            .unwrap_or(if self.receiver.volume_half_steps {
                200
            } else {
                100
            });
        let shown = if self.receiver.volume_half_steps {
            raw_max / 2
        } else {
            raw_max
        };
        shown.min(u8::MAX as u32) as u8
    }

    /// False only if feature detection ran and the receiver rejected the
    /// command group an action kind needs.
    pub fn supports_kind(&self, kind: &str) -> bool {
        Action::kind_feature(kind).is_none_or(|g| self.supports(g))
    }

    /// Receiver input the startup shortcut switches to, if it switches one.
    pub fn startup_input(&self) -> Option<&str> {
        match &self.shortcut(&self.general.startup_shortcut)?.action {
            Action::Switch {
                receiver_input: Some(code),
                ..
            } => Some(code),
            _ => None,
        }
    }

    /// Display name for a receiver input, honouring user labels.
    pub fn input_label(&self, code: &str) -> String {
        match self.receiver.input_names.get(code) {
            Some(name) if !name.is_empty() => name.clone(),
            _ => crate::eiscp::default_input_name(code).to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_round_trip() {
        let cfg = Config::default();
        let text = toml::to_string_pretty(&cfg).unwrap();
        assert_eq!(toml::from_str::<Config>(&text).unwrap(), cfg);
    }

    /// A config as 1.0 wrote it: no audio section, none of the later
    /// settings. It must still load, with the new settings at their defaults.
    #[test]
    fn old_config_loads() {
        let cfg: Config =
            toml::from_str(include_str!("../tests/fixtures/config-1.0.toml")).unwrap();
        assert_eq!(cfg.receiver.host, "192.168.1.50");
        assert_eq!(cfg.receiver.volume_limit, 0);
        assert!(!cfg.general.shutdown_power_off);
        assert!(!cfg.general.sleep_power_off);
        assert!(cfg.general.tray_volume);
        assert!(cfg.general.volume_osd_external);
        assert!(!cfg.audio.keep_alive);
        assert!(cfg.audio.pause_in_standby);
        assert_eq!(cfg.shortcuts.len(), 2);
        assert_eq!(cfg.startup_input(), Some("05"));
        assert_eq!(
            cfg.shortcut("pc").map(|s| &s.action),
            Some(&Action::Switch {
                receiver_input: Some("05".into()),
                power_on: true,
                hdmi_output: None,
                listening_mode: None,
                volume: None,
                extra_commands: vec!["HAO00".into()],
                monitors: vec![MonitorTarget {
                    monitor: MonitorId {
                        mfg: "DEL".into(),
                        model: "DELL S3220DGF".into(),
                        serial: String::new(),
                    },
                    input: 0x0f,
                }],
            })
        );
    }

    #[test]
    fn volume_scales() {
        let mut cfg = Config::default();
        assert_eq!(cfg.volume_raw(40), 40);
        assert_eq!(cfg.volume_text(40), "40");
        assert_eq!(cfg.volume_ceiling(), None);
        cfg.receiver.volume_limit = 60;
        assert_eq!(cfg.volume_ceiling(), Some(60));
        cfg.receiver.volume_half_steps = true;
        assert_eq!(cfg.volume_raw(40), 80);
        assert_eq!(cfg.volume_raw(200), 255);
        assert_eq!(cfg.volume_text(51), "25.5");
        assert_eq!(cfg.volume_ceiling(), Some(120));
        assert_eq!(cfg.volume_max(), 100);
    }

    #[test]
    fn features_from_table_or_probe() {
        let mut cfg = Config::default();
        // Nothing known: everything counts as supported.
        assert!(cfg.supports("DIM"));
        assert!(cfg.supports_value("DIM", "03"));
        cfg.receiver.features.insert("DIM".into(), false);
        assert!(!cfg.supports("DIM"));
        assert!(!cfg.supports_kind("power_on") || cfg.supports("PWR"));
        // A known model uses the spreadsheet, whatever probing said.
        cfg.receiver.model = "DTR-50.7".into();
        assert!(cfg.supports("DIM"));
        assert!(cfg.supports_value("DIM", "02"));
        assert!(!cfg.supports_value("DIM", "03"));
        assert!(!cfg.supports("RES"));
        assert_eq!(cfg.effective_features().get("RES"), Some(&false));
        assert_eq!(cfg.effective_features().get("DIM"), Some(&true));
        assert_eq!(cfg.volume_max(), 100);
    }

    #[test]
    fn inputs() {
        let mut cfg = Config::default();
        assert_eq!(cfg.startup_input(), Some("05"));
        assert_eq!(cfg.input_label("05"), "VIDEO6 / PC");
        cfg.receiver
            .input_names
            .insert("05".into(), "Desktop".into());
        cfg.receiver.input_names.insert("10".into(), String::new());
        assert_eq!(cfg.input_label("05"), "Desktop");
        assert_eq!(cfg.input_label("10"), "BD/DVD");
        cfg.general.startup_shortcut = "power".into();
        assert_eq!(cfg.startup_input(), None);
        cfg.general.startup_shortcut = "missing".into();
        assert_eq!(cfg.startup_input(), None);
    }

    #[test]
    fn action_kinds() {
        for (kind, _) in Action::KINDS {
            assert_eq!(Action::from_kind(kind).kind(), *kind);
            assert_ne!(Action::kind_label(kind), "?");
        }
        assert!(Action::VolumeUp.repeatable());
        assert!(Action::VolumeDown.repeatable());
        assert!(!Action::MuteToggle.repeatable());
        let setting = |group: &str, value: &str| Action::Setting {
            group: group.into(),
            value: value.into(),
        };
        assert!(setting("MVL", "UP1").repeatable());
        assert!(setting("TUN", "UP").repeatable());
        assert!(setting("OSD", "LEFT").repeatable());
        assert!(!setting("OSD", "ENTER").repeatable());
        assert!(!setting("AMT", "TG").repeatable());
    }
}
