//! Linux: StatusNotifierItem tray (ksni) and global shortcuts through the
//! XDG desktop portal, which works on Wayland (KDE Plasma, GNOME 48+, ...).
//! Shortcuts can additionally be bound to `avrcontrol trigger <id>` in any
//! desktop's own shortcut settings.

use std::sync::OnceLock;
use std::sync::mpsc::Sender;

use anyhow::{Result, anyhow};
use ashpd::desktop::Session;
use ashpd::desktop::global_shortcuts::{BindShortcutsOptions, GlobalShortcuts, NewShortcut};
use ksni::TrayMethods;
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

use crate::config::{APP_ID, APP_NAME, Config};
use crate::controller::{Event, Hooks};
use crate::hotkey::Hotkey;

enum Msg {
    Rebind(Config),
    Status(String),
    Update(Option<String>),
    ConfigureShortcuts,
    Quit,
}

static PLATFORM_TX: OnceLock<UnboundedSender<Msg>> = OnceLock::new();

pub fn configure_shortcuts() -> Result<()> {
    PLATFORM_TX
        .get()
        .ok_or_else(|| anyhow!("not running"))?
        .send(Msg::ConfigureShortcuts)
        .map_err(|_| anyhow!("not running"))
}

struct Tray {
    tx: Sender<Event>,
    shortcuts: Vec<(String, String)>,
    /// Receiver status summary shown in the tooltip.
    status: String,
    /// Label of the update entry, when a release is available.
    update: Option<String>,
}

impl ksni::Tray for Tray {
    fn id(&self) -> String {
        "avrcontrol".into()
    }

    fn title(&self) -> String {
        APP_NAME.into()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        let img = crate::icon_rgba();
        let mut data = img.data.clone();
        for px in data.as_chunks_mut::<4>().0 {
            px.rotate_right(1); // RGBA -> ARGB
        }
        vec![ksni::Icon {
            width: img.width as i32,
            height: img.height as i32,
            data,
        }]
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: APP_NAME.into(),
            description: self.status.clone(),
            ..Default::default()
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        crate::daemon::open_settings();
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::*;
        let mut items: Vec<ksni::MenuItem<Self>> = self
            .shortcuts
            .iter()
            .map(|(id, name)| {
                let id = id.clone();
                StandardItem {
                    label: name.clone(),
                    activate: Box::new(move |t: &mut Self| {
                        let _ = t.tx.send(Event::Trigger(id.clone()));
                    }),
                    ..Default::default()
                }
                .into()
            })
            .collect();
        items.push(MenuItem::Separator);
        if let Some(label) = &self.update {
            items.push(
                StandardItem {
                    label: label.clone(),
                    icon_name: "system-software-update".into(),
                    activate: Box::new(|_: &mut Self| crate::update::activate()),
                    ..Default::default()
                }
                .into(),
            );
        }
        items.push(
            StandardItem {
                label: "Settings…".into(),
                icon_name: "configure".into(),
                activate: Box::new(|_: &mut Self| crate::daemon::open_settings()),
                ..Default::default()
            }
            .into(),
        );
        items.push(
            StandardItem {
                label: "Reconnect".into(),
                icon_name: "view-refresh".into(),
                activate: Box::new(|t: &mut Self| {
                    let _ = t.tx.send(Event::Reconnect);
                }),
                ..Default::default()
            }
            .into(),
        );
        items.push(
            StandardItem {
                label: "Quit".into(),
                icon_name: "application-exit".into(),
                activate: Box::new(|t: &mut Self| {
                    let _ = t.tx.send(Event::Quit);
                }),
                ..Default::default()
            }
            .into(),
        );
        items
    }
}

fn menu_entries(cfg: &Config) -> Vec<(String, String)> {
    cfg.shortcuts
        .iter()
        .map(|s| (s.id.clone(), s.name.clone()))
        .collect()
}

/// One live portal session with its Activated listener.
struct PortalBinding {
    session: Session<GlobalShortcuts>,
    listener: tokio::task::JoinHandle<()>,
}

impl PortalBinding {
    async fn close(self) {
        self.listener.abort();
        let _ = self.session.close().await;
    }
}

