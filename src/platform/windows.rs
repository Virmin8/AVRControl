//! Windows: tray icon + RegisterHotKey-based global hotkeys, both serviced
//! by a Win32 message loop on the main thread.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::{Result, bail};
use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::config::{APP_NAME, Action, Config};
use crate::controller::{Event, Hooks};

const WM_APP_RELOAD: u32 = WM_APP + 1;
const WM_APP_QUIT: u32 = WM_APP + 2;
const WM_APP_STATUS: u32 = WM_APP + 3;
const WM_APP_UPDATE: u32 = WM_APP + 4;

const MENU_SETTINGS: &str = "__settings";
const MENU_RECONNECT: &str = "__reconnect";
const MENU_QUIT: &str = "__quit";
const MENU_UPDATE: &str = "__update";

/// The controller's event queue, for sleep and wake-up from the window procedure.
static EVENTS: OnceLock<Sender<Event>> = OnceLock::new();
/// Whether the tray icon's middle click mutes.
static TRAY_VOLUME: AtomicBool = AtomicBool::new(true);

unsafe extern "system" fn session_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    // wParam: the session really is ending. Logging off leaves the receiver alone.
    if msg == WM_ENDSESSION && wp != 0 && (lp as u32 & ENDSESSION_LOGOFF) == 0 {
        crate::shutdown::power_off();
        return 0;
    }
    if msg == WM_POWERBROADCAST {
        match wp as u32 {
            // Windows allows about two seconds here before sleeping anyway.
            PBT_APMSUSPEND => {
                if let Some(tx) = EVENTS.get() {
                    let (done, wait) = std::sync::mpsc::channel();
                    if tx.send(Event::Sleep(done)).is_ok() {
                        let _ = wait.recv_timeout(std::time::Duration::from_millis(1800));
                    }
                }
            }
            PBT_APMRESUMEAUTOMATIC => {
                if let Some(tx) = EVENTS.get() {
                    let _ = tx.send(Event::Resume);
                }
            }
            _ => {}
        }
        return 1;
    }
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

/// A hidden top-level window: only those are sent WM_QUERYENDSESSION and
/// WM_ENDSESSION when Windows shuts down (thread messages and message-only
/// windows aren't).
fn create_session_window() {
    let class: Vec<u16> = "AVRControlSession\0".encode_utf16().collect();
    unsafe {
        let instance = GetModuleHandleW(std::ptr::null());
        let mut wc: WNDCLASSW = std::mem::zeroed();
        wc.lpfnWndProc = Some(session_proc);
        wc.hInstance = instance;
        wc.lpszClassName = class.as_ptr();
        RegisterClassW(&wc);
        CreateWindowExW(
            0,
            class.as_ptr(),
            class.as_ptr(),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        );
    }
}

pub fn configure_shortcuts() -> Result<()> {
    bail!("hotkeys are configured in AVR Control's settings on Windows")
}

fn build_menu(cfg: &Config, update: Option<&str>) -> Menu {
    let menu = Menu::new();
    for s in &cfg.shortcuts {
        let _ = menu.append(&MenuItem::with_id(s.id.as_str(), &s.name, true, None));
    }
    let _ = menu.append(&PredefinedMenuItem::separator());
    if let Some(label) = update {
        let _ = menu.append(&MenuItem::with_id(MENU_UPDATE, label, true, None));
    }
    let _ = menu.append(&MenuItem::with_id(MENU_SETTINGS, "Settings…", true, None));
    let _ = menu.append(&MenuItem::with_id(MENU_RECONNECT, "Reconnect", true, None));
    let _ = menu.append(&MenuItem::with_id(MENU_QUIT, "Quit", true, None));
    menu
}

/// Registers every shortcut's hotkey, returning hotkey id -> (shortcut id, repeatable).
fn register(
    manager: &GlobalHotKeyManager,
    cfg: &Config,
) -> (Vec<HotKey>, HashMap<u32, (String, bool)>) {
    let mut registered = Vec::new();
    let mut map = HashMap::new();
    let mut failed = Vec::new();
    for s in &cfg.shortcuts {
        if s.hotkey.trim().is_empty() {
            continue;
        }
        // Normalise first: our parser accepts aliases (Win, Meta, PgUp, ...)
        // that global-hotkey doesn't.
        let canonical = crate::hotkey::Hotkey::parse(&s.hotkey).map(|h| h.to_string());
        match canonical.as_deref().unwrap_or(&s.hotkey).parse::<HotKey>() {
            Ok(hk) => match manager.register(hk) {
                Ok(()) => {
                    map.insert(hk.id(), (s.id.clone(), s.action.repeatable()));
                    registered.push(hk);
                }
                Err(_) => failed.push(s.hotkey.clone()),
            },
            Err(_) => failed.push(s.hotkey.clone()),
        }
    }
    if !failed.is_empty() {
        let _ = notify_rust::Notification::new()
            .summary(APP_NAME)
            .body(&format!(
                "Could not register hotkeys (in use by another app?): {}",
                failed.join(", ")
            ))
            .show();
    }
    (registered, map)
}

