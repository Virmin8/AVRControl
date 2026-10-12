# AVR Control

AVR Control switches an Onkyo AV receiver and your monitors' inputs from global
hotkeys. It runs in the system tray on Linux (Wayland and X11) and Windows.


## Features

### Shortcuts
- Global hotkeys that switch the **receiver input**, **any number of
  monitors**, or both in one press. A switch shortcut can also power the
  receiver on, pick the HDMI output, set a listening mode and set the volume.
- Built-in actions: power on/off/toggle, volume up/down, mute, HDMI audio
  out, listening mode (pick one or step through), HDMI output, set volume.
- A **Receiver setting** action for everything else the receiver supports:
  mute on/off, 1 dB volume steps, A/V sync, subwoofer and centre level,
  AccuEQ, Music Optimizer, Phase Matching Bass, late night, audio selector,
  display dimmer, sleep timer, HDMI CEC, 12 V triggers, tuner presets,
  network/USB playback, the receiver's on-screen menu, and zone 2/3 power,
  volume, mute and input.
- Holding a volume, level, tuner or menu-arrow key repeats it.
- Shortcuts are also in the tray menu and on the command line
  (`avrcontrol trigger <id>`).

### Receiver
- Finds the receiver on the network, reads its model and the inputs it
  actually has (with the receiver's own names).
- Knows exactly which commands and values each model supports, from Onkyo's
  protocol spreadsheet (252 models). Other models are probed on first
  connection.
- Controls for every supported function without a shortcut, grouped into
  power and volume, sound, HDMI, system and zones.
- Optionally turns the receiver off when the PC shuts down, if it's still on
  the startup input.
- Works with receivers that allow only one network client: the settings
  window and command line share the background service's connection.

### Monitors
- Lists monitors that support DDC/CI input switching, with their current
  input and the inputs they advertise; one click on the Status page
  switches input.
- On KDE Plasma also shows resolution, refresh rate, scale, position,
  rotation, colour depth, HDR, wide colour gamut, adaptive sync and screen
  size, plus brightness and contrast over DDC/CI.

### Status
- A Status page styled like the receiver's front panel: the current input,
  volume and listening mode on an amber display with format indicators
  (HDMI, Atmos, 4K, HDR, ...), input keys, a volume slider, the incoming
  video and audio signal, zones, and one-click input switching for every
  monitor.
- The receiver's connection and power state show at the top of every page.
- The tray tooltip shows the same at a glance.
- Volume changes show a small front-panel style overlay. Actions show
  desktop notifications.

### Desktop integration
- Linux: Wayland and X11. Hotkeys go through the XDG Global Shortcuts portal
  (KDE Plasma, GNOME 48+); the tray uses StatusNotifierItem.
- Windows: tray icon and `RegisterHotKey` hotkeys.
- Start on login, a systemd user unit, and an Arch package (PKGBUILD).
- Checks for new releases and, for the Windows installer and the AppImage,
  updates itself (see [Updates](#updates)).

## Settings window

Open it by clicking the tray icon, choosing **Settings…** from the tray menu,
or running `avrcontrol` while the app is already running.

| Tab | What it shows |
| --- | --- |
| Status | Live receiver state, input keys, the volume slider, video and audio signal, zones, and buttons to switch each monitor's input. |
| Receiver | **Controls**: every supported function, grouped into power and volume, sound, HDMI, system and zones, plus raw commands. **Setup**: IP address (**Find on network** searches for it), connection test, volume steps, supported features, and renaming or hiding receiver inputs. |
| Monitors | Everything each connected display reports: graphics-card connector (e.g. `DP-1`), current input, signal, resolution, HDR, brightness and more. |
| Shortcuts | Add, edit and reorder shortcuts. Set the hotkey (type it or press **Record**), then pick a receiver input and a target input for each monitor. |
| General | Startup action, turning the receiver off at shutdown (only if it is still on the startup input), notifications, volume indicator, start on login, and start/stop of the background service. |

The configuration is stored in `~/.config/avrcontrol/config.toml` (Linux) or
`%APPDATA%\avrcontrol\config\config.toml` (Windows).

## Command line

```
avrcontrol                 start in the background (opens settings if already running)
avrcontrol daemon          start the background service
avrcontrol settings [TAB]  open settings (status|receiver|monitors|shortcuts|general)
avrcontrol trigger <ID>    run a shortcut in the running instance
avrcontrol list            list shortcuts and their IDs
avrcontrol monitors        list monitors, connectors and inputs
avrcontrol receiver        show receiver status
avrcontrol discover        search the network for receivers
avrcontrol reload | quit   control the running instance
```

## Download

Every [release](https://github.com/Virmin8/AVRControl/releases) has:

| File | For |
| --- | --- |
| `avrcontrol-<version>-setup.exe` | Windows installer |
| `avrcontrol-<version>-windows-x64.exe` | Windows, portable: run it from anywhere |
| `avrcontrol-<version>-1-x86_64.pkg.tar.zst` | Arch Linux and derivatives |
| `avrcontrol-<version>-x86_64.AppImage` | Other Linux distributions |

What changed in each version is in [CHANGELOG.md](CHANGELOG.md).

### Updates

AVR Control looks for a new release when it starts and every two days after
that (turn this off in Settings → General, which also has **Check now**). When
one is out you get a notification and an entry in the tray menu:

- **Windows installer and AppImage:** choose **Update to x.y.z** in the tray
  menu. AVR Control downloads the release, checks it against its published
  SHA-256, installs it and starts again.
- **Arch package:** updates arrive with `sudo pacman -Syu` once you've added the
  repository below.
- **Portable Windows `.exe` and source builds:** the tray entry opens the
  release page.

## Linux

### Install (Arch / pacman)

Add the AVR Control repository to the end of `/etc/pacman.conf`:

```ini
[avrcontrol]
SigLevel = Optional TrustAll
Server = https://github.com/Virmin8/AVRControl/releases/latest/download
```

Then install it, and later updates come with your normal system update:

```sh
sudo pacman -Syu avrcontrol
```

The repository is unsigned, hence `TrustAll`; that's the same trust as
downloading the package from the release page and installing it with
`sudo pacman -U avrcontrol-<version>-1-x86_64.pkg.tar.zst`, which also works.

Or build it from source:

```sh
git clone https://github.com/Virmin8/AVRControl.git
cd AVRControl/packaging/arch
makepkg -si
```

After installing, start **AVR Control** from the application menu. To start it
on login, turn on **Start on login** in Settings → General, or run
`systemctl --user enable --now avrcontrol.service`.

To build without packaging, run `cargo build --release`. You also need
`ddcutil` installed and the `i2c-dev` kernel module loaded.

### AppImage

```sh
chmod +x avrcontrol-<version>-x86_64.AppImage
./avrcontrol-<version>-x86_64.AppImage
```

The AppImage doesn't include `ddcutil`, so install it from your distribution
for monitor switching, and load `i2c-dev` (`sudo modprobe i2c-dev`). It also
doesn't install a `.desktop` file, so under Wayland its shortcuts may show
up under an unnamed entry in your desktop's shortcut settings (see below).

### Global hotkeys on Wayland

Under Wayland, applications can't grab keys themselves. AVR Control registers
its shortcuts through the desktop's **Global Shortcuts portal** instead (KDE
Plasma 5.27+, GNOME 48+):

- The first time, Plasma asks you to confirm the shortcuts.
- After that, Plasma owns the key bindings. You can change them in **System
  Settings → Keyboard → Shortcuts → AVR Control**, or with **Open system
  shortcut settings** on the Shortcuts tab.
- The portal only accepts this app's ID (`io.github.virmin8.AVRControl`) when
  the matching `.desktop` file is installed, which the package does. If you run
  a bare `cargo run` build, the shortcuts may show up under an unnamed entry.

If your desktop has no portal, bind `avrcontrol trigger <id>` as a custom
command shortcut in its keyboard settings.

### Monitor access

`ddcutil` needs read/write access to `/dev/i2c-*`. Arch's `ddcutil` package
grants it to the logged-in user. Otherwise, add yourself to the `i2c` group.
DDC/CI also has to be turned on in each monitor's on-screen menu.

## Windows

The installer sets AVR Control up for your user only (no admin prompt) in
`%LOCALAPPDATA%\Programs\AVR Control`, with a Start menu entry and an
uninstaller under Settings → Apps. It can also turn on starting at sign-in.
The portable `.exe` needs no installing; run it from wherever you keep it.

Building needs Rust (via [rustup](https://rustup.rs)) and the Visual Studio
Build Tools with the "Desktop development with C++" workload.

```powershell
cargo build --release
# target\release\avrcontrol.exe

# Installer (needs Inno Setup 6)
iscc /DAppVersion=1.0.0 packaging\windows\avrcontrol.iss
# target\installer\avrcontrol-1.0.0-setup.exe
```

Run `avrcontrol.exe` to start it in the tray. Left-click the tray icon to open
Settings, right-click for the menu.

Hotkeys use `RegisterHotKey`. If another program already uses a hotkey, AVR
Control shows a notification. Start on login adds an entry under
`HKCU\...\CurrentVersion\Run`. Monitors are switched through the Windows
monitor configuration API, so no extra software is needed, but DDC/CI still
has to be turned on in each monitor's on-screen menu.

`avrcontrol.exe` is a GUI program, so cmd and PowerShell don't wait for it
and command-line output can appear after the prompt. Pipe it to see it in
order, e.g. `avrcontrol list | more`.

## Volume indicator

Volume changes show a small overlay in the same front-panel style as the
settings window, on Linux and Windows alike. On Wayland the desktop decides
where it appears.

## Receiver model table

AVR Control knows which commands and values each Onkyo/Integra model supports
from Onkyo's protocol spreadsheet (`ISCP_AVR_*.xlsx`). The table is built into
the binary from `assets/models.json`. Models that aren't in the table are
probed on first connection instead. To regenerate the table from a newer
spreadsheet (needs `tablib[xlsx]` and `pyyaml`):

```sh
python tools/import_protocol_doc.py ISCP_AVR_146.xlsx > eiscp-commands.yaml
python tools/gen_model_table.py eiscp-commands.yaml > assets/models.json
cargo build --release
```

## Licence

MIT, see [LICENSE](LICENSE). Bundled fonts keep their own licences: Inter (SIL
OFL 1.1) and Lucide icons (ISC), in `assets/fonts`.