async fn bind_portal(cfg: &Config, tx: Sender<Event>) -> Result<PortalBinding> {
    use futures_util::StreamExt;

    let portal = GlobalShortcuts::new().await?;
    let session = portal.create_session(Default::default()).await?;
    let triggers: Vec<Option<String>> = cfg
        .shortcuts
        .iter()
        .map(|s| Hotkey::parse(&s.hotkey).map(|h| h.portal_trigger()))
        .collect();
    let shortcuts: Vec<NewShortcut> = cfg
        .shortcuts
        .iter()
        .zip(&triggers)
        .map(|(s, t)| {
            NewShortcut::new(s.id.clone(), s.name.clone()).preferred_trigger(t.as_deref())
        })
        .collect();
    portal
        .bind_shortcuts(&session, &shortcuts, None, BindShortcutsOptions::default())
        .await?
        .response()?;

    // Only one session is live at a time (the old one is closed before
    // rebinding), so every signal on this connection is ours.
    enum Key {
        Down(String),
        Up,
    }
    let repeatable: std::collections::HashSet<String> = cfg
        .shortcuts
        .iter()
        .filter(|s| s.action.repeatable())
        .map(|s| s.id.clone())
        .collect();
    let activated = portal
        .receive_activated()
        .await?
        .map(|ev| Key::Down(ev.shortcut_id().to_string()));
    let deactivated = portal.receive_deactivated().await?.map(|_| Key::Up);
    let mut events = futures_util::stream::select(activated, deactivated);
    let listener = tokio::spawn(async move {
        // Keep the proxy alive for as long as we listen.
        let _portal = portal;
        while let Some(ev) = events.next().await {
            match ev {
                Key::Down(id) => crate::repeat::pressed(&tx, &id, repeatable.contains(&id)),
                Key::Up => crate::repeat::released(),
            }
        }
    });
    Ok(PortalBinding { session, listener })
}

enum PortalMsg {
    Bind(Box<Config>),
    Configure,
}

/// Owns the portal session. Runs separately from the main loop because
/// BindShortcuts only returns once the user has answered the desktop's
/// confirmation dialog, which can take arbitrarily long.
async fn portal_manager(
    mut rx: tokio::sync::mpsc::UnboundedReceiver<PortalMsg>,
    tx: Sender<Event>,
) {
    let mut binding: Option<PortalBinding> = None;
    let mut last_cfg: Option<Config> = None;
    while let Some(msg) = rx.recv().await {
        let cfg = match msg {
            PortalMsg::Bind(cfg) => *cfg,
            PortalMsg::Configure => {
                if let Some(b) = &binding {
                    let result = async {
                        GlobalShortcuts::new()
                            .await?
                            .configure_shortcuts(&b.session, None, Default::default())
                            .await
                    }
                    .await;
                    if let Err(e) = result {
                        eprintln!("configure shortcuts: {e}");
                    }
                    continue;
                }
                // Registration failed or was declined earlier: ask again.
                match &last_cfg {
                    Some(c) => c.clone(),
                    None => continue,
                }
            }
        };
        if let Some(b) = binding.take() {
            b.close().await;
        }
        match bind_portal(&cfg, tx.clone()).await {
            Ok(b) => binding = Some(b),
            Err(e) => notify_portal_error(&e),
        }
        last_cfg = Some(cfg);
    }
}

/// Holds a logind "delay" inhibitor so that, when the computer shuts down,
/// the controller gets a few seconds to turn the receiver off first.
async fn watch_shutdown(tx: Sender<Event>) -> zbus::Result<()> {
    use futures_util::StreamExt;

    let conn = zbus::Connection::system().await?;
    let logind: zbus::Proxy = zbus::proxy::Builder::new(&conn)
        .destination("org.freedesktop.login1")?
        .path("/org/freedesktop/login1")?
        .interface("org.freedesktop.login1.Manager")?
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .await?;
    let mut signals = logind.receive_signal("PrepareForShutdown").await?;
    loop {
        let lock: zbus::zvariant::OwnedFd = logind
            .call(
                "Inhibit",
                &("shutdown", APP_NAME, "Turning off the receiver", "delay"),
            )
            .await?;
        // PrepareForShutdown(true) when it starts, (false) if it's cancelled.
        let mut starting = false;
        while !starting {
            let Some(msg) = signals.next().await else {
                return Ok(());
            };
            starting = msg.body().deserialize::<bool>()?;
        }
        if !is_reboot(&logind).await {
            let tx = tx.clone();
            let _ = tokio::task::spawn_blocking(move || crate::controller::shutdown(&tx)).await;
        }
        drop(lock);
        // Still running: the shutdown was cancelled. Wait for logind to say
        // so, then take a new lock.
        while starting {
            let Some(msg) = signals.next().await else {
                return Ok(());
            };
            starting = msg.body().deserialize::<bool>()?;
        }
    }
}

