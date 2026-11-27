# OpenSights

An ESP32 (NodeMCU-32S) that talks Bluetooth LE to a desktop app written in Rust.

- **firmware/** — PlatformIO project for the ESP32
- **app/** — Rust/egui desktop app (Linux, macOS, Windows)
- **protocol/PROTOCOL.md** — the BLE protocol both sides implement
- **hardware/** — KiCad projects for the sensor hardware

Open `esp-sensor.code-workspace` in VS Code: PlatformIO picks up `firmware/` and
rust-analyzer picks up `app/`.

## What it does

- The sensor is the prototype of an electronic target: an acoustic target that locates
  each shot from the sound arriving at its microphones (being built, see
  [PLAN.md](PLAN.md) and [firmware/MEASUREMENT.md](firmware/MEASUREMENT.md)). Until the
  measuring hardware is running, a short press on BOOT makes a random test shot.
- The app shows the shots on a 10 m air pistol card and scores them: whole rings with X for an inner ten, or decimals (a setting). The series
  list and total sit next to the card.
- No shot is lost when the link drops or the sensor restarts: it keeps its shots in flash
  (about 2,000 of them) and resends the ones the app missed.
- A shot the sensor heard but couldn't measure (a second shot too soon after another), or
  one only the light curtain saw (through an existing hole), is listed in the series as
  "not scored", with a notice on the card: check the paper card for those. Click one (in
  the list or on the notice) to resolve it: place it on the card (click the hole, drag the
  shot until it fits, or type x and y), mark it the same hole as another shot, or count it a
  miss (0). It then takes its place in the series, marked "by hand", and can be changed or
  put back to "not scored" by selecting it.
- The app can save every event the sensor captured (also the rejected ones) to a log file,
  one per day, in the same format as the sensor's serial log (Settings → Measurement; on by
  default). Settings also show the sensor's calibration, settings and air readings.
- Settings → Measurement → **A shot needs the IR curtain's confirmation** switches the light
  curtain's check on the sensor on or off. Off, the sound alone decides what counts as a
  shot (the curtain's lights are off too, and no shots through an existing hole are
  reported). On again, it uses the window last set (45–100 µs if none was).
- Calibrate from the app: Settings → Measurement → **Calibrate…** walks through the spark
  points of the calibration template (16 points, 10 sparks each), fits the microphones'
  positions and timing, shows the result, and writes it to the sensor. **Field check…** makes
  a few sparks at the template's centre and tells you whether the calibration still holds.
- **Card photo** (under the series): a photo of the shot card, taken with a phone, shows
  under the shots, and the app compares each shot with its hole (the sensor's accuracy).
  The series list then shows each shot's score on the paper beside the sensor's.
  Scan the QR code with the phone (on the same Wi-Fi), take the photo with the card flat and
  the phone straight above it. The photo's metadata (location and the like) is removed;
  photos are kept 30 days unless marked to keep. Work in progress, see
  [app/CARDPHOTO.md](app/CARDPHOTO.md).
- **Count card** (in the status bar, with or without a sensor): scores paper cards from
  photos, apart from the series. Scan the code once, then photograph card after card ("Upload
  another card", up to 20); the app counts them in turn and shows each card's holes and score,
  and the total, on the phone too.
- Settings also show the heartbeat (the sensor's `millis()`, every 2 s). The blue LED
  only shows the sensor's status: pairing mode, a PIN reset, a shot.
- The sensor has a name (`OpenSights-xyz` at first) that you can change from the app.
- The app remembers the sensor and reconnects to it automatically.
- Pairing needs the sensor's 6-digit PIN (`000000` at first), which you can change from
  the app. Forgot it? Hold BOOT for 10 s: the PIN is reset and all computers are unpaired.

## Hardware

[hardware/acoustic-sensor/](hardware/acoustic-sensor/) is the first measuring board: one
acoustic channel with a MEMS microphone, a preamp, and two comparator thresholds,
connected to the host board through a 5-pin JST XH connector. Its
[README](hardware/acoustic-sensor/README.md) describes the board, the files and the checks
to do before ordering. The [schematic PDF](hardware/acoustic-sensor/mic_board_schematic.pdf)
can be read without KiCad. Open the project with KiCad 10 or newer.

[hardware/host-board/](hardware/host-board/) is the board the sensors plug into: the
NodeMCU-32S on sockets, 8 sensor connectors, the IR curtain, a DS18B20 and the panel LED
and button, all through-hole. Its [README](hardware/host-board/README.md) has the layout,
the GPIO map and assembly notes; the [schematic](hardware/host-board/host_board_schematic.pdf)
and [assembly drawing](hardware/host-board/host_board_assembly.pdf) are PDFs.

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

- **Linux** (x86_64): unpack, then `./opensights-app`.
- **Windows 11**: unzip and start `opensights-app.exe`. SmartScreen says "Windows
  protected your PC": click **More info → Run anyway**.
- **macOS** (Apple Silicon and Intel): unzip and move **OpenSights.app** to Applications.
  The first start is blocked. Open **System Settings → Privacy & Security** and click
  **Open Anyway**, or run `xattr -dr com.apple.quarantine "/Applications/OpenSights.app"`.
  macOS then asks for Bluetooth access.
- **Firmware** (`pip install esptool`):
  - New board: `esptool.py --chip esp32 write_flash 0x0 opensights-firmware-…-full.bin`.
    This also erases the stored name, PIN and pairings.
  - Update, keeping name, PIN and pairings:
    `esptool.py --chip esp32 write_flash 0x10000 opensights-firmware-…-app.bin`.
  - The app and the firmware must speak the same protocol version (now 3): update both.
    After a firmware update, a Linux computer may keep the sensor's old Bluetooth layout:
    the app then asks you to pair again (hold BOOT for 2 s, press Pair).

To make a release: `git tag v0.1.0 && git push origin v0.1.0`.

## Notes per platform

- **Linux:** the app runs its own pairing agent and asks for the PIN in its own window,
  so no desktop Bluetooth settings are needed.
- **macOS:** a Bluetooth app must be a `.app` bundle with a Bluetooth usage description,
  or macOS refuses access. Running from a terminal uses the terminal's permission.
- **Card photos:** the phone reaches the app on port 8710 of this computer. A firewall may
  have to let it in (Windows asks the first time).
- **Windows:** if no PIN prompt appears, pair the sensor in Settings → Bluetooth while it
  is in pairing mode, then press Connect in the app.

## Forgetting a sensor

"Forget sensor" in the app stops automatic reconnects. The bond itself stays in your
operating system's Bluetooth settings; remove it there to pair from scratch.
