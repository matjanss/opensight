# CLAUDE.md

OpenSights is the prototype of an electronic shooting target. An acoustic target (ESP32
plus MEMS microphones in the frame corners) locates each shot by the time differences of
arrival of the pellet's crack, and reports x/y in mm. A Rust app shows the shots on a
target card and scores them. The current link is BLE; the plan is Wi-Fi/MQTT to a range
server, with viewers on Pis and phones.

Proof-of-concept scope:

- an indoor 25 m range, mainly air guns and subsonic ammunition, with other lanes firing;
- a 250 × 250 mm frame around a 200 × 200 mm scoring area;
- the goal is 0.2–0.5 mm (1σ) after calibration;
- an IR light curtain behind the paper confirms each shot;
- supersonic ammunition is out of scope.

Current focus: get the measuring hardware running and find out how accurate it really is.
The web/wasm viewer is parked.

## Read first: the documents

| Document | What it holds | Read it when |
|---|---|---|
| `PLAN.md` | Decisions D1–D12 (mic layout, model, who does what, calibration, firmware structure, messages, the air, curtain, frame limits, ground truth, the solver), open questions Q1–Q3, phases 0–5 with their tests | Always, before choosing what to work on |
| `firmware/MEASUREMENT.md` | The measurement firmware's steps 1–6, each with its build list and bench test, the step status table, and every bench result (newest first) | Before any measurement work; update it in the same commit as the code |
| `firmware/MODEL.md` | The measurement model: units, mics, arrival time, speed of sound, propagation, the solver and its outputs, drop-one, the log lines (`cfg`, `rec`), the test vector format | Before touching `lib/measure/` or `tools/model.py` (see the model rule) |
| `FUTURE_ARCHITECTURE.md` | The target system: target, range server, viewers, management, network per lane, updates, its steps 1–4 | Before anything network, server, viewer or fleet |
| `protocol/PROTOCOL.md` | The BLE protocol, v2: advertising, GATT, shots, security, buttons and LED | Before touching the link (see the protocol rule) |
| `hardware/acoustic-sensor/README.md` | Sensor board v0.3: one mic channel, files, checks before ordering | Hardware questions |
| `hardware/host-board/README.md` | Host board v0.3: layout, GPIO map, assembly | Hardware questions, pin changes |
| Project summary (claude.ai doc, below) | The hardware design: approaches considered, signal chain and BOM, threshold levels, initial calibration per board, error budget, error sources, valid-shot detection, IR curtain design, host board requirements, its open questions | Hardware or measurement design questions |
| `README.md` | User-facing: what it does, getting started, downloads, notes per platform | Changing user-visible behaviour |
| `app/TODO.md` | The app's to-do list: wishes and known gaps not yet in a release | App work; add new app wishes there |
| `app/assets/icon/README.md` | The app icon and where it is used | Icon work |

**The project summary** is a Claude Docs document: "Electronic shooting target – project
summary", https://claude.ai/code/artifact/a8aff581-45d0-41fc-8f4c-dd17fb9b4ac1 (doc ID
`a8aff581-45d0-41fc-8f4c-dd17fb9b4ac1`).

- **Reading it:** use the Claude Docs connector. `read` the doc (ref object `project`,
  that ID) for its tab, then `read` the tab's content node. The tab is about 130 KB of
  XML, so it comes back as a file: strip the tags to text before reading.
- **Editing it:** it's the user's doc. Change it only when asked, as suggestions.
- **Where it is out of date** (PLAN.md has the corrections):
  - its error budget counts the mic height as 1 mm, but it is 10.5 mm (PLAN D3);
  - its open question on breaking the corner symmetry is answered by PLAN D1/D2.

## Status (2026-11-22) and what's next, per part

