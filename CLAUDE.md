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
| `protocol/PROTOCOL.md` | The BLE protocol, v3: advertising, GATT, hits, events, status, control, calibration, settings, security, buttons and LED | Before touching the link (see the protocol rule) |
| `hardware/acoustic-sensor/README.md` | Sensor board v0.3: one mic channel, files, checks before ordering | Hardware questions |
| `hardware/host-board/README.md` | Host board v0.3: layout, GPIO map, assembly | Hardware questions, pin changes |
| `hardware/host-board/TODO.md` | The host board's to-do: changes before ordering, first bench checks | Host board work; add board changes there |
| Project summary (claude.ai doc, below) | The hardware design: approaches considered, signal chain and BOM, threshold levels, initial calibration per board, error budget, error sources, valid-shot detection, IR curtain design, host board requirements, its open questions | Hardware or measurement design questions |
| `README.md` | User-facing: what it does, getting started, downloads, notes per platform | Changing user-visible behaviour |
| `app/TODO.md` | The app's to-do list: wishes and known gaps not yet in a release | App work; add new app wishes there |
| `app/CARDPHOTO.md` | Card photos (PLAN D13): the steps C1–C6, their tests and results | Card-photo work; update it in the same commit as the code |
| `app/assets/icon/README.md` | The app icon and where it is used | Icon work |

**The project summary** is a Claude Docs document: "Electronic shooting target – project
summary", https://claude.ai/code/artifact/a8aff581-45d0-41fc-8f4c-dd17fb9b4ac1 (doc ID
`a8aff581-45d0-41fc-8f4c-dd17fb9b4ac1`).

- **Reading it:** use the Claude Docs connector. `read` the doc (ref object `project`,
  that ID) for its tab, then `read` the tab's content node. The tab is about 130 KB of
  XML, so it comes back as a file: strip the tags to text before reading.
- **Editing it:** it's the user's doc. Change it only when asked, as suggestions.
- **Where it is out of date** (PLAN.md has the corrections; the first three laid in the doc
  as suggestions on 2026-11-24, PLAN Q3, waiting for the user):
  - its error budget counts the mic height as 1 mm, but it is 5.5 mm (PLAN D3);
  - its open question on breaking the corner symmetry is answered by PLAN D1/D2;
  - it gives the sensor board as 50 × 32 mm with the port 10.5 mm from the paper edge: it
    is 44 × 27 mm, 5.5 mm;
  - its host board is 76 × 118 mm with a 3-pin DS18B20 connector; the board is 76 × 132 mm
    now, with J5 4-pin (BME280 or DS18B20) beside J6/J7 and U4's supply jumper (host board
    README).

## Status (2026-11-22) and what's next, per part

