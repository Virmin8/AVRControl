//! Update checks against the latest GitHub release. The AppImage replaces
//! itself and Windows runs the next release's installer, both after checking
//! the download against its `.sha256` asset; package installs (pacman, NixOS)
//! are only told about the release, since their package manager updates them.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use notify_rust::Notification;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::config::{APP_NAME, Config};
use crate::controller::Event;

const LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/Virmin8/AVRControl/releases/latest";
/// Lets the first check wait until the tray and receiver connection are up.
const FIRST_CHECK_DELAY: Duration = Duration::from_secs(20);
const CHECK_INTERVAL: Duration = Duration::from_secs(48 * 60 * 60);

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InstallKind {
    AppImage,
    Windows,
    Pacman,
    NixOS,
    /// Built from source or copied somewhere by hand.
    Other,
}

pub fn install_kind() -> InstallKind {
    if cfg!(windows) {
        // Only an installed copy updates through the installer; the portable
        // exe (no Inno uninstaller beside it) is pointed at the release page.
        let installed = std::env::current_exe()
            .ok()
            .and_then(|exe| Some(exe.parent()?.join("unins000.exe").exists()))
            .unwrap_or(false);
        return if installed {
            InstallKind::Windows
        } else {
            InstallKind::Other
        };
    }
    if std::env::var_os("APPIMAGE").is_some() {
        return InstallKind::AppImage;
    }
    let os_release = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
    if os_release
        .lines()
        .any(|l| matches!(l.trim(), "ID=nixos" | "ID=\"nixos\"" | "ID='nixos'"))
    {
        return InstallKind::NixOS;
    }
    let packaged = std::env::current_exe()
        .map(|exe| exe.starts_with("/usr"))
        .unwrap_or(false);
    if packaged && Path::new("/usr/bin/pacman").exists() {
        return InstallKind::Pacman;
    }
    InstallKind::Other
}

#[derive(Debug, Clone, Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

#[derive(Debug, Clone, Deserialize)]
struct Release {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<Asset>,
}

#[derive(Debug, Clone)]
pub struct Offer {
    pub version: String,
    page: String,
    /// The file to install and its checksum; None means "tell, don't install".
    download: Option<(Asset, Asset)>,
}

impl Offer {
    /// The tray menu entry.
    fn label(&self) -> String {
        if self.download.is_some() {
            format!("Update to {}", self.version)
        } else {
            format!("Update available: {}", self.version)
        }
    }

    fn how_to(&self, kind: InstallKind) -> String {
        match (kind, &self.download) {
            (_, Some(_)) => format!("Choose \"{}\" in the tray menu.", self.label()),
            (InstallKind::Pacman, _) => {
                "Update with sudo pacman -Syu or your system updater.".into()
            }
            (InstallKind::NixOS, _) => "Update through your NixOS configuration or flake.".into(),
            _ => "Download it from the release page.".into(),
        }
    }
}

fn parse_version(v: &str) -> Option<(Vec<u64>, bool)> {
    let v = v.trim().trim_start_matches('v');
    let (core, pre) = match v.split_once('-') {
        Some((core, _)) => (core, true),
        None => (v, false),
    };
    let mut parts: Vec<u64> = core
        .split('.')
        .map(|p| p.parse().ok())
        .collect::<Option<_>>()?;
    parts.resize(3, 0);
    Some((parts, pre))
}

/// True when `candidate` is a newer version than `current`. A prerelease
/// ranks below the release it leads to.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    match (parse_version(candidate), parse_version(current)) {
        (Some((a, a_pre)), Some((b, b_pre))) => a > b || (a == b && b_pre && !a_pre),
        _ => false,
    }
}

/// The release file this install updates from.
fn is_update_asset(name: &str, kind: InstallKind) -> bool {
    match kind {
        InstallKind::AppImage => name.ends_with(".AppImage"),
        InstallKind::Windows => name.starts_with("avrcontrol-") && name.ends_with("-setup.exe"),
        _ => false,
    }
}