| Part | Where it stands | Next (see) |
|---|---|---|
| Firmware: BLE, pairing, shot buffer | Done; protocol v2 | Flash shot store, uplink interface (PLAN D6, phase 3–4) |
| Firmware: measurement | Step 1 passed on a bare NodeMCU (on the v0.3 pin map, tests 3 and 14 are due again). Step 2's simulated tests pass; its real-board tests wait for the boards. Step 3 (solver and calibration, PLAN phase 0) passes all its simulated tests on a bare NodeMCU (2026-11-22, merged to `main`): the solver on the board, drop-one, the blast check, calibration mode and `tools/calibrate.py`. The DS18B20 and BME280 tests (3a-3, 3a-4) wait for the parts | Step 4 (validity) or the boards' arrival (PLAN phase 1: the same tests on real sensors); 3a-3/3a-4 when the parts are there |
| Sensor board (`hardware/acoustic-sensor/`) | v0.3 designed (KiCad 10), not yet on the bench | Its "check before ordering" list; then the summary's initial calibration, polarity, PLAN phase 1 |
| Host board (`hardware/host-board/`) | v0.3 designed and routed, not yet on the bench; firmware already on its pin map | Order and assemble; MEASUREMENT step 1 tests 3 and 14 on the real board |
| Frame and mechanics | 250 × 250 mm, PETG mic holders on an aluminium backbone (summary); enclosure open | 6 mics (PLAN D2): the top-right corner moved out along the diagonal (d about 15–25 mm, PLAN D1's update; principle in `hardware/frame/top-right-holder.svg`), plus bottom-middle and right-middle mics; the user designs and prints the holders. The firmware defaults still use the old slid position (65, 105) until d is fixed |
| IR curtain | Designed (receiver on the host board, emitter board), firmware not started | Its bench test first (summary), then step 4 |
| App (`app/`) | v0.1.2, desktop (Linux, Windows, macOS), BLE, 10 m air pistol card, scoring, series, replay | Raw events and the calibration wizard with a Rust fit (PLAN D5, phase 3) |
| Range server, MQTT, management, wasm viewer | Not started (wasm parked) | FUTURE_ARCHITECTURE step 2, PLAN phase 4 |

## Repository

```
CLAUDE.md, PLAN.md, FUTURE_ARCHITECTURE.md, README.md
protocol/PROTOCOL.md          the BLE protocol (source of truth)
firmware/                     PlatformIO, Arduino-ESP32 3.x on pioarduino (IDF 5), NimBLE-Arduino 2.3
  MEASUREMENT.md              measurement plan and bench results
  MODEL.md                    the measurement model (source of truth)
  src/                        the firmware (modules below)
  lib/measure/                the portable measurement core: air.h, model.h, solver,
                              record (the log lines), blast (the blast check)
  lib/bme280/                 BME280/BMP280 compensation (portable)
  test/test_measure/          Unity tests of the core on the PC (`pio test -e native`)
  test/vectors/               shared test vectors, written by tools/model.py
  tools/model.py              the model's reference implementation (numpy)
  tools/replay.cpp            replays a serial log's events through lib/measure (g++)
  tools/calibrate.py          the calibration fit (numpy, scipy)
  tools/layout_study.py       mic layout study behind PLAN D1/D2 (numpy)
app/                          Rust 2024, eframe/egui 0.37, btleplug, resvg
  src/                        main.rs (UI), ble.rs, agent.rs, card.rs, protocol.rs
  TODO.md                     the app's to-do list
  assets/                     cards/, fonts/ (IBM Plex Sans), icon/
hardware/acoustic-sensor/     sensor board, KiCad 10
hardware/host-board/          host board, KiCad 10
hardware/frame/               frame drawings: top-right-holder.svg (the moved corner mic's holder, principle)
.github/workflows/build.yml   CI; a v* tag builds a GitHub Release (app for 3 OSes + firmware)
```

## Build and test

- **Firmware** (NodeMCU-32S): `pio run -d firmware`. Add `-t upload` to flash over USB, and
  use `pio device monitor -d firmware --echo` for the serial console (115200; type `help`
  for the commands). Add `--filter log2file` to log to a file. If `pio` is not on PATH,
  use `~/.platformio/penv/bin/pio`.
- **Measurement core on the PC:** `pio test -d firmware -e native` (also in CI). After a
  model change: `python3 firmware/tools/model.py` writes the vectors again.
- **App:** `cargo run` / `cargo build --release` in `app/`. `cargo test` checks the protocol
  encoding against the examples in PROTOCOL.md. On Linux the app builds libdbus from
  source (`vendored`), so no `libdbus-1-dev` is needed.
- **Tools:** `python3 firmware/tools/layout_study.py [--drop] [--offsets]` (a few
  minutes). The replay, from `firmware/`: `g++ -std=gnu++17 -O2 -mfma -ffp-contract=fast
  -I lib/measure -o tools/replay tools/replay.cpp lib/measure/solver.cpp
  lib/measure/record.cpp` (FMA like the ESP32, so it matches the board to 0.001 mm; no
  `-mfma` on ARM), then
  `tools/replay [--h mm] [--temp C] [--without ch] <log>` (also built in CI).
- **Release:** `git tag vX.Y.Z && git push origin vX.Y.Z`.
- **Hardware can't be tested here: the user runs every bench test.** The radio link and
  the measurement can only be checked by compiling (and by host-side checks of pure code,
  such as a solver with no Arduino dependencies). Write each bench test so the user can run it and report back, with
  the numbers to expect.

## Rules

- **The protocol rule:** `protocol/PROTOCOL.md` is the source of truth, mirrored by
  `firmware/src/protocol.h` and `app/src/protocol.rs`. Any protocol change touches all
  three in the same commit, updates the examples and tests in `protocol.rs`, and bumps
  `PROTOCOL_VERSION` when an old app could misread new firmware or the reverse.
- **The measurement rule:** a measurement step's code and its MEASUREMENT.md update (step
  status, build notes, bench test, bench results) go in the same commit. Every bench
  result the user reports goes into Bench results, failures included, with what it
  changed.