| Part | Where it stands | Next (see) |
|---|---|---|
| Firmware: BLE, pairing, uplink | Protocol v3 (step 5b): `uplink.cpp` with the hits and the event ring, Status, Control, Calibration and Settings; bench test passes over BLE (2026-11-23, merged to `main`). The shot store in flash (5e, PLAN D6): every hit kept across reboots, requests and clears across boots; bench 5e-1 to 5e-7 pass (2026-11-24, merged to `main`) | 5e-8, a real power cut (the user); MQTT (phase 4) |
| Firmware: measurement | Step 1 passed on a bare NodeMCU (on the v0.3 pin map, tests 3 and 14 are due again). Step 2's simulated tests pass; its real-board tests wait for the boards. Step 3 (solver and calibration, PLAN phase 0) passes all its simulated tests on a bare NodeMCU (2026-11-22, merged to `main`): the solver on the board, drop-one, the blast check, calibration mode and `tools/calibrate.py`. The DS18B20 and BME280 tests (3a-3, 3a-4) wait for the parts. Step 4a (the verdict) passes its simulated bench tests (2026-11-22, merged to `main`). Step 4b (the curtain) passes its simulated bench tests (2026-11-22, run by Claude, merged to `main`) Step 7 (the real boards, PLAN phase 1): the tests written and `tools/bench.py` passing its self-test; 7-0 (h 5.5 mm, simulated) passes (2026-11-24, branch `phase1-prep`) | The boards' arrival (PLAN phase 1, MEASUREMENT 7); 3a-3/3a-4 when the parts are there |
| Sensor board (`hardware/acoustic-sensor/`) | v0.3 designed (KiCad 10), not yet on the bench | Its "check before ordering" list; then the summary's initial calibration, polarity, PLAN phase 1 |
| Host board (`hardware/host-board/`) | v0.3 ready to order: the J5 (BME280) and U4-supply changes on the board, the south half laid out again and the board routed again (2026-11-24, 76 × 132 mm, 0.4/0.6/0.8 mm tracks, DRC and ERC clean); firmware already on its pin map | The user's look in KiCad, then order and assemble; MEASUREMENT step 1 tests 3 and 14 on the real board |
| Frame and mechanics | 250 × 250 mm, PETG mic holders on an aluminium backbone (summary); enclosure open | 6 mics (PLAN D2): the top-right corner moved out along the diagonal by d = 25 mm, to (122.68, 122.68) (PLAN D1's update; principle in `hardware/frame/top-right-holder.svg`), plus bottom-middle and right-middle mics; the user designs and prints the holders. The firmware defaults, the simulator and `model.py` use it from 2026-11-24 (MEASUREMENT 7-0b) |
| IR curtain | Designed (receiver on the host board, emitter board); firmware built and passing simulated (step 4b, on GPIO 13) | Its bench test (summary, PLAN phase 1.6), then the real gate |
| App (`app/`) | v0.1.2, desktop (Linux, Windows, macOS), BLE, 10 m air pistol card, scoring, series, replay; protocol v3 (step 5c, merged to `main`): unscored holes and missed shots, the event log, a read-only Measurement section; the calibration wizard and the field check with the Rust fit (step 5d, merged to `main`); card photos C1–C4 (PLAN D13, merged to `main` 2026-11-24; the series panel with the photo's results, and the holes made fast, 41 shots 85 s → 1.2 s, merged 2026-11-24): registration, holes, the upload from the phone, the photo under the shots and the ground-truth panel, passing their synthetic and HTTP tests, the phone test and the user's look at the UI (2026-11-24) | Card photos C5: the user's real photos (a clean card, then one with holes); a settings editor; the shot store in flash (PLAN D6) |
| Range server, MQTT, management, wasm viewer | Not started (wasm parked) | FUTURE_ARCHITECTURE step 2, PLAN phase 4 |

## Repository

```
CLAUDE.md, PLAN.md, FUTURE_ARCHITECTURE.md, README.md
protocol/PROTOCOL.md          the BLE protocol (source of truth)
firmware/                     PlatformIO, pinned: pioarduino 55.03.312-1 (Arduino-ESP32 3.3.12, IDF 5.5), NimBLE-Arduino 2.5.2
  MEASUREMENT.md              measurement plan and bench results
  MODEL.md                    the measurement model (source of truth)
  src/                        the firmware (modules below)
  lib/measure/                the portable measurement core: air.h, model.h, solver,
                              record (the log lines), blast (the blast check)
  lib/bme280/                 BME280/BMP280 compensation (portable)
  lib/proto/                  protocol v3's messages, encoded and decoded (portable)
  lib/store/                  the shot store: hits in a flash ring across reboots (portable)
  partitions.csv              the partition table: Arduino's default with a 128 KB `shots` partition
  test/test_measure/          Unity tests of the core on the PC (`pio test -e native`)
  test/test_proto/            Unity tests of lib/proto against PROTOCOL.md's examples
  test/test_store/            Unity tests of lib/store: reboots, wrap-around, torn writes, clears
  test/vectors/               shared test vectors, written by tools/model.py
  tools/model.py              the model's reference implementation (numpy)
  tools/replay.cpp            replays a serial log's events through lib/measure (g++)
  tools/calibrate.py          the calibration fit (numpy, scipy)
  tools/bench.py              phase 1 bench analysis: repeat, kcheck, verify, swap (MEASUREMENT 7)
  tools/layout_study.py       mic layout study behind PLAN D1/D2 (numpy)
app/                          Rust 2024, eframe/egui 0.37, btleplug, resvg
  src/                        main.rs (UI), ble.rs, agent.rs, card.rs, protocol.rs, eventlog.rs,
                              calwizard.rs, upload.rs (card photos from the phone)
  cardphoto/                  card photos (workspace member): registration from the rings,
                              the holes, synthetic photos (CARDPHOTO.md)
  calib/                      the calibration crate (workspace member): the target's solver
                              (Rust port), the calibration fit (calibrate.py's), log-line text
  examples/probe.rs           a protocol v3 test client without the UI (bench tests over BLE)
  TODO.md                     the app's to-do list
  assets/                     cards/, fonts/ (IBM Plex Sans), icon/, templates/ (calibration)
hardware/acoustic-sensor/     sensor board, KiCad 10
hardware/host-board/          host board, KiCad 10
hardware/frame/               frame drawings: top-right-holder.svg (the moved corner mic's holder, principle)
.github/workflows/build.yml   CI on a push to main: only the parts a change touches (docs and hardware
                              alone: nothing); a v* tag builds all and a GitHub Release (app for 3 OSes + firmware)
```

## Build and test

- **Firmware** (NodeMCU-32S): `pio run -d firmware`. Add `-t upload` to flash over USB, and
  use `pio device monitor -d firmware --echo` for the serial console (115200; type `help`
  for the commands). Add `--filter log2file` to log to a file. If `pio` is not on PATH,
  use `~/.platformio/penv/bin/pio`.
- **Measurement core on the PC:** `pio test -d firmware -e native` (also in CI). After a
  model change: `python3 firmware/tools/model.py` writes the vectors again.
- **App:** `cargo run` / `cargo build --release` in `app/`. `cargo test` checks the protocol
  encoding against the examples in PROTOCOL.md, and the event log's lines against the
  firmware's. On Linux the app builds libdbus from source (`vendored`), so no
  `libdbus-1-dev` is needed.
- **Card photos' holes, two versions compared:** `cargo run --release -p cardphoto --example
  holes_bench -- [--jobs N] [--only a,b] [--photos <the app's photos/>] > out.jsonl` in `app/`
  (CARDPHOTO.md C2): fixed synthetic cases with their truth, and stored photos; one JSON line
  per case.
- **The calibration crate:** `cargo test` in `app/` also runs `calib/` (the workspace's
  default members): the solver against `firmware/test/vectors`, the fit against
  `calibrate.py`'s self-test (`calib/tests/data`, made by `calibrate.py --selftest
  --write-log ... --json ...`).
- **Over BLE without the UI:** `cargo run --example probe -- <seconds> <log> [writes|drive]` in
  `app/` connects to a bonded sensor, subscribes to everything, writes the event log as the
  app does, and with `writes` exercises Settings, Calibration and Control (MEASUREMENT 5);
  `drive` runs the calibration wizard and the field check from stdin (`check`, `calibrate`,
  `quit`; MEASUREMENT 5d). `PROBE_FROM=<boot hex>:<seq>` asks for shots from there,
  `PROBE_CLEAR=1` clears up to the last one received (MEASUREMENT 5e).
  Opening the serial port resets the board (Linux): start a serial session first, then the
  probe.
- **Tools:** `python3 firmware/tools/layout_study.py [--drop] [--offsets]` (a few
  minutes). The replay, from `firmware/`: `g++ -std=gnu++17 -O2 -mfma -ffp-contract=fast
  -I lib/measure -o tools/replay tools/replay.cpp lib/measure/solver.cpp
  lib/measure/record.cpp` (FMA like the ESP32, so it matches the board to 0.001 mm; no
  `-mfma` on ARM), then
  `tools/replay [--h mm] [--temp C] [--without ch] [--lost n|off] [--area mm|off]
  [--together us|off] [--pulse us|off] [--curtain lo,hi|off] <log>` (also built in CI).
- **Release:** `git tag vX.Y.Z && git push origin vX.Y.Z`.
- **Claude runs the tests (from 2026-11-23); the user reviews the results** and says when a
  part of the testing needs revisiting.
  - The bare NodeMCU is on USB (`/dev/ttyUSB1`): flash with `pio run -t upload`, drive the
    serial console by script (opening the port resets the board on Linux: open one session
    and keep it), log to `firmware/logs/` (not in git: upload, serial and probe logs).
  - BLE from this computer, which is bonded with the sensor: `app/examples/probe.rs`.
  - Write each test down first (steps, numbers to expect), then run it, then record the
    results, failures included.
  - What can't be done here (the sensor boards, the IR curtain, real shots, photos of real
    cards, pressing BOOT) is written for the user to provide or run, with the numbers to
    expect.

## Rules

- **The protocol rule:** `protocol/PROTOCOL.md` is the source of truth, mirrored by
  `firmware/src/protocol.h` with `firmware/lib/proto/`, and `app/src/protocol.rs`. Any
  protocol change touches all of them in the same commit, updates the examples and tests in
  `test/test_proto` and `protocol.rs`, and bumps
  `PROTOCOL_VERSION` when an old app could misread new firmware or the reverse.
- **The measurement rule:** a measurement step's code and its MEASUREMENT.md update (step
  status, build notes, bench test, bench results) go in the same commit. Every bench
  result goes into Bench results (Claude's runs and the user's), failures included, with
  what it changed.
- **The model rule:** `firmware/MODEL.md` is the source of truth for the measurement
  model, mirrored by `firmware/lib/measure/` and `firmware/tools/model.py` (later also the
  Rust fit). A model change touches all of them in one commit, with the vectors made
  again and `pio test -e native` passing.
- **Pins and timings** go in `firmware/src/config.h`. The pin map must match the host
  board README, and after a map change MEASUREMENT step 1 test 14 is due again.
- **Commit at will; never push.** The user reviews and pushes (and makes release tags).
- **Branches:** one branch per measurement step (or any other work that needs a bench
  test), named after it, for example `step3-solver`. It is merged into `main`
  (fast-forward when possible) once its tests pass; then the branch is deleted. Small
  fixes and docs-only changes go straight to `main`. So `main` always means "passed its
  tests", and release tags are only made there.
- **Docs style:** plain English, short sentences, numbers with units. Say what was
  measured and what it changed.

## Hardware in brief

- **Sensor board v0.3:** one channel. GTA2718B-3 bottom-port MEMS mic → TLV9062 two-stage
  preamp (×6.3–17.7, ×12 at RV1 mid-travel, three 16 kHz high-pass poles) → TLV3202 dual
  comparator, giving OUT_L (low threshold, trips first) and OUT_H (high threshold, a few
  µs later).
  - Trimpots: RV1 gain, RV2 low threshold, RV3 high threshold. Start at VTH_L = VMID +
    100 mV and VTH_H = VMID + 400 mV.
  - 44 × 27 × 1.0 mm, clamped (no holes). The board stands on its long edge, the mic
    port 5.5 mm from the paper edge (21.5 mm flipped). The holders put that edge on the
    paper: h = 5.5 mm, the firmware default (`MIC_H_DEFAULT_MM`), a setting per mic. 3D
    files, flat (the leads cut off): `hardware/*/mechanical/` (`hardware/export_mechanical.py`,
    with CadQuery).
  - J1 is JST XH 5-pin: VA, GND, OUT_L, GND, OUT_H. JLC assembles the SMD parts, 5 boards
    per order.
- **Host board v0.3:** 76 × 132 mm, through-hole, hand-soldered.
  - The NodeMCU-32S on sockets.
  - 8 sensor connectors (CH1–4 west, CH5–8 east, 6 fitted at first), each with a 470 Ω
    series resistor, a 100 k pull-down and an optional scope header.
  - LP2950 low-noise 3.3 V for the sensors; an air sensor on a cable at J5, 4-pin, for a
    BME280 (SDA GPIO 14, SCL GPIO 15) or a DS18B20 (pins 1–3, GPIO 14).
  - IR curtain receiver (BPW34F strip → MCP6002 TIA and gain → LM393, IR_IN GPIO 13, own
    AGND) and emitter drive (IR_EN GPIO 27). U4 has a supply jumper (JP1): 3.3 V for a
    TLC372, 5 V for an LM393 (whose inputs only work up to 1.8 V at 3.3 V, below most of
    RV1's threshold range).
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
- `curtain.cpp` (step 4b): the IR curtain. IR_IN (GPIO 13) is high while the beam is
  blocked. MCPWM group 0's capture unit latches its rises and falls in hardware (80 MHz,
  8 ticks per sample), tied to the samples at boot by a pulse on GPIO 12 that both see (a software catch at the I2S start can be µs off), so a dip is
  timed on the event's clock however late its interrupt runs. Not a GPIO interrupt: that
  can be 22 µs late. An event claims the dips within 1.5 ms of its first edge (the `rec`
  line's `ir=`); a dip no event claims is printed as a `dip` line (a shot through an
  existing hole). `curtain on|off|window` (in `calib.cpp`, NVS `"cal"`, the `cfg` line's
  `ir=`; off by default) also switches IR_EN. The pulse generator uses group 0's timers;
  the capture unit is separate.
- `arrival.cpp`: t0 = t1 − k·(t2 − t1) − offset per channel (`measure::arrivalT0`), for
  the printed t0 lines and the simulator.
- `position.cpp`: every event (but the self-test's) solved in `loop()` by
  `measure::solveEvent` (4+ channels with a mic, both edges and a pulse shape in range;
  drop-one when the full fit is over `reslimit`), which also gives the verdict (step 4a,
  MODEL.md "Validity": `ok`, `few`, `together`, `nofit`, `residual`, `lost`, `outside`,
  and last `curtain` (4b): no dip 45–100 µs after the emission time);
  prints the `shot` line (ending in the verdict, `[ok]`) and the `rec` line; sends an `ok`
  shot to the app (`shots.h`, the buffer in `main.cpp`). The limits (`valid` in
  `calib.cpp`) are in the `cfg` line (`lost= area= tog= pmax= ir=`; missing means off, so
  old logs replay as before). Also `testshot`, `stat`'s solver line, and the
  blast check `mic` runs (`lib/measure/blast.h`). `tools/replay` solves a log through the
  same `solveEvent`, so it gets what the board got, verdicts included.
- `simulate.cpp` (`sim ...`): simulated shots at known positions on CH3–CH8 (6 mics, the
  D1/D2 layout; the pins drive their own inputs), checked against the truth: the capture,
  t0, the position and the verdict. Faults: late, early, missing, H only (one channel or
  two), slow (a pulse out of shape), double, blast, spike, silent (a curtain dip with no
  sound); trip levels per channel (`sim thr`). `sim dip <us>|none|off` (4b): it drives
  GPIO 13 and makes a dip that long after each shot, timed by the cycle counter against
  the pulse generator's sync edge (`pulsegenFire(&cycle)`), with interrupts off for 25 µs
  before each edge; it checks the firmware's dip time against that (limit 1 µs). Only
  with no IR receiver on the pin (host board: U4 out). It stands in until the sensor boards arrive. It takes the
  firmware's air by default; `sim temp` sets another temperature. Its position check counts
  for PASS only when the firmware has its mics (`mic sim`), air, offsets and k; with a
  late or early channel, drop-one must drop it; a blast or a spike must not be sent; every
  shot must get the run's expected verdict.
- `tools/calibrate.py`: the calibration fit (mics, k, offset per channel from sparks at
  known points; Deming-scaled residuals, see PLAN D5), `--selftest`. The tag
  `archive/step3-first-try` holds a first try that was set aside (it never ran on the board).
- `airtemp.cpp`: the air sensor on J5, read by a task on core 0 once a minute: a BME280
  or BMP280 on I2C (SDA 14, SCL 15; compensation in `lib/bme280/`), or a DS18B20 on
  GPIO 14 through UART2 as a 1-Wire bus. It looks for one 3 times, a minute apart, then
  stops until `temp scan`. Missing values are assumed (18 °C, 50 % RH,
  1013.25 hPa, marked "assumed"). The speed of sound (Cramer) is in `lib/measure/air.h`,
  the start of the portable measurement core (step 3b).
- `main.cpp`: the main loop.
  - The button: short press = a random test shot (`testshot on|off`); 2 s
    hold = pairing mode; 10 s hold = PIN reset.
  - The heartbeat, and the name and PIN in NVS (namespace `"sensor"`).
  - The status LED: pairing blink, PIN reset, shot flash. The app can't switch it.
  - Serial lines go to `captureCommand`, but `pairing` (pairing mode, as BOOT held 2 s).
  - A simple 10 ms loop.
- **Cores:** NimBLE and the air sensor task run on core 0. `loop()` and the measurement task
  (higher priority) run on core 1, and so does the capture interrupt. Arrival times run in
  `loop()` (microseconds per event). The interrupt and the code it calls
  stay in IRAM, because NVS writes stall the flash cache.
- `uplink.cpp` (step 5b, PLAN D6): what goes to the app, in protocol v3 (`lib/proto/`).
  - The hits (kind shot, hole or missed) go into the shot store (`lib/store/`, the `shots`
    partition in `partitions.csv`, 5e): every hit in flash across reboots, the newest 1,921
    at least; indicated one at a time until the app confirms each. A shot request continues
    from any boot the store holds; a clear request clears up to (boot, seq), earlier boots
    too, kept in NVS (`"shots"`). The sector ahead is erased after 2 s with no shot or event.
    Serial `shots`, `shots clear`, and `reboot` (a clean restart for tests).
  - The event ring: the last 32 events (every verdict, everything a `rec` line has), the
    same way, after any waiting hit. `position.cpp` adds each event and its hit, `curtain.cpp`
    a hole, `capture.cpp` a missed shot.
  - Status (rebuilt every 0.5 s, notified on a change), Control (calibration mode, template
    point), Calibration and Settings (written whole by the app, checked, then
    `calibSetChannels`, `calibSetPolicy`, `captureSetSettings`).
- `ble_link.cpp`: NimBLE GATT server. NimBLE callbacks run in NimBLE's task: they only
  store requests, which `loop()` picks up with `bleTake*()`. Don't touch pins, NVS or app
  state from a callback.
  - Two indicated streams (`BLE_SHOT`, `BLE_EVENT`) share the link's one indication in
    flight.
  - After a firmware update, every bonded computer is told "services changed" on its next
    secured connection (NVS `"blesc"`). BlueZ ignores it with NimBLE (bench 5): a Linux
    computer has to pair again after a GATT change; the app waits 8 s, then says so.
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
  Holes and missed shots (no position) are kept apart (`unscored`), listed in their place
  and on a card notice, never scored.
- **Events (protocol v3):** `ble.rs` subscribes to events and Status, asks for missed events
  like shots, and reads the calibration and settings again when Status shows a new cfg ID.
  `eventlog.rs` writes them as the firmware's log lines (`cfg`, `rec`, `dip`, `MISSED`; C's
  `%.9g` and CRC-32, so the IDs match) to `logs/YYYY-MM-DD.log` in the app's data folder (a
  setting, on by default). The sensor settings show them read-only ("Measurement").
- **Card photos (PLAN D13, `CARDPHOTO.md`):** `upload.rs` runs a `tiny_http` server while
  the dialog is open (port 8710, a single-use token per link, shown as a QR code); the phone's
  page sends the photo as it is; `cardphoto::meta` drops EXIF/GPS/XMP from the file without
  re-encoding; stored in `photos/` next to `logs/` with a JSON beside it; originals removed
  after 30 days unless marked to keep. The registration and the holes are in `cardphoto/`: the hole fit with direct derivatives,
  big groups and the groups themselves on the cores (`par_map`), holes not pinned down
  reported as not found (CARDPHOTO.md C2).
  `photo.rs` processes a photo on a thread (registration, holes against the series' shots,
  the photo resampled onto the card's grid through the placement, `<photo>.result.json`).
  `main.rs`: "Card photo" under the series opens the photo window (QR, status, drop a file);
  it closes once a photo is processed (an error keeps it open). The results go into the
  series panel: a table (#, OpenSights, Paper, both red where they differ, shot x, y, hole −
  shot, off), the rms line and warnings above it, the photo's details folded, and "Keep the
  photo", "Take another photo" (the window again, a new link), "Open the folder", "Remove
  from the card" below it; the side column widens to `SIDE_WIDTH_PHOTO`. The card view stays
  in the sensor's frame (the shots never turn):
  the photo where the real card hung, the app's card drawn there too (`photo::CardPose`, the
  print's way up from its numbers), a Card–Photo fade bar (top right), the holes as rings.
  The Paper column: each hole scored on the printed card (`ShotCheck::paper`, the hole
  through the pose). Clicking a scored row of the series list opens that shot's popup on
  the card (`selected_shot`, as clicking the shot; again closes it).
  `OPENSIGHTS_DEMO_PHOTO=1` adds a synthetic photo of the series (`photo::demo_photo`).
- **Calibration (step 5d):** `calwizard.rs` holds the wizard and the field check as state
  machines (events, Status and Calibration in; actions out: calibration mode, the template
  point, the record), run by the calibration window (`CalTask`) or the probe. The fit is
  `calib::fit` (`calibrate.py` ported; the record rounded as it prints it). The template is
  `assets/templates/grid-4x4.json`. A report goes next to the event logs.
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
    PC; since step 5, protocol v3: every event's raw data, hits of every kind, the status,
    and the calibration and settings to the app (the calibration wizard is step 5d).
  - **Validity** (step 4): 4a the verdict (pulse shape, all at once, residual, lost
    channels, area), 4b the curtain (a dip 45–100 µs after the emission, judged apart
    from the acoustic checks; a dip alone is a shot through an existing hole), 4c
    loudness and sharpness from real pulses. Every event is logged with its verdict.
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