/// A reboot rather than a power-off; the receiver is left on since this PC
/// will be back shortly. Needs systemd 255+; older versions count as power-off.
async fn is_reboot(logind: &zbus::Proxy<'_>) -> bool {
    use std::collections::HashMap;
    use zbus::zvariant::OwnedValue;

    let Ok(meta) = logind
        .get_property::<HashMap<String, OwnedValue>>("PreparingForShutdownWithMetadata")
        .await
    else {
        return false;
    };
    meta.get("type")
        .and_then(|v| <&str>::try_from(&**v).ok())
        .is_some_and(|t| matches!(t, "reboot" | "soft-reboot" | "kexec"))
}

pub fn run(
    cfg: Config,
    tx: Sender<Event>,
    start: impl FnOnce(Config, Hooks) + Send + 'static,
) -> Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(async move {
        let (ptx, mut prx) = unbounded_channel::<Msg>();
        let _ = PLATFORM_TX.set(ptx.clone());

        let reload_tx = ptx.clone();
        let status_tx = ptx.clone();
        let update_tx = ptx.clone();
        let quit_tx = ptx;
        start(
            cfg.clone(),
            Hooks {
                on_reload: Box::new(move |c| {
                    let _ = reload_tx.send(Msg::Rebind(c.clone()));
                }),
                on_quit: Box::new(move || {
                    let _ = quit_tx.send(Msg::Quit);
                }),
                on_status: std::sync::Arc::new(move |text| {
                    let _ = status_tx.send(Msg::Status(text));
                }),
                on_update: std::sync::Arc::new(move |label| {
                    let _ = update_tx.send(Msg::Update(label));
                }),
            },
        );

        // Must happen before any other portal call on this connection so the
        // portal can associate us with our .desktop file.
        if let Err(e) = ashpd::register_host_app(APP_ID.try_into()?).await {
            eprintln!("portal host registration failed (fine if not installed): {e}");
        }

        let tray = Tray {
            tx: tx.clone(),
            shortcuts: menu_entries(&cfg),
            status: String::new(),
            update: None,
        };
        let tray = match tray.spawn().await {
            Ok(h) => Some(h),
            Err(e) => {
                eprintln!("tray icon unavailable: {e}");
                None
            }
        };

        let (portal_tx, portal_rx) = unbounded_channel();
        tokio::spawn(portal_manager(portal_rx, tx.clone()));
        let shutdown_tx = tx.clone();
        tokio::spawn(async move {
            if let Err(e) = watch_shutdown(shutdown_tx).await {
                eprintln!("shutdown handling unavailable: {e}");
            }
        });
        let _ = portal_tx.send(PortalMsg::Bind(Box::new(cfg)));

        while let Some(msg) = prx.recv().await {
            match msg {
                Msg::Rebind(cfg) => {
                    if let Some(t) = &tray {
                        let entries = menu_entries(&cfg);
                        t.update(move |t| t.shortcuts = entries).await;
                    }
                    let _ = portal_tx.send(PortalMsg::Bind(Box::new(cfg)));
                }
                Msg::Status(text) => {
                    if let Some(t) = &tray {
                        t.update(move |t| t.status = text).await;
                    }
                }
                Msg::Update(label) => {
                    if let Some(t) = &tray {
                        t.update(move |t| t.update = label).await;
                    }
                }
                Msg::ConfigureShortcuts => {
                    let _ = portal_tx.send(PortalMsg::Configure);
                }
                Msg::Quit => break,
            }
        }
        // The portal session ends with our D-Bus connection; just make sure
        // the tray icon disappears promptly.
        if let Some(t) = tray {
            let _ = tokio::time::timeout(std::time::Duration::from_secs(2), t.shutdown()).await;
        }
        Ok::<(), anyhow::Error>(())
    })
}

fn notify_portal_error(e: &anyhow::Error) {
    eprintln!("global shortcuts not registered: {e:#}");
    let body = format!(
        "Global shortcuts were not registered ({e}). To try again, use \"Open system \
         shortcut settings\" in Settings > Shortcuts, or bind `avrcontrol trigger <id>` \
         in your desktop's shortcut settings."
    );
    // notify-rust's blocking API must not run on a tokio worker thread.
    std::thread::spawn(move || {
        let _ = notify_rust::Notification::new()
            .summary(APP_NAME)
            .body(&body)
            .show();
    });
}
