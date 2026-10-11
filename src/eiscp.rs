//! Minimal Onkyo eISCP client (ISCP messages wrapped in a TCP/UDP header).
//!
//! Packet layout: "ISCP" | header size (u32 BE, always 16) | data size (u32 BE)
//! | version (u8, 1) | 3 reserved bytes | data. Data is "!1<CMD><PARAM>\r";
//! the receiver answers with "!1<CMD><PARAM>\x1a\r\n".

use std::io::{ErrorKind, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream, ToSocketAddrs, UdpSocket};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};

pub const DEFAULT_PORT: u16 = 60128;
const HEADER_LEN: usize = 16;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
/// The spec says 50 ms, real receivers often need several hundred.
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(3);

/// Receiver input selectors (SLI / SLZ codes) with readable names. Which of
/// these a given receiver actually has varies by model.
pub const INPUTS: &[(&str, &str)] = &[
    ("00", "VIDEO1 / VCR/DVR / STB"),
    ("01", "VIDEO2 / CBL/SAT"),
    ("02", "VIDEO3 / GAME"),
    ("03", "VIDEO4 / AUX1"),
    ("04", "VIDEO5 / AUX2 / GAME2"),
    ("05", "VIDEO6 / PC"),
    ("06", "VIDEO7"),
    ("07", "EXTRA1"),
    ("08", "EXTRA2"),
    ("09", "EXTRA3"),
    ("10", "BD/DVD"),
    ("11", "STRM BOX"),
    ("12", "TV"),
    ("20", "TAPE / TV"),
    ("21", "TAPE2"),
    ("22", "PHONO"),
    ("23", "CD / TV/CD"),
    ("24", "FM"),
    ("25", "AM"),
    ("26", "TUNER"),
    ("27", "MUSIC SERVER / DLNA"),
    ("28", "INTERNET RADIO"),
    ("29", "USB (front)"),
    ("2A", "USB (rear)"),
    ("2B", "NETWORK"),
    ("2C", "USB (toggle)"),
    ("2D", "AIRPLAY"),
    ("2E", "BLUETOOTH"),
    ("2F", "USB DAC IN"),
    ("30", "MULTI CH"),
    ("31", "XM"),
    ("32", "SIRIUS"),
    ("33", "DAB"),
    ("40", "UNIVERSAL PORT"),
    ("41", "LINE"),
    ("42", "LINE2"),
    ("44", "OPTICAL"),
    ("45", "COAXIAL"),
    ("55", "HDMI 5"),
    ("56", "HDMI 6"),
    ("57", "HDMI 7"),
];

/// Listening modes (LMD codes). Which ones are available depends on the
/// model and on the incoming signal.
pub const LISTENING_MODES: &[(&str, &str)] = &[
    ("00", "Stereo"),
    ("01", "Direct"),
    ("02", "Surround"),
    ("03", "Film / Game-RPG"),
    ("04", "THX"),
    ("05", "Action / Game-Action"),
    ("06", "Musical / Game-Rock"),
    ("07", "Mono Movie"),
    ("08", "Orchestra"),
    ("09", "Unplugged"),
    ("0A", "Studio-Mix"),
    ("0B", "TV Logic"),
    ("0C", "All Ch Stereo"),
    ("0D", "Theater-Dimensional"),
    ("0E", "Enhanced / Game-Sports"),
    ("0F", "Mono"),
    ("11", "Pure Audio"),
    ("12", "Multiplex"),
    ("13", "Full Mono"),
    ("14", "Dolby Virtual"),
    ("15", "DTS Surround Sensation"),
    ("16", "Audyssey DSX"),
    ("1F", "Whole House"),
    ("40", "Straight Decode"),
    ("41", "Dolby EX"),
    ("42", "THX Cinema"),
    ("43", "THX Surround EX"),
    ("44", "THX Music"),
    ("45", "THX Games"),
    ("50", "THX U2/S2/I/S Cinema"),
    ("51", "THX U2/S2/I/S Music"),
    ("52", "THX U2/S2/I/S Games"),
    ("80", "Dolby Surround / PLII Movie"),
    ("81", "PLII Music"),
    ("82", "DTS Neural:X / Neo:6 Cinema"),
    ("83", "Neo:6 Music"),
    ("84", "Dolby Surround THX Cinema"),
    ("85", "DTS Neural:X THX Cinema"),
    ("86", "PLII Game"),
    ("87", "Neural Surround"),
    ("88", "Neural THX"),
    ("89", "Dolby Surround THX Games"),
    ("8A", "DTS Neural:X THX Games"),
    ("8B", "Dolby Surround THX Music"),
    ("8C", "DTS Neural:X THX Music"),
    ("90", "PLIIz Height"),
    ("A0", "PLII Movie + Audyssey DSX"),
    ("A1", "PLII Music + Audyssey DSX"),
    ("A2", "PLII Game + Audyssey DSX"),
    ("FF", "Auto Surround"),
];

