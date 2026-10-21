//! Auto-repeat while a hotkey is held (used for volume up/down). The platform
//! layers report presses and releases; this re-sends the trigger at a fixed
//! rate in between.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::thread;
use std::time::{Duration, Instant};

use crate::controller::Event;

const INITIAL_DELAY: Duration = Duration::from_millis(350);
const INTERVAL: Duration = Duration::from_millis(80);
/// How often to check whether the previous repeat has been executed.
const POLL: Duration = Duration::from_millis(5);
/// Hard stop in case a release is ever missed.
const MAX_HOLD: Duration = Duration::from_secs(10);

/// Set while a repeated trigger is queued but not yet executed, so a slow
/// receiver can't build up a backlog that keeps going after the key is let go.
static IN_FLIGHT: AtomicBool = AtomicBool::new(false);
/// Whether the platform has ever reported a release. Until it has, we can't
/// know when a key goes up, so we don't repeat at all.
static RELEASE_SEEN: AtomicBool = AtomicBool::new(false);
/// Bumped on every press and release; a repeat loop stops once it changes.
static GENERATION: AtomicU64 = AtomicU64::new(0);
/// The repeatable shortcut currently held down, if any. Some desktops resend
/// "pressed" at their own key-repeat rate while a key is held; those extra
/// presses are ignored so the rate is always ours. Expires with MAX_HOLD in
/// case a release is missed.
static HELD: Mutex<Option<(String, Instant)>> = Mutex::new(None);

pub fn pressed(tx: &Sender<Event>, id: &str, repeatable: bool) {
    if repeatable && RELEASE_SEEN.load(Ordering::SeqCst) {
        let mut held = HELD.lock().unwrap();
        if held
            .as_ref()
            .is_some_and(|(h, at)| h == id && at.elapsed() < MAX_HOLD)
        {
            return;
        }
        *held = Some((id.to_string(), Instant::now()));
    }
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    let _ = tx.send(Event::Trigger(id.to_string()));
    if !repeatable || !RELEASE_SEEN.load(Ordering::SeqCst) {
        return;
    }
    let tx = tx.clone();
    let id = id.to_string();
    thread::spawn(move || {
        let start = Instant::now();
        thread::sleep(INITIAL_DELAY);
        while GENERATION.load(Ordering::SeqCst) == generation && start.elapsed() < MAX_HOLD {
            // Still waiting on the receiver: check again shortly rather than
            // skipping a whole interval, which halved the rate.
            if IN_FLIGHT.swap(true, Ordering::SeqCst) {
                thread::sleep(POLL);
                continue;
            }
            if tx.send(Event::Trigger(id.clone())).is_err() {
                break;
            }
            thread::sleep(INTERVAL);
        }
    });
}

pub fn released() {
    *HELD.lock().unwrap() = None;
    RELEASE_SEEN.store(true, Ordering::SeqCst);
    GENERATION.fetch_add(1, Ordering::SeqCst);
}

/// For platforms that always report releases, so the first hold repeats too.
#[cfg(windows)]
pub fn releases_reported() {
    RELEASE_SEEN.store(true, Ordering::SeqCst);
}

/// Called by the controller after it has run a trigger.
pub fn executed() {
    IN_FLIGHT.store(false, Ordering::SeqCst);
}
