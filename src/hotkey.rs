//! Hotkey strings like "Ctrl+Shift+Alt+F1". The canonical key names are the
//! ones the `global-hotkey` crate parses (used on Windows); they are mapped
//! to xkb keysym names for the XDG GlobalShortcuts portal (used on Linux).

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Hotkey {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub logo: bool,
    pub key: String,
}

/// (canonical name, xkb keysym, accepted aliases)
const KEYS: &[(&str, &str, &[&str])] = &[
    ("Up", "Up", &["arrowup"]),
    ("Down", "Down", &["arrowdown"]),
    ("Left", "Left", &["arrowleft"]),
    ("Right", "Right", &["arrowright"]),
    ("Space", "space", &[]),
    ("Enter", "Return", &["return"]),
    ("Tab", "Tab", &[]),
    ("Escape", "Escape", &["esc"]),
    ("Backspace", "BackSpace", &[]),
    ("Delete", "Delete", &["del"]),
    ("Insert", "Insert", &["ins"]),
    ("Home", "Home", &[]),
    ("End", "End", &[]),
    ("PageUp", "Prior", &["pgup", "prior"]),
    ("PageDown", "Next", &["pgdown", "next"]),
    ("Minus", "minus", &["-"]),
    ("Equal", "equal", &["equals", "="]),
    ("Comma", "comma", &[","]),
    ("Period", "period", &["."]),
    ("Slash", "slash", &["/"]),
    ("Semicolon", "semicolon", &[";"]),
    ("Quote", "apostrophe", &["'"]),
    ("BracketLeft", "bracketleft", &["openbracket", "["]),
    ("BracketRight", "bracketright", &["closebracket", "]"]),
    ("Backslash", "backslash", &["\\"]),
    ("Backquote", "grave", &["backtick", "`"]),
    ("VolumeUp", "XF86AudioRaiseVolume", &["audiovolumeup"]),
    ("VolumeDown", "XF86AudioLowerVolume", &["audiovolumedown"]),
    ("VolumeMute", "XF86AudioMute", &["audiovolumemute", "mute"]),
    ("MediaPlayPause", "XF86AudioPlay", &["playpause"]),
    ("MediaStop", "XF86AudioStop", &[]),
    ("MediaTrackNext", "XF86AudioNext", &["medianext"]),
    (
        "MediaTrackPrevious",
        "XF86AudioPrev",
        &["mediaprev", "mediatrackprev"],
    ),
];

/// Returns (canonical, xkb) for a key name, accepting aliases and any case.
fn lookup_key(name: &str) -> Option<(String, String)> {
    let lower = name.to_ascii_lowercase();
    if lower.len() == 1 {
        let c = lower.chars().next().unwrap();
        if c.is_ascii_alphabetic() {
            return Some((c.to_ascii_uppercase().to_string(), lower));
        }
        if c.is_ascii_digit() {
            return Some((lower.clone(), lower));
        }
    }
    if let Some(n) = lower.strip_prefix('f').and_then(|n| n.parse::<u8>().ok())
        && (1..=24).contains(&n)
    {
        return Some((format!("F{n}"), format!("F{n}")));
    }
    if let Some(n) = lower
        .strip_prefix("numpad")
        .or_else(|| lower.strip_prefix("num"))
        .and_then(|n| n.parse::<u8>().ok())
        && n <= 9
    {
        return Some((format!("Num{n}"), format!("KP_{n}")));
    }
    KEYS.iter()
        .find(|(canon, _, aliases)| {
            canon.eq_ignore_ascii_case(&lower) || aliases.contains(&lower.as_str())
        })
        .map(|(canon, xkb, _)| (canon.to_string(), xkb.to_string()))
}

impl Hotkey {
    pub fn parse(s: &str) -> Option<Hotkey> {
        let mut hk = Hotkey::default();
        let tokens: Vec<&str> = s
            .split('+')
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .collect();
        let (key, mods) = tokens.split_last()?;
        for m in mods {
            match m.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => hk.ctrl = true,
                "shift" => hk.shift = true,
                "alt" | "option" => hk.alt = true,
                "super" | "logo" | "meta" | "win" | "cmd" => hk.logo = true,
                _ => return None,
            }
        }
        hk.key = lookup_key(key)?.0;
        Some(hk)
    }

    /// Builds a hotkey from an egui key press (used by the "Record" button).
    pub fn from_egui(key: eframe::egui::Key, mods: eframe::egui::Modifiers) -> Option<Hotkey> {
        Some(Hotkey {
            ctrl: mods.ctrl,
            shift: mods.shift,
            alt: mods.alt,
            logo: mods.mac_cmd,
            key: lookup_key(key.name())?.0,
        })
    }

    fn mods(&self) -> Vec<&'static str> {
        let mut v = Vec::new();
        if self.ctrl {
            v.push("Ctrl");
        }
        if self.shift {
            v.push("Shift");
        }
        if self.alt {
            v.push("Alt");
        }
        if self.logo {
            v.push("Super");
        }
        v
    }

    /// Trigger string in the format of the freedesktop "shortcuts" spec,
    /// e.g. "CTRL+SHIFT+ALT+F1".
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub fn portal_trigger(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if self.ctrl {
            parts.push("CTRL".into());
        }
        if self.shift {
            parts.push("SHIFT".into());
        }
        if self.alt {
            parts.push("ALT".into());
        }
        if self.logo {
            parts.push("LOGO".into());
        }
        parts.push(
            lookup_key(&self.key)
                .map(|(_, x)| x)
                .unwrap_or_else(|| self.key.clone()),
        );
        parts.join("+")
    }
}

impl std::fmt::Display for Hotkey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut parts = self.mods();
        parts.push(&self.key);
        write!(f, "{}", parts.join("+"))
    }
}

/// Checks a user-entered hotkey: empty is fine (no hotkey), otherwise it
/// must parse and should have a modifier.
pub fn validate(s: &str) -> Result<(), String> {
    if s.trim().is_empty() {
        return Ok(());
    }
    match Hotkey::parse(s) {
        None => Err(format!("\"{s}\" is not a valid hotkey")),
        Some(h)
            if !(h.ctrl || h.alt || h.logo)
                && !h.key.starts_with('F')
                && !h.key.starts_with("Volume")
                && !h.key.starts_with("Media") =>
        {
            Err("add Ctrl, Alt or Super so the hotkey doesn't swallow normal typing".into())
        }
        Some(_) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_format() {
        let h = Hotkey::parse("ctrl+shift+alt+f1").unwrap();
        assert_eq!(h.to_string(), "Ctrl+Shift+Alt+F1");
        assert_eq!(h.portal_trigger(), "CTRL+SHIFT+ALT+F1");
        let h = Hotkey::parse("Ctrl+Alt+p").unwrap();
        assert_eq!(h.to_string(), "Ctrl+Alt+P");
        assert_eq!(h.portal_trigger(), "CTRL+ALT+p");
        assert_eq!(
            Hotkey::parse("Ctrl+Shift+Alt+Up").unwrap().portal_trigger(),
            "CTRL+SHIFT+ALT+Up"
        );
        assert_eq!(
            Hotkey::parse("Super+PageUp").unwrap().portal_trigger(),
            "LOGO+Prior"
        );
        assert!(Hotkey::parse("Ctrl+Bogus").is_none());
        assert!(Hotkey::parse("Hyper+A").is_none());
    }

    #[test]
    fn validation() {
        assert!(validate("").is_ok());
        assert!(validate("A").is_err());
        assert!(validate("Ctrl+A").is_ok());
        assert!(validate("VolumeUp").is_ok());
    }
}