/// Listening-mode commands that step through modes rather than pick one.
pub const LISTENING_MODE_STEPS: &[(&str, &str)] = &[
    ("UP", "Next mode"),
    ("DOWN", "Previous mode"),
    ("MOVIE", "Next movie mode"),
    ("MUSIC", "Next music mode"),
    ("GAME", "Next game mode"),
];

pub const HDMI_OUTPUTS: &[(&str, &str)] = &[
    ("01", "Main"),
    ("02", "Sub"),
    ("03", "Main + Sub"),
    ("UP", "Next output"),
];

/// A receiver setting that can be bound to a shortcut ("Receiver setting"
/// action). Which entries and options are offered depends on the model.
pub struct Setting {
    pub group: &'static str,
    pub label: &'static str,
    pub options: &'static [(&'static str, &'static str)],
    /// Values that keep repeating while the hotkey is held.
    pub repeat: &'static [&'static str],
    /// Notify with the resulting value (off for menu navigation etc.).
    pub notify: bool,
    /// Offer the receiver's inputs as options (zone input selectors).
    pub inputs: bool,
}

const fn setting(
    group: &'static str,
    label: &'static str,
    options: &'static [(&'static str, &'static str)],
) -> Setting {
    Setting {
        group,
        label,
        options,
        repeat: &[],
        notify: true,
        inputs: false,
    }
}

const ON_OFF_TOGGLE: &[(&str, &str)] = &[("01", "On"), ("00", "Off"), ("TG", "Toggle")];
const ON_OFF_NEXT: &[(&str, &str)] = &[("01", "On"), ("00", "Off"), ("UP", "Toggle")];
const UP_DOWN: &[(&str, &str)] = &[("UP", "Up"), ("DOWN", "Down")];
const POWER: &[(&str, &str)] = &[("01", "On"), ("00", "Standby")];

pub const SETTINGS: &[Setting] = &[
    setting("AMT", "Mute", ON_OFF_TOGGLE),
    Setting {
        repeat: &["UP1", "DOWN1"],
        ..setting(
            "MVL",
            "Volume (1 dB steps)",
            &[("UP1", "Up 1 dB"), ("DOWN1", "Down 1 dB")],
        )
    },
    setting(
        "SLI",
        "Input (step through)",
        &[("UP", "Next input"), ("DOWN", "Previous input")],
    ),
    setting(
        "HAO",
        "HDMI audio out",
        &[("01", "On"), ("00", "Off"), ("02", "Auto"), ("UP", "Next")],
    ),
    setting("HAS", "HDMI audio out (sub)", ON_OFF_NEXT),
    setting(
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
            ("UP", "Next"),
        ],
    ),
    setting(
        "AEQ",
        "AccuEQ",
        &[
            ("01", "On"),
            ("02", "On (except front)"),
            ("00", "Off"),
            ("UP", "Next"),
        ],
    ),
    setting("MOT", "Music Optimizer", ON_OFF_NEXT),
    setting("PMB", "Phase Matching Bass", ON_OFF_TOGGLE),
    setting(
        "LTN",
        "Late night",
        &[
            ("00", "Off"),
            ("01", "Low"),
            ("02", "High"),
            ("03", "Auto"),
            ("UP", "Next"),
        ],
    ),
    Setting {
        repeat: &["UP", "DOWN"],
        ..setting(
            "AVS",
            "A/V sync (lip sync)",
            &[("UP", "Delay more"), ("DOWN", "Delay less")],
        )
    },
    Setting {
        repeat: &["UP", "DOWN"],
        ..setting("SWL", "Subwoofer level", UP_DOWN)
    },
    Setting {
        repeat: &["UP", "DOWN"],
        ..setting("CTL", "Centre level", UP_DOWN)
    },
    setting(
        "DIM",
        "Display dimmer",
        &[
            ("00", "Bright"),
            ("01", "Dim"),
            ("02", "Dark"),
            ("03", "Off"),
            ("DIM", "Next"),
        ],
    ),
    setting(
        "DIF",
        "Front display info",
        &[
            ("TG", "Next"),
            ("02", "Show audio format"),
            ("03", "Show video format"),
        ],
    ),
    setting(
        "SLP",
        "Sleep timer",
        &[
            ("OFF", "Off"),
            ("1E", "30 min"),
            ("3C", "60 min"),
            ("5A", "90 min"),
            ("UP", "Next"),
        ],
    ),
    setting("CEC", "HDMI CEC", ON_OFF_NEXT),
    setting("TGA", "12V trigger A", &[("01", "On"), ("00", "Off")]),
    setting("TGB", "12V trigger B", &[("01", "On"), ("00", "Off")]),
    setting("TGC", "12V trigger C", &[("01", "On"), ("00", "Off")]),
    setting(
        "PRS",
        "Tuner preset",
        &[("UP", "Next preset"), ("DOWN", "Previous preset")],
    ),
    Setting {
        repeat: &["UP", "DOWN"],
        ..setting("TUN", "Tuner frequency", UP_DOWN)
    },
    Setting {
        notify: false,
        ..setting(
            "NTC",
            "Network / USB playback",
            &[
                ("P/P", "Play/pause"),
                ("PLAY", "Play"),
                ("PAUSE", "Pause"),
                ("STOP", "Stop"),
                ("TRUP", "Next track"),
                ("TRDN", "Previous track"),
                ("REPEAT", "Repeat"),
                ("RANDOM", "Shuffle"),
            ],
        )
    },
    Setting {
        notify: false,
        repeat: &["UP", "DOWN", "LEFT", "RIGHT"],
        ..setting(
            "OSD",
            "Receiver menu",
            &[
                ("MENU", "Menu"),
                ("UP", "Up"),
                ("DOWN", "Down"),
                ("LEFT", "Left"),
                ("RIGHT", "Right"),
                ("ENTER", "Enter"),
                ("EXIT", "Exit / back"),
                ("QUICK", "Quick menu"),
                ("HOME", "Home"),
            ],
        )
    },
    setting("ZPW", "Zone 2 power", POWER),
    Setting {
        repeat: &["UP", "DOWN"],
        ..setting("ZVL", "Zone 2 volume", UP_DOWN)
    },
    setting("ZMT", "Zone 2 mute", ON_OFF_TOGGLE),
    Setting {
        inputs: true,
        ..setting(
            "SLZ",
            "Zone 2 input",
            &[("80", "Same as main"), ("UP", "Next input")],
        )
    },
    setting("PW3", "Zone 3 power", POWER),
    Setting {
        repeat: &["UP", "DOWN"],
        ..setting("VL3", "Zone 3 volume", UP_DOWN)
    },
    setting("MT3", "Zone 3 mute", ON_OFF_TOGGLE),
    Setting {
        inputs: true,
        ..setting(
            "SL3",
            "Zone 3 input",
            &[("80", "Same as main"), ("UP", "Next input")],
        )
    },
];