fn decide(release: Release, current: &str, kind: InstallKind) -> Option<Offer> {
    let version = release.tag_name.trim_start_matches('v').to_string();
    if release.draft || release.prerelease || !is_newer(&version, current) {
        return None;
    }
    let download = release
        .assets
        .iter()
        .find(|a| is_update_asset(&a.name, kind))
        .and_then(|file| {
            let sum_name = format!("{}.sha256", file.name);
            let sum = release.assets.iter().find(|a| a.name == sum_name)?;
            Some((file.clone(), sum.clone()))
        });
    Some(Offer {
        version,
        page: release.html_url,
        download,
    })
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(15))
        // Per read, so a slow but steady download is never cut off.
        .timeout_read(Duration::from_secs(60))
        .user_agent(&format!("AVRControl/{}", env!("CARGO_PKG_VERSION")))
        .build()
}

fn fetch_latest() -> Result<Release> {
    let body = agent()
        .get(LATEST_RELEASE_URL)
        .set("Accept", "application/vnd.github+json")
        .call()
        .context("could not reach GitHub")?
        .into_string()?;
    Ok(serde_json::from_str(&body)?)
}

/// Checks GitHub once and describes the result, without touching the tray.
/// Used by the settings window when the background service isn't running.
pub fn describe_latest() -> Result<String> {
    let kind = install_kind();
    Ok(
        match decide(fetch_latest()?, env!("CARGO_PKG_VERSION"), kind) {
            Some(offer) => format!("Version {} is available. {}", offer.version, offer.how_to(kind)),
            None => format!("You're up to date ({}).", env!("CARGO_PKG_VERSION")),
        },
    )
}

type OnUpdate = Arc<dyn Fn(Option<String>) + Send + Sync>;

struct Ctx {
    tx: Sender<Event>,
    on_update: OnUpdate,
    offer: Option<Offer>,
    /// Version a notification was last shown for, so each release is announced once.
    notified: Option<String>,
    busy: bool,
}

static CTX: Mutex<Option<Ctx>> = Mutex::new(None);
/// Set once an AppImage update is in place: the daemon starts it after it
/// has released the IPC socket.
static RESTART: Mutex<Option<PathBuf>> = Mutex::new(None);

fn notify(body: &str) {
    if let Err(e) = Notification::new()
        .summary(APP_NAME)
        .appname(APP_NAME)
        .body(body)
        .show()
    {
        eprintln!("notification failed: {e}");
    }
}

/// Starts the background checks: shortly after launch, then every 48 hours,
/// unless "Check for updates" is off in Settings.
pub fn spawn(tx: Sender<Event>, on_update: OnUpdate) {
    *CTX.lock().unwrap() = Some(Ctx {
        tx,
        on_update,
        offer: None,
        notified: None,
        busy: false,
    });
    std::thread::spawn(|| {
        std::thread::sleep(FIRST_CHECK_DELAY);
        loop {
            let enabled = Config::load()
                .map(|c| c.general.check_updates)
                .unwrap_or(true);
            if enabled && let Err(e) = check(false) {
                eprintln!("update check failed: {e:#}");
            }
            std::thread::sleep(CHECK_INTERVAL);
        }
    });
}

/// Checks now and returns a line for the settings window.
pub fn check_now() -> Result<String> {
    let kind = install_kind();
    Ok(match check(true)? {
        Some(offer) => format!("Version {} is available. {}", offer.version, offer.how_to(kind)),
        None => format!("You're up to date ({}).", env!("CARGO_PKG_VERSION")),
    })
}

fn check(manual: bool) -> Result<Option<Offer>> {
    let kind = install_kind();
    let offer = decide(fetch_latest()?, env!("CARGO_PKG_VERSION"), kind);
    let mut guard = CTX.lock().unwrap();
    let Some(ctx) = guard.as_mut() else {
        return Ok(offer);
    };
    if ctx.busy {
        return Ok(offer);
    }
    (ctx.on_update)(offer.as_ref().map(Offer::label));
    ctx.offer = offer.clone();
    if let Some(o) = &offer
        && !manual
        && ctx.notified.as_deref() != Some(o.version.as_str())
    {
        ctx.notified = Some(o.version.clone());
        let body = format!("{APP_NAME} {} is available. {}", o.version, o.how_to(kind));
        std::thread::spawn(move || notify(&body));
    }
    Ok(offer)
}