- **The model rule:** `firmware/MODEL.md` is the source of truth for the measurement
  model, mirrored by `firmware/lib/measure/` and `firmware/tools/model.py` (later also the
  Rust fit). A model change touches all of them in one commit, with the vectors made
  again and `pio test -e native` passing.
- **Pins and timings** go in `firmware/src/config.h`. The pin map must match the host
  board README, and after a map change MEASUREMENT step 1 test 14 is due again.
- **Commit only when the user asks.**
- **Branches:** one branch per measurement step (or any other work that needs a bench
  test), named after it, for example `step3-solver`. It is merged into `main`
  (fast-forward when possible) only once the user reports its bench test passing; then
  the branch is deleted. Small fixes and docs-only changes go straight to `main`. So
  `main` always means "passed on the hardware", and release tags are only made there.
- **Docs style:** plain English, short sentences, numbers with units. Say what was
  measured and what it changed.

## Hardware in brief

- **Sensor board v0.3:** one channel. GTA2718B-3 bottom-port MEMS mic → TLV9062 two-stage
  preamp (×6.3–17.7, ×12 at RV1 mid-travel, three 16 kHz high-pass poles) → TLV3202 dual
  comparator, giving OUT_L (low threshold, trips first) and OUT_H (high threshold, a few
  µs later).
  - Trimpots: RV1 gain, RV2 low threshold, RV3 high threshold. Start at VTH_L = VMID +
    100 mV and VTH_H = VMID + 400 mV.
  - 50 × 32 mm. The board stands on its long edge, mic port 10.5 mm from the paper
    (21.5 mm flipped).
  - J1 is JST XH 5-pin: VA, GND, OUT_L, GND, OUT_H. JLC assembles the SMD parts, 5 boards
    per order.
- **Host board v0.3:** 76 × 118 mm, through-hole, hand-soldered.
  - The NodeMCU-32S on sockets.
  - 8 sensor connectors (CH1–4 west, CH5–8 east, 6 fitted at first), each with a 470 Ω
    series resistor, a 100 k pull-down and an optional scope header.
  - LP2950 low-noise 3.3 V for the sensors; an air sensor on a cable at J5. Before
    ordering, J5 becomes 4-pin for a BME280 (SDA GPIO 14, SCL GPIO 15); a DS18B20 still
    fits (GPIO 14). See the host board README's to-do.
  - IR curtain receiver (BPW34F strip → MCP6002 TIA and gain → LM393, IR_IN GPIO 13, own
    AGND) and emitter drive (IR_EN GPIO 27).
  - Panel LED (GPIO 2) and button (BOOT, GPIO 0).
  - Powered from USB 5 V through a panel switch and a Schottky diode; no battery on this
    version.