pub fn setting_info(group: &str) -> Option<&'static Setting> {
    SETTINGS.iter().find(|s| s.group == group)
}

fn lookup<'a>(table: &'a [(&str, &'a str)], code: &str) -> Option<&'a str> {
    table
        .iter()
        .find(|(c, _)| c.eq_ignore_ascii_case(code))
        .map(|(_, n)| *n)
}

pub fn listening_mode_name(code: &str) -> String {
    lookup(LISTENING_MODES, code)
        .or_else(|| lookup(LISTENING_MODE_STEPS, code))
        .map(str::to_string)
        .unwrap_or_else(|| format!("Mode {code}"))
}

pub fn hdmi_output_name(code: &str) -> String {
    match code {
        "00" => "Off".to_string(),
        "04" => "Both (main)".to_string(),
        "05" => "Both (sub)".to_string(),
        _ => lookup(HDMI_OUTPUTS, code)
            .map(str::to_string)
            .unwrap_or_else(|| format!("Output {code}")),
    }
}

pub fn default_input_name(code: &str) -> &str {
    INPUTS
        .iter()
        .find(|(c, _)| c.eq_ignore_ascii_case(code))
        .map(|(_, n)| *n)
        .unwrap_or("Unknown input")
}

pub fn packet(data: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER_LEN + data.len());
    out.extend_from_slice(b"ISCP");
    out.extend_from_slice(&(HEADER_LEN as u32).to_be_bytes());
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(&[1, 0, 0, 0]);
    out.extend_from_slice(data.as_bytes());
    out
}

/// Strips "!1" and the EOF/CR/LF terminators from an ISCP message.
fn parse_message(body: &[u8]) -> String {
    let text = String::from_utf8_lossy(body);
    let text = text.trim_end_matches(['\x1a', '\r', '\n', '\0']);
    text.strip_prefix('!')
        .map(|t| t.get(1..).unwrap_or(""))
        .unwrap_or(text)
        .to_string()
}

/// Takes one complete packet off the front of `buf`, if there is one.
fn take_packet(buf: &mut Vec<u8>) -> Option<String> {
    // Resync on garbage: drop everything before the next "ISCP" magic.
    match buf.windows(4).position(|w| w == b"ISCP") {
        Some(0) => {}
        Some(n) => {
            buf.drain(..n);
        }
        None => {
            let keep = buf.len().min(3);
            buf.drain(..buf.len() - keep);
            return None;
        }
    }
    if buf.len() < HEADER_LEN {
        return None;
    }
    let header_len = u32::from_be_bytes(buf[4..8].try_into().unwrap()) as usize;
    let data_len = u32::from_be_bytes(buf[8..12].try_into().unwrap()) as usize;
    let header_len = header_len.max(HEADER_LEN);
    if buf.len() < header_len + data_len {
        return None;
    }
    let msg = parse_message(&buf[header_len..header_len + data_len]);
    buf.drain(..header_len + data_len);
    Some(msg)
}

pub struct Receiver {
    host: String,
    port: u16,
    /// Command groups known to be unsupported (from feature detection);
    /// status() doesn't ask for them.
    unsupported: std::collections::HashSet<String>,
    stream: Option<TcpStream>,
    buf: Vec<u8>,
}

impl Receiver {
    pub fn new(host: &str, port: u16) -> Self {
        Self {
            host: host.trim().to_string(),
            port,
            unsupported: Default::default(),
            stream: None,
            buf: Vec::new(),
        }
    }

    pub fn set_features(&mut self, features: &std::collections::BTreeMap<String, bool>) {
        self.unsupported = features
            .iter()
            .filter(|(_, ok)| !**ok)
            .map(|(g, _)| g.clone())
            .collect();
    }

    /// Like `query`, but with a short timeout and None for "N/A", for
    /// optional status fields.
    fn query_optional(&mut self, group: &str) -> Option<String> {
        if self.unsupported.contains(group) {
            return None;
        }
        let reply = self
            .exchange(&format!("{group}QSTN"), Duration::from_millis(1200))
            .ok()??;
        let value = reply.get(3..)?.to_string();
        (value != "N/A" && !value.is_empty()).then_some(value)
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn is_connected(&self) -> bool {
        self.stream.is_some()
    }

    fn addr(&self) -> Result<SocketAddr> {
        if self.host.is_empty() {
            bail!("no receiver IP address configured");
        }
        (self.host.as_str(), self.port)
            .to_socket_addrs()
            .with_context(|| format!("resolving {}", self.host))?
            .next()
            .ok_or_else(|| anyhow!("could not resolve {}", self.host))
    }

    pub fn connect(&mut self) -> Result<()> {
        self.disconnect();
        let addr = self.addr()?;
        let stream = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT)
            .with_context(|| format!("connecting to {addr}"))?;
        stream.set_nodelay(true).ok();
        self.stream = Some(stream);
        Ok(())
    }

    pub fn disconnect(&mut self) {
        if let Some(s) = self.stream.take() {
            let _ = s.shutdown(std::net::Shutdown::Both);
        }
        self.buf.clear();
    }

    fn ensure_connected(&mut self) -> Result<()> {
        if self.stream.is_none() {
            self.connect()?;
        }
        Ok(())
    }

    /// Reads the next message, or Ok(None) once `deadline` passes.
    fn read_message(&mut self, deadline: Instant) -> Result<Option<String>> {
        loop {
            if let Some(msg) = take_packet(&mut self.buf) {
                return Ok(Some(msg));
            }
            let now = Instant::now();
            if now >= deadline {
                return Ok(None);
            }
            let stream = self
                .stream
                .as_mut()
                .ok_or_else(|| anyhow!("not connected"))?;
            stream.set_read_timeout(Some((deadline - now).max(Duration::from_millis(1))))?;
            let mut chunk = [0u8; 4096];
            match stream.read(&mut chunk) {
                Ok(0) => bail!("connection closed by receiver"),
                Ok(n) => self.buf.extend_from_slice(&chunk[..n]),
                Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                    return Ok(None);
                }
                Err(e) => return Err(e.into()),
            }
        }
    }

    /// Sends one command and waits for the reply with the same 3-letter
    /// command group (e.g. "MVLUP" is answered by "MVL2A").
    fn exchange(&mut self, command: &str, timeout: Duration) -> Result<Option<String>> {
        self.ensure_connected()?;
        // Discard unsolicited status updates that piled up since last time,
        // so they are not mistaken for the answer.
        let drain_deadline = Instant::now() + Duration::from_millis(2);
        while self.read_message(drain_deadline)?.is_some() {}

        let stream = self.stream.as_mut().unwrap();
        stream.write_all(&packet(&format!("!1{command}\r")))?;

        let group = command.get(..3).unwrap_or(command);
        let deadline = Instant::now() + timeout;
        while let Some(msg) = self.read_message(deadline)? {
            if msg.starts_with(group) {
                return Ok(Some(msg));
            }
        }
        Ok(None)
    }

    /// Sends a command and returns the receiver's reply (None if it did not
    /// answer in time). A dropped connection is re-opened and the command
    /// retried once: Onkyo receivers close idle eISCP connections.
    pub fn send(&mut self, command: &str) -> Result<Option<String>> {
        let command = command.trim().to_ascii_uppercase();
        let mut last_err = None;
        for attempt in 0..3 {
            if attempt > 0 {
                // The receiver drops idle connections, and some models only
                // allow one client: give a competing client a moment.
                std::thread::sleep(Duration::from_millis(300 * attempt));
                if let Err(e) = self.connect() {
                    last_err = Some(e);
                    continue;
                }
            }
            match self.exchange(&command, RESPONSE_TIMEOUT) {
                Ok(r) => return Ok(r),
                Err(e) => last_err = Some(e),
            }
        }
        Err(last_err.unwrap())
    }

    /// Like `send`, but a missing reply is an error and the "XXX" prefix
    /// is stripped, e.g. query("PWR") -> "01".
    pub fn query(&mut self, group: &str) -> Result<String> {
        let reply = self
            .send(&format!("{group}QSTN"))?
            .ok_or_else(|| anyhow!("receiver did not answer {group}QSTN"))?;
        Ok(reply.get(3..).unwrap_or("").to_string())
    }

    /// Asks the receiver about every entry in FEATURES. Unsupported commands
    /// are answered with "N/A" or not at all.
    /// Works out what the receiver supports. If its model is in Onkyo's
    /// protocol spreadsheet (models.json) that is used as-is and only the
    /// input list is asked for; otherwise every FEATURES group is queried.
    pub fn probe(&mut self) -> Result<Capabilities> {
        let model = identify(&self.host, self.port).map(|r| r.model);
        let table = model.as_deref().and_then(crate::models::lookup);
        let mut caps = Capabilities {
            model: model.clone(),
            ..Default::default()
        };

        if let Some(table) = table {
            caps.source = format!("Onkyo protocol spreadsheet ({})", crate::models::source());
            for (group, _) in FEATURES {
                caps.features
                    .push((group.to_string(), table.contains_key(*group), String::new()));
            }
            if table.contains_key("NRI") {
                let reply = self.exchange("NRIQSTN", Duration::from_secs(3))?;
                if let Some(xml) = reply.as_deref().and_then(|r| r.get(3..)) {
                    caps.inputs = parse_nri_selectors(xml);
                }
            }
            return Ok(caps);
        }

        caps.source = "asked the receiver (model not in the spreadsheet)".into();
        for (group, _) in FEATURES {
            let wait = if *group == "NRI" {
                Duration::from_secs(3)
            } else {
                Duration::from_millis(1200)
            };
            let reply = match self.exchange(&format!("{group}QSTN"), wait) {
                Ok(r) => r,
                Err(_) => {
                    // Connection dropped mid-probe: reconnect and carry on.
                    self.connect()?;
                    None
                }
            };
            let value = reply
                .as_deref()
                .and_then(|r| r.get(3..))
                .unwrap_or("")
                .to_string();
            // Any answer means the command exists; "N/A" only means it isn't
            // available right now (e.g. zone 2 volume while zone 2 is off).
            // Unsupported commands get no answer at all.
            let supported = reply.is_some();
            if *group == "NRI" && supported && value != "N/A" {
                caps.inputs = parse_nri_selectors(&value);
            }
            let shown = if value.len() > 40 {
                String::new()
            } else {
                value
            };
            caps.features.push((group.to_string(), supported, shown));
        }
        Ok(caps)
    }

    /// Current state, one query per field. Fields the receiver doesn't
    /// support (or can't report while in standby) are None.
    pub fn status(&mut self) -> Result<ReceiverStatus> {
        let power = self.query("PWR")?;
        let mut q = |group: &str| self.query_optional(group);
        let input = q("SLI");
        let volume = q("MVL").and_then(|v| u8::from_str_radix(v.get(..2).unwrap_or(&v), 16).ok());
        let muted = q("AMT").map(|m| m == "01");
        let listening_mode = q("LMD");
        let hdmi_output = q("HDO");
        let video = q("IFV").and_then(|v| VideoInfo::parse(&v));
        let audio_raw = q("IFA");
        let audio = audio_raw
            .as_deref()
            .map(|v| v.trim_end_matches(',').replace(',', ", "));
        let audio_info = audio_raw.as_deref().and_then(AudioInfo::parse);
        let extra = EXTRA_STATUS
            .iter()
            .filter_map(|g| Some((g.to_string(), q(g)?)))
            .collect();
        Ok(ReceiverStatus {
            model: None,
            power_on: power == "01",
            input,
            volume,
            muted,
            listening_mode,
            hdmi_output,
            video,
            audio,
            audio_info,
            extra,
        })
    }

    /// `status()` plus the model name (an extra UDP round-trip).
    pub fn status_with_model(&mut self) -> Result<ReceiverStatus> {
        let mut s = self.status()?;
        s.model = identify(&self.host, self.port).map(|r| r.model);
        Ok(s)
    }
}

