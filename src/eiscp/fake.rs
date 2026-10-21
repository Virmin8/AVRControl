//! A fake receiver for tests: a TCP server on 127.0.0.1 that speaks ISCP
//! framing, answers `QSTN` queries from its state, applies set commands and
//! records everything it was sent. Faults can be scripted.

use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use super::{packet, take_packet};

#[derive(Default)]
pub struct State {
    /// Command group -> current value, e.g. "MVL" -> "2A".
    pub values: HashMap<String, String>,
    /// Groups the receiver doesn't have: never answered.
    pub silent: HashSet<String>,
    /// Groups whose next query is dropped once (as if lost in a rush).
    pub drop_once: HashSet<String>,
    /// Network latency: each reply goes out this long after its command
    /// came in (commands are still read and handled one after another).
    pub delay: Duration,
    /// Every command received, without "!1" and the terminator.
    pub received: Vec<String>,
    clients: Vec<TcpStream>,
}

pub struct FakeReceiver {
    pub port: u16,
    pub state: Arc<Mutex<State>>,
}

impl FakeReceiver {
    /// A receiver that's on, on input 05 at volume 0x2A, with a few more
    /// values set. Every other group answers "N/A".
    pub fn start() -> FakeReceiver {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let state = Arc::new(Mutex::new(State::default()));
        {
            let mut s = state.lock().unwrap();
            for (g, v) in [
                ("PWR", "01"),
                ("SLI", "05"),
                ("MVL", "2A"),
                ("AMT", "00"),
                ("LMD", "0C"),
                ("HDO", "01"),
                ("IFA", "HDMI 1,PCM,48 kHz,2.0 ch,All Ch Stereo,5.1 ch,"),
                ("HAO", "01"),
            ] {
                s.values.insert(g.into(), v.into());
            }
        }
        let shared = state.clone();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { break };
                shared
                    .lock()
                    .unwrap()
                    .clients
                    .push(stream.try_clone().unwrap());
                let state = shared.clone();
                thread::spawn(move || serve(stream, state));
            }
        });
        FakeReceiver { port, state }
    }

    pub fn set(&self, group: &str, value: &str) {
        self.state
            .lock()
            .unwrap()
            .values
            .insert(group.into(), value.into());
    }

    pub fn received(&self) -> Vec<String> {
        self.state.lock().unwrap().received.clone()
    }

    /// Sends a message to every connected client, as the receiver does when
    /// its front panel or remote is used.
    pub fn push(&self, msg: &str) {
        let data = packet(&format!("!1{msg}\x1a\r\n"));
        for c in &mut self.state.lock().unwrap().clients {
            let _ = c.write_all(&data);
        }
    }

    /// Closes every client connection.
    pub fn drop_clients(&self) {
        for c in self.state.lock().unwrap().clients.drain(..) {
            let _ = c.shutdown(Shutdown::Both);
        }
    }
}

fn serve(mut stream: TcpStream, state: Arc<Mutex<State>>) {
    let writer = Arc::new(Mutex::new(stream.try_clone().unwrap()));
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let n = match stream.read(&mut chunk) {
            Ok(0) | Err(_) => return,
            Ok(n) => n,
        };
        buf.extend_from_slice(&chunk[..n]);
        while let Some(cmd) = take_packet(&mut buf) {
            let (reply, delay) = {
                let mut s = state.lock().unwrap();
                s.received.push(cmd.clone());
                (answer(&mut s, &cmd), s.delay)
            };
            if let Some(reply) = reply {
                let writer = writer.clone();
                thread::spawn(move || {
                    thread::sleep(delay);
                    let data = packet(&format!("!1{reply}\x1a\r\n"));
                    let _ = writer.lock().unwrap().write_all(&data);
                });
            }
        }
    }
}

