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

- `main.cpp`: button (short press = test shot, 2 s hold = pairing mode, 10 s hold = PIN
  reset), heartbeat, name and PIN in NVS (namespace `"sensor"`), LED output, and the shot
  buffer: the last 64 shots, indicated one at a time until the app confirms each
  (`serviceShots`). Simple 10 ms loop.
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
  (boot, seq). The app keeps the series in memory.
- `main.rs`: egui UI. While connected, the main window shows the target card with the
  series' shots (the newest in the UI's only color; clicking one opens a small info popup; zoom with the wheel or the zoom bar (bottom right), drag to pan; the replay bar (bottom left) shows the series again shot by shot; both bars can be hidden in settings; zoomed in, a sharp render of just the visible part replaces the stretched card once the view stops moving) and the series list; everything else
  (dark mode, zoom/replay bars, decimal scoring (off by default: whole rings, X = inner ten), sensor list and pairing, name, PIN, heartbeat, LED, disconnect/forget) is in a settings window,
  an immediate viewport opened from the gear in the status bar. The PIN popup and messages
  show in the settings window while it's open. The remembered sensor ID and theme are
  saved with eframe's storage.
  Grayscale styling, IBM Plex Sans embedded from `assets/fonts/`.
