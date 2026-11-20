# CLAUDE.md

OpenSights: an ESP32 BLE peripheral (`firmware/`) and a Rust desktop app (`app/`) that
talk over the protocol in `protocol/PROTOCOL.md`. The prototype of an electronic target
system; where it is heading is described in `FUTURE_ARCHITECTURE.md`.

## Build

- Firmware (NodeMCU-32S): `pio run -d firmware`. Add `-t upload` to flash over USB, and
  `pio device monitor -d firmware` for the serial log (115200).
  If `pio` is not on PATH, use `~/.platformio/penv/bin/pio`.
- App: `cargo run` / `cargo build --release` in `app/`. `cargo test` checks the protocol
  encoding against the examples in PROTOCOL.md.
- Linux: the app builds libdbus from source (`vendored`), so no `libdbus-1-dev` is needed.
- The radio link can't be tested here; the user tests with the hardware.

## The protocol rule

`protocol/PROTOCOL.md` is the source of truth. It is mirrored by `firmware/src/protocol.h`
and `app/src/protocol.rs`. **Any protocol change touches all three in the same commit**,
updates the examples and tests in `protocol.rs`, and bumps `PROTOCOL_VERSION` when an old
app could misread new firmware or the reverse.

## Firmware

- Measuring shots: `firmware/MEASUREMENT.md` is the plan (steps, each ending in a bench
  test) and the record of bench results. Read it before working on measurement; update its
  step status and results in the same commit as the code.
- `capture.cpp`: I2S0 in camera mode samples the 16 comparator lines into a DMA ring
  (register level: IDF 5 has no driver for it); an IRAM interrupt scans each finished
  block and freezes the ring around an event (spare DMA blocks), a task on core 1 extracts
  the edges, and `loop()` prints them. The window length follows from the mic span. Serial
  commands (`help`) set the channel mask, dead time and mic span and run the self-test on
  CH8's pins.
- `main.cpp`: button (short press = test shot, 2 s hold = pairing mode, 10 s hold = PIN
  reset), heartbeat, name and PIN in NVS (namespace `"sensor"`), the status LED (pairing blink, PIN reset, shot flash; the app can't switch it), and the shot
  buffer: the last 64 shots, indicated one at a time until the app confirms each
  (`serviceShots`); a clear request from the app drops the shots it has (`clearedSeq`). Simple 10 ms loop.
- `ble_link.cpp`: NimBLE GATT server. NimBLE callbacks run in NimBLE's task: they only store
  requests, which `loop()` picks up with `bleTake*()`. Don't touch pins, NVS or app state
  from a callback.
- Security: every characteristic needs an encrypted, MITM-authenticated link (passkey =
  the PIN, 6 digits, default 000000, NVS key `"pin"`; bonding). Unbonded centrals are
  disconnected unless pairing mode is on. A pairing without the passkey (no MITM) is
  rejected and its bond deleted at once: NimBLE would otherwise delete it later anyway,
  when the central re-pairs on the same link. Writing a new PIN
  deletes all bonds but the writer's; BOOT held 10 s resets the PIN and deletes all bonds.
- `BOOT_PIN` is defined by the Arduino core; ours is `BOOT_BTN_PIN`.
- NimBLE only calls `onPassKeyDisplay()` while its static passkey is the default 123456,
  so `bleSetPasskey` must not call `NimBLEDevice::setSecurityPasskey`.

## App

- `ble.rs`: a background thread with a tokio runtime owns btleplug. The UI sends `Command`s
  and receives `Event`s over channels; each event requests a repaint. Keep btleplug calls
  out of the UI thread.
- `agent.rs` (Linux): a BlueZ pairing agent on its own D-Bus connection. The app calls
  `Device1.Pair` itself so BlueZ asks this agent. For the sensor being paired only, the
  agent hands the request to the UI (a `PasskeyReply` in `Event::PasskeyRequest`), which
  shows a PIN popup and answers it directly: the BLE loop is busy awaiting `Pair` meanwhile.
  Desktops run no agent unless their Bluetooth settings are open.
  `Pair` must come **before** connecting: on a link that is already up, BlueZ pairs with the
  system-wide "no input" capability, so without the passkey (no MITM). Connecting to a
  sensor in pairing mode deletes BlueZ's old keys first (stale bonds). The sensor list
  shows BlueZ's real pairing state; unpaired sensors can only be paired in pairing mode.
- `card.rs`: target cards. Each card is an SVG (the picture) plus JSON (scoring geometry);
  scoring never looks at the SVG. The cards are rendered with resvg, using the embedded IBM
  Plex font rather than system fonts. The built-in card is in `assets/cards/`.
  1 SVG unit = 1 mm.
- Shots: `ble.rs` asks for missed shots on every connect and drops duplicates by
  (boot, seq). The app keeps the series in memory. "Clear series" also sends a clear
  request, so the sensor forgets those shots and doesn't resend them after an app restart.
- `main.rs`: egui UI. While connected, the main window shows the target card with the
  series' shots (the newest in the UI's only color; clicking one opens a small info popup; zoom with the wheel or the zoom bar (bottom right), drag to pan; the replay bar (bottom left) shows the series again shot by shot; both bars can be hidden in settings; a close-call notice at the top left lists every shot of the series within the close-call margin (0.5 mm by default) of a higher score (`Card::higher`, same rule as scoring) for manual review; zoomed in, a sharp render of just the visible part replaces the stretched card once the view stops moving) and the series list with a shots-per-score chart below it (bars per ring, or points per tenth when decimal; painted by hand in `score_chart`); everything else
  (dark mode, decimal scoring (off by default: whole rings, X = inner ten), the target card's zoom/replay bars, default replay speed, close-call notice and margin, sensor list and pairing, and one "Sensor settings" panel with name, PIN, heartbeat and disconnect/forget) is in a settings window,
  an immediate viewport opened from the gear in the status bar. The PIN popup and messages
  show in the settings window while it's open. The remembered sensor ID and the preferences are
  saved with eframe's storage.
  Grayscale styling, IBM Plex Sans embedded from `assets/fonts/`; emoji come from egui's
  monochrome emoji fonts (`monochrome_emoji_fonts`), not the system's color emoji.
