# AVR Control

AVR Control switches an Onkyo AV receiver and your monitors' inputs from global
hotkeys. It runs in the system tray on Linux (Wayland and X11) and Windows.

- **Receiver**: controlled over the network with Onkyo's eISCP protocol. It can
  power on and off, select inputs, change volume and mute, toggle HDMI audio
  out, and send any raw eISCP command.
- **Monitors**: inputs are switched over DDC/CI (VCP feature `0x60`). This uses
  `ddcutil` on Linux and the Monitor Configuration API on Windows.
- **Shortcuts**: each shortcut can switch the receiver, any number of monitors,
  or both. You can also run a shortcut from the tray menu or with
  `avrcontrol trigger <id>`.

## Settings window

Open it by clicking the tray icon, choosing **Settings…** from the tray menu,
or running `avrcontrol` while the app is already running.

| Tab | What it shows |
| --- | --- |
| Receiver | IP address (**Find on network** searches for it), connection test, model, power, current input and volume. Also lets you rename or hide receiver inputs. |
| Monitors | Every connected display with its graphics-card connector (e.g. `DP-1`), its current input, the inputs it supports, and buttons to switch inputs immediately. |
| Shortcuts | Add, edit and reorder shortcuts. Set the hotkey (type it or press **Record**), then pick a receiver input and a target input for each monitor. |
| General | Startup action, notifications, volume indicator, start on login, and start/stop of the background service. |

The configuration is stored in `~/.config/avrcontrol/config.toml` (Linux) or
`%APPDATA%\avrcontrol\config\config.toml` (Windows).

## Command line

```
avrcontrol                 start in the background (opens settings if already running)
avrcontrol daemon          start the background service
avrcontrol settings [TAB]  open settings (receiver|monitors|shortcuts|general)
avrcontrol trigger <ID>    run a shortcut in the running instance
avrcontrol list            list shortcuts and their IDs
avrcontrol monitors        list monitors, connectors and inputs
avrcontrol receiver        show receiver status
avrcontrol discover        search the network for receivers
avrcontrol reload | quit   control the running instance
```

## Linux

### Install (Arch / pacman)

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

```powershell
cargo build --release
# target\release\avrcontrol.exe
```

Hotkeys use `RegisterHotKey`. If another program already uses a hotkey, AVR
Control shows a notification. Start on login adds an entry under
`HKCU\...\CurrentVersion\Run`.

## Volume indicator

On KDE Plasma, volume changes use Plasma's own on-screen display. Elsewhere,
AVR Control shows a small overlay window of its own.

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
