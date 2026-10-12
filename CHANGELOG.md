# Changelog

All notable changes to AVR Control. Versions follow
[semantic versioning](https://semver.org); each heading links to its release.

## [1.2.1] - 2026-10-12

### Changed
- At startup, monitors that are already on the startup shortcut's input are
  left alone instead of being switched to it again.

## [1.2.0] - 2026-10-12

### Changed
- New look for the settings window, styled after the receiver's front panel:
  near-black surfaces, an amber accent and an icon rail for navigation.
- The receiver's model, address and connection state ("On · PC", "Standby",
  "Unreachable") now show at the top of every page, together with Save and
  Revert.
- The Status page opens with an amber front-panel display showing the input,
  volume, listening mode and HDMI output, with indicators for HDMI, Atmos,
  DTS:X, 4K, HDR, mute, zone 2 and the sleep timer. Below it are input keys,
  a volume slider, the video and audio signal, zones, and one-click input
  switching for each monitor.
- The volume slider updates as soon as you click, drag or scroll, sends the
  volume while you drag, and keeps your level until the receiver confirms
  it.
- The Monitors tab now shows everything each monitor reports (connector,
  signal, resolution, HDR, brightness and more). Input switching moved to
  the Status page.
- The Receiver tab is split into **Controls** and **Setup**. Controls groups
  every supported function into cards (power and volume, sound, HDMI,
  system, zones, raw command) with selector switches. Setup holds the volume
  steps, supported features and input names, which used to open in a
  separate window.
- The volume overlay has the same front-panel style, and is now used on
  Linux too instead of Plasma's own volume display.

### Fixed
- The Receiver settings window was slow to respond. It's now part of the
  Receiver tab.
- On Windows, the volume overlay could miss updates when a new value arrived
  just as it was closing.
- Holding a volume key repeats at the same rate on Linux as on Windows.
  Desktops that resend key presses while a key is held no longer queue up
  extra steps.

## [1.1.2] - 2026-10-12

### Fixed
- Turning the receiver off at shutdown now works on Linux. KDE Plasma stopped
  the background service while logging out, before the shutdown signal
  arrived; the shutdown watcher now runs as its own systemd user service so
  it outlives the desktop session.

## [1.1.1] - 2026-10-12

### Added
- Optional setting under General to turn the receiver off when the PC shuts
  down, but only if it is still on the startup input. It stays on when
  another input is in use (a console, a TV box), on restart and on log-off.

## [1.1.0] - 2026-10-12

### Added
- Update checks at launch and every two days, with a notification and a tray
  entry when a new release is out. Settings → General can turn this off or
  check now.
- In-app updates for the Windows installer and the AppImage: the download is
  checked against its published SHA-256 before it is installed, then the app
  restarts.
- A pacman repository on the releases page, so Arch users get updates with
  `pacman -Syu`.
- Releases include `.sha256` files for the AppImage and Windows downloads.

## [1.0.0] - 2026-10-11

First release.

### Added
- Global hotkeys that switch the receiver's input and any number of monitors
  in one press, plus actions for power, volume, mute, listening mode, HDMI
  output and every other setting the receiver supports. Holding a volume key
  repeats it.
- Receiver control over the network (eISCP): network discovery, the
  receiver's own input names, and per-model command support from Onkyo's
  protocol spreadsheet (252 models), with probing for other models.
- Monitor input switching over DDC/CI, with the current input, the inputs each
  monitor offers and, on KDE Plasma, full display details.
- A settings window with a live Status page, receiver controls, monitors,
  shortcuts and general settings; a tray icon and tooltip; desktop
  notifications; and a command line (`avrcontrol trigger <id>` and more).
- Linux (Wayland through the Global Shortcuts portal, and X11) and Windows.
- Windows installer and portable exe, AppImage, and Arch package, built and
  published for every release.

### Fixed
- Windows: network discovery, hotkey aliases such as `Win+` and `PgUp`,
  left-click on the tray icon opening Settings, and monitors that ignore
  DDC/CI for a few seconds while switching inputs.

[1.2.1]: https://github.com/Virmin8/AVRControl/releases/tag/v1.2.1
[1.2.0]: https://github.com/Virmin8/AVRControl/releases/tag/v1.2.0
[1.1.2]: https://github.com/Virmin8/AVRControl/releases/tag/v1.1.2
[1.1.1]: https://github.com/Virmin8/AVRControl/releases/tag/v1.1.1
[1.1.0]: https://github.com/Virmin8/AVRControl/releases/tag/v1.1.0
[1.0.0]: https://github.com/Virmin8/AVRControl/releases/tag/v1.0.0