pub fn run(
    cfg: Config,
    tx: Sender<Event>,
    start: impl FnOnce(Config, Hooks) + Send + 'static,
) -> Result<()> {
    let thread_id = unsafe { GetCurrentThreadId() };
    let pending: Arc<Mutex<Option<Config>>> = Arc::new(Mutex::new(None));
    let pending_status: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let pending_update: Arc<Mutex<Option<Option<String>>>> = Arc::new(Mutex::new(None));

    let img = crate::icon_rgba();
    let icon = Icon::from_rgba(img.data.clone(), img.width, img.height)?;
    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(build_menu(&cfg, None)))
        .with_icon(icon)
        .with_tooltip(APP_NAME)
        // Left click opens settings like on Linux; the menu is on right click.
        .with_menu_on_left_click(false)
        .build()?;

    let _ = EVENTS.set(tx.clone());
    TRAY_VOLUME.store(cfg.general.tray_volume, Ordering::Relaxed);
    let tray_tx = tx.clone();
    TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| match e {
        TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } => crate::daemon::open_settings(),
        TrayIconEvent::Click {
            button: MouseButton::Middle,
            button_state: MouseButtonState::Up,
            ..
        } if TRAY_VOLUME.load(Ordering::Relaxed) => {
            crate::controller::tray_action(&tray_tx, Action::MuteToggle)
        }
        _ => {}
    }));

    let menu_tx = tx.clone();
    MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
        let event = match e.id.0.as_str() {
            MENU_SETTINGS => {
                crate::daemon::open_settings();
                return;
            }
            MENU_UPDATE => {
                crate::update::activate();
                return;
            }
            MENU_RECONNECT => Event::Reconnect,
            MENU_QUIT => Event::Quit,
            id => Event::Trigger(id.to_string()),
        };
        let _ = menu_tx.send(event);
    }));

    create_session_window();

    let manager = GlobalHotKeyManager::new()?;
    let (mut registered, map) = register(&manager, &cfg);
    let map = Arc::new(Mutex::new(map));
    let hk_map = map.clone();
    let hk_tx = tx.clone();
    GlobalHotKeyEvent::set_event_handler(Some(move |e: GlobalHotKeyEvent| match e.state {
        HotKeyState::Pressed => {
            if let Some((id, repeatable)) = hk_map.lock().unwrap().get(&e.id) {
                crate::repeat::pressed(&hk_tx, id, *repeatable);
            }
        }
        HotKeyState::Released => crate::repeat::released(),
    }));

    let reload_pending = pending.clone();
    let status_pending = pending_status.clone();
    let update_pending = pending_update.clone();
    // The menu is rebuilt on reload and on update changes, each needing the other.
    let mut menu_cfg = cfg.clone();
    let mut update_label: Option<String> = None;
    start(
        cfg,
        Hooks {
            on_reload: Box::new(move |c| {
                *reload_pending.lock().unwrap() = Some(c.clone());
                unsafe { PostThreadMessageW(thread_id, WM_APP_RELOAD, 0, 0) };
            }),
            on_quit: Box::new(move || unsafe {
                PostThreadMessageW(thread_id, WM_APP_QUIT, 0, 0);
            }),
            on_status: std::sync::Arc::new(move |text| {
                *status_pending.lock().unwrap() = Some(text);
                unsafe { PostThreadMessageW(thread_id, WM_APP_STATUS, 0, 0) };
            }),
            on_update: std::sync::Arc::new(move |label| {
                *update_pending.lock().unwrap() = Some(label);
                unsafe { PostThreadMessageW(thread_id, WM_APP_UPDATE, 0, 0) };
            }),
        },
    );

    unsafe {
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            if msg.hwnd.is_null() && msg.message == WM_APP_QUIT {
                break;
            }
            if msg.hwnd.is_null() && msg.message == WM_APP_STATUS {
                if let Some(text) = pending_status.lock().unwrap().take() {
                    // Windows tray tooltips are limited to 127 characters.
                    let tip: String = format!("{APP_NAME}\n{text}").chars().take(127).collect();
                    let _ = tray.set_tooltip(Some(tip));
                }
                continue;
            }
            if msg.hwnd.is_null() && msg.message == WM_APP_UPDATE {
                if let Some(label) = pending_update.lock().unwrap().take() {
                    update_label = label;
                    tray.set_menu(Some(Box::new(build_menu(
                        &menu_cfg,
                        update_label.as_deref(),
                    ))));
                }
                continue;
            }
            if msg.hwnd.is_null() && msg.message == WM_APP_RELOAD {
                if let Some(cfg) = pending.lock().unwrap().take() {
                    let _ = manager.unregister_all(&registered);
                    let (r, m) = register(&manager, &cfg);
                    registered = r;
                    *map.lock().unwrap() = m;
                    tray.set_menu(Some(Box::new(build_menu(&cfg, update_label.as_deref()))));
                    TRAY_VOLUME.store(cfg.general.tray_volume, Ordering::Relaxed);
                    menu_cfg = cfg;
                }
                continue;
            }
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    let _ = manager.unregister_all(&registered);
    drop(tray);
    Ok(())
}
