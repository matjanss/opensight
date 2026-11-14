# ESP Sensor

An ESP32 (NodeMCU-32S) that talks Bluetooth LE to a desktop app written in Rust.

- **firmware/** — PlatformIO project for the ESP32
- **app/** — Rust/egui desktop app (Linux, macOS, Windows)
- **protocol/PROTOCOL.md** — the BLE protocol both sides implement

Open `esp-sensor.code-workspace` in VS Code: PlatformIO picks up `firmware/` and
rust-analyzer picks up `app/`.

## What it does

- The sensor is the prototype of an electronic target. Until it has measuring hardware,
  a short press on BOOT makes a test shot.
- The app shows the shots on a 10 m air pistol card and scores them: whole rings with X for an inner ten, or decimals (a setting). The series
  list and total sit next to the card.
- No shot is lost when the link drops: the sensor keeps its last 64 shots and resends the
  ones the app missed.
- Settings also show the heartbeat (the sensor's `millis()`, every 2 s) and switch the
  blue LED.
- The sensor has a name (`ESPSensor-xyz` at first) that you can change from the app.
- The app remembers the sensor and reconnects to it automatically.
- Pairing needs the sensor's 6-digit PIN (`000000` at first), which you can change from
  the app. Forgot it? Hold BOOT for 10 s: the PIN is reset and all computers are unpaired.

## Getting started

From source:

1. Flash the firmware: `pio run -d firmware -t upload`
2. Start the app: `cd app && cargo run`
3. Pair: hold **BOOT** for 2 s until the LED blinks fast, press **Pair** in the app, and
   enter the PIN (**000000**) when asked.

After pairing once, the app connects by itself whenever the sensor is in range.

## Downloads

Every version tag (`v0.1.0`, …) builds a GitHub Release with the app for Linux, Windows
and macOS and the firmware. The binaries aren't signed, so each system warns once:

- **Linux** (x86_64): unpack, then `./esp-sensor-app`.
- **Windows 11**: unzip and start `esp-sensor-app.exe`. SmartScreen says "Windows
  protected your PC": click **More info → Run anyway**.
- **macOS** (Apple Silicon and Intel): unzip and move **ESP Sensor.app** to Applications.
  The first start is blocked. Open **System Settings → Privacy & Security** and click
  **Open Anyway**, or run `xattr -dr com.apple.quarantine "/Applications/ESP Sensor.app"`.
  macOS then asks for Bluetooth access.
- **Firmware** (`pip install esptool`):
  - New board: `esptool.py --chip esp32 write_flash 0x0 esp-sensor-firmware-…-full.bin`.
    This also erases the stored name, PIN and pairings.
  - Update, keeping name, PIN and pairings:
    `esptool.py --chip esp32 write_flash 0x10000 esp-sensor-firmware-…-app.bin`.

To make a release: `git tag v0.1.0 && git push origin v0.1.0`.

## Notes per platform

- **Linux:** the app runs its own pairing agent and asks for the PIN in its own window,
  so no desktop Bluetooth settings are needed.
- **macOS:** a Bluetooth app must be a `.app` bundle with a Bluetooth usage description,
  or macOS refuses access. Running from a terminal uses the terminal's permission.
- **Windows:** if no PIN prompt appears, pair the sensor in Settings → Bluetooth while it
  is in pairing mode, then press Connect in the app.

## Forgetting a sensor

"Forget sensor" in the app stops automatic reconnects. The bond itself stays in your
operating system's Bluetooth settings; remove it there to pair from scratch.
