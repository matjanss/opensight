# ESP Sensor

An ESP32 (NodeMCU-32S) that talks Bluetooth LE to a desktop app written in Rust.

- **firmware/** — PlatformIO project for the ESP32
- **app/** — Rust/egui desktop app (Linux, macOS, Windows)
- **protocol/PROTOCOL.md** — the BLE protocol both sides implement

Open `esp-sensor.code-workspace` in VS Code: PlatformIO picks up `firmware/` and
rust-analyzer picks up `app/`.

## What it does

- The sensor sends a heartbeat (its `millis()`) every 2 seconds; the app shows it.
- The BOOT button toggles the blue LED; the app can toggle it too, and both stay in sync.
- The sensor has a name (`ESPSensor-xyz` at first) that you can change from the app.
- The app remembers the sensor and reconnects to it automatically.

## Getting started

1. Flash the firmware: `pio run -d firmware -t upload`
2. Start the app: `cd app && cargo run`
3. Pair: hold **BOOT** for 2 s until the LED blinks fast, press **Pair** in the app, and
   enter **1234** when your computer asks for a PIN.

After pairing once, the app connects by itself whenever the sensor is in range.

## Notes per platform

- **Linux:** pairing uses your desktop's Bluetooth agent (GNOME and KDE show the PIN
  prompt). Without a desktop agent, pair first with `bluetoothctl` (`agent on`,
  `pair <address>`).
- **macOS:** a Bluetooth app must be a `.app` bundle with a Bluetooth usage description,
  or macOS refuses access. Running from a terminal uses the terminal's permission.
- **Windows:** if no PIN prompt appears, pair the sensor in Settings → Bluetooth while it
  is in pairing mode, then press Connect in the app.

## Forgetting a sensor

"Forget sensor" in the app stops automatic reconnects. The bond itself stays in your
operating system's Bluetooth settings; remove it there to pair from scratch.
