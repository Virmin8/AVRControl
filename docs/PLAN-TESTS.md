# AVRControl test coverage plan

Status (2026-10-21): phases 1 and 2 done (46 tests; fake receiver in
`src/eiscp/fake.rs`, old-config fixture in `tests/fixtures/`). Phase 3 and
later not started.

Today: 17 unit tests across `eiscp` (4), `update` (5), `monitors` (4),
`hotkey` (2), `models` (1), `audio` (1). CI runs `cargo test --release` on
Linux and Windows. Untested: `controller`, `config`, `daemon`/`ipc`,
`repeat`, `shutdown`, most of `eiscp::Receiver`, and the settings GUI.

Goal: catch regressions in the parts that talk to the receiver and that
interpret the user's config, without real hardware and without any test
touching the desktop (notifications, monitors, hotkeys).

## Phase 1: pure logic (no refactoring)

Plain `#[cfg(test)]` modules next to the code.

- **eiscp parsing**
  - `packet` / `take_packet`: byte-exact ISCP header, multiple packets in one
    read, packet split across reads, garbage before the magic, bad length.
  - `AudioInfo::parse`, `VideoInfo::parse`: replies captured from a real
    receiver (keep them as string fixtures), empty/`N/A` fields.
  - `listening_mode_name`, `hdmi_output_name`, `default_input_name`: known
    code, unknown code falls back sensibly.
  - `ReceiverStatus::summary` against a fixed `Config`.
- **controller helpers**: `parse_volume` (`MVL3A`, `MVLN/A`, short reply).
- **config**
  - Defaults round-trip: `Config::default()` → TOML → `Config` is equal.
  - Old config files still load: check in a few `tests/fixtures/config-*.toml`
    from earlier releases (1.0, 1.1, 1.2) and assert they parse with the
    fields we expect. Add one per release that changes the format.
  - `volume_raw` / `volume_text` / `volume_max` across volume-step settings.
  - `effective_features`, `supports`, `supports_value`, `supports_kind` with a
    model from the table and with an unknown (probed) model.
  - `startup_input`, `input_label` with renamed and hidden inputs.
  - `Action::from_kind(a.kind()) == a` for every kind; `repeatable` matches
    the README's list (volume, level, tuner, menu arrows).
- **models**: every table entry has a non-empty input list; `lookup` is
  case/punctuation tolerant the way discovery reports names; a few spot
  checks beyond DTR-50.7 (one per receiver family).
- **hotkey**: round-trip every key name `format` can produce.

Size: ~40 tests, a day or so. No production code changes.

## Phase 2: fake receiver

A test-only TCP server in `src/eiscp/fake.rs` (`#[cfg(test)]`) or
`tests/support/`:

- Listens on `127.0.0.1:0`, speaks ISCP framing (reusing `packet`).
- Holds receiver state (power, input, volume, mute, listening mode, model
  XML for `NRI`), answers `QSTN` queries from it and applies set commands.
- Scriptable faults: drop the connection, delay a reply, never reply, reply
  `N/A`, send unsolicited status updates (the receiver does this when the
  front panel is used).
- Records every command it received so tests can assert on the sequence.

Tests against `Receiver::new("127.0.0.1", port)`:

- `send` / `query` happy path; `N/A` handling; timeout on missing reply.
- Unsolicited messages interleaved with a reply don't get mistaken for it.
- Reconnect after the server drops the connection.
- `probe` builds the right `Capabilities` from the fake's NRI XML.
- `status` / `status_with_model` fill every field.
- `set_features`: unsupported commands are refused before being sent.

Size: the fake is ~200 lines; ~15 tests.

## Phase 3: controller behaviour

`Controller` calls `Feedback` (desktop notifications, OSD) and the monitor
switching code directly, so a test would pop notifications and switch real
monitors over DDC. Add a seam first:

- Put the side effects behind a small trait (e.g. `Effects` with `info`,
  `error`, `volume`, `switch_monitor`), with the real implementation wrapping
  `Feedback` + `monitors`, and a recording one for tests. Pass it into
  `Controller::new`. Keep the change mechanical; no behaviour change.

Then, with the fake receiver and recording effects, drive `Controller::run`
on a thread through its `Event` channel:

- A switch shortcut sends the expected commands in order (power on, input,
  HDMI out, listening mode, volume) and switches each monitor once.
- Startup shortcut: monitors already on the target input are left alone.
- Receiver offline: one error notification, not one per keepalive; back
  online: one info notification.
- `Query` events are answered through the controller's single connection
  (the fake rejects a second client, as some models do).
- `Reload` picks up a new host/port and reconnects.
- Keepalive fires after 30 s idle (make the interval configurable for the
  test rather than sleeping).

Size: refactor ~half a day, ~10 tests.

## Phase 4: daemon / IPC (optional)

- `daemon::handle_request` with a channel and no socket: each request kind
  produces the right `Event` and reply.
- `ipc::serve` + `request` over a socket in a temp dir (needs `socket_path`
  to be overridable, e.g. an env var used only by tests). Linux only in CI
  unless the Windows named pipe is also easy to point elsewhere.

## Out of scope

- The settings GUI (egui). Covered by using it; not worth a harness.
- `shutdown`, `autostart`, `platform/*`: thin OS glue, tested by hand.
- Real DDC/CI and the XDG portal.

## CI

No workflow change needed: everything runs under `cargo test`. Keep the fake
on loopback only, with short timeouts (≤200 ms) so the suite stays fast.