fn answer(s: &mut State, cmd: &str) -> Option<String> {
    let (group, arg) = (cmd.get(..3)?, cmd.get(3..)?);
    if s.silent.contains(group) || s.drop_once.remove(group) {
        return None;
    }
    let value = match arg {
        "QSTN" => s.values.get(group).cloned().unwrap_or_else(|| "N/A".into()),
        "UP" | "DOWN" if group == "MVL" => {
            let v = u8::from_str_radix(s.values.get("MVL")?, 16).ok()?;
            let v = if arg == "UP" {
                v + 1
            } else {
                v.saturating_sub(1)
            };
            let v = format!("{v:02X}");
            s.values.insert("MVL".into(), v.clone());
            v
        }
        "TG" => {
            let on = s.values.get(group).is_some_and(|v| v == "01");
            let v = if on { "00" } else { "01" }.to_string();
            s.values.insert(group.into(), v.clone());
            v
        }
        _ => {
            s.values.insert(group.into(), arg.into());
            arg.into()
        }
    };
    Some(format!("{group}{value}"))
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Instant;

    use super::super::{Push, Receiver};
    use super::*;

    fn receiver(fake: &FakeReceiver) -> Receiver {
        Receiver::new("127.0.0.1", fake.port)
    }

    /// A listener that forwards everything it's told to a channel.
    fn listen(r: &mut Receiver) -> mpsc::Receiver<(String, bool)> {
        let (tx, rx) = mpsc::channel();
        r.set_listener(Arc::new(move |p| {
            let _ = tx.send(match p {
                Push::Message { msg, solicited } => (msg.to_string(), solicited),
                Push::Closed => ("<closed>".into(), false),
            });
        }));
        rx
    }

    #[test]
    fn send_and_query() {
        let fake = FakeReceiver::start();
        let mut r = receiver(&fake);
        assert_eq!(r.query("PWR").unwrap(), "01");
        assert_eq!(r.send("mvlup").unwrap().as_deref(), Some("MVL2B"));
        assert_eq!(r.query("MVL").unwrap(), "2B");
        assert_eq!(fake.received(), ["PWRQSTN", "MVLUP", "MVLQSTN"]);
    }

    #[test]
    fn status_fills_fields_and_na_is_none() {
        let fake = FakeReceiver::start();
        fake.set("IFV", "N/A");
        let mut r = receiver(&fake);
        let s = r.status().unwrap();
        assert!(s.power_on);
        assert_eq!(s.input.as_deref(), Some("05"));
        assert_eq!(s.volume, Some(0x2A));
        assert_eq!(s.muted, Some(false));
        assert_eq!(s.listening_mode.as_deref(), Some("0C"));
        assert_eq!(s.hdmi_output.as_deref(), Some("01"));
        assert_eq!(s.video, None);
        assert_eq!(s.audio_info.unwrap().format, "PCM");
        assert_eq!(s.extra.get("HAO").map(String::as_str), Some("01"));
        // Unset groups answer N/A and stay out of `extra`.
        assert!(!s.extra.contains_key("DIM"));
    }

    #[test]
    fn status_is_pipelined() {
        let fake = FakeReceiver::start();
        // One query at a time would take 28 round trips of 50 ms.
        fake.state.lock().unwrap().delay = Duration::from_millis(50);
        let mut r = receiver(&fake);
        let start = Instant::now();
        r.status().unwrap();
        assert!(
            start.elapsed() < Duration::from_millis(700),
            "{:?}",
            start.elapsed()
        );
    }

    #[test]
    fn unanswered_queries_time_out_together() {
        let fake = FakeReceiver::start();
        for g in ["DIM", "SLP", "CEC"] {
            fake.state.lock().unwrap().silent.insert(g.into());
        }
        let mut r = receiver(&fake);
        let start = Instant::now();
        r.status().unwrap();
        // One query at a time: three timeouts of 1.2 s each.
        assert!(
            start.elapsed() < Duration::from_millis(2000),
            "{:?}",
            start.elapsed()
        );
    }

    #[test]
    fn unsupported_groups_are_not_asked() {
        let fake = FakeReceiver::start();
        let mut r = receiver(&fake);
        let features = [("DIM", false), ("SLP", false), ("MVL", true)]
            .into_iter()
            .map(|(g, ok)| (g.to_string(), ok))
            .collect();
        r.set_features(&features);
        r.status().unwrap();
        let received = fake.received();
        assert!(received.contains(&"MVLQSTN".to_string()));
        assert!(!received.contains(&"DIMQSTN".to_string()));
        assert!(!received.contains(&"SLPQSTN".to_string()));
    }

    #[test]
    fn missed_supported_query_is_asked_again() {
        let fake = FakeReceiver::start();
        fake.state.lock().unwrap().drop_once.insert("MVL".into());
        let mut r = receiver(&fake);
        r.set_features(&[("MVL".to_string(), true)].into_iter().collect());
        assert_eq!(r.status().unwrap().volume, Some(0x2A));
    }

    #[test]
    fn silent_group_is_missing_not_an_error() {
        let fake = FakeReceiver::start();
        fake.state.lock().unwrap().silent.insert("LMD".into());
        let mut r = receiver(&fake);
        let replies = r.query_batch(&["PWR", "LMD", "SLI"]).unwrap();
        assert_eq!(replies.get("PWR").map(String::as_str), Some("PWR01"));
        assert_eq!(replies.get("SLI").map(String::as_str), Some("SLI05"));
        assert!(!replies.contains_key("LMD"));
    }

    #[test]
    fn pushed_messages_go_to_the_listener_not_the_reply() {
        let fake = FakeReceiver::start();
        fake.state.lock().unwrap().delay = Duration::from_millis(100);
        let mut r = receiver(&fake);
        let pushes = listen(&mut r);
        r.connect().unwrap();
        let push = {
            let state = fake.state.clone();
            let port = fake.port;
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(30));
                FakeReceiver { port, state }.push("SLI01");
            })
        };
        assert_eq!(r.send("MVLQSTN").unwrap().as_deref(), Some("MVL2A"));
        push.join().unwrap();
        let seen: Vec<_> = pushes.try_iter().collect();
        assert!(seen.contains(&("SLI01".to_string(), false)), "{seen:?}");
        assert!(seen.contains(&("MVL2A".to_string(), true)), "{seen:?}");
    }

    #[test]
    fn pushes_arrive_while_idle() {
        let fake = FakeReceiver::start();
        let mut r = receiver(&fake);
        let pushes = listen(&mut r);
        r.connect().unwrap();
        // Wait for the fake to register the connection.
        while fake.state.lock().unwrap().clients.is_empty() {
            thread::sleep(Duration::from_millis(5));
        }
        fake.push("MVL30");
        assert_eq!(
            pushes.recv_timeout(Duration::from_secs(2)).unwrap(),
            ("MVL30".to_string(), false)
        );
    }

    #[test]
    fn reconnects_after_the_receiver_drops_the_connection() {
        let fake = FakeReceiver::start();
        let mut r = receiver(&fake);
        let pushes = listen(&mut r);
        assert_eq!(r.query("PWR").unwrap(), "01");
        fake.drop_clients();
        while pushes.recv_timeout(Duration::from_secs(2)).unwrap().0 != "<closed>" {}
        assert_eq!(r.query("SLI").unwrap(), "05");
    }

    #[test]
    fn closing_ourselves_is_not_reported() {
        let fake = FakeReceiver::start();
        let mut r = receiver(&fake);
        let pushes = listen(&mut r);
        r.query("PWR").unwrap();
        r.disconnect();
        thread::sleep(Duration::from_millis(100));
        assert!(pushes.try_iter().all(|(m, _)| m != "<closed>"));
    }

    /// Against a real receiver: `AVR_HOST=192.168.1.30 cargo test hardware
    /// -- --ignored --nocapture`. Read-only. Uses the feature list from the
    /// saved settings.
    #[test]
    #[ignore]
    fn hardware_batched_status_matches_sequential() {
        let host = std::env::var("AVR_HOST").expect("set AVR_HOST");
        let cfg = crate::config::Config::load().unwrap();
        let mut r = Receiver::new(&host, super::super::DEFAULT_PORT);
        r.set_features(&cfg.effective_features());
        r.connect().unwrap();

        let all: Vec<&str> = super::super::STATUS_GROUPS
            .iter()
            .chain(super::super::EXTRA_STATUS)
            .copied()
            .collect();
        let start = Instant::now();
        let replies = r.query_batch(&all).unwrap();
        let unanswered: Vec<_> = all.iter().filter(|g| !replies.contains_key(**g)).collect();
        println!(
            "batch only: {:?}, unanswered: {unanswered:?}",
            start.elapsed()
        );

        let start = Instant::now();
        let batched = r.status().unwrap();
        let batched_time = start.elapsed();

        let start = Instant::now();
        let mut sequential = super::super::ReceiverStatus::default();
        let groups: Vec<&str> = super::super::STATUS_GROUPS
            .iter()
            .chain(super::super::EXTRA_STATUS)
            .copied()
            .filter(|g| !r.unsupported.contains(*g))
            .collect();
        for g in groups {
            if let Ok(Some(msg)) = r.exchange(&format!("{g}QSTN"), super::super::OPTIONAL_TIMEOUT) {
                sequential.apply(&msg);
            }
        }
        let sequential_time = start.elapsed();
        r.disconnect();

        println!("batched:    {batched_time:?}\nsequential: {sequential_time:?}");
        println!("{batched:#?}");
        assert_eq!(
            serde_json::to_string(&batched).unwrap(),
            serde_json::to_string(&sequential).unwrap()
        );
    }

    #[test]
    fn probe_unknown_model_asks_every_feature() {
        let fake = FakeReceiver::start();
        fake.set(
            "NRI",
            r#"<selectorlist><selector id="05" value="1" name="PC"/></selectorlist>"#,
        );
        fake.state.lock().unwrap().silent.insert("TUN".into());
        let mut r = receiver(&fake);
        let caps = r.probe().unwrap();
        assert_eq!(caps.model, None);
        assert_eq!(caps.inputs, vec![("05".to_string(), "PC".to_string())]);
        let supported = |g: &str| caps.features.iter().find(|f| f.0 == g).map(|f| f.1);
        assert_eq!(supported("PWR"), Some(true));
        // "N/A" still means the command exists.
        assert_eq!(supported("DIM"), Some(true));
        assert_eq!(supported("TUN"), Some(false));
    }
}