/// The tray's update entry: installs the offered release, or opens its page
/// when the package manager does the updating.
pub fn activate() {
    std::thread::spawn(|| {
        if let Err(e) = install() {
            eprintln!("update failed: {e:#}");
            notify(&format!("The update didn't install: {e:#}"));
        }
        if let Some(ctx) = CTX.lock().unwrap().as_mut() {
            ctx.busy = false;
        }
    });
}

fn install() -> Result<()> {
    let (offer, tx) = {
        let mut guard = CTX.lock().unwrap();
        let ctx = guard.as_mut().ok_or_else(|| anyhow!("not running"))?;
        if ctx.busy {
            return Ok(());
        }
        let offer = ctx.offer.clone().ok_or_else(|| anyhow!("no update available"))?;
        ctx.busy = offer.download.is_some();
        (offer, ctx.tx.clone())
    };
    let Some((file, sum)) = &offer.download else {
        open_url(&offer.page);
        return Ok(());
    };

    match install_kind() {
        InstallKind::AppImage => {
            let target = PathBuf::from(
                std::env::var_os("APPIMAGE").ok_or_else(|| anyhow!("not running as an AppImage"))?,
            );
            let name = target
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            // Next to the AppImage, so the final rename stays on one filesystem.
            let temp = target.with_file_name(format!(".{name}.update"));
            notify(&format!("Downloading {APP_NAME} {}…", offer.version));
            download_verified(file, sum, &temp)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o755))?;
            }
            if let Err(e) = std::fs::rename(&temp, &target) {
                let _ = std::fs::remove_file(&temp);
                return Err(e).context(format!("could not replace {}", target.display()));
            }
            *RESTART.lock().unwrap() = Some(target);
            let _ = tx.send(Event::Quit);
        }
        InstallKind::Windows => {
            let dir = std::env::temp_dir().join("AVRControl-update");
            std::fs::create_dir_all(&dir)?;
            let setup = dir.join(&file.name);
            notify(&format!("Downloading {APP_NAME} {}…", offer.version));
            download_verified(file, sum, &setup)?;
            // /SILENT shows only a progress window; the installer quits this
            // instance and starts the new one (see avrcontrol.iss).
            std::process::Command::new(&setup)
                .args(["/SILENT", "/SUPPRESSMSGBOXES", "/NORESTART"])
                .spawn()
                .context("could not start the installer")?;
            let _ = tx.send(Event::Quit);
        }
        _ => open_url(&offer.page),
    }
    Ok(())
}