/// Features that can be detected with a `QSTN` query: (command group, label).
pub const FEATURES: &[(&str, &str)] = &[
    ("PWR", "Power"),
    ("MVL", "Master volume"),
    ("AMT", "Mute"),
    ("SLI", "Input selector"),
    ("LMD", "Listening mode"),
    ("HAO", "HDMI audio out (main)"),
    ("HAS", "HDMI audio out (sub)"),
    ("HDO", "HDMI output selector"),
    ("RES", "Monitor out resolution"),
    ("CEC", "HDMI CEC"),
    ("ARC", "Audio return channel"),
    ("DIM", "Display dimmer"),
    ("SLP", "Sleep timer"),
    ("LTN", "Late night"),
    ("ADY", "Audyssey MultEQ"),
    ("ADQ", "Audyssey Dynamic EQ"),
    ("ADV", "Audyssey Dynamic Volume"),
    ("TUN", "Tuner"),
    ("IFA", "Audio signal information"),
    ("IFV", "Video signal information"),
    ("ZPW", "Zone 2 power"),
    ("ZVL", "Zone 2 volume"),
    ("SLZ", "Zone 2 input selector"),
    ("PW3", "Zone 3 power"),
    ("AEQ", "AccuEQ room correction"),
    ("MOT", "Music Optimizer"),
    ("SLA", "Audio selector"),
    ("TGA", "12V trigger A"),
    ("TGB", "12V trigger B"),
    ("TGC", "12V trigger C"),
    ("ZMT", "Zone 2 mute"),
    ("MT3", "Zone 3 mute"),
    ("VL3", "Zone 3 volume"),
    ("SL3", "Zone 3 input selector"),
    ("NRI", "Receiver information (input list)"),
];

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Capabilities {
    /// Model name the receiver reported, if it answered the ECN query.
    #[serde(default)]
    pub model: Option<String>,
    /// Where the feature list came from (spreadsheet or probing).
    #[serde(default)]
    pub source: String,
    /// (command group, supported, current value if any)
    pub features: Vec<(String, bool, String)>,
    /// Inputs from the NRI selector list: (SLI code, name). Empty if the
    /// receiver doesn't support NRI.
    pub inputs: Vec<(String, String)>,
}

