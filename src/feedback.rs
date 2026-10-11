//! User feedback: desktop notifications and the volume on-screen display.

use std::io::Write;
use std::process::{Child, ChildStdin, Command, Stdio};

use notify_rust::Notification;

use crate::config::{APP_ID, APP_NAME, Config};

pub struct Feedback {
    notifications: bool,
    volume_osd: bool,
    max_volume: u8,
    half_steps: bool,
    #[cfg(target_os = "linux")]
    last_id: Option<u32>,
    #[cfg(target_os = "linux")]
    kde_osd: Option<zbus::blocking::Connection>,
    osd_child: Option<(Child, ChildStdin)>,
}

fn notification_icon() -> String {
    #[cfg(target_os = "linux")]
    {
        let installed = [
            "/usr/share/icons/hicolor/256x256/apps",
            "/usr/local/share/icons/hicolor/256x256/apps",
        ]
        .iter()
        .any(|dir| {
            std::path::Path::new(dir)
                .join(format!("{APP_ID}.png"))
                .exists()
        });
        if installed {
            return APP_ID.to_string();
        }
    }
    let _ = APP_ID;
    "audio-card".to_string()
}

impl Feedback {
    pub fn new(cfg: &Config) -> Self {
        let mut f = Self {
            notifications: true,
            volume_osd: true,
            max_volume: 100,
            half_steps: false,
            #[cfg(target_os = "linux")]
            last_id: None,
            #[cfg(target_os = "linux")]
            kde_osd: None,
            osd_child: None,
        };
        f.apply_config(cfg);
        f
    }

    pub fn apply_config(&mut self, cfg: &Config) {
        self.notifications = cfg.general.notifications;
        self.volume_osd = cfg.general.volume_osd;
        self.max_volume = cfg.receiver.max_volume.max(1);
        self.half_steps = cfg.receiver.volume_half_steps;
    }

    /// Informational message; suppressed when notifications are off.
    pub fn info(&mut self, msg: &str) {
        if self.notifications {
            self.show(msg);
        }
    }

    /// Errors are always shown, otherwise a failing hotkey is silent.
    pub fn error(&mut self, msg: &str) {
        eprintln!("error: {msg}");
        self.show(msg);
    }

    fn show(&mut self, msg: &str) {
        let mut n = Notification::new();
        n.summary(APP_NAME)
            .body(msg)
            .appname(APP_NAME)
            .icon(&notification_icon())
            .timeout(2500);
        // Replace the previous bubble instead of stacking a new one per keypress.
        #[cfg(target_os = "linux")]
        {
            if let Some(id) = self.last_id {
                n.id(id);
            }
            match n.show() {
                Ok(handle) => self.last_id = Some(handle.id()),
                Err(e) => eprintln!("notification failed: {e}"),
            }
        }
        #[cfg(not(target_os = "linux"))]
        if let Err(e) = n.show() {
            eprintln!("notification failed: {e}");
        }
    }

    /// Shows the volume indicator for a raw `MVL` value.
    pub fn volume(&mut self, raw: u8, muted: bool) {
        if !self.volume_osd {
            return;
        }
        let (display, text) = if self.half_steps {
            let v = raw as f32 / 2.0;
            (v, format!("{v:.1}"))
        } else {
            (raw as f32, raw.to_string())
        };
        let text = if muted { "muted".to_string() } else { text };
        let percent = if muted {
            0
        } else {
            (display / self.max_volume as f32 * 100.0)
                .round()
                .clamp(0.0, 100.0) as i32
        };
        #[cfg(target_os = "linux")]
        if self.kde_volume(percent, &text) {
            return;
        }
        self.overlay_volume(percent, &text);
    }

    /// Plasma's own volume OSD. It only renders a percentage bar, so the exact
    /// receiver value goes into the label.
    #[cfg(target_os = "linux")]
    fn kde_volume(&mut self, percent: i32, text: &str) -> bool {
        if self.kde_osd.is_none() {
            self.kde_osd = zbus::blocking::Connection::session().ok();
        }
        let Some(conn) = &self.kde_osd else {
            return false;
        };
        conn.call_method(
            Some("org.kde.plasmashell"),
            "/org/kde/osdService",
            Some("org.kde.osdService"),
            "mediaPlayerVolumeChanged",
            &(percent, format!("Onkyo receiver  {text}"), "audio-card"),
        )
        .is_ok()
    }

    /// Our own overlay window, run as a child process (`avrcontrol osd`)
    /// that reads "<percent> <text>" lines from stdin and closes itself after
    /// a short idle period.
    fn overlay_volume(&mut self, percent: i32, text: &str) {
        let line = format!("{percent} {text}\n");
        if let Some((child, stdin)) = &mut self.osd_child {
            let alive = matches!(child.try_wait(), Ok(None));
            if alive
                && stdin
                    .write_all(line.as_bytes())
                    .and_then(|_| stdin.flush())
                    .is_ok()
            {
                return;
            }
        }
        if let Some((mut child, _)) = self.osd_child.take() {
            let _ = child.wait();
        }
        let Ok(exe) = std::env::current_exe() else {
            return;
        };
        match Command::new(exe).arg("osd").stdin(Stdio::piped()).spawn() {
            Ok(mut child) => {
                let mut stdin = child.stdin.take().unwrap();
                let _ = stdin.write_all(line.as_bytes());
                let _ = stdin.flush();
                self.osd_child = Some((child, stdin));
            }
            Err(e) => eprintln!("could not start volume overlay: {e}"),
        }
    }
}