fn download_verified(file: &Asset, sum: &Asset, dest: &Path) -> Result<()> {
    let agent = agent();
    let expected = agent
        .get(&sum.browser_download_url)
        .call()
        .context("could not download the checksum")?
        .into_string()?
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if expected.len() != 64 {
        bail!("the release's checksum file is malformed");
    }

    let result = (|| -> Result<()> {
        let mut reader = agent
            .get(&file.browser_download_url)
            .call()
            .context("could not download the update")?
            .into_reader();
        let mut out = std::fs::File::create(dest)?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let n = reader.read(&mut buf).context("the download was interrupted")?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            std::io::Write::write_all(&mut out, &buf[..n])?;
        }
        out.sync_all()?;
        let actual: String = hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        if actual != expected {
            bail!("the download didn't verify");
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(dest);
    }
    result
}

fn open_url(url: &str) {
    #[cfg(windows)]
    let result = std::process::Command::new("rundll32")
        .args(["url.dll,FileProtocolHandler", url])
        .spawn();
    #[cfg(not(windows))]
    let result = std::process::Command::new("xdg-open").arg(url).spawn();
    if let Err(e) = result {
        eprintln!("could not open {url}: {e}");
    }
}

/// Called by the daemon after it has shut down: starts the updated AppImage.
pub fn restart_if_updated() {
    if let Some(path) = RESTART.lock().unwrap().take()
        && let Err(e) = std::process::Command::new(&path).arg("daemon").spawn()
    {
        eprintln!("could not start {}: {e}", path.display());
    }
}

/// Removes a half-finished AppImage download left by a crash or power loss.
pub fn clean_stale_download() {
    if let Some(target) = std::env::var_os("APPIMAGE").map(PathBuf::from)
        && let Some(name) = target.file_name()
    {
        let _ = std::fs::remove_file(
            target.with_file_name(format!(".{}.update", name.to_string_lossy())),
        );
    }
    #[cfg(windows)]
    let _ = std::fs::remove_dir_all(std::env::temp_dir().join("AVRControl-update"));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str, assets: &[&str]) -> Release {
        Release {
            tag_name: tag.into(),
            html_url: "https://example.invalid/release".into(),
            draft: false,
            prerelease: false,
            assets: assets
                .iter()
                .map(|n| Asset {
                    name: n.to_string(),
                    browser_download_url: format!("https://example.invalid/{n}"),
                })
                .collect(),
        }
    }

    #[test]
    fn version_ordering() {
        assert!(is_newer("1.0.1", "1.0.0"));
        assert!(is_newer("v1.10.0", "1.9.9"));
        assert!(is_newer("1.1", "1.0.9"));
        assert!(is_newer("1.0.0", "1.0.0-beta.1"));
        assert!(!is_newer("1.0.0-beta.1", "1.0.0"));
        assert!(!is_newer("1.0.0", "1.0.0"));
        assert!(!is_newer("0.9.0", "1.0.0"));
        assert!(!is_newer("garbage", "1.0.0"));
    }

    #[test]
    fn appimage_offer_needs_its_checksum() {
        let with_sum = release(
            "v1.1.0",
            &["avrcontrol-1.1.0-x86_64.AppImage", "avrcontrol-1.1.0-x86_64.AppImage.sha256"],
        );
        let offer = decide(with_sum, "1.0.0", InstallKind::AppImage).unwrap();
        assert_eq!(offer.version, "1.1.0");
        assert_eq!(offer.download.unwrap().0.name, "avrcontrol-1.1.0-x86_64.AppImage");

        // Without a checksum it is still announced, but not installed.
        let no_sum = release("v1.1.0", &["avrcontrol-1.1.0-x86_64.AppImage"]);
        assert!(decide(no_sum, "1.0.0", InstallKind::AppImage).unwrap().download.is_none());
    }

    #[test]
    fn windows_picks_the_installer_not_the_portable_exe() {
        let r = release(
            "v1.1.0",
            &[
                "avrcontrol-1.1.0-windows-x64.exe",
                "avrcontrol-1.1.0-windows-x64.exe.sha256",
                "avrcontrol-1.1.0-setup.exe",
                "avrcontrol-1.1.0-setup.exe.sha256",
            ],
        );
        let offer = decide(r, "1.0.0", InstallKind::Windows).unwrap();
        assert_eq!(offer.download.unwrap().0.name, "avrcontrol-1.1.0-setup.exe");
    }

    #[test]
    fn packages_are_told_not_installed() {
        let r = release(
            "v1.1.0",
            &["avrcontrol-1.1.0-x86_64.AppImage", "avrcontrol-1.1.0-x86_64.AppImage.sha256"],
        );
        let offer = decide(r, "1.0.0", InstallKind::Pacman).unwrap();
        assert!(offer.download.is_none());
        assert!(offer.how_to(InstallKind::Pacman).contains("pacman -Syu"));
    }

    #[test]
    fn skips_old_draft_and_prerelease() {
        assert!(decide(release("v1.0.0", &[]), "1.0.0", InstallKind::Other).is_none());
        let mut draft = release("v2.0.0", &[]);
        draft.draft = true;
        assert!(decide(draft, "1.0.0", InstallKind::Other).is_none());
        let mut pre = release("v2.0.0-rc.1", &[]);
        pre.prerelease = true;
        assert!(decide(pre, "1.0.0", InstallKind::Other).is_none());
    }
}