/// Pulls `<selector id=".." value="1" name=".."/>` entries out of NRI XML.
fn parse_nri_selectors(xml: &str) -> Vec<(String, String)> {
    let attr = |tag: &str, name: &str| -> Option<String> {
        let start = tag.find(&format!("{name}=\""))? + name.len() + 2;
        let end = tag[start..].find('"')? + start;
        Some(tag[start..end].to_string())
    };
    xml.split("<selector ")
        .skip(1)
        .filter_map(|rest| {
            let tag = &rest[..rest.find('>').unwrap_or(rest.len())];
            let id = attr(tag, "id")?.to_ascii_uppercase();
            let enabled = attr(tag, "value").is_none_or(|v| v == "1");
            let name = attr(tag, "name").unwrap_or_default();
            (enabled && id.len() == 2).then_some((id, name))
        })
        .collect()
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ReceiverStatus {
    pub model: Option<String>,
    pub power_on: bool,
    pub input: Option<String>,
    pub volume: Option<u8>,
    pub muted: Option<bool>,
    pub listening_mode: Option<String>,
    pub hdmi_output: Option<String>,
    pub video: Option<VideoInfo>,
    /// IFA reply, comma separated (input port, format, sample rate, ...).
    pub audio: Option<String>,
    #[serde(default)]
    pub audio_info: Option<AudioInfo>,
    /// Current values of the EXTRA_STATUS groups the receiver answered.
    #[serde(default)]
    pub extra: std::collections::BTreeMap<String, String>,
}

/// Additional settings read for the control panel.
pub const EXTRA_STATUS: &[&str] = &[
    "HAO", "HAS", "LTN", "DIM", "SLP", "CEC", "AEQ", "MOT", "SLA", "TGA", "TGB", "TGC", "ZPW",
    "SLZ", "ZVL", "ZMT", "PW3", "SL3", "VL3", "MT3",
];

/// Parsed `IFV` reply: "HDMI 4,1920x1080 60Hz,RGB,24bit,MAIN+SUB,1920x1080 60Hz,RGB,24bit,Game,".
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct VideoInfo {
    pub input_port: String,
    /// Resolution + colour + depth, for one-line display.
    pub input_format: String,
    pub output_port: String,
    pub output_format: String,
    #[serde(default)]
    pub input_resolution: String,
    #[serde(default)]
    pub input_color: String,
    #[serde(default)]
    pub output_resolution: String,
    #[serde(default)]
    pub output_color: String,
    #[serde(default)]
    pub picture_mode: String,
}

/// Parsed `IFA` reply: "HDMI 1,PCM,48 kHz,2.0 ch,All Ch Stereo,5.1 ch,".
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AudioInfo {
    pub input_port: String,
    pub format: String,
    pub sample_rate: String,
    pub input_channels: String,
    pub listening_mode: String,
    pub output_channels: String,
    pub output_sample_rate: String,
}

impl AudioInfo {
    pub fn parse(reply: &str) -> Option<AudioInfo> {
        let f: Vec<&str> = reply.split(',').map(str::trim).collect();
        if f.len() < 2 || f[0].is_empty() {
            return None;
        }
        let field = |i: usize| f.get(i).copied().unwrap_or("").to_string();
        Some(AudioInfo {
            input_port: field(0),
            format: field(1),
            sample_rate: field(2),
            input_channels: field(3),
            listening_mode: field(4),
            output_channels: field(5),
            output_sample_rate: field(6),
        })
    }
}

impl VideoInfo {
    pub fn parse(reply: &str) -> Option<VideoInfo> {
        let f: Vec<&str> = reply.split(',').map(str::trim).collect();
        if f.len() < 4 || f[0].is_empty() {
            return None;
        }
        let join = |parts: &[&str]| {
            parts
                .iter()
                .filter(|p| !p.is_empty())
                .copied()
                .collect::<Vec<_>>()
                .join(" ")
        };
        let field = |i: usize| f.get(i).copied().unwrap_or("").to_string();
        Some(VideoInfo {
            input_port: f[0].to_string(),
            input_format: join(&f[1..4]),
            output_port: field(4),
            output_format: join(f.get(5..8).unwrap_or(&[])),
            input_resolution: field(1),
            input_color: join(&f[2..4]),
            output_resolution: field(5),
            output_color: join(f.get(6..8).unwrap_or(&[])),
            picture_mode: field(8),
        })
    }
}

impl ReceiverStatus {
    /// Multi-line summary for the tray tooltip.
    pub fn summary(&self, cfg: &crate::config::Config) -> String {
        if !self.power_on {
            return "Receiver in standby".into();
        }
        let mut lines = Vec::new();
        if let Some(i) = &self.input {
            lines.push(format!("Input: {}", cfg.input_label(i)));
        }
        if let Some(v) = self.volume {
            let muted = if self.muted == Some(true) {
                " (muted)"
            } else {
                ""
            };
            lines.push(format!("Volume: {}{muted}", cfg.volume_text(v)));
        }
        if let Some(m) = &self.listening_mode {
            lines.push(format!("Mode: {}", listening_mode_name(m)));
        }
        if let Some(v) = &self.video {
            lines.push(format!("Video: {} {}", v.input_port, v.input_format));
        }
        if let Some(a) = &self.audio {
            lines.push(format!("Audio: {a}"));
        }
        lines.join("\n")
    }
}

#[derive(Debug, Clone)]
pub struct DiscoveredReceiver {
    pub host: String,
    pub port: u16,
    pub model: String,
    pub identifier: String,
}

/// The IPv4 address of the interface that routes to the internet (no packet
/// is actually sent by a UDP connect).
fn primary_ipv4() -> Option<Ipv4Addr> {
    let sock = UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect(("192.0.2.1", 9)).ok()?;
    match sock.local_addr().ok()?.ip() {
        IpAddr::V4(ip) if !ip.is_loopback() => Some(ip),
        _ => None,
    }
}

/// Sends the ECN query to `targets` and collects answers until `wait` passes.
fn query_ecn(targets: &[SocketAddr], wait: Duration) -> Result<Vec<DiscoveredReceiver>> {
    let sock = UdpSocket::bind("0.0.0.0:0")?;
    sock.set_broadcast(true)?;
    for target in targets {
        for magic in ["!xECNQSTN", "!pECNQSTN"] {
            // Unreachable hosts in a sweep are expected; ignore send errors.
            let _ = sock.send_to(&packet(magic), target);
        }
    }
    let deadline = Instant::now() + wait;
    let mut found: Vec<DiscoveredReceiver> = Vec::new();
    let mut buf = [0u8; 1024];
    while Instant::now() < deadline {
        sock.set_read_timeout(Some(deadline - Instant::now()))?;
        let (n, from) = match sock.recv_from(&mut buf) {
            Ok(r) => r,
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => break,
            // ICMP port-unreachable from a swept host surfaces as
            // ConnectionRefused on Linux and ConnectionReset on Windows.
            Err(e)
                if matches!(
                    e.kind(),
                    ErrorKind::ConnectionRefused | ErrorKind::ConnectionReset
                ) =>
            {
                continue;
            }
            Err(e) => return Err(e.into()),
        };
        let mut data = buf[..n].to_vec();
        let Some(msg) = take_packet(&mut data) else {
            continue;
        };
        // "ECNDTR-50.7/60128/DX/0009B0123456"
        let Some(rest) = msg.strip_prefix("ECN") else {
            continue;
        };
        let mut parts = rest.split('/');
        let model = parts.next().unwrap_or("").to_string();
        let port = parts
            .next()
            .and_then(|p| p.parse().ok())
            .unwrap_or(DEFAULT_PORT);
        let identifier = parts.nth(1).unwrap_or("").to_string();
        let host = from.ip().to_string();
        if !found.iter().any(|r| r.host == host) {
            found.push(DiscoveredReceiver {
                host,
                port,
                model,
                identifier,
            });
        }
    }
    Ok(found)
}

/// Finds receivers on the local network. Besides broadcasting, every
/// address in the local /24 is asked directly: many desktop firewalls drop
/// replies to broadcasts because they can't be matched to an outgoing flow.
pub fn discover(wait: Duration) -> Result<Vec<DiscoveredReceiver>> {
    let mut targets = vec![SocketAddr::from((Ipv4Addr::BROADCAST, DEFAULT_PORT))];
    if let Some(ip) = primary_ipv4() {
        let [a, b, c, _] = ip.octets();
        targets.extend(
            (1..=254u8).map(|d| SocketAddr::from((Ipv4Addr::new(a, b, c, d), DEFAULT_PORT))),
        );
    }
    query_ecn(&targets, wait)
}

/// Asks one known host for its model name.
pub fn identify(host: &str, port: u16) -> Option<DiscoveredReceiver> {
    let addr = (host, port).to_socket_addrs().ok()?.next()?;
    query_ecn(&[addr], Duration::from_millis(800))
        .ok()?
        .into_iter()
        .next()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_info() {
        let v = VideoInfo::parse("HDMI 4,VGA,RGB,24bit,MAIN+SUB,VGA,RGB,24bit,,").unwrap();
        assert_eq!(v.input_port, "HDMI 4");
        assert_eq!(v.input_format, "VGA RGB 24bit");
        assert_eq!(v.output_port, "MAIN+SUB");
        assert_eq!(v.output_format, "VGA RGB 24bit");
        assert_eq!(v.input_resolution, "VGA");
        assert_eq!(v.output_color, "RGB 24bit");
        let a = AudioInfo::parse("HDMI 1,PCM,48 kHz,2.0 ch,All Ch Stereo,5.1 ch,").unwrap();
        assert_eq!(a.format, "PCM");
        assert_eq!(a.output_channels, "5.1 ch");
        assert_eq!(VideoInfo::parse(",,,,,,,,,"), None);
    }

    #[test]
    fn nri_selectors() {
        let xml = r#"<selectorlist count="3"><selector id="10" value="1" name="BD/DVD" zone="01" iconid="00"/><selector id="2B" value="1" name="NET"/><selector id="01" value="0" name="CBL/SAT"/></selectorlist>"#;
        assert_eq!(
            parse_nri_selectors(xml),
            vec![("10".into(), "BD/DVD".into()), ("2B".into(), "NET".into())]
        );
    }

    #[test]
    fn roundtrip_packet() {
        let mut buf = packet("!1PWR01\x1a\r\n");
        buf.extend(packet("!1SLI05\x1a\r\n"));
        assert_eq!(take_packet(&mut buf).as_deref(), Some("PWR01"));
        assert_eq!(take_packet(&mut buf).as_deref(), Some("SLI05"));
        assert_eq!(take_packet(&mut buf), None);
    }

    #[test]
    fn resyncs_and_waits_for_partial() {
        let full = packet("!1MVL32\x1a\r\n");
        let mut buf = b"junk".to_vec();
        buf.extend_from_slice(&full[..10]);
        assert_eq!(take_packet(&mut buf), None);
        buf.extend_from_slice(&full[10..]);
        assert_eq!(take_packet(&mut buf).as_deref(), Some("MVL32"));
    }
}