- **GPIO map:** in `config.h` (`CAPTURE_LINE_PINS`) and the host board README.
  - L/H per channel: CH1 39/36, CH2 35/34, CH3 33/32, CH4 26/25, CH5 22/23, CH6 19/21,
    CH7 17/18, CH8 4/16.
  - GPIO 5: the capture clock loopback. The pulse generator's sync loopback is GPIO 12
    from step 3a (GPIO 15 before; 15 is now the BME280's SCL).
  - GPIO 34–39 (CH1, CH2) are input-only and have no internal pull-downs.

## Firmware

- Measuring shots: read `firmware/MEASUREMENT.md` first, and PLAN.md for where step 3
  onward is heading.
- `capture.cpp`: I2S0 in camera mode samples the 16 comparator lines at 10 MHz into a DMA
  ring.
  - It works at register level, because IDF 5 has no driver for this.
  - An IRAM interrupt scans each finished block, and freezes the ring around an event by
    linking in two spare DMA blocks.
  - A task on core 1 extracts the edges, and `loop()` prints them.
  - The window length follows from the mic span (`span`).
  - Serial commands (`help`) set the channel mask, dead time and mic span, and run the
    self-test (a pulse pair on CH8's pins).
  - `event.h` holds the event result the other modules read.
- `pulsegen.cpp`: hardware-timed pulse pairs for the self-test and the simulator.
  - MCPWM, one operator per channel, both units started from edges on GPIO 12, by
    register from IRAM.
  - A CPU-driven generator is not exact under BLE load.
- `calib.cpp`: the calibration per channel (mic x, y, h; k; offset; sigma), NVS namespace
  `"cal"` (k and the offsets moved there from `"measure"` in 3c), and the solver's policy
  (`reslimit`, `dropone`). Commands `mic`, `k`, `offset`, `sigma`, `reslimit`, `dropone`.
  Every change prints the `cfg` line, whose id every `rec` line carries. Also calibration
  mode (`cal on`, `cal pt`) and the calibration record (`cal ch`, `cal id`, as
  `tools/calibrate.py` prints them; stale after any later change).
- `arrival.cpp`: t0 = t1 − k·(t2 − t1) − offset per channel (`measure::arrivalT0`), for
  the printed t0 lines and the simulator.
- `position.cpp`: every event (but the self-test's) solved in `loop()` by
  `measure::solveEvent` (4+ channels with a mic and both edges; drop-one when the full
  fit is over `reslimit`); prints the `shot` and `rec` lines; sends a good shot to the app
  (`shots.h`, the buffer in `main.cpp`). Also `testshot`, `stat`'s solver line, and the
  blast check `mic` runs (`lib/measure/blast.h`). `tools/replay` solves a log through the
  same `solveEvent`, so it gets what the board got.
- `simulate.cpp` (`sim ...`): simulated shots at known positions on CH3–CH8 (6 mics, the
  D1/D2 layout; the pins drive their own inputs), checked against the truth: the capture,
  t0 and the position. Faults: late, early, missing, H only, double, blast; trip levels
  per channel (`sim thr`). It stands in until the sensor boards arrive. It takes the
  firmware's air by default; `sim temp` sets another temperature. Its position check counts
  for PASS only when the firmware has its mics (`mic sim`), air, offsets and k; with a
  late or early channel, drop-one must drop it; a blast must not be sent.
- `tools/calibrate.py`: the calibration fit (mics, k, offset per channel from sparks at
  known points; Deming-scaled residuals, see PLAN D5), `--selftest`. The tag
  `archive/step3-first-try` holds a first try that was set aside (it never ran on the board).
- `airtemp.cpp`: the air sensor on J5, read by a task on core 0 once a minute: a BME280
  or BMP280 on I2C (SDA 14, SCL 15; compensation in `lib/bme280/`), or a DS18B20 on
  GPIO 14 through UART2 as a 1-Wire bus. It looks for one 3 times, a minute apart, then
  stops until `temp scan`. Missing values are assumed (18 °C, 50 % RH,
  1013.25 hPa, marked "assumed"). The speed of sound (Cramer) is in `lib/measure/air.h`,
  the start of the portable measurement core (step 3b).
- `main.cpp`: the main loop and the shot buffer.
  - The button: short press = a random test shot (`testshot on|off`); 2 s
    hold = pairing mode; 10 s hold = PIN reset.
  - The heartbeat, and the name and PIN in NVS (namespace `"sensor"`).
  - The status LED: pairing blink, PIN reset, shot flash. The app can't switch it.
  - The shot buffer: the last 64 shots, in RAM, indicated one at a time until the app
    confirms each (`serviceShots`). A clear request from the app drops the shots it has
    (`clearedSeq`).
  - Serial lines go to `captureCommand`.
  - A simple 10 ms loop.
- **Cores:** NimBLE and the air sensor task run on core 0. `loop()` and the measurement task
  (higher priority) run on core 1, and so does the capture interrupt. Arrival times run in
  `loop()` (microseconds per event). The interrupt and the code it calls
  stay in IRAM, because NVS writes stall the flash cache.
- `ble_link.cpp`: NimBLE GATT server. NimBLE callbacks run in NimBLE's task: they only
  store requests, which `loop()` picks up with `bleTake*()`. Don't touch pins, NVS or app
  state from a callback.
- **Security:**
  - Every characteristic needs an encrypted, MITM-authenticated link. The passkey is the
    PIN: 6 digits, default 000000, NVS key `"pin"`, with bonding.
  - Unbonded centrals are disconnected unless pairing mode is on.
  - A pairing without the passkey (no MITM) is rejected and its bond deleted at once.
    NimBLE would otherwise delete it later anyway, when the central re-pairs on the same
    link.
  - Writing a new PIN deletes all bonds but the writer's. BOOT held 10 s resets the PIN
    and deletes all bonds.
- `BOOT_PIN` is defined by the Arduino core; ours is `BOOT_BTN_PIN`.
- NimBLE only calls `onPassKeyDisplay()` while its static passkey is the default 123456,
  so `bleSetPasskey` must not call `NimBLEDevice::setSecurityPasskey`.

## App

- `ble.rs`: a background thread with a tokio runtime owns btleplug. The UI sends
  `Command`s and receives `Event`s over channels; each event requests a repaint. Keep
  btleplug calls out of the UI thread.
- `agent.rs` (Linux): a BlueZ pairing agent on its own D-Bus connection.
  - The app calls `Device1.Pair` itself, so BlueZ asks this agent.
  - For the sensor being paired only, the agent hands the request to the UI (a
    `PasskeyReply` in `Event::PasskeyRequest`). The UI shows a PIN popup and answers it
    directly, because the BLE loop is busy awaiting `Pair` meanwhile.
  - Desktops run no agent unless their Bluetooth settings are open.
  - `Pair` must come **before** connecting: on a link that is already up, BlueZ pairs
    with the system-wide "no input" capability, so without the passkey (no MITM).
  - Connecting to a sensor in pairing mode deletes BlueZ's old keys first (stale bonds).
  - The sensor list shows BlueZ's real pairing state; unpaired sensors can only be paired
    in pairing mode.
- `card.rs`: target cards. Each card is an SVG (the picture) plus JSON (the scoring
  geometry); scoring never looks at the SVG. The cards are rendered with resvg, using the
  embedded IBM Plex font rather than system fonts. The built-in card is in
  `assets/cards/` (ISSF 10 m air pistol). 1 SVG unit = 1 mm.
- **Shots:** `ble.rs` asks for missed shots on every connect and drops duplicates by
  (boot, seq). The app keeps the series in memory. "Clear series" also sends a clear
  request, so the sensor forgets those shots and doesn't resend them after an app restart.
- `main.rs`: the egui UI.
  - **The main window, while connected:** the target card with the series' shots, and the
    series list with a shots-per-score chart below it (bars per ring, or points per tenth
    when decimal; painted by hand in `score_chart`).
  - **On the card:**
    - the newest shot is in the UI's only colour; clicking a shot opens a small info
      popup;
    - zoom with the wheel or the zoom bar (bottom right), drag to pan;
    - the replay bar (bottom left) shows the series again shot by shot; both bars can be
      hidden in settings;
    - a close-call notice at the top left lists every shot of the series within the
      close-call margin (0.5 mm by default) of a higher score, for manual review
      (`Card::higher`, the same rule as scoring);
    - zoomed in, a sharp render of just the visible part replaces the stretched card once
      the view stops moving.
  - **The settings window** (an immediate viewport, opened from the gear in the status
    bar) holds everything else:
    - dark mode;
    - decimal scoring (off by default: whole rings, X = inner ten);
    - the card's zoom and replay bars, the default replay speed;
    - the close-call notice and its margin;
    - the sensor list and pairing;
    - one "Sensor settings" panel: name, PIN, heartbeat, disconnect/forget.

    The PIN popup and messages show in the settings window while it's open.
  - **Storage:** the remembered sensor ID and the preferences are saved with eframe's
    storage.
  - **Styling:** grayscale, IBM Plex Sans embedded from `assets/fonts/`. Emoji come from
    egui's monochrome emoji fonts (`monochrome_emoji_fonts`), not the system's colour
    emoji.
- **App icon** (`assets/icon/`, see its README):
  - the PNG is the window icon of every viewport (`with_app_icon` in `main.rs`);
  - `build.rs` embeds the .ico in the Windows .exe;
  - the release workflow puts the .icns in the macOS bundle;
  - Wayland ignores window icons: it needs `linux/opensights-app.desktop` installed,
    matching the app ID `APP_ID`. `APP_ID` is also the storage name, so changing it loses
    saved settings.

## Where each part is heading (summary of the plans)

- **Measurement** (PLAN, MEASUREMENT steps 3–5):
  - **Layout** (D1/D2): never all mics on one circle, or blasts fit as centre hits. The
    top-right corner mic moves out along the diagonal (it slid 40 mm along its edge
    before, in front of the card), plus bottom-middle and right-middle mics: 6 mics, so a
    bad channel can be dropped and blasts stay out.
  - **Model** (D3, D8): the mic height per mic (a setting, later in the app), k per
    channel, the speed of sound from the BME280's T, RH and p.
  - **Solver** (D12): on the ESP, x, y and the emission time by Levenberg–Marquardt in
    float, residuals and an uncertainty per shot, drop-one; a portable core tested on the
    PC; shots to the app in the v2 message, the rest on serial until protocol v3.
  - **Validity** (step 4): residual, drop-one, loudness, sharpness, curtain, dead time.
    Every event is logged.
  - **Ground truth** (D11): scanned cards.
- **Firmware structure** (D6):
  - a portable measurement core, tested on the PC with replay of recorded events;
  - the platform code apart from it;
  - an uplink interface (BLE, then MQTT);
  - the shot store in flash;
  - separate NVS namespaces for settings, calibration and identity.
- **Calibration** (D5):
  - the target measures and stores; the fit runs in Rust in the app (a wizard), later in
    the management UI;
  - the record has an ID, is copied to the server, and every shot carries it.
- **Messages** (D7): one message set for BLE and MQTT, adding the raw event and
  calibration messages (protocol v3).
- **System** (FUTURE_ARCHITECTURE):
  - one range is one local system;
  - targets publish over MQTT to a range server (a Pi or mini PC with Mosquitto and a
    Rust service on SQLite);
  - viewers get shots over WebSocket: the egui app built as wasm for phones and kiosk
    Pis, natively for development;
  - management in the browser behind an admin login;
  - signed configuration;
  - OTA updates with rollback.
