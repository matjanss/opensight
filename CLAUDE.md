# CLAUDE.md

ESP Sensor: an ESP32 BLE peripheral (`firmware/`) and a Rust desktop app (`app/`) that
talk over the protocol in `protocol/PROTOCOL.md`. A test bed for the firmware + app pattern.

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

- `main.cpp`: button (short press = LED, 2 s hold = pairing mode), heartbeat, name in NVS
  (namespace `"sensor"`), LED output. Simple 10 ms loop.
- `ble_link.cpp`: NimBLE GATT server. NimBLE callbacks run in NimBLE's task: they only store
  requests, which `loop()` picks up with `bleTake*()`. Don't touch pins, NVS or app state
  from a callback.
- Security: every characteristic needs an encrypted, MITM-authenticated link (passkey 1234,
  bonding). Unbonded centrals are disconnected unless pairing mode is on.
- `BOOT_PIN` is defined by the Arduino core; ours is `BOOT_BTN_PIN`.

## App

- `ble.rs`: a background thread with a tokio runtime owns btleplug. The UI sends `Command`s
  and receives `Event`s over channels; each event requests a repaint. Keep btleplug calls
  out of the UI thread.
- `main.rs`: egui UI. The remembered sensor ID and theme are saved with eframe's storage.
  Grayscale styling, IBM Plex Sans embedded from `assets/fonts/`.
