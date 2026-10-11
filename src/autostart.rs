//! Start-on-login toggle: an XDG autostart entry on Linux, the HKCU Run key
//! on Windows.

use anyhow::Result;

fn exe() -> Result<String> {
    Ok(std::env::current_exe()?.to_string_lossy().into_owned())
}

#[cfg(target_os = "linux")]
fn desktop_file() -> std::path::PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))
        .unwrap_or_default();
    base.join("autostart").join("avrcontrol.desktop")
}

#[cfg(target_os = "linux")]
pub fn is_enabled() -> bool {
    desktop_file().exists()
}

#[cfg(target_os = "linux")]
pub fn set_enabled(enabled: bool) -> Result<()> {
    let path = desktop_file();
    if !enabled {
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        return Ok(());
    }
    std::fs::create_dir_all(path.parent().unwrap())?;
    let text = format!(
        "[Desktop Entry]\nType=Application\nName=AVR Control\nComment=Onkyo receiver and monitor input hotkeys\n\
         Exec=\"{}\" daemon\nIcon={}\nTerminal=false\nX-GNOME-Autostart-enabled=true\n",
        exe()?,
        crate::config::APP_ID
    );
    std::fs::write(path, text)?;
    Ok(())
}

#[cfg(windows)]
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

#[cfg(windows)]
pub fn is_enabled() -> bool {
    winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
        .open_subkey(RUN_KEY)
        .and_then(|k| k.get_value::<String, _>("AVRControl"))
        .is_ok()
}

#[cfg(windows)]
pub fn set_enabled(enabled: bool) -> Result<()> {
    let (key, _) =
        winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER).create_subkey(RUN_KEY)?;
    if enabled {
        key.set_value("AVRControl", &format!("\"{}\" daemon", exe()?))?;
    } else {
        let _ = key.delete_value("AVRControl");
    }
    Ok(())
}
