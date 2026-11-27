# Measurement firmware: plan and bench results

How the firmware turns the comparator lines of the acoustic sensor boards into shots. The
hardware and the design decisions behind it (mics, preamp, thresholds, error budget) are in
the project summary doc ("Electronic shooting target – project summary"); the sensor board
is in `hardware/acoustic-sensor/`. This file is the firmware's plan: it changes in the same
commit as the code, and every bench test result goes into [Bench results](#bench-results).

## Scope

- 6 mics (PLAN D1/D2: four corners with the top right slid 40 mm, the bottom middle and
  the right middle), up to 8 channels (the host board has 8 connectors). Each
  channel gives two comparator lines: OUT_L (low threshold, trips first) and OUT_H (high
  threshold, a few µs later). Lines idle low and go high while SIG is above the threshold.
- **The IR light curtain** comes in step 4b, simulated first: IR_IN (GPIO 13) is timed by
  the MCPWM capture, and IR_EN (GPIO 27) is high while its gate is on (`curtain on`; off
  by default).
- Results go out over **serial** (115200), and over BLE: from step 3c solved shots in the v2
  shot message, from step 5 protocol v3 (`protocol/PROTOCOL.md`): every event with its raw
  data, hits of every kind (shot, hole, missed), the sensor's status, calibration mode, and
  the calibration and settings read and written by the app.
- Accuracy target after calibration: 0.2–0.5 mm (1σ). Every 1 µs of timing error is about
  0.37 mm, so the firmware must add well under 0.1 µs.

Until a real curtain is fitted and tested (PLAN phase 1.6), two cases stay uncovered: a
shot through an existing hole (almost no crack: missed), and a blast from a neighbouring
lane that fits as a hit about 10 mm from centre on the centre lines (only absolute level
and edge sharpness can reject it). Logging every rejected event matters more because of
this.

## Pins (host board v0.3)

From `hardware/host-board/README.md`; `CAPTURE_LINE_PINS` in `config.h`. (Step 1 was
bench-tested on the v0.2 map; v0.3 moves every line, see Bench results.)

| Channel | OUT_L | OUT_H |
|---|---|---|
| CH1 | 39 | 36 |
| CH2 | 35 | 34 |
| CH3 | 33 | 32 |
| CH4 | 26 | 25 |
| CH5 | 22 | 23 |
| CH6 | 19 | 21 |
| CH7 | 17 | 18 |
| CH8 | 4 | 16 |

Other: IR_IN 13, IR_EN 27, STAT 2 (blue LED + panel LED), BOOT 0. J5, the air sensor:
GPIO 14 (BME280 SDA, or DS18B20 DQ; 4.7 k pull-up) and 15 (BME280 SCL, 4.7 k pull-up;
the host board change before ordering). Spare: 5 (the capture clock loopback) and 12 (the
pulse generator's sync loopback from step 3a; was 15). Empty connectors are held low by 100 k pull-downs. A breadboard
setup uses the same map, so nothing changes when the host board arrives.

## Pipeline

| Stage | Runs on | Does | Output |
|---|---|---|---|
| I2S parallel capture | DMA | Samples the 16 lines at ~10 MHz into a ring of ~64 KB (~3 ms) | 16-bit samples |
| Activity scan | DMA end-of-block interrupt (IRAM) | ORs each finished block; on activity, marks the event window and freezes the ring around it | Event window |
| Edge extraction | Measurement task, core 1 | First rising edge per line in the window, plus pulse length | t1 (L), t2 (H) per channel |
| Arrival times | Measurement task | t0 = t1 − k·(t2 − t1) − offset per channel | t0 per channel |
| Position solver | `loop()` (microseconds per event) | Fit of x, y and the emission time over 4–8 mics (PLAN D12), speed of sound from T, RH, p | x, y in mm, residuals, uncertainty |
| Validity checks | `loop()`, with the solve (`measure::solveEvent`) | Pulse shape, all-at-once, residual, lost channels, area (4a); curtain (4b); loudness, sharpness (4c). Dead time: the capture | A verdict: `ok` or the reason |
| Shot buffer | `loop()` (existing) | Indicates shots to the app, resends after a link drop | Protocol v2 shot |

Tasks: NimBLE runs on core 0, and so does the air sensor task (once a minute).
Arduino's `loop()` runs on core 1; it stays the light
10 ms loop (button, LED, BLE requests, shot buffer). The measurement task runs on core 1 at
a higher priority and hands finished shots to `loop()` through a queue. The DMA interrupt
and the code it calls live in IRAM, because NVS writes (name, PIN) stall the flash cache.

Code: `capture.cpp/.h` (I2S, ring, activity scan, edge extraction, output, commands, the
self-test), `event.h` (the event result), `pulsegen.cpp/.h` (MCPWM pulse pairs for the
self-test and the simulator), `arrival.cpp/.h` (t0), `simulate.cpp/.h` (the sensor
simulator), `airtemp.cpp/.h` (the air sensor); from step 3, the portable measurement core
in `lib/measure/` (no Arduino; tested on the PC) and `solver.cpp/.h` (its glue: settings,
commands, output). The model is specified in `MODEL.md` (from step 3b). Pins and timings go in `config.h`; a channel mask says which connectors
are fitted.

## Steps

The decisions behind the next steps (mic layout, model, who calibrates) are in
`../PLAN.md`; step 3 below is rewritten from it. Each step ends
with a bench test the user runs. The next step starts only when it passes,
so a problem shows up in the step that caused it.

| # | Step | Status |
|---|---|---|
| 1 | Capture and raw dump | done on the v0.2 map; on the v0.3 map: retest 3 (self-test) and 14 (pin map) |
| 2 | Arrival times | simulated tests 2a-2d pass (2d without a sensor); real-board tests wait for the boards |
| 3 | Solver and calibration | **simulated tests pass** (bare NodeMCU, 2026-11-22; merged to `main`): 3a-1, 3a-2, 3a-5, 3b (PC), 3c, 3d, 3e. Waiting for the parts: 3a-3 and 3a-4 (DS18B20, BME280). With the boards: the same solver and calibration on real sensors (PLAN phase 1) |
| 4 | Validity checks | 4a (the verdict) passes its simulated bench tests on a bare NodeMCU (2026-11-22). 4b (the curtain) passes its simulated bench tests on a bare NodeMCU (2026-11-22, run by Claude). 4c (loudness, sharpness) planned. 4d (a moderately off mic: Huber weights or an earlier drop-one) studied, synthetic and on the board's path (2026-11-27, run by Claude): drop-one started by sigma wins; the firmware change waits for the user |
| 5 | Protocol and app | 5a (protocol v3), 5b (firmware) and 5c (app) built; their bench test passes over BLE from the PC (2026-11-23, run by Claude, merged to `main`). 5d (the calibration wizard, the Rust fit, the field check) passes its bench test over BLE (2026-11-23, run by Claude, merged to `main`). 5e (the shot store in flash) passes 5e-1 to 5e-7 (2026-11-24, run by Claude, merged to `main`); 5e-8 (a real power cut) is the user's. 5f (the app's curtain switch, and unscored hits against a real sensor) passes 5f-1 to 5f-5 (2026-11-27, run by Claude, merged to `main`). 5g (the settings editor) passes against the sensor (2026-11-27, run by Claude, merged to `main`) |
| 6 | Wi-Fi and MQTT | planned |
| 7 | The real boards (PLAN phase 1) | tests written and the analysis tool (`tools/bench.py`) passing its self-test (2026-11-24); 7-0 (h 5.5 mm, simulated) passes; 7-1 to 7-7 wait for the boards |

### 1. Capture and raw dump

Build:
- I2S0 in camera (parallel input) mode, 16 bits, about 10 MHz, DMA into a ring of 16 ×
  4 KB blocks. Arduino-ESP32 3.x (IDF 5) has no driver for this on the classic ESP32, so it
  is register level, as esp32-camera and the ESP32 logic-analyzer projects do it. The
  sample clock comes from LEDC on GPIO 5 (spare, leave it unconnected) and is read back
  from the same pad as the I2S clock input.
- Activity scan in the end-of-block interrupt: a fitted line that changes level inside the
  block, or between the previous block's last sample and this block's first (an edge
  exactly on a block boundary). On activity, the measurement task reads an event window
  that starts one block (200 µs) before.
- Window length from the **mic span**, the largest distance between two mics (the diagonal
  for corner mics): sound reaches two mics at most span / speed of sound apart, wherever
  the shot lands. The window covers span / 331 m/s (0 °C) + 20 % + 100 µs after the first
  edge, rounded up to whole blocks; the first edge can be at the end of its block, so the
  trigger block doesn't count. Serial `span <mm>`, stored in NVS. Default 300 mm: corner
  mics just outside a 200 × 200 mm card (frame 250 × 250 mm), window 1.2 ms after the
  first edge, 1.6 ms in all. Up to about 690 mm fits in the ring.
- The ring freezes around each event, so a window can't be overwritten however long the
  task is held up (a flash write stalls every task, but not the IRAM interrupt or the
  DMA). At the trigger, the interrupt links the window's last descriptor to two spare
  blocks that point to each other; the DMA keeps sampling into them until the task has
  read the window and links the ring back in. Spare blocks still count as time. Activity
  in a spare block after the dead time is printed as `MISSED`: a shot that came while
  the previous event was still being read. The first ring block after the relink follows
  the last spare block in time, not its ring neighbour: it is scanned against that spare's
  last sample, and a window starting there has no block before the trigger.
- Edge extraction: XOR with the previous sample, then the changed bits; stop once every
  line has its first rising edge. Times in ns from the window start; block numbers from
  the DMA descriptors.
- Dead time after an event, 30 ms to start (a setting), never shorter than the event
  window: anything inside the window belongs to the event (a second shot there would be
  merged in, unreported). After a freeze that saw a `MISSED` shot it starts again at the
  relink, so that shot's remainder can't open a window in its middle.
- Serial output, one line per event: event number, then per channel t1 and t2 in ns and
  the pulse length of each line; quiet lines marked `-`, a line already high at the window
  start `<x` (it rose more than x ns before the first edge). A verbose mode lists every
  transition of every line in the window (ringing, double trips).
- Serial commands (type `help`): `stat`, `mask <hex>`, `deadtime <ms>` (both stored in
  NVS, namespace `"measure"`), `verbose on|off`, `selftest [ns] [n]`.
- Self-test: MCPWM drives a pulse pair out on CH8's own pins (GPIO 4 = L, 16 = H), OUT_H
  a set gap after OUT_L (12.5 ns steps), both falling together. The I2S reads them back
  on the same pins, so no jumpers are needed; CH8 must have no sensor board plugged in.
  A jumper from GPIO 4 to another input checks that input. The gap is made by the timer
  hardware, so the captured gap must be within one sample of it on every pulse. A
  CPU-driven generator did not work (bench tests 1-4): BLE traffic, flash writes and the
  Xtensa write buffer shifted its pin writes by up to several µs, and reading the pins
  back to check it changed its timing again.

Bench test (bare NodeMCU, no sensor boards needed). Flash, then open the monitor with
`pio device monitor -d firmware --echo` and type the commands:
1. `mask 0`: a bare NodeMCU has nothing on the inputs, and GPIO 34-39 have no internal
   pull-downs, so they float. The self-test adds CH8 by itself.
2. `stat` twice, a few seconds apart: the sample rate reads 10.000 MHz and the block count
   grows. 0 blocks: the I2S doesn't run. 5 MHz: each DMA block is only half filled.
3. `selftest`: 20 of 20 pulse pairs, captured gap 1000 ns (within one sample on every
   pulse), no extra transitions, PASS. `selftest 1050 200`: a mix of 1000 and 1100
   (one sample is 100 ns, so no single pulse can read 1050), about half at 1100, and
   the mean within the printed rounding error of 1050 (each pulse starts at a random
   point within a sample, as real sound does). Then `selftest 0` (no skew between
   lines: gap 0), `selftest 500`, `selftest 10000`, `selftest 100000`.
4. `verbose on`, `selftest 1000 3`: each CH8 line shows one + and one -, pulse lengths
   about 11 µs (L) and 10 µs (H).
5. Pin map: `mask FF`, a jumper from GPIO 4 to one input at a time, `selftest 1000 3`:
   only CH8 and the jumpered input have edges, at the same time as CH8 L. The floating
   inputs 34-39 may show noise unless they are tied to GND with a resistor or jumper.
6. With the app connected: `selftest 1000 200` while changing the sensor name in the app
   (an NVS write) and while shots go to the app. 200 of 200 pulses, nothing `MISSED`,
   every gap within one sample, PASS.
   `stat` shows how long the ring was frozen at most: normally under 1 ms, longer while
   flash was written.
7. `stat`: the sample rate still reads 10.000 MHz after many events (event timestamps:
   pulses 50 ms apart show up 50 ms apart). Note the interrupt load, the longest freeze
   and the free DMA memory in Bench results.
8. `span 600` then `selftest 1000 3`: the window covers 2.4 ms; `span 300` back.

Extra tests (each pulse pair lands at a random point in its DMA block; the summary counts
L per quarter of the block and pairs with L and H in different blocks):
9. `selftest 1000 400`: L spread over all four quarters, PASS.
10. `selftest 150000 200`: L and H in different blocks in most pairs (H 150 µs later),
    PASS.
11. Long gaps up to the window: `selftest 1000000 100`, then the largest gap `selftest`
    accepts (it prints the limit when asked for more, e.g. `selftest 2000000`). PASS.
12. Short pulses: `selftest 0 200 100`, `selftest 300 200 200`, `selftest 1000 200 500`
    (gap, count, H pulse length in ns). Every pair seen, lengths right, PASS.
13. Soak with the app connected: `selftest 1050 1000` (about 50 s) with renames and test
    shots during it. About 50 % at 1100, PASS; `stat` after.

Break-it tests (past the limits: it must report what it loses and keep its time):
14. Pin map, every line: `mask 7F`, a jumper from GPIO 4 (CH8 L) to one pin at a time,
    `selftest 1000 5`: only that line has edges besides CH8, at the same ns as CH8 L
    ("also had edges"; still PASS, the test is of CH8). Needed again after any change
    to the map.
15. Flood: `mask 03`, `deadtime 1`, `verbose on`, a finger on GPIO 39 for 30 s (mains
    hum). Test shots and the heartbeat still reach the app, commands still work; `stat`:
    10.000 MHz. Back: `deadtime 30`, `verbose off`.

Done when: every pulse shows up once, at the right time, on the right line, with BLE busy.

Fallback if I2S parallel capture can't be made to work: MCPWM capture (12.5 ns, 6 inputs,
so three channels with both lines), and revisit the design for 4+ channels.

### 2. Arrival times

Until the sensor boards arrive, a simulator stands in for them: shots at known positions,
made as hardware-timed edges on CH5-CH8's own pins and captured like real ones. It checks
everything from the pins on (capture, t0, later the solver) against the truth; it can't
check the analog side (mics, preamp, thresholds, polarity, noise). The real-board tests
below stay.

Build:
- **t0 = t1 − k·(t2 − t1) − offset** per channel (`arrival.cpp`). For a signal rising in a
  straight line both comparators trip late by an amount that grows as the rise gets
  slower; k = VL / (VH − VL) takes it out (k = 1/3 for VH = 4·VL). Serial `k <value>` and
  `offset <ch> <ns>` (`offset` lists, `offset clear`), stored in NVS (`"measure"`: `k`,
  `off1`..`off8`), so firmware updates keep them.
- Under each event line: t0 and t2 − t1 per channel, then the t0 differences to the first
  channel that has one, and the air temperature (not for self-test events).
- **Pulse generator** (`pulsegen.cpp`), for the self-test and the simulator: one MCPWM
  operator per channel, its two comparators set when L and H rise, both fall at the end of
  the run. Channels 0-2 on MCPWM unit 0 (one timer: exact against each other), 3-5 on unit 1.
  Both timers take their count from edges on the sync pin (GPIO 15, GPIO 12 from step 3a;
  looped back inside the chip): a sync edge sets both stopped counts to 0, both timers are
  started, a second edge sets both to 0 again at the same moment. All from IRAM with
  interrupts off, by register (the driver's start runs from flash); the rises come ≥ 20 µs later, and every run that took longer to
  start is counted as a late start. The self-test now runs on it (1 channel).
- **Simulator** (`simulate.cpp`, `sim ...`): 4 mics on CH5-CH8 (GPIO 22/23, 19/21, 17/18,
  4/16; CH1/CH2 are input-only) at the corners of a 210 mm square, a speed of sound from a
  simulated temperature (20 °C; from step 3a the firmware's air), and a trip model: the pulse rises like A·sin(πt/2T)
  (`sim model sine`) or its straight start (`linear`, default), T = 12 µs, OUT_L trips at 1
  and OUT_H at 4, A = amp·100 mm / distance (`sim amp`, default 40). 25 ns ticks; all lines
  fall together 10 µs after the shot's last rise (the generator can't end them one by one,
  so the first mic's pulse is as long as the spread between the mics); each shot
  at a random point in a sample and a DMA block. Per shot it prints t0's error against
  t0 from the edges it sent (only the 100 ns sampling between them: within ±(1 + 2k)
  samples + one tick, about 56 ns rms) and against the true arrival (the t0 model); the
  summary has mean, rms and worst per channel and PASS/FAIL.
  `sim at <x> <y> [n]`, `sim random [n]`, `sim raw <a> <b> <c> <d> [n] [gap]` (fixed edges,
  to check the generator), `sim fault late|missing|honly <ch>` (+24 µs on a channel, a
  channel with no edges, H without L), `sim fault double <us>` (a second shot: measured,
  `MISSED`, or ignored inside the dead time, never silently lost), `sim offset <ch> <ns>`
  (a channel's own delay), `sim mic`, `sim temp`, `sim stop`. CH5-CH8 must have no sensor
  boards plugged in while it runs.
- **Air temperature** (`airtemp.cpp`): a DS18B20 on GPIO 14, read by a task on core 0 once
  a minute through UART2 as a 1-Wire bus (the UART times the slots, no interrupts held off).
  Checked by CRC, the configuration byte's fixed bits, the 85 °C power-on value and a
  −20..60 °C range. No sensor, or no good reading for 10 minutes: 18 °C, marked
  "(assumed)". 0.1 °C is about 0.01 mm, so once a minute is plenty. `temp` and `stat` show
  it.

Bench test, simulated (bare NodeMCU, nothing on CH5-CH8 and the sync pin, GPIO 12 from step 3a), `mask 0`, `span 300`,
`deadtime 30` (both are stored: check with `stat`):
- **2a, generator.** Step 1 test 3 again (`selftest`, `selftest 1050 200`, `selftest 0`,
  `selftest 100000`): PASS as before, now on the new generator, 0 late starts. Then
  `sim raw 0 0 0 0 200`: every line within a sample of CH5 L. CH6 and CH7 share CH5's
  timer: mean about 0. A mean on CH8 alone is an offset between the two MCPWM units (fine up
  to a tick, 25 ns). Then `sim raw 0 100000 200000 300000 100 1500`: the same, at
  different times.
- **2b, t0.** `sim at 0 0 100` and `sim at 60 -30 100`: every t0 error within the limit,
  rms about 56 ns, mean about 0, and 0 ± a tick against the true arrival (linear model,
  k = 1/3). Then `k 0.25`, `sim at 60 -30 50`: still PASS against the edges sent, but
  against the true arrival a clear error that grows with distance (the wrong k). `k 0.3333`
  back. `sim model sine`, `sim random 100`: what the straight-line formula leaves on a
  curved rise (note it in Bench results); `sim model linear` back.
- **2c, random shots and faults.** `sim random 500` with the app connected and a rename or
  two during it: PASS. `sim offset 6 500` then `sim random 50`: CH6 about +500 ns against
  the true arrival; `offset 6 500` (the firmware's): back to about 0; then `offset clear`,
  `sim offset 6 0`. `sim fault late 7`, `sim at 30 30 20`: CH7 +24000 ns against the true
  arrival, still PASS against the edges sent. `sim fault missing 8` and `sim fault honly 8`:
  CH8 has no t0, as set. `sim fault double 2500`, `sim at 0 0 20`: with `deadtime 30` all
  ignored inside the dead time; with `deadtime 1`, each second shot measured or `MISSED`,
  none lost and no partial events (a window opened in the middle of a shot). Then
  `verbose on`, `sim at 0 0 3`, `verbose off`. `sim fault none`, `deadtime 30`.
- **2d, temperature.** Without a DS18B20: `temp` says 18.0 °C assumed, the event lines end
  in "air 18.0 °C (assumed)". With one (data on GPIO 14, 4.7 k to 3V3): the reading
  appears within a minute and updates every minute; `stat`'s interrupt load and longest
  interrupt unchanged; the app stays connected through 10 minutes.

Bench test, real (sensor boards and host board):
1. Per board, the initial calibration from the project doc (VMID, gain, thresholds).
2. Piezo-lighter spark or snapped pencil lead near one mic: OUT_L before OUT_H, and the
   verbose dump matches the scope on SIG and OUT_L/OUT_H.
3. Mic polarity: SIG must first go up. A half-cycle-late trip shows as about 24 µs extra
   on one channel.
4. Spark at the centre of the mounted frame, 20 times: the four t0 agree within a few
   hundred ns after per-channel offsets.
5. The DS18B20 on the host board against a thermometer next to the mics, in the closed
   target: the board may read warm (IR LEDs, regulator).

Done when: the simulated tests pass, centre sparks give consistent t0 on all four
channels, polarity is confirmed, and there are no half-cycle-late trips.

### 3. Solver and calibration

Rewritten 2026-11-21 from `../PLAN.md` (D2, D3, D8, D12). One branch, `step3-solver`, in
five sub-steps, each with its own test; the next starts when the last one passes. All of
it is tested on a bare NodeMCU and the PC: the real-board tests move to PLAN phase 1. Step
3 is done, and the branch merges, when 3a-3e pass.

The archived first try (`git show archive/step3-first-try:<path>`) supplies parts: the
solver in `firmware/src/tdoa.h`, `mic` and the `cal` lines in `solver.cpp`, the
simulator's position check, and `firmware/tools/calibrate.py`.

#### 3a. The air sensor and the sync pin

Build:
- **The sync loopback moves from GPIO 15 to GPIO 12** (`SYNC_PIN`), because GPIO 15
  becomes the BME280's SCL. GPIO 12 is a strapping pin (flash voltage): nothing may pull
  it up at boot, and the firmware only drives it after boot, inside the chip.
- **J5 takes either sensor** (`airtemp.cpp`). At boot, then once a minute, 3 times in all
  (`AIR_PROBE_TRIES`), the task looks for:
  - a BME280 on I2C (SDA 14, SCL 15, address 0x76 or 0x77, chip ID 0x60): temperature,
    humidity and pressure, in forced mode once a minute, compensated with the datasheet's
    integer formulas;
  - a BMP280 (chip ID 0x58): temperature and pressure, humidity assumed;
  - a DS18B20 on GPIO 14 (1-Wire through UART2, as in step 2): temperature only.

  With none found after the third try (2 minutes after boot) it stops looking, and `temp`
  says so; `temp scan` looks again, 3 more tries. A sensor that fails 3 readings in a row
  counts as gone, and the search starts again the same way. So plug a sensor in before
  boot, or type `temp scan` after plugging it in.

  Missing values are assumed: 18 °C, 50 % RH, 1013.25 hPa, each marked "(assumed)". A
  reading older than 10 minutes counts as missing. The BME280's readings are checked for
  range (−20..60 °C, 0..100 % RH, 300..1100 hPa) and for the all-ones of a bus with
  nothing on it.
- **The speed of sound** from T, RH and p (Cramer 1993, with 400 ppm CO₂), in the
  portable core (`lib/measure/air.h`) so the solver and the PC share it.
- **Output:** `temp` and `stat` show the sensor type, T, RH, p and c, each marked measured
  or assumed. Event lines end in `air 21.3 °C 45 %RH 1009.8 hPa, c 344.6 m/s`.
- **The simulator takes the firmware's air** (and the same formula for c), so the two agree
  by default. `sim temp <C>` sets a different temperature on purpose (to test a
  temperature error, 3c), `sim temp air` goes back. Until 3a it had its own fixed 20 °C and
  a linear formula.

Reference values for c (Cramer, 1013.25 hPa): 0 °C dry 331.45 m/s; 18 °C 50 % 342.74;
20 °C 50 % 343.99; 25 °C 50 % 347.13. Per °C +0.62 m/s, per % RH +0.011 (18 °C).

Bench test, bare NodeMCU, nothing on GPIO 12, `mask 0`:
- **3a-1, no sensor.** `temp`: "no sensor on J5", then 18.0 °C, 50 %RH and 1013.2 hPa,
  each "(assumed)", c 342.74 m/s. A simulated shot (`sim at 0 0 1`) ends in the same air
  line.
- **3a-2, the sync pin.** Step 2a again: `selftest`, `selftest 1050 200`, `selftest 0`,
  `selftest 100000`, then `sim raw 0 0 0 0 200` and
  `sim raw 0 100000 200000 300000 100 1500`. PASS as before, 0 late starts.
- **3a-5, it stops looking.** No sensor, 3 minutes after boot: `temp` says "looked 3
  times, stopped looking", and the count stays at 3 a minute later. `temp scan`: the count
  goes to 4 at once, 6 two minutes later, then stays.
- **3a-3, a DS18B20** (data on GPIO 14, 4.7 k to 3V3; plugged in before boot, or
  `temp scan` after): found within a minute, T measured,
  RH and p still assumed.
- **3a-4, a BME280 breakout** (SDA 14, SCL 15, 3V3, GND; most breakouts have their own
  pull-ups; before boot, or `temp scan`): found within a minute as a BME280 at 0x76 or 0x77. T within about 1 °C of a
  room thermometer; RH plausible indoors (30-60 %), and up by several % within a minute of
  breathing on it; p within a few hPa of the local weather station's pressure at your
  altitude (not the sea-level QNH). c matches the reference table for those values. With
  the BME280 on GPIO 15, 3a-2's tests again: still PASS (the sync loopback no longer
  shares a pin with it). `stat`'s interrupt load and longest interrupt unchanged.

3a-3 and 3a-4 need the parts; whichever is missing waits, and is noted in Bench results.

#### 3b. The model and the measurement core (PC)

Build:
- **`firmware/MODEL.md`**, the source of truth for the model (the model rule in
  CLAUDE.md):
  - frame coordinates: mm from the centre, x to the right, y up, as seen by the shooter;
  - per mic: x, y, h (set, not fitted), k, offset, σ;
  - t0 = t1 − k·(t2 − t1) − offset;
  - t0ᵢ = τ + dᵢ / c, dᵢ = √((x − xᵢ)² + (y − yᵢ)² + hᵢ²);
  - c(T, RH, p);
  - the solver and its outputs (PLAN D12), the uncertainty, drop-one;
  - the test vector format.
- **`lib/measure/`**: `air.h` (c), `model.h` (distances, the forward model), `solver.h/.cpp`
  (the fit, drop-one, the covariance), the pure part of `arrival.cpp`. No Arduino, no
  `Serial`, no NVS.
- **`tools/model.py`**: the Python reference of MODEL.md (numpy), which writes the test
  vectors to `test/vectors/*.txt`, one case per line: the mics, the air, the t0s, and the
  expected position, τ, residuals and uncertainty.
- **`pio test -e native`**: Unity tests on the PC against the vectors; CI runs them.

Tests (PC; I run them, you can too with `pio test -d firmware -e native`):
- c matches `model.py` to 0.001 m/s over −10..40 °C, 0..100 % RH, 900..1050 hPa.
- Noise-free shots on a 5 mm grid over the 200 × 200 mm area, h 10.5 and 21.5 mm, the
  D1/D2 layout: position within 0.001 mm of the truth, converged within 15 iterations
  from the planned start.
- The same shots with 50 ns rms of noise per channel: rms error per point within 10 % of
  the uncertainty the solver reports (about 0.02 mm in the ring area).
- Points up to 100 mm outside the frame: they converge or are reported as not converged,
  never a wrong position flagged as converged.
- Every vector: C++ (float) and Python (double) agree to 0.001 mm, residuals to 0.5 ns
  (float's steps near 10⁶ ns are 0.06 ns), σ to 0.1 %.
- A channel 24 µs late: the full fit's residual over 4 µs, drop-one finds that channel
  and the position within 0.01 mm.
- Blasts (10 and 25 m in front, 0-3 m off the axis): none fits as a hit, with or without
  drop-one.
- Time per solve on the PC is not the ESP's; 3c measures that.

#### 3c. The solver on the board

Build (done, on `step3-solver`):
- **The calibration** (`calib.cpp`), in NVS namespace `"cal"`, one record per channel: its
  mic (x, y, h), k, offset and sigma (50 ns until 3e). k and the offsets were in
  `"measure"`; on the first boot with 3c they are carried over (to every channel's k) and
  removed there. With nothing stored, the mics are `mic default`.
  - `mic` lists them and runs the **blast check** (below);
  - `mic <ch> <x> <y> [h]`, `mic h <mm>` (every mic), `mic h <ch> <mm>`;
  - `mic default`: the D1/D2 layout on CH1-CH6, h 10.5 mm (5.5 mm from 2026-11-24), no mic on CH7/CH8;
  - `mic sim`: the simulator's mics on CH5-CH8 and none elsewhere (to test it);
  - `mic clear <ch>`: no mic on that channel;
  - `k` lists k; `k <value>` sets every channel's, `k <ch> <value>` one; `offset` as before.
- **The blast check** (`lib/measure/blast.h`): the layout study's 624 blasts (point sources
  10 and 25 m in front, 0-3 m off the axis) through the board's own solver, with every mic
  and with each one dropped. A blast "scores" if its fit converges inside ±100 mm within
  `reslimit`. Any score is a warning: the mics, or the ones left after dropping one, are
  close to one circle. This replaces the planned "within 25 mm of one circle" test: it
  measures what matters directly. On the PC (`pio test -e native`) it gives the study's
  numbers: the square 484 of 624, the D1 corners 0 (smallest residual 9.0 µs; the study
  9.1), D2 0 with every mic and with any one dropped (smallest 13.5 µs, without CH2).
- **Every event** (except the self-test's) is solved in `loop()` (`position.cpp`) if 4 or
  more channels have a mic and both edges. Under the event's lines:
  - `shot <evt>: x .. y .. mm`, the rms residual, the 1σ along the error ellipse's long
    axis, the iterations, each channel's residual, and whether it was sent;
  - a `cfg` line whenever the settings change (and at boot, `mic`, the start of a `sim`
    run), and a `rec` line for every event: its edges, the air, c and the cfg's id.
    The format is in MODEL.md "Log lines" (`lib/measure/record.h`).
- **Solved shots go to the app** (the v2 shot message) when the fit converged with an rms
  residual within `reslimit` (default 1500 ns, stored in `"measure"`). The rest are logged
  only. Validity proper is step 4. `testshot on|off` (stored, default on) keeps BOOT's
  random test shot.
- **Replay on the PC:** `tools/replay.cpp` reads a log's `cfg` and `rec` lines and solves
  every event again through the same code (`measure::solveInput`, `measure::solve`). With
  the board's `shot` lines in the log it compares them. Options change the settings for
  the replay: `--h`, `--temp`, `--c`, `--without <ch>` (5 mics instead of 6), `--reslimit`.
  Build it from `firmware/`:
  `g++ -std=gnu++17 -O2 -mfma -ffp-contract=fast -I lib/measure -o tools/replay tools/replay.cpp lib/measure/solver.cpp lib/measure/record.cpp`
  (`-mfma` on x86-64 PCs; leave it out on ARM). The ESP32 fuses `a·b + c` into one
  instruction, which rounds differently; with FMA on the PC too, a replay matches the
  board to the printed 0.001 mm (498 of 500 shots; 2 one step off), against 0.007 mm worst
  without it (on shots with a late channel).
- `stat` shows the solver: events solved, converged and sent, the time per solve (mean
  and longest, µs), `reslimit`, test shots, the settings' id.
- **The simulator** has a height per mic (`sim mic <ch> <x> <y> [h]`, default 10.5 mm; 5.5 mm from 2026-11-24), still
  4 channels on CH5-CH8, and checks the position against the point it sent. The check is
  part of PASS (every shot solved and converged, the worst error within 0.1 mm) only when
  the firmware solves the simulator's world: the same mics, heights and offsets, its own
  air (no `sim temp`), the linear model with k = 1/3, no fault. Otherwise it is shown for
  information, with the reason. Shots are 150 ms apart now (an event prints about 1000
  characters).
- Serial: the TX buffer is 4096 bytes; an event is printed only with 1700 free.

Expected values come from a simulation of the simulator on the PC (the square, h 10.5 mm,
the 25 ns steps and the 100 ns sampling, 2000 random shots).

Bench test, bare NodeMCU, `mask 0`, nothing on CH5-CH8 or GPIO 12:
- **3c-0, the first boot.** It prints "cal: no calibration stored yet: the default mics
  (D1/D2 on CH1-CH6), k and the offsets carried over from step 2" and a `cfg` line. `k`:
  every channel 0.3333 (the stored value from step 2). Then `k 0.33333` (the linear
  model's 1/3, or the simulator says its position check is for information only), and
  `mic`: D1/D2, the blast check with 0 scored on every line ("no blast scores, with every
  mic or with any one dropped"), the smallest residuals about 34, 35, 13, 37, 37, 22 and
  35 µs, in under a second (the time is printed). `stat`: the solver line.
- **3c-1, the square.** `mic sim`: CH5-CH8 at the corners, and the blast check warns
  (about 484 of 624 scored: the 4 sim mics are on one circle; expected). `sim random 200`:
  "position check on" at the start; every shot solved, the position error rms about
  0.015 mm, worst about 0.04 mm (under 0.1 mm), mean within ±0.003 mm; the rms residual
  mean about 16 ns, worst under 100 ns; "position check PASS" and PASS. The shots appear
  in the app. `stat`: the solve takes well under 1 ms (measured: 278 µs mean, 453 µs
  longest).
- **3c-2, the height.** `mic h 0`, `sim random 100`: the check is for information ("mic
  heights are not the simulator's"); the error about 0.33 mm mean, 0.43 rms, up to
  about 2 mm near the corners (the study's "mic height not modelled"). `mic h 10.5`, and
  it is 3c-1 again.
- **3c-3, the air.** `sim temp` 1 °C above the firmware's air (`temp`; `sim temp 19`
  without a sensor), `sim random 100`: the error about 0.16 mm rms, up to about 0.27 mm,
  growing with the distance from the centre. `sim temp air`.
- **3c-4, a late channel.** `sim fault late 7`, `sim at 30 30 20`: the rms residual over
  4 µs, "not sent: rms over the 1500 ns limit", no shot in the app. `sim fault none`.
- **3c-5, replay.** Log 3c-1 with `pio device monitor -d firmware --echo --filter log2file`
  (a `platformio-device-monitor-<date>.log` in the folder it was run from). `tools/replay <log>`: 200 events, the same
  positions as the board ("worst 0.0000 mm" against the board's shot lines; the board
  prints 0.001 mm steps). `sim fault none` first, or 3c-4's late channel is still on. Then `tools/replay --h 0 <log>`: about 0.4 mm rms off, as 3c-2.
- **3c-6, the test shot.** BOOT short press with `testshot on`: a test shot in the app as
  before; `testshot off`: "BOOT: test shots are off", none in the app. `testshot on`.

Afterwards `mic default` (the D1/D2 layout for the real sensors).

#### 3d. The simulator on six channels, and drop-one

Build (done, on `step3-solver`):
- **Six simulated channels on CH3-CH8** (GPIO 33/32, 26/25, 22/23, 19/21, 17/18, 4/16;
  CH1/CH2 are input-only). The pulse generator's 6 operators: CH3-CH5 on MCPWM unit 0,
  CH6-CH8 on unit 1. The simulator's mics default to the D1/D2 layout in order: CH3 bottom
  left, CH4 bottom right, CH5 top right (slid), CH6 top left, CH7 bottom middle, CH8 right
  middle (`sim` lists them, `sim mic` sets one, `mic sim` copies them to the firmware).
  `sim raw` takes 6 times.
- **Trip levels per channel:** `sim thr <ch> <L> <H>` (default 1 and 4; the true k is
  L / (H − L)). The position check counts only when the firmware's k matches it.
- **Faults:** `sim fault blast <m> <mm>` (a point source that many metres in front and
  that far off the axis, in a random direction each shot; every channel trips alike),
  `sim fault early <ch>` (a channel 24 µs early), with the old `late`, `missing`,
  `honly`, `double`.
- **Drop-one** in the solve (moved forward from step 4): if the full fit converged
  within `reslimit`, that's the shot; otherwise, with 5 or more channels, the best fit with
  one left out; the shot is sent if that one is within the limit, and the `shot` line marks
  the dropped channel. `dropone on|off` (stored, default on). `measure::solveEvent` does
  it, on the board and in `tools/replay` alike.
- **The policy is in the `cfg` line** (`lim=`, `drop=`), with `reslimit` and `dropone`
  stored in NVS `"cal"` (`reslimit` moves there from `"measure"`, once). The replay takes
  them from the log; `--reslimit` and `--dropone on|off` change them.
- **The simulator's checks:** with a late or early channel the position check counts
  (with drop-one on): every shot sent, that channel dropped in every one, the worst error
  within 0.15 mm (5 mics left). The same limit with a channel missing or H only (5 mics in
  the fit). With a blast: none may be sent (when the firmware has the simulator's mics).
  `stat` counts the shots with a channel dropped; its time is per event now (drop-one runs
  up to 7 fits).

Expected values from a simulation on the PC (the D1/D2 layout, h 10.5 mm, the 25 ns steps
and the 100 ns sampling; 300-3000 shots).

Bench test, bare NodeMCU, `mask 0`, nothing on CH3-CH8 or GPIO 12:
- **3d-0, the new settings.** The boot's `cfg` line ends in `lim=1500 drop=1`. `stat`:
  "drop-one on". `sim`: the six mics on CH3-CH8, each "trips at 1/4 (k 0.3333)".
- **3d-1, the generator again.** `sim raw 0 0 0 0 0 0 200`: PASS, every line within a
  sample of CH3 L, t0 differences 0. A mean on CH6-CH8 alone would be an offset between
  the units (fine up to a tick, 25 ns).
- **3d-2, six mics.** `mic sim`: the blast check scores 0 with every mic and with any one
  dropped (the smallest residuals as `mic default`'s: about 34, 35, 13, 37, 37, 22, 35 µs).
  `sim random 500`: PASS; position error rms about 0.013 mm (3c-1's square: 0.014), worst
  about 0.06 mm; rms residual mean about 26 ns, worst about 60 ns; no channel dropped.
- **3d-3, a bad channel.** `sim fault late 5` (the slid top-right mic), `sim random 100`:
  every shot sent with CH5 dropped; error rms about 0.017 mm, worst up to about 0.1 mm
  (limit 0.15); PASS. `sim fault early 8` (right middle): the same with CH8, rms about
  0.015 mm, worst about 0.05 mm. `stat`: the longest time per event now a few ms (the
  drop-one fits).
- **3d-4, blasts.** `sim fault blast 10 0`, `sim at 0 0 20`; then `sim fault blast 10 500`
  and `sim fault blast 25 2000`, 20 each: "blast check PASS", 0 of 20 sent, the shot lines
  "not sent ... even with one channel dropped", rms residuals about 20 µs. Then the
  square: `sim mic 5 105 105`, `mic 5 105 105`, `mic clear 7`, `mic clear 8` (the blast
  check warns: 484 of 624), `sim fault blast 10 0`, `sim at 0 0 20`: "for information
  only" (the firmware's mics differ: CH7/CH8), and 20 of 20 sent, at about (0, 0): the
  reason for D1. Back: `sim mic 5 65 105`, `mic sim`.
- **3d-5, k per channel.** `sim fault none`, `sim thr 4 1 3` (k 0.5 on CH4),
  `sim random 100`: "for information only" (k differs); error rms about 0.017 mm, rms
  residual mean about 40 ns (up from 26). `k 4 0.5`, `sim random 100`: the check counts
  again, PASS, as 3d-2. Back: `sim thr 4 1 4`, `k 4 0.33333`.

Afterwards `sim fault none` and `mic default`.

#### 3e. Calibration mode and the fit

Build (done, on `step3-solver`):
- **Calibration mode:** `cal on|off` (not stored), `cal pt <x> <y>` (the template point the
  next sparks are at; `cal pt -` clears it). In calibration mode every event's `rec` line
  carries `mode=cal` and its point (`pt=x,y`), and nothing is sent to the app. During a
  `sim` run the `rec` line carries the simulator's true point anyway.
- **`tools/calibrate.py <log>`** (Python, numpy and scipy): reads the `cfg` and the `rec`
  lines with a `pt`, and fits per channel the mic x and y, k and the offset over all
  sparks, h fixed (PLAN D3), each spark's emission time free. A robust first fit (soft
  L1), sparks beyond 5 MADs left out, then the final fit. The offsets have mean 0 (a delay
  shared by every channel is the emission time). sigma per channel = its rms residual. It
  prints the fit's residual, each value with its uncertainty and its change, the position
  error per point before and after (through the solver), and the lines to send: one
  `cal ch <ch> <x> <y> <h> <k> <offset> <sigma>` per channel, then `cal id <id> <date>
  <points> <sparks> <residual>`. `--selftest` simulates a calibration as the simulator
  makes one and checks the fit against the truth.
- **The fit models the edge noise:** both t1 and t2 carry the sampling's noise, so
  t0 = (1 + k) t1 − k t2 carries √((1 + k)² + k²) times one edge's. Each channel's residual
  is divided by that (Deming regression, the maximum-likelihood fit). Without it the noise
  in t2 − t1 pulled every k low, by 0.02-0.05 in the self-test, and the offsets with it,
  though the positions still came out right (k and the offset trade off).
- **The calibration record** (PLAN D5) in NVS `"cal"`: the id of the values it fitted
  (`measure::calibrationId`: the CRC-32 of the `cfg` line's channel fields, without the
  policy), the date (from the PC; the target has no clock), points, sparks and residual.
  `cal id` stores it only if the board's values have that id. `cal` and `stat` show it.
  Any later change to a mic, h, k, offset or sigma marks it **stale**, until the next
  `cal id`, even if the value is changed back (a record is only trusted as written).
  Every `rec` line carries `cal=<id> stale=0|1`, or `cal=-`. `cal clear` removes it.
- **`sigma [ch] [ns]`:** a channel's timing error (the solver's weight), stored.
- **The simulator:** `sim cal truth` sets its world off from the nominal values, as a frame
  is before its calibration: mics up to 1.5 mm off, own delays up to ±500 ns, k 0.25-0.6
  per channel (trip levels 1 and 1 + 1/k); `sim cal reset` goes back. `sim cal <n>` (in
  calibration mode): n sparks at each point of a 4 × 4 grid (±25 and ±75 mm), each at a
  random loudness (×0.5-2: that tells k from the distance).
- **`tools/replay --fit-c`:** one factor on every event's speed of sound, fitted for the
  least total residual, and what air temperature that means.
- The blast check runs on `mic`, `mic default` and `mic sim` only (it takes about a
  second with 6 mics), so a pasted block of commands isn't lost; the serial input buffer is
  1024 bytes and a line 127 characters.

Expected values from `calibrate.py --selftest` (6 seeds, 10 sparks per point): residual
about 38 ns; mics within 0.03 mm of the truth; k within 0.03 and offsets within 90 ns, each
within about 2.5 of its reported uncertainty; the position error over the 16 points
0.51 mm rms before, 0.013 mm after (as with the exact values, 3d-2). With 40 sparks per
point the mics come within 0.01 mm and k within 0.01. On a fake log 2 °C off, `--fit-c`
found +1.96 °C.

Bench test, bare NodeMCU, as 3d (`mic sim`, `sim fault none`, `k 0.33333`):
- **3e-1, uncalibrated.** `sim cal truth` (`sim` lists the truth), `sim random 100`: "for
  information only" (the mics differ); error about 0.5-1.5 mm rms (it depends on the
  truth drawn; 1.3 mm on the bench).
- **3e-2, calibrate.** Logged to a file: `cal on`, `sim cal 10` (160 sparks, about 25 s),
  `cal off`. `python3 firmware/tools/calibrate.py <log>` (it takes the newest calibration
  run in the log, calibration mode's sparks only): residual about 40 ns, the
  position error per point from about 0.5 mm down to about 0.013 mm rms. Compare its
  values with `sim` (the truth): mics within about 0.03 mm, k within about 0.03, offsets
  within about 90 ns of the truth minus its mean. Paste the 7 printed lines into the
  console: `cal` says the record is in use.
- **3e-3, verify.** `sim random 200`: still "for information only" (the fitted values
  are close to the truth, not equal), error about 0.013-0.015 mm rms, worst under 0.1 mm,
  as 3d-2. `mic h 4 21.5`: "the calibration record is stale now", `cal` says STALE and the
  `rec` lines `stale=1`; `mic h 4 10.5`: still stale, until the 7 lines are sent again.
- **3e-4, temperature on the PC.** `sim temp` 2 °C above the firmware's air (`sim temp 20`
  without a sensor), `sim random 200`, logged. `tools/replay --fit-c <log>`: "+2.0 °C",
  to about 0.1 °C. `sim temp air`.

Afterwards `sim cal reset`, `mic default`, `cal clear`, `offset clear`, `sigma 50` (the
fitted offsets and sigmas stay in NVS otherwise).

The exact commands of 3d and 3e may still change as they are built; this file is updated
with them.

### 4. Validity checks

Every event gets a verdict: `ok` (a shot, sent to the app) or the first check it failed.
In three parts, because only the first can be tested before the boards and the curtain
are there:

- **4a, the verdict** (passes, simulated): the checks that need only the edges and the fit.
- **4b, the curtain** (simulated tests pass): IR_IN (GPIO 13) timed against the
  shot's emission time, the gate (a dip 45–100 µs later, PLAN D9), and a dip with no
  acoustic event (a shot through an existing hole: printed as a `dip` line; sent without
  a position from protocol v3). Simulated with a dip on GPIO 13; the real test waits for
  the curtain's bench test (PLAN phase 1.6).
- **4c, loudness and sharpness:** the nearest mic must be the loudest (shortest t2 − t1)
  and the edges sharp. Tuned from real pulses (PLAN phase 1–2); nothing to simulate yet.
- **4d, a moderately off mic** (a study, 2026-11-27; nothing in the firmware yet): Huber
  weights against drop-one started from the mics' sigmas. See 4d below.
- **Then, on the range** (PLAN phase 2): no false shots from blasts or electrical
  spikes, and real shots accepted. Every event is logged with its verdict, so the limits
  are tuned by replaying range logs (`tools/replay --lost/--area/--together/--pulse/--curtain`).

The dead time stays in the capture (a second event inside it is `MISSED`, step 2).

#### 4a. The verdict

Build (on `step4-validity`):
- **In the core** (`lib/measure/record.h`, MODEL.md "Validity"): `measure::solveEvent`
  gives `ok` or the first failed check, in this order: `few` (under 4 channels to fit),
  `together` (every first edge within 20 µs: a spike, or a blast head-on; a real shot
  spreads over 126 µs or more), `nofit`, `residual` (over `reslimit`), `lost` (more than 1
  of the event's mic channels left out: no edge, pulse shape, dropped), `outside` (beyond
  ±105 mm). Before the fit, a channel whose t2 − t1 is not within 0–10 µs is left out
  (pulse shape).
- **The limits** go in the `cfg` line (`lost= area= tog= pmax=`), so a replay judges as the
  board did; a `cfg` line without them (before 4a) means off. They are not part of the
  calibration id.
- **`valid`** lists them; `valid lost <n>|off`, `valid area <mm>|off`, `valid together
  <us>|off`, `valid pulse <us>|off`, `valid default`. Stored in NVS `"cal"`.
- **The `shot` line** ends with the verdict: `; sent as shot 12 [ok]`, `; not sent
  [lost]: 2 channels lost (CH5 CH7), 1 allowed`. A channel left out for its pulse says so.
  `stat` counts the verdicts since boot.
- **The blast check** (`mic`) counts a blast as scored inside the validity area (105 mm)
  instead of the card (100 mm). D2: still 0 with every mic and with any one dropped.
- **The simulator** expects a verdict per run and counts the ones it got (`sim: verdicts:
  ...; verdict check PASS`). New faults: `sim fault spike` (every line at once), `sim
  fault slow <ch> [ch]` (H 40 µs after L), and a second channel for `missing` and `honly`.
  Expected: one faulty channel `ok` (dropped or left out); two `lost`; a spike `together`;
  a blast anything but `ok`; `sim at` beyond ±105 mm `outside`.
- **The replay** gives the verdicts, compares them with the board's (`against the board's
  verdicts: N compared, 0 differ`), and takes `--lost`, `--area`, `--together`,
  `--pulse` to try other limits on a recorded log.
- **Found on the PC:** a t0 5 µs early on one channel leaves an rms residual of 1291 ns,
  under the 1500 ns limit, with the shot 0.47 mm off. The simulator's residuals stay under
  100 ns, so `reslimit` can probably come down a lot once real pulses show their spread
  (phase 1.2). Not changed yet.

On the 3d log (1300 events), the replay with the 4a limits turns the 200 `sim raw` runs
(every line at 0 ns, a perfect centre fit on four mics) and the head-on blasts into
`together`; the rest keeps its verdicts.

##### 4a bench test

**What it shows:** every simulated event gets the verdict it should, an `ok` shot is
sent to the app and nothing else is, and the replay on the PC gives the board's verdicts
from the log alone.

**Setup:** the bare NodeMCU on USB, nothing connected to CH3–CH8 (the simulator drives
those pins itself) or GPIO 12. The app may stay connected; `ok` shots then show up on its
card. From the repository root:

```
pio run -d firmware -t upload
pio device monitor -d firmware --echo --filter log2file
```

Keep this one monitor session (and its log file) open for 4a-0 to 4a-6: 4a-7 replays it.
The log is a `device-monitor-<date>-<time>.log` file in the folder the monitor was started
from (your earlier ones are in `firmware/logs/`). Every `sim` run prints the `cfg` line first, so the log carries its own settings.

How to read a run. Each simulated shot prints:
- the event's `evt` and t0 lines (as in step 2);
- the board's `shot` line, ending with the verdict in brackets: `; sent as shot 7 [ok]`,
  or `; not sent [lost]: ...` with the reason; then its `rec` line (for the replay);
- the simulator's line `sim (30.0, 30.0) mm, t0 error ns vs sent/vs true: ...; [ok];
  position error 0.009 mm`; a `(!)` there marks a shot that broke the check.

Then the run's summary, with these lines (the numbers per test are below):

```
sim: verdicts: ok 20 (expected: every shot ok); verdict check PASS
sim: position vs the truth: 20 of 20 shots solved and converged, error rms ..., worst ...
sim: 20 of 20 sent to the app; drop-one: ...
sim: position check PASS (every shot solved, every one sent, none dropped, worst error within 0.10 mm)
sim: PASS
```

If a run starts with `verdict check for information only` or `position check for
information only: the firmware's offsets are not the simulator's`, the settings are not
clean: the final `sim: PASS` then covers the capture only. Do 4a-0 again, then the run.

The last line is the result of the test. The expected values come from a simulation of
the simulator on the PC (the same core, 25 ns ticks, 100 ns sampling at a random phase).
The board is a little noisier: the error up to about 1.5 times the values given.

**4a-0, the settings.**

Start from clean settings. The offsets and sigmas a calibration (3e) stored stay in NVS
until cleared, and so do the `valid` limits of an earlier run (4a-4 leaves `together off`
if it is not finished).

```
valid default
offset clear
sigma 50
mic default
mic sim
k 0.33333
sim fault none
stat
```

- After each of them the `cfg` line ends in
  `lim=1500 drop=1 lost=1 area=105 tog=20000 pmax=10000`.
- `valid default`: `valid: lost 1, area +-105 mm, together 20.0 us, pulse 10.0 us;
  reslimit 1500 ns, drop-one on`.
- `offset clear`, `sigma 50`: every channel `0`, every channel `50`. In the `cfg` line each
  mic reads `...,0.333330005,0,50` after `k`.
- `mic default` and `mic sim` each run the blast check. The first line reads `scored = a
  fit inside +-105 mm within the 1500 ns limit` (it said ±100 mm before 4a). Then seven
  lines, `every mic` and `without CH..` for each mic, all `0 of 624 scored`. The smallest
  residuals are about 34.3, 34.7, 13.5, 37.5, 37.3, 22.1 and 35.4 µs, the same as in 3d.
  The last line: `mic: no blast scores, with every mic or with any one dropped`.
  `mic sim` gives the same numbers, for CH3–CH8.
- `stat` ends with `verdicts since boot: ok 0 few 0 together 0 nofit 0 residual 0 lost 0
  outside 0` (plus any events since boot, e.g. BOOT test shots).

**4a-1, clean shots.**

```
sim random 200
```

- At the start: `sim: verdict check on: every shot ok` and `sim: position check on: the
  firmware has the simulator's mics and air (limit 0.10 mm)`.
- Every `shot` line ends `; sent as shot N [ok]`. 200 shots appear in the app.
- Summary: `verdicts: ok 200 ... verdict check PASS`; the position error about 0.013 mm
  rms, worst about 0.06 mm (as 3d-2); `200 of 200 sent`; `a channel dropped in 0`;
  `position check PASS`; `sim: PASS`.

**4a-2, one bad channel: still a shot.** Four runs of 20 shots at (30, 30). CH5 is the
slid top-right mic.

```
sim fault late 5
sim at 30 30 20
sim fault missing 5
sim at 30 30 20
sim fault slow 5
sim at 30 30 20
valid pulse off
sim at 30 30 20
valid pulse 10
sim fault none
```

Every run: `verdict check on: every shot ok`, the position check's limit 0.15 mm, every
`shot` line `[ok]` and sent, `verdicts: ok 20`, `20 of 20 sent to the app`, the error
about 0.009 mm rms, worst about 0.015 mm, `sim: PASS`. What differs:

| Run | In each `shot` line | `drop-one:` in the summary |
|---|---|---|
| late 5 | `CH5 +2xxxx (dropped)`, about +24 000 ns | `CH5 dropped in 20, another channel in 0, none in 0` |
| missing 5 | no CH5 in the residuals; the sim line says `CH5 - (as sent)` | `a channel dropped in 0` |
| slow 5 | no CH5 in the residuals, and `; pulse shape out of range on CH5 (t2 - t1 not within 0-10000 ns: left out)` | `a channel dropped in 0` |
| slow 5, `valid pulse off` | CH5 back in, about −13 000 ns `(dropped)`: its t0 is 13 µs early, and drop-one catches it instead | `CH5 dropped in 20, another channel in 0, none in 0` |

- With the slow pulse, the summary also says `CH5 t0 vs true arrival: mean -13xxx ns ...
  (the slow pulse: its t0 is off, it must be left out)`. That is the model's error on
  that channel, not a capture error, and it doesn't fail the test.
- `valid pulse off` and `valid pulse 10` each print the `valid` line (`pulse off`, then
  `pulse 10.0 us`) and a new `cfg` line (`pmax=0`, then `pmax=10000`).

**4a-3, two bad channels: not a shot.**

```
sim fault missing 5 7
sim at 30 30 20
valid lost 2
sim at 30 30 20
valid lost 1
sim fault none
```

- `sim fault missing 5 7` prints `fault missing CH5 and CH7`.
- First run: `verdict check on: every shot lost`.
  - Every `shot` line, with residuals for CH3, CH4, CH6 and CH8 only, ends `; not sent
    [lost]: 2 channels lost (CH5 CH7), 1 allowed`. Nothing appears in the app.
  - Summary: `verdicts: lost 20 (expected: every shot lost); verdict check PASS`, `0 of 20
    sent`, `position check PASS (every shot solved, none sent, none dropped, the error for
    information only)`, `sim: PASS`.
  - The error is shown anyway: about 0.013 mm rms, worst about 0.025 mm.
- After `valid lost 2` (the `cfg` line now has `lost=2`): `every shot ok`. The same four
  mics are now accepted, `[ok]`, `20 of 20 sent`, the error checked (limit 0.15 mm),
  `sim: PASS`.

**4a-4, an electrical spike: every line at once.**

```
sim fault spike
sim at 0 0 20
valid together off
sim at 0 0 20
valid together 20
sim fault none
```

- At the start: `sim: spike check on: none may be sent to the app` and `verdict check on:
  every shot together`.
- Each `shot` line:
  - is a nonsense fit (about (−26, +10) mm in the PC simulation, rms about 21 600 ns, one
    channel `(dropped)`);
  - ends `; not sent [together]: every first edge within 0.0 us (limit 20): an electrical
    spike, or a blast from straight ahead`. It may read 0.1 us when the two MCPWM units
    are a tick apart.
- The sim lines start `sim spike, t0 error ns vs sent/vs true:` and end `[together]; not
  sent`.
- Summary: `verdicts: together 20 ... PASS`, `spikes: 0 of 20 sent to the app`, `spike
  check PASS (none may score)`, `sim: PASS`.
- With `valid together off` (`tog=0`): `verdict check on: every shot anything but ok`.
  Every shot is `[residual]` (`rms over the 1500 ns limit even with one channel
  dropped`), 0 of 20 sent, PASS. The fit's residual alone rejects it here, but the
  `together` reason says what it was.

**4a-5, the area.**

```
sim at 115 0 20
sim at 100 0 20
```

- (115, 0) lies beyond the right-middle mic. At the start: `verdict check on: every shot
  outside`.
  - Each shot: x about +115.0, `; not sent [outside]: outside the +-105 mm area`.
  - Summary: `verdicts: outside 20`, `0 of 20 sent`, `position check PASS (every shot
    solved, none sent, none dropped, the error for information only)`, `sim: PASS`.
  - The error is about 0.035 mm rms, worst about 0.06 mm. It grows out there, which is
    why it isn't checked.
- (100, 0): `every shot ok`, `[ok]`, `20 of 20 sent`, the error about 0.012 mm rms,
  `sim: PASS`.

**4a-6, blasts.**

```
sim fault blast 10 0
sim at 0 0 20
sim fault blast 10 500
sim at 0 0 20
sim fault blast 25 2000
sim at 0 0 20
sim fault none
stat
```

- Every run: `blast check on: none may be sent to the app` and `verdict check on: every
  shot anything but ok`. The rms is about 21 000–23 000 ns with one channel dropped.
  Summary: `blasts ...: 0 of 20 sent to the app`, `blast check PASS (none may score)`,
  `verdict check PASS`, `sim: PASS`.
- `blast 10 0` (straight ahead): every shot `[together]`, `every first edge within 1.6
  us` or so.
- `blast 10 500` and `blast 25 2000`: every shot `[residual]`. The spread is 30–50 µs,
  over the together limit, and the residual rejects them.
- `stat`, if 4a-1 to 4a-6 each ran once since boot: `verdicts since boot: ok 320 few 0
  together 40 nofit 0 residual 60 lost 20 outside 20`.

**4a-7, replay on the PC.** Stop the monitor (Ctrl-C) so the log is complete. Build the
replay and run it on the log, from `firmware/`. If the monitor was restarted between
tests, give every log of 4a-0 to 4a-6, in order: `<log>` stands for that list (for
example `logs/device-monitor-261122-1847*.log logs/device-monitor-261122-1851*.log ...`).
Leave out logs of runs that were done again.

```
g++ -std=gnu++17 -O2 -mfma -ffp-contract=fast -I lib/measure -o tools/replay tools/replay.cpp lib/measure/solver.cpp lib/measure/record.cpp
tools/replay --quiet <log>
tools/replay --quiet --together off <log>
tools/replay --quiet --lost 2 <log>
```

- **As logged:** `replay: 460 events, ...`. Then:
  - `verdicts: ok 320 few 0 together 40 nofit 0 residual 60 lost 20 outside 20`, the same
    as `stat`;
  - `against the board's shot lines (as printed, 0.001 mm steps): 460 compared, ...
    worst 0.0010 mm` (or 0.0000);
  - `against the board's verdicts: 460 compared, 0 differ`.

  Any difference is a fault: the replay and the board don't judge alike. The counts are
  for 4a-1 to 4a-6 run once each; repeated runs add to them.
- **`--together off`:** `together 0`, `residual 100`: the 20 spikes and the 20 head-on
  blasts fall to the residual check instead. `ok` stays 320. The verdict line says `40
  differ (settings changed for the replay: differences expected)`.
- **`--lost 2`:** `ok 340`, `lost 0`: 4a-3's first run would have been sent on four
  mics. `20 differ`.

**4a-8, stored settings.** Start the monitor again (no log needed).

```
valid area 110
```

- It prints `valid: ... area +-110 mm ...`, then `valid: `mic` runs the blast check for
  the new area`, and a `cfg` line with `area=110`.
- Press EN (reset). The boot's `cfg` line has `area=110`, and `valid` says `area +-110
  mm`.

```
valid default
valid foo
mic default
```

- `valid default` gives the 4a-0 line again (`area +-105 mm`), with a `cfg` line `lost=1
  area=105 tog=20000 pmax=10000`.
- `valid foo` prints the usage: `valid [default] | lost <0..4>|off | area <10..1000
  mm>|off | together <us>|off | pulse <us>|off`.
- `mic default` sets the D1/D2 layout back for the real sensors.

**Report back:** the `sim: PASS`/`FAIL` line of each run, the error numbers of 4a-1, the
`stat` line of 4a-6, and the replay's three summaries. For any FAIL, send the log too.

#### 4b. The curtain

Build (done, on `step4b-curtain`):
- **The signal:** IR_IN (GPIO 13) is the curtain comparator, U4 (LM393) on the host board.
  It is low while the beam is clear and high while something crosses the sheet. The netlist
  shows why: the TIA's output is AC-coupled into a ×21 stage at VMID, and RV1 puts the
  threshold above VMID, so only a fall in light trips it. The project summary says the
  same ("high on a shadow"). A pellet's shadow lasts 30–40 µs.
- **The gate** (PLAN D9, MODEL.md "Validity"): a shot needs a dip whose rise comes 45–100
  µs after its emission time (the fit's τ): the sheet 15 mm behind the paper at 150–350
  m/s. It is checked last, after the acoustic checks, and judged apart from them, so each
  defence's rejections are counted on their own. New verdict `curtain`.
- **The timing:** MCPWM group 0's capture unit latches each rise and fall of IR_IN in
  hardware, on two channels, at the 80 MHz APB clock. That is 8 ticks per 10 MHz sample,
  from the same crystal. The pulse generator uses the group's timers, not its capture
  unit. At boot, one edge that both see ties the capture timer to the samples: a 20 µs
  pulse on GPIO 12, latched by capture channel 2 and, for that moment, read by the I2S in
  place of CH1 L. A dip's rise is then known in ns from the event's first edge, however
  late its interrupt runs. (A software catch at the I2S start gives a first tie, but
  register writes to the MCPWM unit can stall for µs: it was 0.1 or 4.1 µs off, by boot.
  The edge replaces it; if the edge isn't seen by both, the boot prints a warning and
  keeps the catch.) The summary suggested a plain GPIO interrupt; on this board one
  can be up to 22 µs late (the capture interrupt), too much for a 55 µs window.
- **An event's dips:** those rising within 1.5 ms of its first edge (the emission is up to
  0.9 ms before it), up to 4, go in the `rec` line (`ir=rise/len,...`; `ir=-`: none). A
  dip no event claims is printed 50 ms later as a `dip` line (no acoustic event: a shot
  through an existing hole). It has no position, so protocol v2 can't send it. At most 10
  are printed a second; `stat` counts the rest.
- **`curtain`** shows the gate, IR_EN, IR_IN's level and the counts. `curtain on` sets the
  gate to 45–100 µs and lights the IR LEDs (IR_EN high); `curtain window <lo> <hi>` sets
  another window; `curtain off`. Stored in NVS `"cal"` (`irlo`, `irhi`); off by default.
  The window goes in the `cfg` line (`ir=45000,100000`, or `ir=-`); a line without it
  (before 4b) means off, so old logs keep their verdicts.
- **The `shot` line** gives the dip before the verdict: `; curtain dip +70.0 us after the
  shot (35.0 us long)`, with `, outside the window` if it is. `[curtain]` says `no curtain
  dip 45-100 us after the shot (the nearest +30.0 us)`, or `(none within 1.5 ms)`. `stat`
  and `curtain` count: acoustic ok with a dip in the window, without; acoustic reject
  with, without; dips with no event. `valid` ends with the curtain's window.
- **The simulator:** `sim dip <us> [len]` drives GPIO 13 itself and makes a dip that long
  after each shot's emission time (default 35 µs long). `sim dip none` drives it low with
  no dips; `sim dip off` releases it. On the host board, take U4 out first: the simulator
  and the LM393 would both drive the pin. `sim fault silent` is the dip with no sound.
  - The CPU makes the dip: its rise, its fall and the pulse generator's start, in time
    order (the dip often comes before the first edge). Interrupts are off from 25 µs
    before each until just after it, so they come on time; the capture interrupt then
    waits at most that long, inside its 200 µs blocks.
  - The simulator reads the cycle counter at each dip edge and at the sync edge, so it
    knows when the dip really came. It checks the firmware's time of the dip against that
    (`the firmware's dip time vs the truth`, limit 1 µs) and expects each shot's verdict
    from the dip as it came.
  - Blasts, spikes, `sim cal` sparks and double shots get no dip (a blast doesn't cross
    the sheet).
- **The replay** takes `--curtain <lo>,<hi>|off` (µs), gives each event's dip, the
  curtain's counts and the number of `dip` lines.
- **PC checks:** 16 native tests pass (`test_curtain`: off, on with no dips, early, late,
  the window's edges, two dips, an acoustic reject with a dip, too few channels, the `rec`
  and `cfg` lines). The 4a logs replay with 0 of 480 verdicts different. With
  `--curtain 45,100` their 320 shots all become `curtain` (they have no dips), as they
  should. The firmware builds (RAM 13.3 %, flash 58.6 %).
- **Not yet:** the curtain's own delay (amplifier and comparator, a few µs: the window
  absorbs it until it is measured), a check on the dip's length, sending dip-only shots
  (protocol v3), and the real curtain (its bench test, PLAN phase 1.6).

##### 4b bench test

**What it shows:** the firmware times a curtain dip against the shot's emission time on
its own clock, to well under a microsecond. Only a dip in the window lets a shot through,
an acoustic reject keeps its verdict, a dip with no sound is reported as a `dip` line, and
the replay gives the board's verdicts from the log.

**Setup:** the bare NodeMCU on USB, nothing on CH3–CH8, GPIO 12 or GPIO 13 (the simulator
drives them). On the host board, U4 (LM393) must be out of its socket. Upload, which also
gives a fresh boot, so the counts start at 0. Start the logging monitor, and keep it open
for 4b-0 to 4b-6:

```
pio run -d firmware -t upload
pio device monitor -d firmware --echo --filter log2file
```

At boot, look for the lines starting `curtain:`. There should be one: `curtain: capture
tied to the samples by an edge on GPIO 12 (the start's catch was -4100 ns off)` (the
number differs by boot: about −4100 or 0). `curtain: WARNING: no tie by the edge on GPIO
12 (...)` means the tie failed: dip times may be several µs off; send the boot lines, and
check nothing is on GPIO 12. `curtain: could not set up the MCPWM capture on GPIO 13
(...)` means the IDF refused it: send the boot lines.
The IDF may also warn that GPIO 13 is used twice; both capture channels read that pin, so
that is expected. Note it if it appears.

How to read a run: as in 4a, plus the curtain.
- The board's `shot` line gives the dip before the verdict: `; curtain dip +70.0 us after
  the shot (35.0 us long); sent as shot 12 [ok]`. The `rec` line ends with
  `ir=-167812/35000`: the dip's rise in ns from the first edge (negative: the dip came
  before the first sound reached a mic), and its length.
- The simulator's line adds `; dip +70.0 us, error +40 ns`: when the dip really came after
  the shot, and the firmware's time of it minus that.
- The summary adds two lines: `sim: curtain: a dip 70 us after each shot, 35 us long (as
  they came: ...)` and `sim: the firmware's dip time vs the truth: 20 timed, mean ...,
  rms ..., worst ... (limit 1000); curtain check PASS`.

**The dip time's error** (bench, 2026-11-22, the edge tie): a constant part (the mean) of
−0.12 to −0.15 µs, the same on every boot (the I2S sees an edge about 1.4 samples after
the capture unit does); rms within 0.01 µs of the mean's size; the worst under 0.25 µs. A
mean further off is still a PASS up to 1 µs, but tell me the number: it is a fixed offset
between the two clocks.

**4b-0, the settings.**

```
valid default
offset clear
sigma 50
mic sim
k 0.33333
sim fault none
sim dip none
curtain on
valid
stat
```

- Until `curtain on`, every `cfg` line ends
  `lim=1500 drop=1 lost=1 area=105 tog=20000 pmax=10000 ir=-`; after it,
  `... pmax=10000 ir=45000,100000`.
- `sim dip none`: `sim: GPIO 13 (IR_IN) is now driven by the simulator, low at rest. ...`,
  then the settings, with `sim: curtain dip off (GPIO 13 driven low)`.
- `curtain on`: `curtain: on, a dip 45-100 us after the shot; IR_EN high; IR_IN (GPIO 13)
  low (clear) now, driven by the simulator`, then `curtain: since boot 0 dips; acoustic ok
  with a dip in the window 0, without 0; acoustic reject with 0, without 0; 0 dips with no
  event; capture 80 MHz, 8 ticks per sample`.
- `valid` ends `; reslimit 1500 ns, drop-one on; curtain 45-100 us`.
- `stat` ends with the same two `curtain:` lines.

**4b-1, no dip: no shot.**

```
sim at 30 30 20
```

- At the start: `sim: verdict check on: every shot curtain`, `sim: curtain on (a dip
  45-100 us after the shot); no dips (GPIO 13 held low)`, `sim: position check on ...
  (limit 0.10 mm)`.
- Every `shot` line ends `; not sent [curtain]: no curtain dip 45-100 us after the shot
  (none within 1.5 ms)`; every `rec` line ends `ir=-`. Nothing in the app.
- Summary: `verdicts: curtain 20 (expected: every shot curtain); verdict check PASS`, `sim:
  curtain on, and no dips: every shot is turned away by it`, the error about 0.009 mm rms
  (the fit is the same as 4a-2's), `0 of 20 sent`, `position check PASS (every shot
  solved, none sent, none dropped, worst error within 0.10 mm)`, `sim: PASS`.

**4b-2, a dip in the window: a shot.**

```
sim dip 70
sim at 30 30 20
sim random 200
```

- `sim dip 70`: the settings say `sim: curtain dip 70 us after each shot, 35 us long (GPIO
  13 driven by the simulator)`.
- At the start: `every shot ok`, and `sim: curtain on (a dip 45-100 us after the shot); a
  dip 70 us after each shot, 35 us long`.
- Every `shot` line: `; curtain dip +70.0 us after the shot (35.0 us long); sent as shot
  N [ok]`. The time reads +69.7 to +70.3 us, the length 35.0 us. At (30, 30) the `rec`
  line ends about `ir=-1678xx/350xx`: the nearest mic (CH8) hears the shot 238 µs after it,
  so the dip at 70 µs comes about 168 µs before the first edge.
- The simulator's line: `; dip +70.0 us, error ...` (the mean error, see above).
- Summary of `sim at`: `verdicts: ok 20 ...; verdict check PASS`; `sim: curtain: a dip 70
  us after each shot, 35 us long (as they came: +0.0x us from that on average, 0.0x us at
  most)`, that is within about 0.1 µs; `the firmware's dip time vs the truth: 20 timed, ...
  (limit 1000); curtain check PASS`; the error about 0.009 mm rms; `20 of 20 sent`; `sim:
  PASS`.
- `sim random 200`: the same, with 200 shots, the error about 0.013 mm rms, worst about
  0.06 mm (as 4a-1). Here the dip comes after the first edge for holes near a mic, before
  it elsewhere: both orders are tested.

**4b-3, a dip outside the window: no shot.**

```
sim dip 30
sim at 30 30 20
sim dip 120
sim at 30 30 20
```

- Both runs: `every shot curtain`. Every `shot` line: `; curtain dip +30.0 us after the
  shot (35.0 us long), outside the window; not sent [curtain]: no curtain dip 45-100 us
  after the shot (the nearest +30.0 us)`. The second run gives +120.0 instead.
- Summaries: `verdicts: curtain 20 ...; verdict check PASS`, `curtain check PASS` (the dip
  is still timed), `0 of 20 sent`, the error about 0.009 mm, `sim: PASS`.

**4b-4, another window.**

```
curtain window 25 35
sim dip 30
sim at 30 30 20
curtain on
```

- `curtain window 25 35`: `curtain: on, a dip 25-35 us after the shot`; the `cfg` line
  ends `ir=25000,35000`.
- The run (the dip at 30 µs): `every shot ok`, every shot `[ok]` and sent, `sim:
  PASS`.
- `curtain on` puts back 45–100 µs (`ir=45000,100000`).

**4b-5, an acoustic reject keeps its verdict.** The dip is in the window, but the
acoustic checks turn the shot away first.

```
sim dip 70
sim fault missing 5 7
sim at 30 30 20
sim fault none
sim at 115 0 20
```

- `missing 5 7`: `every shot lost`. Every `shot` line: `; curtain dip +70.0 us after the
  shot (35.0 us long); not sent [lost]: 2 channels lost (CH5 CH7), 1 allowed`. `verdict
  check PASS`, `curtain check PASS`, `sim: PASS`.
- `(115, 0)`: `every shot outside`, the same dip, `not sent [outside]: outside the +-105
  mm area`, `sim: PASS`. The hole is 10 mm from CH8, which hears it 42 µs after the shot,
  before the dip: the other order.

**4b-6, a dip with no sound: a shot through an existing hole.**

```
sim fault silent
sim at 30 30 20
sim fault none
stat
```

- At the start: `sim: curtain on (45-100 us); a dip only, 35 us long, no sound: every one
  must be printed as a `dip` line, and no event come`.
- 20 times, about 50 ms after each dip: `dip n=1 t=1234.567890123 len=35000` (`n` counts
  from 1 since boot; `len` 34975–35025), then `  a curtain dip with no acoustic event
  within 1.5 ms: a shot through an existing hole? (no position: not sent)`.
- Summary: `sim: 20 shots sent, 0 events, 0 not seen, 0 unexpected, ...`, `sim: dips with
  no sound: 20 of 20 printed as `dip` lines, 0 not; 0 events (none may come); ...`, `sim:
  dip check PASS`, `sim: PASS`.
- `stat`, if each run was done once since the upload: `verdicts since boot: ok 240 few 0
  together 0 nofit 0 residual 0 lost 20 outside 20 curtain 60`, and `curtain: since boot
  340 dips; acoustic ok with a dip in the window 240, without 60; acoustic reject with 40,
  without 0; 20 dips with no event`.

**4b-7, replay on the PC.** Stop the monitor (Ctrl-C). From `firmware/`, with every log of
4b-0 to 4b-6 (if the monitor was restarted) in order:

```
g++ -std=gnu++17 -O2 -mfma -ffp-contract=fast -I lib/measure -o tools/replay tools/replay.cpp lib/measure/solver.cpp lib/measure/record.cpp
tools/replay --quiet <log>
tools/replay --quiet --curtain off <log>
tools/replay --quiet --curtain 45,100 <log>
```

- **As logged:** `replay: 340 events, ...`, `verdicts: ok 240 few 0 together 0 nofit 0
  residual 0 lost 20 outside 20 curtain 60`, `curtain (4b): acoustic ok with a dip in the
  window 240, without 60; acoustic reject with 40, without 0; 0 not judged (gate off, or
  no fit); 340 events logged with the curtain on; 20 `dip` lines (a dip with no event)`,
  `against the board's verdicts: 340 compared, 0 differ`, the shot lines `worst 0.0000
  mm` (or 0.0010), the true points about 0.012 mm rms over 240 good shots.
- **`--curtain off`:** `ok 300`, `curtain 0`, `340 not judged`, `60 differ`.
- **`--curtain 45,100`:** 4b-4's run (its dip at 30 µs, in its own 25–35 µs window) is
  turned away now: `ok 220`, `curtain 80`, `20 differ`.

**4b-8, back to normal.**

```
curtain off
sim dip off
curtain
mic default
```

- `curtain off`: `curtain: off (no gate); IR_EN low; ...`; the `cfg` line ends `ir=-`.
- `sim dip off`: `sim: curtain dip off (GPIO 13 not driven)`.
- `curtain`: GPIO 13 floats now, so it may read high or low.

**If something fails:**
- `sim: could not set up the MCPWM pulse generator`: the curtain's capture timer and the
  pulse generator clash in MCPWM group 0. Send the boot lines and that line.
- `NOT FOUND` on the simulator's line: the firmware saw no dip within 1.5 ms of the event.
  Check `curtain` (on? IR_IN driven by the simulator?).
- A `dip` line during a normal run: a dip no event claimed. Send the log.

**Report back:** each run's `sim: PASS`/`FAIL` line, the `the firmware's dip time vs the
truth` lines of 4b-2 (both runs), the `stat` lines of 4b-6 and the replay's three
summaries. For any FAIL, send the log too.

#### 4d. A moderately off mic: Huber weights or an earlier drop-one (a study)

The app's TODO ("The solver: Huber weights and mic health", 2026-11-26): a mic 0.5–3 µs off
(a drifted threshold, a blocked port, a moved mic) keeps the rms residual under the 1500 ns
limit, so drop-one never starts, yet it moves the hit by tenths of a mm. The TODO proposed
Huber weights in the solver, then one refit without a far-out mic. This step is its first
item: try it offline before anything goes into the firmware. Branch `huber-study`.

Built (tools only; `lib/measure`, MODEL.md and the firmware unchanged):

- `tools/huber.h`: on top of `solve`/`solveDropOne`, unchanged. Huber by reweighting (each
  round mic i's sigma becomes sigma_i / sqrt(w_i), w_i = min(1, c sigma_i / |r_i|)), the
  TODO's two steps (`solveHuber`), and two others to compare: `solveDropSigma`, today's
  drop-one started when the rms is over a trigger times the rms the sigmas give
  (sigma sqrt((m − 3) / m) for m mics; with a guard option), and `solveHuberSearch`, Huber
  with the drop-one search when a mic is far out.
- `tools/huber_study.cpp`: synthetic shots uniform over ±100 mm, the D1/D2 mics (h 5.5 mm),
  Gaussian noise per mic, 0–2 mics with a bias (a random mic and sign per shot); every
  method judged as the firmware judges (unweighted rms within 1500 ns, at most one lost,
  inside the area). `--n --c --drop --trigger --seed --only`. Built in CI.
- `tools/replay --alt dropS|dropSg|huber|huberS[:p]`: every event solved that way too, its
  error against the true points and its drops per channel beside the firmware's (for the
  real logs of 7-3 and 7-4).

The study (5,000 shots per case, seed 1; noise 50 ns unless said; error rms against the
truth, mm; today = the firmware, `drop`):

| Case | today | Huber | the TODO's 2 steps | dropS ×3 | dropS ×5 | Huber + search |
|---|---|---|---|---|---|---|
| 6 mics healthy (50 / 150 / 300 ns) | 0.016 / 0.047 / 0.096 | the same | the same | the same | the same | the same |
| 1 mic 0.5 µs off | 0.067 | 0.050 | 0.046 | 0.048 | 0.064 | 0.049 |
| 1 mic 1 µs off | 0.131 | 0.084 | 0.073 | **0.042** | 0.065 | 0.049 |
| 1 mic 2 µs off | 0.258 | 0.153 | 0.150 | **0.030** | 0.042 | 0.030 |
| 1 mic 3 µs off | 0.385 | 0.222 | 0.221 | **0.028** | 0.028 | 0.028 |
| 1 mic 10 µs off | 0.484 | 0.764 | 0.764 | **0.020** | 0.020 | 0.020 |
| 1 mic 24 µs off | 0.021 | 1.911 | 1.911 | 0.021 | 0.021 | 0.021 |
| 1 mic 3 µs off, noise 150 ns | 0.386 | 0.240 | 0.209 | **0.123** | 0.192 | 0.140 |
| 2 mics 1 µs off | 0.183 | 0.187 | 0.187 | 0.267 | 0.267 | 0.239 |
| 2 mics 10 µs off | 2.620 (34 % sent) | 1.874 | 1.874 | 2.618 (34 %) | 2.618 | 2.367 |
| 5 mics, 1 mic 1 µs off | 0.155 | 0.129 | 0.126 | **0.087** | 0.097 | 0.092 |
| 4 mics, 1 mic 1 µs off | 0.186 | 0.196 | 0.196 | 0.186 | 0.186 | 0.196 |
| 6 healthy, fit sigma 3× too low | 0.047 | 0.050 | 0.057 (25 % wrong drops) | 0.064 (39 %) | 0.050 (3.9 %) | 0.058 (21 %) |
| 6 healthy, fit sigma 3× too high | 0.016 | the same | the same | the same | the same | the same |

What it shows:

- **Huber alone** helps a little at 0.5–3 µs (−30 to −45 % rms) but not the worst shots
  (the 95th percentile at 1 µs: 0.195 mm with the two steps, today 0.196), and it is far worse than today
  against a big fault (1.9 mm at 24 µs, today 0.021): it caps a bad mic's pull, it doesn't
  remove it.
- **The TODO's two steps** fail at big faults: the fit spreads one large error so that two
  mics look far out in about 20 % of shots, and "two far out: drop nothing" then keeps the
  bad mic. Its residual limit over the weighted residuals (as the TODO proposed) also
  passed 99 % of shots with two mics 10 µs off, 1.9 mm wrong, where today's unweighted
  limit rejects two thirds: the weighted limit is unsafe. (The table above judges every
  method by the unweighted limit.)
- **Drop-one started by the sigmas (dropS)** is the change that helps: the same search the
  firmware has, started when the rms is over 3 (or 5) times the rms healthy mics give
  instead of over 1500 ns. With the sigmas right, healthy shots are never touched (0 drops in
  15,000), and from 1 µs at 50 ns noise the bad mic is dropped in 97–100 % of shots, the
  wrong one in 0–1.1 %. Huber + search is within a few hundredths of a mm of it: a little
  better where the bias is under the trigger (1 µs at 150 ns noise: 0.120 against 0.130;
  sigma 3× too high: 0.098 against 0.116), a little worse elsewhere (3 µs at 150 ns: 0.140
  against 0.123), for about ten times the solver's work on a big fault (about 395 LM
  iterations a shot against about 28 for the drop-one search).
- **Its price:** it relies on the sigmas. With the fit's sigma 3× too low, trigger 3 drops a
  healthy mic in 39 % of shots (error 0.047 → 0.064 mm); trigger 5 in 3.9 % (0.050 mm),
  and still fixes 2 µs and more at 50 ns noise. Too high a sigma only makes it start later.
- **Two mics off:** no method fixes it. With a guard (still over the trigger after the
  drop: keep the full fit, mark the shot uncertain) the wrong drops fall from 43 % to 6 %
  and 87–100 % of those shots are marked; but going back to the full fit cost a little on
  big single faults (24 µs: 0.046 against 0.021 mm), so marking without reverting is the
  better form. What an uncertain shot does (sent, held, flagged) is the user's call.
- **4 mics:** nothing helps; one redundant measurement can't tell a bad mic (as the TODO said).
- **Healthy mics' soft zone:** at 2 sigma only 0.5–0.9 % of a healthy mic's shots, not
  the 5 % the TODO assumed: with 6 mics and 3 unknowns the residuals keep half the noise's
  variance (0.71 sigma).
- **Mic health:** with dropS the drops per channel are the health signal (the bad mic
  dropped in 97–100 % of shots, the others in 0–1 %); a mean residual per channel (in
  sigmas) covers drifts too small to drop.

Bench test (run by Claude):

1. **4d-1, PC:** `huber_study --n 5000` (the table above), then `--trigger 3, 4, 5, 8`.
2. **4d-2, the board's path:** the bare NodeMCU, `mic sim`, `sim random 300` three times:
   healthy, then `sim offset 5 1000` (CH5, the moved corner, 1 µs late without the
   firmware knowing), then `sim offset 5 3000`; each run replayed with `--alt dropS:3`,
   `dropS:5`, `huber:2`, `huberS:2`. Expect: healthy the same 4 ways; the firmware's error
   about 0.13 and 0.38 mm (the study), dropS well under.

**For the user to decide** (nothing in the firmware yet): whether drop-one should start by
the sigmas (a trigger of 3–5 times the expected rms, apart from the verdict's 1500 ns
limit); the sigmas per channel from the calibration fit's residuals (they are the scale);
what an uncertain shot does; and the drops per channel as the app's mic-health flag. Before
that, the same replay on the real logs of 7-3 and 7-4: real errors aren't Gaussian, and a
mic position error gives a bias that depends on where the shot lands.

### 5. Protocol and app

Protocol v3 (`protocol/PROTOCOL.md`, PLAN D4, D5, D7, D9): what step 3 and 4 kept on serial
goes to the app too, in messages that will go over MQTT unchanged. Decisions (2026-11-23):
holes and missed shots are hits in the shot stream, shown unscored; events are delivered
reliably (indications, a ring of 32); the calibration record is one long write; the template
point is kept on the target.

#### 5a. The protocol

Built (done, on `step5-protocol`):

- **The shot** grows to 32 bytes: the event it came from, the calibration ID, the 1σ
  uncertainty, a kind (`shot`, `hole`, `missed`) and flags (dropped, air assumed, stale,
  curtain confirmed).
- **New characteristics:** Event (180 bytes: everything the `rec` line has, plus the fit and
  the verdicts), Status (52 bytes), Control (calibration mode, template point), Calibration
  (213 bytes, long read and write), Settings (36 bytes). `PROTOCOL_VERSION` 3.
- Floats travel as the firmware's `f32` bits, so the app writes the firmware's `cfg` and `rec`
  lines exactly. Worked byte examples for every message; both sides' tests check them.

#### 5b. The firmware

Built:

- **`lib/proto/`** (portable): the encoders and decoders, an event from a `rec` and its fit
  and back, the range checks of writes. **`test/test_proto`** (native, 7 tests): the examples,
  the round trip `rec` → event → `rec` for every line shape, the checks.
- **`uplink.cpp`** (PLAN D6: the uplink interface): the shot ring (moved from `main.cpp`,
  64 hits), the event ring (32), delivery (one indication in flight, hits first), Status
  (rebuilt every 0.5 s, notified on a change), the Control commands, and the Calibration and
  Settings writes (checked whole, then `calibSetChannels`, `calibSetPolicy`,
  `captureSetSettings`; rejected writes change nothing).
- Every event (`position.cpp`) goes to the event ring; an `ok` one outside calibration mode
  is a hit too, linked to it. An unclaimed curtain dip (`curtain.cpp`) is a hole; activity in
  the dead time (`capture.cpp`) a missed shot: an event, and a hit outside calibration mode.
- **`ble_link.cpp`:** two indicated streams sharing the link's one indication, the new
  characteristics, the MTU (`bleMtu`). After a firmware update every bonded computer is told
  "services changed" on its next secured connection (kept in NVS `"blesc"` until then).
- Serial `pairing`: pairing mode for 60 s, as BOOT held 2 s.

#### 5c. The app

Built:

- **`protocol.rs`:** v3's messages, tested against PROTOCOL.md's examples.
- **`eventlog.rs`:** every event as the firmware's log line (`cfg` when Status shows a new ID,
  `rec`, `dip` for a hole, `MISSED`), with C's `%.9g` and the CRC-32, so the IDs match. One
  file per day in the app's data folder (`logs/`); a setting, on by default.
- **`ble.rs`:** the new characteristics, events deduplicated and asked for again after a
  reconnect like shots, the calibration and settings read again when Status shows a new cfg
  ID. Right after a firmware update it waits 8 s for the computer to read the new layout,
  then asks the user to pair again.
- **UI:** holes and missed shots as unscored rows in the series and a notice on the card;
  the shot popup shows the 1σ and the flags; a read-only "Measurement" section in the sensor
  settings (calibration, settings, air, event counts, the log).
- **`examples/probe.rs`:** a v3 test client without the UI (the bench test below).

##### 5 bench test

**What it shows:** over real BLE from a PC, every event reaches the app with the board's
data, so the app's log replays like the serial log; hits of every kind arrive and link to
their events; Status, Control, Calibration and Settings work, and wrong writes change
nothing.

**Setup:** the bare NodeMCU on USB, nothing on CH3–CH8, GPIO 12 or GPIO 13. The PC bonded
with the sensor after the v3 firmware was flashed (on Linux: pair again, as the app asks).
The app closed (one connection at a time).

1. Flash, then start the plan on the serial port. It begins with a few `stat` (no events),
   then: `valid default`, `offset clear`, `sigma 50`, `mic sim`, `k 0.33333`, `sim fault
   none`, `sim dip none`, `curtain off`, `deadtime 30`; `sim at 30 30 10`; `sim fault missing
   5 7`, `sim at 30 30 5`; `sim fault none`, `sim at 115 0 5`; `sim dip 70`, `curtain on`, `sim
   at 30 30 5`; `sim fault silent`, `sim at 30 30 5`; `sim fault none`, `curtain off`, `sim dip
   off`; `deadtime 1`, `sim fault double 2500`, `sim at 0 0 10`; `sim fault none`, `deadtime
   30`, `stat`; `mic default`, `cal clear`, `cal`.
2. Right after the boot, from `app/`: `cargo run --example probe -- 200 <log> writes`.

**Expected:** every `sim` run PASS. The probe: `cfg id from the reads ... MATCH`; the writes
as labelled (serial: `settings from the app`, `settings write rejected`, `a calibration record
from the app`, calibration mode on and off, the point, `control command ignored`); a summary
with 0 event gaps and every hit linked to a received event. Then the app log's `rec` and `cfg`
lines identical to the serial log's, and `tools/replay <app log>` giving the board's verdicts.

#### 5d. The calibration wizard

Decisions (2026-11-23): the template is the simulator's 4 × 4 grid at ±25 and ±75 mm, 10
sparks per point, with a check point at the centre; the fit uses `nalgebra`; the field check
comes now.

Built (done, on `step5d-calibration`):

- **`app/calib/`** (a library crate, pure Rust, part of the app's workspace: `cargo test` runs
  it): the target's solver (`solver.rs`, a line-for-line port in f32 of `lib/measure`), the
  fit (`fit.rs`, `calibrate.py` ported), the log-line text (`%.9g`, CRC-32, the IDs; the
  app's event log uses it), and a `cfg`/`rec` reader.
  - The fit: per channel x, y, k and the offset, h fixed, each spark's emission time solved
    exactly for the current values (variable projection: the same minimum as `calibrate.py`'s
    joint fit, over 4n − 1 values); a robust first fit (soft L1, 50 ns, by reweighting), the
    sparks beyond 5 MADs out, the final fit Deming-scaled with k within 0–2
    (Levenberg-Marquardt); uncertainties from the covariance; the error per point before and
    after through the solver; the record rounded as `calibrate.py` prints it, and its ID.
  - `calibrate.py`: `--json` (its results) and `--write-log` (the self-test's log), and tighter
    tolerances in its final fit (see the results).
- **`app/src/calwizard.rs`:** the wizard and the field check as state machines without UI.
  They take the events, Status and Calibration, and give the actions to send (calibration
  mode, the point, the record). A spark counts only in calibration mode, at the current point,
  with the cfg the wizard started with, and with both edges on every mic channel; a changed
  cfg stops the run; cancel sends calibration mode off. After the write, the run is done when
  Status reports the record's ID. The field check: N sparks at the check point, the sensor's
  solved positions against it, mean within the limit.
- **`app/assets/templates/grid-4x4.json`:** the template (points, sparks per point, check
  point, check sparks 5, limit 0.5 mm).
- **The app:** "Calibrate…" and "Field check…" in the sensor settings' Measurement section
  open a window: the template drawn with the current point and progress, Next / Redo /
  Cancel, then the fit (residual, error before and after, per channel with ±, per point,
  warnings), "Write to the sensor", the confirmation; the report saved next to the event logs
  (`calibration-<date>-<id>.txt`). Closing the window cancels. New BLE commands: Control and
  the Calibration write.
- **`examples/probe.rs` `drive`:** runs the same wizard and check from commands on stdin, in
  test mode (each spark at the point it carries: the simulator's own grid).

##### 5d bench test

**On the PC** (`cargo test` in `app/`): the solver against the firmware's vectors (as its
native test: positions within 0.001 mm, residuals and tau within 0.5 ns); the fit against
`calibrate.py` on the self-test's log (`app/calib/tests/data`): every value within 0.1 of its
uncertainty, the Rust fit's objective not higher than `calibrate.py`'s, the same error after
(0.0129 mm rms); against the truth as the self-test (mics within 0.05 mm, k and offsets within
3.5 sd); the wizard point by point, with stray and wrong-cfg sparks, redo, cancel, a changed
cfg; the field check.

**On the board**, over BLE with the probe (`drive`) and the simulator on serial, coordinated
by a script: `valid default`, `offset clear`, `sigma 50`, `mic sim`, `k 0.33333`, `sim fault
none`, `curtain off`, `cal clear`, `deadtime 30`, `sim cal truth`. Then: probe `check`, `sim at
0 0 5`; probe `calibrate`, `sim cal 10`; probe `check`, `sim at 0 0 5`; `sim random 200`
(calibration mode off). Afterwards `sim cal reset`, `mic default`, `cal clear`, `offset
clear`, `sigma 50`, `k 0.33333`.

**Expected:** the first check a few tenths of a mm off (the simulator's truth isn't the
nominal layout), `CALIBRATED <id>` (the sensor holds the record: serial `cal: record <id> ...
in use`), the second check about 0.01 mm, `sim random 200` about 0.013–0.015 mm rms (as 3e-3).

#### 5e. The shot store in flash

PLAN D6 ("no shot is ever lost, and that includes a reboot"), phase 3's last item. Branch
`step5e-shotstore`.

Built:

- **`lib/store/`** (portable, tested on the PC in `test/test_store`): a ring of 64-byte
  records in 4 KB sectors, each hit with a store number H (from 1, never reused, also
  across reboots) in slot H mod (slots); magic, H, boot, seq, the Shot message as sent, a
  CRC-32. A record is written once; a sector is erased whole before reuse. A record torn
  by a power cut fails its CRC and is skipped; H moves on past a slot that isn't blank.
  `begin` rebuilds the boots from the slots (two passes, no big buffers). One sector ahead
  is kept erased (`eraseAhead`); the newest 1,921 hits at least are held in 128 KB.
- **`partitions.csv`:** Arduino's default table with a 128 KB `shots` partition (data,
  subtype 0x40) cut from the unused `spiffs` area; nvs, otadata and both app slots at the
  same offsets, so settings, calibration and bonds survive the new table.
- **`uplink.cpp`:** every hit goes into the store (reads through the partition's memory
  map, writes and erases through the partition API). A shot request (boot, next) starts at
  that boot's hit with seq ≥ next and goes on through the later boots; a boot the store has
  no hits of (0: the app has none): every hit not cleared. A clear request clears up to
  (boot, seq), earlier boots too; the cleared H is kept in NVS (`"shots"`, `clr`). The
  sector ahead is erased after 2 s without a shot or an event, and no hit waiting. Events
  stay in the 32-event RAM ring. With no `shots` partition (an old table): a 16 KB ring in
  RAM, and a warning at boot.
- Serial: `shots` (held, H range, cleared, the boot's hits, capacity, torn at boot, the
  slowest write and erase), `shots clear`.
- PROTOCOL.md: the shot request and clear request across boots (no message changes, no
  version bump: an old app works as before, and gets more).
- The probe: `PROBE_FROM=<boot hex>:<seq>` asks from there, `PROBE_CLEAR=1` clears up to the
  last hit received, and a summary per boot (seq range, gaps).

Bench test (bare NodeMCU over serial and the probe over BLE, run by Claude):

1. **5e-1, PC:** `pio test -e native`: `test_store` 8 of 8 (reboots, requests across
   boots, five laps of the ring, a write cut at each of its 64 bytes, a cut erase, clears).
2. **5e-2, the new table:** flashed with `pio run -t upload` (it writes the partition
   table). Boot: `shots: 0 hits held from 0 boot(s), 0 torn, capacity 1921`. The NVS
   survived: `mic` as before, the probe connects without pairing again.
3. **5e-3, hits survive reboots:** `mic sim`, `sim random 30`; reboot: `30 hits held from
   1 boot(s)`; `sim random 10`; reboot: `40 hits held from 2 boot(s)`, 0 torn.
4. **5e-4, across boots over BLE:** the probe from (0, 0): boot 1 seq 1–30, then boot 2
   seq 1–10, in order, 0 gaps. `PROBE_FROM=<boot 1>:21`: boot 1 21–30, then boot 2 1–10.
   `PROBE_FROM=<boot 2>:11`: none.
5. **5e-5, clear:** the probe with `PROBE_CLEAR=1`: serial `Shots up to 10 of boot <2>
   cleared`; reboot; the probe from (0, 0): no shots; `shots`: cleared up to H 40.
6. **5e-6, a full ring under load:** `sim random 2100`, no app: PASS as before (every shot
   solved and sent, none missed, no late starts), with the ring wrapping (an erase every 64
   hits, about 45 ms, on entering a sector when no quiet moment came). Then `shots`: 1,921
   to 2,048 held; the slowest write under a millisecond, the erase about 30–60 ms; `stat`
   as before.
7. **5e-7, resets at any moment:** during `sim random 300`, an EN reset (RTS pulsed, as
   esptool does) at a random moment, 5 times. Each boot: one boot more, 0 torn, the hits
   before the reset held. Then the probe from (0, 0): every boot's hits in order, no gap.
   (An EN reset restarts the ESP32 but the flash chip keeps power and finishes a write it
   has begun, so it can't tear a record: the PC tests do that, byte by byte, and 5e-8.)
8. **5e-8, a real power cut (the user, once):** a serial session open, `mic sim`, `sim
   random 1000`; pull the USB cable at a random moment, plug it in again, 5 times. Expect
   at each boot (`shots: ... hits held from ... boot(s), N torn`): N 0 almost always (a
   write takes 0.6–3 ms of every 150 ms), 1 at most; then `shots` and the app connected:
   every boot's hits in order, a gap in seq only where a boot line reported a torn record.

#### 5f. The curtain switch, and unscored hits against a real sensor

The app's TODO: the first tests run without the curtain, so the app gets a switch for it;
and resolving holes and missed shots by hand (merged 2026-11-26) still needed a run against
a sensor. Branch `app-curtain-switch`. No firmware or protocol change: Settings with the
window 0, 0 switch the gate and IR_EN off (PROTOCOL.md "Settings").

Built (app):

- `Command::WriteSettings` (`ble.rs`); `Settings::curtain()` and `with_curtain()`
  (`protocol.rs`): the settings as last read, only bytes 20–27 changed.
- In "Measurement": "A shot needs the IR curtain's confirmation". Off writes 0, 0; on writes
  the window last used (the sensor's own when it was on, kept in the app's storage; 45–100
  µs if none). It shows the sensor's state, with "Switching…", until the new settings come
  back; the sensor drops a write it can't take without an error, so after 5 s it says "The
  sensor didn't take the change". Disabled while calibrating.
- Found by the UI test: the bundled fonts have no "●", "◌" or "✕", so the status bar's
  connected and connecting dots and the notices' and counted cards' close buttons were
  empty boxes. The dot is now painted; the buttons use "×".
- The probe: only a sensor advertising now (BlueZ also lists bonded ones out of range), and
  `PROBE_NAME=<text>`.
- `live_sensor` (ignored test in `main.rs`): the app with its own BLE thread, driven
  headless (egui_kittest), against a sensor running the simulator; its serial commands go
  to a session's FIFO.

Bench test (a bare NodeMCU over serial, the app over BLE, run by Claude):

1. **5f-1, PC:** `cargo test` (the unit test `curtain_switch_changes_only_the_window`, the UI
   test `curtain_switch` against a sensor played by the test), clippy clean.
2. **5f-2, pairing a new board:** serial `pairing`; `bluetoothctl` pairs with the passkey
   000000 (the default PIN on a fresh NVS); the probe connects, the cfg ID from the reads
   matches Status.
3. **5f-3, the switch:** `curtain window 50 90` on serial; the app connects (the remembered
   sensor) and reads the curtain on at 50–90 µs. Off: the serial log shows `BLE: settings
   from the app` and the `cfg` line with `ir=-`; the app's settings equal those read but for
   0, 0 in the window. On: `ir=50000,90000` again, the settings equal those read (the window
   remembered from the sensor, not the default 45–100). No message.
4. **5f-4, hits not scored:** `sim dip 60`, `sim fault silent`, `sim at 0 0 2`: 2 holes in
   the app. `sim fault none`, `sim at 10 10 2`: 2 shots. The switch off; `sim dip off`,
   `deadtime 1`, `sim fault double 2500`, `sim at -20 30 2`: 2 shots and 2 missed hits (the
   second shot 2.5 ms later, while the first is read out; bench 2c).
5. **5f-5, resolved by hand:** a hole placed at (5, 5) mm (a click on the card, Confirm); a
   missed hit a miss; the other hole the same hole as the shot at (10, 10) (picked on the
   card). The series: 7 shots, 1 missed hit not scored. The day's log: 3 `manual` lines
   (`as=place x=5.0`, `as=miss`, `as=same`). Disconnect, reconnect: the same series and
   scores, the same hit not scored, nothing twice.

#### 5g. The settings editor

The app's TODO: the sensor's settings were shown read-only. Branch `app-settings-editor`. No
firmware or protocol change (Settings is written whole, PROTOCOL.md).

Built (app):

- "Change the settings…" in Measurement opens the editor: drop-one, the residual limit, the
  mics that may be lost, the area, the together limit, the longest pulse, the curtain's
  window, the mic span, the dead time and the channels. `protocol::settings_problem` checks
  the ranges as the firmware's `settingsValid` does, but for the span's largest (the
  sensor's own, from its capture buffer); "Write to the sensor" only for a valid change;
  "Undo changes", "Close". The editor follows the sensor's settings unless it holds changes.
- One pending write for the curtain switch and the editor (`settings_pending`): it waits for
  the sensor's settings to come back the same; after 5 s it says the sensor didn't take
  them (the curtain's own message when only the curtain changed). The switch waits while
  the editor is open.

Bench test (run by Claude):

1. **5g-1, PC:** `cargo test`: `settings_ranges` (every range at its edges), the UI test
   `settings_editor` (a sensor played by the test: three fields written, the rest as read;
   an invalid window not written; Undo; a dropped write's message; Close), the curtain
   tests as before; clippy clean.
2. **5g-2, the sensor:** `live_sensor` with a phase after the switch: the dead time 40 ms
   and the area 100 mm written and read back; a span of 5000 mm: the serial log shows
   `settings write rejected`, the app says so after 5 s and its settings stay; the settings
   as read written back. The rest of the test as in 5f.

### 6. Wi-Fi and MQTT

FUTURE_ARCHITECTURE.md step 2. Check the GPIO 36/39 erratum here (see Risks).

### 7. The real boards (PLAN phase 1)

Written 2026-11-24, before the boards arrive, so the first bench days give numbers. The
user runs the tests with the boards; Claude analyses the logs. Branch `phase1-prep` for the
tools and this plan; the bench runs get their own branch when they start.

**h is 5.5 mm** from 2026-11-24 (`MIC_H_DEFAULT_MM`, PLAN D3): the sensor board's paper
edge on the paper, the port 5.5 mm from it (21.5 mm flipped). A holder that leaves a gap
adds it: measure each mic's h with a calliper and set it (`mic h <ch> <mm>`).

**What the goal allows.** With the six mics (D1/D2) and h 5.5 mm, a random timing error of
1 µs per channel moves a shot by 0.21 mm (1σ, long axis) in the ring area, 0.35 mm near
the corners. So 0.2 mm allows about 0.9 µs of random noise per channel, and 0.5 mm about
2.3 µs. The 100 ns sampling alone gives about 0.03–0.06 µs. Random noise is unlikely to be
the limit; the systematic errors (k, the rise's shape, the geometry, the air) are what 7-4
to 7-6 measure.

**The analysis tool:** `tools/bench.py` (numpy, scipy), on serial or app logs. `--n A-B`
takes the events numbered A to B (one test's sparks), `--cal` only calibration mode's.

- `bench.py repeat LOG`: sparks at one point. Each channel's own timing noise (1σ, from the
  spread of every pair's t0 difference), the solved positions' scatter, and the scatter that
  noise predicts. Lists outliers and half-cycle-late trips (a channel 10–40 µs late).
- `bench.py kcheck LOG [--jitter NS]`: sparks at known points (`pt`). Per channel, the k and
  offset that fit its t0 to the geometry, k's uncertainty, and whether a curve fits clearly
  better than the straight line (a curved rise: the straight-line k is wrong). Then
  whether the boards' k differ beyond their uncertainty. It corrects k for the edges'
  sampling error (it is in t1 and in t2 − t1 alike, and pulls the slope low); `--jitter`
  adds each edge's analog noise (from `repeat`). It gives no verdict on the boards when
  t2 − t1 varies by under 1 µs (1σ): the comparison misfires there (the self-test).
- `bench.py verify LOG... [--drop]`: sparks at known points. The position error against
  the truth: bias, rms, 95 %, worst, 1σ per axis, per point; per log, so several logs
  compare (7-5). `--drop`: each channel left out in turn (what the sixth adds, D2).
- `bench.py swap LOG --ch N`: after channel N's board is replaced, sparks at known points
  with the old calibration. Refits only that channel's k and offset, shows the error
  before and after, and prints the serial lines.
- `bench.py --selftest` (also in CI, about 25 s): synthetic logs with known noise, k,
  curvature and a swapped board; every command must find them. On a short t2 − t1 range
  (the simulator's 0.1–1.6 µs), equal boards must get no verdict.

**The spark:** a piezo lighter's spark gap (or a snapped pencil lead), held in a fixed
stand for 7-3, through the template's holes for 7-4 to 7-6 (`assets/templates/grid-4x4.json`,
the calibration wizard, 5d). Keep the spark gap at the paper plane.

Tests (each run's events numbered in the log: note the first and last `n`):

- **7-0, h 5.5 mm (simulated, done 2026-11-24).** Bare NodeMCU: `mic default` and `mic sim`
  list h 5.50 mm; `sim at 20 -30 100` and `sim random 160` PASS; `bench.py repeat`,
  `verify --drop` and `kcheck` run on that log.
- **7-0b, the top-right mic moved out (simulated, 2026-11-24).** PLAN D1's update with
  d = 25 mm: CH3 at (122.68, 122.68). On the PC: the vectors made again, `pio test -e
  native` (the blast check: the four corners alone score none, smallest residual about
  6.9 µs as the study; all six and any five none), the app's `cargo test` (the Rust solver
  against the vectors), `layout_study.py --drop` with the new layouts (O25, O25+B,
  O25+BR: no blast scores, a late channel found). On the bare NodeMCU: `mic default` (its
  blast check: none), `mic sim`, `sim random 160` PASS, then `sim fault late 5` (the
  simulator's CH5 is the moved mic) and `sim random 60`: dropped in every shot, PASS.
- **7-1, each sensor board alone** (the summary's initial calibration; step 2's real tests 1–3).
  Power it at 3.3 V on J1 (or plug it into the host board), scope on SIG, OUT_L, OUT_H.
  - Scope ground on TP5 (GND). Measure the thresholds with the meter's black lead on VMID
    (TP4), so it reads the offset directly.
  - VMID about 1.64 V to GND (half the local supply), within ±30 mV: nothing to adjust.
    SIG (the SIG loop) at rest within ±10 mV of VMID, a few mV peak to peak in a quiet room;
    a large sine (oscillation) or 50 Hz (hum) needs fixing first.
  - RV1 to one end (over 25 turns), then back 12–13 turns: mid-travel, about ×12. RV2 for
    VTH_L = VMID + 100 mV (pad TP2), RV3 for VTH_H = VMID + 400 mV (TP3).
  - A spark 50–100 mm from the mic: OUT_L and OUT_H (J1 pins 3 and 5) low at rest, pulsing
    on the spark, OUT_L first; SIG first goes **up** (polarity); t2 − t1 from about 0.3 µs
    (loud) to a few µs; no second trip from ringing.
  - Once mounted, match the gains: a spark at the frame's exact centre, RV1 set so the
    first-lobe amplitudes agree within about 10 %, at 2–3 × VTH_H or more (clipping at the
    peak is fine). Then leave RV1 alone: a gain change shifts the timing.
  - Note per board (the summary's initial calibration, PLAN D5): its number, VMID, VTH_L,
    VTH_H, RV1's position, and t2 − t1 at 50 and 100 mm. A board whose SIG first goes down is wired the wrong way: stop before more boards
    are ordered.
- **7-2, the host board's pins** (step 1 tests 3 and 14 on the v0.3 map): `selftest` PASS
  as before; each channel's L and H on its own pin (a spark near each mic in turn, or a
  jumper to 3V3 through 1 k on each J1x pin 3 and 5): the right channel and edge appear.
- **7-3, repeatability (the noise floor; PLAN 1.2).** The frame with six mics, the spark at
  one point (say the centre) in a fixed stand, 50 sparks, `mic` with the nominal layout
  (no calibration needed). `bench.py repeat LOG --n A-B`.
  - Expect: each channel's own noise from about 50 ns (the sampling alone) to a few
    hundred ns; the position scatter close to the noise's prediction.
  - Pass: the long axis ≤ 0.5 mm (1σ), the goal's best ≤ 0.2 mm; no outliers. A channel
    with an outlier 10–40 µs late: polarity or the thresholds (7-1).
  - Over 0.5 mm here, the goal can't be met by calibration: look at that channel's
    thresholds and gain before going on.
- **7-4, calibration and verification (PLAN 1.3).** The frame with the D1/D2 layout, each
  mic's h measured, the template on the card holder.
  1. The calibration wizard (app, 5d): the 4 × 4 grid, 10 sparks per point (160), the fit,
     the record sent; its field check at the centre.
  2. Verification: 10 sparks at each of 9 other points (a 3 × 3 grid at 0 and ±50 mm), in
     calibration mode with `cal pt` for each point (so the `rec` lines carry `pt`).
     `bench.py verify LOG --n A-B --drop`.
  3. `bench.py kcheck LOG --n C-D` on the 160 calibration sparks (with the calibrated
     cfg): is a straight-line k right per board, and do the boards' k differ?
  - Pass: 1σ per axis ≤ 0.5 mm (the goal's best: ≤ 0.2 mm), no point's bias over 0.5 mm.
    `--drop` shows what each channel adds; the middle mics matter most near the edges.
  - A channel marked CURVED: its rise isn't straight over the range used. Note the rms the
    straight line leaves; if it is over about 300 ns, a curved arrival model is the next
    step (4c).
- **7-5, temperature (PLAN 1.4).** Needs the air sensor on J5 (3a-3 or 3a-4 first). The
  verification of 7-4 at room temperature, then the room at least 4 °C warmer or colder (a
  heater, a window; wait 30 minutes for the frame), the same verification with the same
  calibration. `bench.py verify A.log B.log`.
  - Pass: the bias moves by ≤ 0.1 mm, the 1σ within 20 % of the first. For comparison, 1 °C
    of error in the air costs about 0.15 mm rms (PLAN): a larger shift means the air isn't
    measured right (the sensor reads the board, not the air: summary).
- **7-6, a board swap (PLAN 1.5).** After 7-4, replace one sensor board (say CH4's) with a
  spare set up as in 7-1. The verification of 7-4 again, with the old calibration. Then
  `bench.py swap LOG --ch 4`.
  - Expect: the error grows before the refit (a new board's k and offset); after it, back
    within 20 % of 7-4's rms.
  - Pass: then a board swap needs only that channel's k and offset (send `bench.py`'s two
    lines; calibrate fully when convenient). If not, it needs a full calibration: note
    what was left.
- **7-7, the IR curtain on the bench (PLAN 1.6).** The host board's bench list
  (`hardware/host-board/TODO.md`): RV1's threshold with pellets dropped through the beam,
  U3 compared (MCP6002 against MCP602), then the dip's delay against real shots, and
  `curtain window` set from it (the receiver adds about 3–8 µs; the window is 45–100 µs).

Done when: 7-1 to 7-4 pass on six boards, with 7-3's noise floor and 7-4's error written
into Bench results. 7-5 to 7-7 follow as the parts allow.

## Risks

- **I2S parallel capture on IDF 5:** register-level, untested here; settled by the step 1
  self-test. Fallback: MCPWM capture.
- **Relinking descriptors while the DMA runs (ring freeze):** assumes the DMA reads a
  descriptor's next link when it gets to it, as esp32-camera relies on. If not, the
  self-test shows wrong or repeated windows; fallback: stop and restart the DMA.
- **GPIO 36/39 (CH1):** ESP32 erratum: spurious pulses on these pins while Wi-Fi or ADC1
  is active. Not an issue with BLE only; check again in step 6.
- **The two MCPWM units starting together:** both take their count from one edge on
  the sync pin, but their prescalers may not line up: up to a tick (25 ns) between CH5-CH7 and
  CH8. Settled by test 2a; if it is more, or not stable, the simulator corrects for a
  fixed offset or drops to 3 channels.
- **Edge shape, mic polarity, half-cycle-late trips, range noise:** can't be settled on
  paper; all show up in the step 2 dump.

## Bench results

Newest first. Date, step, setup, what was measured, and what it changed.

| Date | Step | Setup | Result |
|---|---|---|---|
| 2026-11-27 | 5g | The settings editor, branch `app-settings-editor`, run by Claude: the bare NodeMCU ("OpenSights-487"), serial `logs/serial-261127-5g.log`, the live test `logs/live-261127-5g.out` with the app's log in `logs/app-261127-5g/` | **Pass.** 5g-1: `cargo test` (the app 58, 3 ignored), clippy clean. 5g-2: the dead time 40 ms and the area 100 mm written by the editor and read back in 0.2 s (serial `capture: ... dead time 40 ms ... (from the app)`, `BLE: settings from the app`); a span of 5000 mm: serial `BLE: settings write rejected ... nothing changed`, the app's message after 5.0 s, its settings unchanged; the settings as read written back (dead time 30 ms). The rest of `live_sensor` as in 5f: 6 shots and 2 missed hits, 3 `manual` lines, the series kept across a reconnect. The first run failed in the test, not the app: both shots at (10, 10) lie 0.03 mm apart, the click took the nearer (seq 4), the test expected seq 3; it now takes either. |
| 2026-11-27 | 4d | A moderately off mic, branch `huber-study`, run by Claude. PC: `tools/huber_study --n 5000` (seed 1), then `--trigger 3, 4, 5, 8`. Board: the bare NodeMCU ("OpenSights-487"), `mic sim`, `sim random 300` healthy, with `sim offset 5 1000` and with `sim offset 5 3000`; log `logs/serial-261127-4d.log`, split into `-4d-healthy.log`, `-4d-ch5-1us.log`, `-4d-ch5-3us.log`; each replayed with `--alt` | **The study: drop-one started by the sigmas beats Huber.** Synthetic (table in 4d): with one mic 1–10 µs off at 50 ns noise, dropS ×3 gives 0.020–0.042 mm rms where the firmware gives 0.131–0.484 and Huber 0.084–0.764; healthy shots are never touched; Huber alone fails a 24 µs fault (1.9 mm, the firmware 0.021); the TODO's two steps keep the bad mic in about 20 % of big faults ("two far out"), and its weighted residual limit passed shots 1.9 mm wrong with two mics off. dropS's price: with sigma 3× too low, trigger 3 drops a healthy mic in 39 % of shots, trigger 5 in 3.9 %. **On the board's path** (100 ns sampling, the firmware's own fit): healthy 0.0128 mm all four ways, 0 drops; CH5 1 µs late: the firmware 0.1410 mm (rms residual 260 ns, nothing dropped), dropS ×3 0.0325 (CH5 dropped in 293 of 300, CH6 in 3), dropS ×5 0.0718, Huber 0.0926, Huber + search 0.0436; CH5 3 µs late: the firmware 0.4086 (residual 800 ns, nothing dropped), dropS ×3 and ×5 and Huber + search 0.0170 (CH5 in 300 of 300), Huber 0.2429. The simulator's own check: every run PASS. Nothing in the firmware yet: the user decides (4d). |
| 2026-11-27 | 5f | The curtain switch and unscored hits, branch `app-curtain-switch`. A new NodeMCU (ESP32-D0WD-V3, MAC 70:4b:ca:8f:0c:cc, "OpenSights-487", fresh NVS) on `/dev/ttyUSB0`, flashed by the user; run by Claude: serial `logs/serial-261127-B.log`, the probe `logs/probe-261127-5f-0.*`, the live test `logs/live-261127-5f-run1.out`, `-run2.out` with the app's event logs in `logs/app-261127-5f-run1/` and `app-261127-5f/` | **Pass.** 5f-1: `cargo test` (the app 55, 3 ignored), clippy clean; the UI test found the bundled fonts lack "●", "◌" and "✕" (the status dots and close buttons drew as empty boxes): the dot is painted now, the buttons use "×". 5f-2: paired by `bluetoothctl` with the passkey 000000; the probe connected (cfg ID F5B887F6 from the reads and in Status: match) once it skipped BlueZ's cached, out-of-range "sensor123" (it now takes only a sensor advertising). 5f-3: the app connected in 1.2–1.5 s and read the curtain on at 50–90 µs; off and on again each took under 0.5 s; serial `BLE: settings from the app` and `ir=-`, then `ir=50000,90000` (the window remembered from the sensor); the settings equal those read but for the window; no message. 5f-4: 2 holes, 2 shots; then with the curtain off, 3 double shots: 7 shots and 1 missed hit in both runs (each second shot is MISSED only while the first is still read out, else measured on its own; the first try's 6 and 2 differ only in that, none lost). 5f-5: a hole placed at (5.00, 5.00) (10.1), the missed hit a miss, the other hole the same hole as the shot at (10.01, 10.00) (9.2); 3 `manual` lines; disconnected and reconnected: the same 10 shots and scores, nothing twice. **Found on the way:** (1) the clone CP2102 on this board floods the port with replayed data (about 0.5 MB/s at any baud) when opened with DTR and RTS asserted (pyserial's default): open it with both released. (2) The live test first sent settings while a simulator run was ending, which it refuses ("settings can't change while it runs"): the test now waits 1.5 s after a run's hits. (3) With `sim fault silent` the dips come 7.2 µs late and 27.1 µs long instead of 35 (the simulator's known first-step stall, 4b; its own check passes). The air-sensor search prints two I2C errors at boot with nothing on J5. |
| 2026-11-24 | 5e | The shot store. PC: `pio test -e native` (31 cases). Bare NodeMCU flashed with the new table (logs `logs/upload-261124-*-5e.log`), run by Claude over serial (`logs/device-monitor-261124-131158-5e-a.log`, `-131401-5e-b.log`, `-5e-c.log`, `-5e-6.log`, `-5e-6b.log`, `-5e-7.log`, `-5e-7b.log`) and over BLE with the probe (`logs/probe-261124-5e-*.out`) | **PASS, 5e-1 to 5e-7.** 5e-1: `test_store` 8 of 8 (five laps of an 8-sector ring, 449 held at the end against 385 guaranteed; a write cut at each of its 64 bytes: nothing earlier lost, the torn hit absent; a cut erase). 5e-2: boot `capacity 1921`; the NVS kept (`mic` as before), and the store kept through the next reflash too (uploads don't touch `shots`). 5e-3: 30 hits, `reboot` (added for this: a clean restart): `30 hits held from 1 boot(s), 0 torn`, read in 42 ms; 10 more, reboot: 40 from 2 boots. 5e-4: the probe from (0, 0): boot 074B3DEF seq 1–30 then 651E3407 1–10, in order. **On that first connection after the reflash every hit came twice in a row** (80 for 40); the next connections with the same firmware: never (20, 0, 40, 1938, 1954 shots, none twice). The new build's "services changed" with BlueZ (bench 5) is the likely cause; the app drops duplicates by (boot, seq), so no score changes. `PROBE_FROM=074B3DEF:21`: seq 21–30 then the second boot's 1–10; `PROBE_FROM=651E3407:11`: none. 5e-5: `PROBE_CLEAR=1` after the 40: cleared up to H 40 (both boots), kept through a reboot; the probe then got 0. 5e-6: `sim random 2000` is the simulator's limit (its first try refused 2100), so two runs of 1100: both PASS (1100 of 1100 ok, sent, 0 missed, 0 late starts, 0.0125 / 0.0124 mm rms), the ring wrapped: 1921 held (H 320–2240), one erase done at a quiet moment (2 s); the slowest write 3.3 ms (typically 0.6), the slowest erase 29 ms; `stat`: interrupt 8.4 %, longest 22.4 µs (as before), 2200 events, 0 possibly missed. Found on the way: the quiet erase also waited for "no hit waiting to be sent", which never comes with no app connected: removed (an erase only delays the next indication). 5e-7, first try: reopening the port right after closing it didn't reset the board (the hits all stayed one boot: the store's "1 boot" was right); then with EN pulsed through RTS: 5 resets, each boot one boot more, 0 torn, the ring read in 98–100 ms; the probe got 1954 hits: the long run's boot seq 984–2601, then the five reset boots 1–85, 1–75, 1–27, 1–74, 1–75, 0 gaps, 336 in all = the 336 events logged before the resets. |
| 2026-11-24 | 7-0b | The top-right mic moved out 25 mm along its diagonal, to (122.68, 122.68) (PLAN D1's update). PC: `model.py` (vectors made again), `pio test -e native`, the app's `cargo test`, `layout_study.py --drop` (log `logs/layout-study-261124-o25.txt`). Bare NodeMCU, run by Claude over serial: `mic default`, `mic sim`, `sim random 160`, `sim fault late 5`, `sim random 60`; log `logs/device-monitor-261124-125746-o25.log` | **PASS.** Native tests 23 of 23; the blast check: the four corners alone score 0 (smallest residual 6871 ns, the study's 6.9 µs), all six 0 (50.9 µs), any one dropped 0 (31.5–46.0 µs). The app's tests 68 pass (1 ignored, the phone), the Rust solver against the new vectors. The study: O25 (4 corners) 0 of 624 blasts, 6.88 µs; O25+B (5) 0, 31.6 µs, a late channel found 555 of 555 and then 0 blasts (S40+B: 553 of 555); O25+BR (6, the layout now) 0, 51.1 µs (S40+BR: 34.5 µs), a late channel found 666 of 666, then 0 blasts. On the board: `mic default` and `mic sim` put CH3 (the simulator's CH5) at (122.68, 122.68), and the board's blast check finds none, with every mic or any one dropped (1049 ms). `sim random 160`: 160 of 160 ok, 0.0123 mm rms, worst 0.028. The moved mic 24 µs late (`sim fault late 5`): dropped in 60 of 60, every shot ok, 0.0145 mm rms. Note: the board's boot text has ROM bytes in it, so `grep` takes the log for binary; use `grep -a`. |
| 2026-11-24 | 7-0 | Bare NodeMCU, h 5.5 mm (the new default), run by Claude over serial: `mic default`, `mic sim`, `sim at 20 -30 100`, `sim random 160`; log `logs/device-monitor-261124-124225-h55.log`, upload `logs/upload-261124-*.log` (the first try: "Failed to start stub flasher", the second flashed); then `tools/bench.py` on that log, and `bench.py --selftest` | **PASS.** Every mic at h 5.50 mm after `mic default` and `mic sim`. `sim at`: 100 of 100 ok; `sim random 160`: 160 of 160 ok, error 0.0130 mm rms, worst 0.0276, none dropped. `bench.py verify --drop`: 0.013 mm rms, worst 0.028 (the simulator's own figures); without CH7 or CH8 (the middle mics) 0.014, without a corner 0.016–0.020. `bench.py repeat` (the 100 `sim at` shots): runs, but its predicted scatter is half the measured (0.007 against 0.014 mm): at one fixed point the simulator's only error is the sampling, nearly the same every shot (some channels' t0 differences vary by 3 ns), not the independent noise the method assumes; real analog noise is. `bench.py kcheck` on the simulator: k 0.305–0.361 (true 1/3 on all) with t2 − t1 spanning only 0.1–1.6 µs. This found two faults in the tool, both fixed: (1) the edges' sampling error is in t1 and in t2 − t1 alike and pulls the slope low (by σq²(2k + 1)/var(t2 − t1)): now corrected, the mean k 0.334; (2) over so short a range the boards' comparison misfires (a synthetic check: χ² up to 31 for equal boards, on the 25 ns grid or not; 4–7 over 1–10 µs): now no verdict under 1 µs (1σ). Self-test: 7 checks PASS (noise per channel within 6 %, a 24 µs late trip found, k within 0.0024, a curved channel found, equal boards agreeing in 5 of 5, no verdict on a short range in 5 of 5, the swap's k and offset recovered and its error 0.170 → 0.015 mm). |
| 2026-11-23 | 5d | Bare NodeMCU, BLE from the PC (the probe in `drive` mode, `target/release/examples/probe`), the simulator on serial, run by Claude; serial `logs/device-monitor-261123-074754-5d.log`, probe `logs/probe-261123-074754-5d.out`, app log `logs/app-events-261123-074754-5d.log` | **Passes.** Field check before: 5 sparks at (0, 0), 0.436 mm (within the 0.5 mm limit: at the centre `sim cal truth` is least off). The wizard took the 160 sparks of `sim cal 10` (0 ignored), fitted: residual 38.5 ns, the error over the 16 points 0.814 → 0.013 mm rms (every point's worst under 0.035 mm); wrote record `E2BA6297`, which the sensor confirmed in Status and serial (`in use`). Field check after: 0.010 mm. `sim random 200` afterwards: 0.0145 mm rms, worst 0.061 mm (3e-3 with calibrate.py: 0.0140). `calibrate.py` on the same 160 sparks: mics within 0.0004 mm, k within 0.0001, offsets within 1.3 ns (±50 ns), the same 38.5 ns and 0.814 → 0.013 mm. On the PC: the solver meets the firmware's vector tolerances (worst position 0.000055 mm); the fit against calibrate.py's self-test within 0.1 sd. **Found:** scipy's defaults (1e-8) stop a few ns short of the minimum along the flat k–offset valley (objective 629 427 against the Rust fit's 629 420): `calibrate.py`'s final fit now uses tolerances of 1e-14 and central differences (629 420.24; offsets within 0.25 ns of the Rust fit). The last printed digits can still differ, so the two records' IDs aren't compared. |
| 2026-11-23 | 5a–5c | Bare NodeMCU, the PC's Bluetooth (BlueZ 5.85) with the probe (`app/examples/probe.rs`), run by Claude; serial `logs/device-monitor-261123-071418-5b.log`, probe `logs/probe-261123-071418.out`, app log `logs/app-events-261123-071418.log` | **Passes.** All 6 `sim` runs PASS. Over BLE: 50 events (38 acoustic, 5 holes, 7 missed), 0 gaps; 40 hits (28 shots, 5 holes, 7 missed), all 40 linked to a received event marked sent; 28 Status notifications. The app log's 38 `rec` lines and 3 `cfg` lines are identical to the serial log's (IDs included); `tools/replay` on the app log: ok 28, lost 5, outside 5 (the board's), 5 `dip` lines, the shots 0.0084 mm rms from the true points. Writes: settings unchanged accepted, span 10 mm rejected, the 213-byte calibration record (a long write) became the record (`05FB1059`, the ID the app computes too), calibration mode and the point in Status, an unknown command ignored. Native tests 23 (7 new), app tests 33. **Found on the way:** after the update BlueZ kept the v2 GATT layout cached. The sensor's "services changed" (at boot, then on the live link, then directly on its handle: rc 0) didn't make it read again. Pairing again does; the app now waits 8 s for a new layout, then says so. NimBLE's GATT caching (a database hash) doesn't build in NimBLE-Arduino 2.3. The re-pair needed the sensor's PIN (123456, not the default) and pairing mode: the serial `pairing` command was added for that. |
| 2026-11-22 | 4b | 4b-0 to 4b-8 again with the edge tie, bare NodeMCU, run by Claude over serial, logs `logs/device-monitor-261122-230319.log` (diagnostic) and `-230420.log` (the full test), upload `logs/upload-261122-230227.log` | **Passes.** The tie now comes from one 20 µs pulse on GPIO 12 seen by both the I2S and capture channel 2 (the user chose this over a fixed correction). First try (`-203458.log`): 10.58 ms off and every dip unclaimed, because `gpio_reset_pin` briefly pulled GPIO 12 up and that rise opened the tie's event 10 ms before the real pulse. Fixed: the pin is set up as a quiet low output before it is routed, the tie waits out the dead time, and it checks the two agree within 50 µs (else a warning, and the catch stays). Both boots: `the start's catch was -4100 ns off` and `-4175 ns off`, so the edge corrected the bad case. The firmware's dip time against the hardware truth: mean −122 to −146 ns per run, rms 126–150 ns, worst 224 ns (200 shots of `sim random 200`), the same over both boots. All runs PASS; 4b-1 to 4b-6 as in the row below. Replay: 340 events, 0 verdicts differ, 20 `dip` lines; positions 0.013 mm rms against the truth. Native tests: 16 pass. Still in the code: the simulator's diagnostics (`steps late`, `tick 0 vs the sync edge`). |
| 2026-11-22 | 4b | 4b-0 to 4b-8, bare NodeMCU, run by Claude over serial (scripted; the user's OK for this step), logs `logs/device-monitor-261122-195051.log` (first run) to `-201814.log`, uploads `logs/upload-261122-*.log` | **Not passed yet: an open problem in the clock tie.** Boot: no `curtain:` error, so the capture unit and the pulse generator share MCPWM group 0. **Bugs found and fixed on the way:** (1) firmware: when the capture interrupt runs late, the driver hands over a dip's fall before its rise, and the fall was dropped (`len=-`); the ring now keeps a fall until its rise. (2) replay: a `rec` line ending in `ir=-` with its line end did not parse (0 events); fixed, native test added. (3) this bench test: 4b-4 never set the dip back to 30 µs (it ran 4b-3's 120 µs); `sim dip 30` added. (4) simulator: its truth came from CPU cycle counts, and one shot of 200 was off by 1 µs from that; it now comes from the capture unit itself (channel 2 latches the sync edge during a run). Edges closer than 60 µs stay in one masked stretch. **What passed** (`-200234.log`, all runs PASS): the firmware's dip time against the hardware truth was +83 to +124 ns mean per run, rms about 95 ns, worst 196 ns. 4b-1: 20 `[curtain]`. 4b-2: 20 + 200 `[ok]`, position 0.012 mm rms. 4b-3: 40 `[curtain]` with the dip at +30 and +120 µs. 4b-5: `[lost]` and `[outside]` kept with a dip in the window (acoustic reject with a dip: 40). 4b-6: 20 `dip` lines, 0 events. Replay: 340 events, 0 verdicts differ; `--curtain off` 80 differ. **The open problem:** register writes to the MCPWM unit can stall for several µs. At boot the tie's catch took 3866 cycles (16 µs), the I2S start right after it 42 cycles. Where in that stall the catch lands decides the tie: in 3 boots the dip time was +0.1 µs from the truth, in 3 others +4.1 µs, steady within a boot. On a real curtain that is a dip-time offset of up to about 4 µs, different each boot. The simulator's own dips also come 3–12 µs late when a dip edge is the run's first step (the GPIO write stalls too); that doesn't affect its check, since the truth is latched in hardware. Still in the code: the diagnostics (`steps late`, `tick 0 vs the sync edge`, `curtain: tie:` at boot). |
| 2026-11-22 | 4a | 4a-8, bare NodeMCU, serial console (user report, pasted) | **PASS; step 4a passes.** `valid area 110`: `area +-110 mm`, the hint "`mic` runs the blast check for the new area", `area=110` in the `cfg` line. After the EN reset the boot `cfg` line and `valid` still read 110 (stored in NVS). `valid default`: back to `lost 1, area +-105 mm, together 20.0 us, pulse 10.0 us`, `cfg` id 5C14D0AC again (the same settings give the same id). `valid foo`: the usage line. `mic default`: the blast check 0 of 624 everywhere, `area=105`. At boot, with nothing on J5, the Arduino I2C driver logs four lines `i2cWriteReadNonStop ... ESP_ERR_INVALID_STATE` (error 259): the BME280 probe got no acknowledge. They were already in the logs of 3b-3e; harmless, the `air:` line reports no sensor. All of 4a-0 to 4a-8 pass. |
| 2026-11-22 | 4a | 4a-6's missing run, bare NodeMCU, the same log (`-185126.log`); 4a-7 again on both logs | **PASS.** Blast 10 m/500 mm: 20 of 20 `[residual]` (rms 20500-22100 ns, fits at x −19 to −33, y +3 to +16 mm), 0 sent, blast check PASS. 4a-7 on all 480 events: ok 320, together 60, residual 60, lost 20, outside 20 (the doc's counts plus the repeated 10 m/0 run, 20 more together). 0 of 480 verdicts differ from the board's. `--together off`: residual 120, 60 differ. `--lost 2`: ok 340, 20 differ. True points 0.0120 mm rms, worst 0.0315 mm. Left: 4a-8. |
| 2026-11-22 | 4a | 4a-4 to 4a-6, bare NodeMCU, logged (`logs/device-monitor-261122-185126.log`, after 4a-3); 4a-7 on the PC on that log and `-184742.log` | **PASS, but one blast run is still to do.** 4a-4 spike: 20 of 20 `[together]`, "every first edge within 0.0 us", fit (−26.291, +9.846) mm, rms 21572 ns, CH4 dropped, 0 sent. With `valid together off`: 20 of 20 `[residual]`, 0 sent. 4a-5: (115, 0) 20 of 20 `[outside]`, 0 sent, error 0.0347 mm rms, worst 0.0622 mm (PC 0.035). (100, 0) 20 of 20 `[ok]`, 0.0120 mm rms, worst 0.0128 mm (PC 0.012). 4a-6 blast 10 m/0 mm: 20 of 20 `[together]`, "within 1.6 us", rms about 21300 ns, 0 sent. A typo (`saim`) left the next command unrun, so that blast ran twice and 10 m/500 mm is missing. Blast 25 m/2000 mm: 20 of 20 `[residual]`, rms 20000-22700 ns, 0 sent. `stat` since boot: ok 920, together 80, residual 60, lost 60, outside 20; the solve took 422 µs mean, 1.27 ms longest. 4a-7: 460 events, ok 320, together 60, residual 40, lost 20, outside 20 (the doc's 40/60 with the missing run; the second 10 m/0 run in its place). Against the board, 0 of 460 verdicts differ, shot lines 0.0000 mm worst. True points: 320 good, 0.0120 mm rms, worst 0.0315 mm. `--together off`: together 0, residual 100, 60 differ. `--lost 2`: ok 340, lost 0, 20 differ. To do: 4a-6's 10 m/500 mm, then 4a-8. |
| 2026-11-22 | 4a | 4a-3, bare NodeMCU, logged (`logs/device-monitor-261122-185126.log`, a new monitor session) | **PASS.** `sim fault missing 5 7`: 20 of 20 `not sent [lost]: 2 channels lost (CH5 CH7), 1 allowed`, verdict check PASS (lost 20), 0 sent; residuals on CH3, CH4, CH6 and CH8 only, the error 0.0128 mm rms, worst 0.0236 mm, for information (PC: 0.013, 0.025). `valid lost 2` (`lost=2` in the `cfg` line): 20 of 20 `[ok]` and sent, verdict check PASS, 0.0134 mm rms, worst 0.0236 mm. `valid lost 1` restored `lost=1`. **Changed:** those shots said `(with a channel lost)` with two lost; the shot line now says `(with 2 channels lost)` (from the next upload; the replay reads only the `[ok]`). 4a-7 now says the replay takes every log of 4a-0 to 4a-6 when the monitor was restarted. |
| 2026-11-22 | 4a | 4a-0 to 4a-2 again, bare NodeMCU, clean settings, logged (`logs/device-monitor-261122-184742.log`) | **PASS.** 4a-0: `valid default`, `offset clear` (all 0), `sigma 50` (all 50); the `cfg` line ends `lost=1 area=105 tog=20000 pmax=10000`. The blast check found 0 of 624 everywhere, smallest 34.3, 34.7, 13.5, 37.5, 37.3, 22.1, 35.4 µs, exactly as in 3d. `k` was typed as 0.3333 (0.333299994 in the `cfg` line, not 0.33333); that makes no visible difference. 4a-1 `sim random 200`: verdict check PASS (ok 200), error rms 0.0127 mm, worst 0.0315 mm, mean (−0.0007, +0.0004) mm, residual mean 26 ns, worst 51 ns; 200 of 200 sent, none dropped (PC: 0.013, 0.06). 4a-2, 20 shots each at (30, 30), all `[ok]` and sent, 0 `(!)`: late CH5 dropped in 20 (residual +23914 to +24088 ns), rms 0.0086 mm; missing CH5 rms 0.0094 mm; slow CH5 with the shape note in 20 of 20, none dropped, rms 0.0100 mm; slow with `valid pulse off` CH5 dropped in 20 (−13251 to −13077 ns), rms 0.0095 mm. Worst 0.0154 mm in every 4a-2 run (PC: 0.009 rms, 0.015 worst). |
| 2026-11-22 | 4a | 4a-0 to 4a-2, bare NodeMCU, logged (`logs/device-monitor-261122-154003.log`, `-184123.log`) | **Verdicts right, numbers not counted: the settings were not clean.** The offsets and sigmas of the 3e calibration were still in NVS (CH3 +130.7, CH4 −108.8, CH5 −476.0, CH6 +395.4, CH7 +169.8, CH8 −111.1 ns; sigma 36-41 ns). The second session also had `together off`, left by 4a-4 in the first. So every run said "for information only", and its `sim: PASS` covered only the capture. What did hold: 4a-0's blast check found 0 of 624 everywhere (smallest 34.5, 34.9, 13.5, 37.6, 37.6, 22.2, 35.5 µs; 3d: 34.3, 34.7, 13.5, 37.5, 37.3, 22.1, 35.4). 4a-1: 200 of 200 `[ok]` and sent, none dropped. 4a-2: every shot `[ok] (with a channel lost)` and sent. Late CH5 was dropped (residual +24564 ns, expected about +24000). Missing CH5 shows as `-`. Slow CH5 gets the shape note "t2 - t1 not within 0-10000 ns: left out" (t2 − t1 = 40000 ns). With `valid pulse off` it was dropped instead (residual −12458 ns, expected about −13000). The position errors carry the offsets' bias: 4a-1 0.141 mm rms, worst 0.343 mm, mean (−0.122, −0.047) mm, residual mean 170 ns (expected 0.013 mm, 0.06 mm). 4a-2 0.085 mm rms (expected 0.009); it is lower there because CH5, the largest offset, is out of the fit. **Changed:** 4a-0 now starts with `valid default`, `offset clear`, `sigma 50`. "How to read a run" says what "for information only" means. 3e's clean-up ends with `offset clear`, `sigma 50`. To do: 4a-0 to 4a-2 again. |
| 2026-11-22 | 3e | 3e-1 to 3e-4, bare NodeMCU, logged | **PASS (user report; numbers from the logs).** 3e-1 after `sim cal truth`: error 1.30 mm rms, worst 1.74 mm, residual mean 882 ns (more than the 0.5 mm expected: this truth's k were 0.35-0.49 against the firmware's 1/3). 3e-2: 160 sparks; `calibrate.py`: residual 39-40 ns, the error over the 16 points 1.0 → 0.014 mm rms; against the truth k within 0.024 and the offsets within 61 ns (mean removed). The record was stored ("in use"). 3e-3: `sim random 200` 0.0140 mm rms, worst 0.0612 mm, residual mean 29 ns (3d-2: 0.0125); `mic h` made the record stale, and it stayed stale after the value was set back. 3e-4 (`sim temp 20`): error 0.365 mm rms; `replay --fit-c`: +1.99 °C, and with it the error against the true points 0.0152 mm rms. **Found:** the stored record said 116 points and 260 sparks: `calibrate.py` took every event with a `pt`, including 3e-1's 100 random simulated shots (right here, as the truth hadn't changed, but a log with an earlier truth would have spoiled the fit; run on the whole log afterwards, it picked the post-calibration runs, the wrong-temperature one included: 575 ns). It now uses calibration mode's sparks only (`--any-pt` for the simulator's other shots), from the newest session; on the same log: 160 sparks, 16 points, the same values within the noise. `sim` lists the simulator's mics to 0.001 mm now (0.1 before, too coarse to compare). Step 3's simulated tests are done. |
| 2026-11-22 | 3e | Built; PC checks | Calibration mode (`cal on`, `cal pt`), `pt`/`mode`/`cal`/`stale` in the `rec` line, the calibration record (`cal ch`, `cal id`, stale on any change), `sigma`, `sim cal truth|reset|<n>`, `tools/calibrate.py`, `replay --fit-c`. **Found on the way:** the fit with plain residuals pulled every k low by 0.02-0.05 and the offsets with it (noise in t2 − t1, errors in variables); dividing each residual by √((1 + k)² + k²) fixes it (Deming), with the plain robust fit first and k bounded (the scaled cost has a false minimum at k → ∞; one seed ran away to k = 656). `calibrate.py --selftest`, 6 seeds: residual 37-40 ns, mics within 0.03 mm, k within 0.03 (≤ 1.8 sd), offsets within 85 ns (≤ 2.4 sd), position 0.51 → 0.013 mm rms. The `cal id` the PC prints matches the board's (the same lines read with `sscanf`). `--fit-c` on a log 2 °C off: +1.96 °C. 14 native tests pass; the firmware builds. Run 3e-1 to 3e-4. |
| 2026-11-22 | 3d | 3d-0 to 3d-5, bare NodeMCU | **PASS (user report; numbers from the log).** 3d-1 `sim raw 0 0 0 0 0 0 200`: PASS, 0 late starts. 3d-2 `mic sim`: blast check 0 scored everywhere (smallest 34.3, 34.7, 13.5, 37.5, 37.3, 22.1, 35.4 µs, as on the PC); `sim random 500`: error rms 0.0125 mm, worst 0.0516 mm, residual mean 27 ns, worst 52 ns, none dropped (PC: 0.013, 0.06, 26, 60). 3d-3 late CH5: 100 of 100 sent with CH5 dropped, rms 0.0154 mm, worst 0.0433 mm; early CH8: 100 of 100 with CH8 dropped, rms 0.0154 mm, worst 0.0436 mm. `stat` after them: 477 µs per event mean, 2.4 ms longest (drop-one's 7 fits). 3d-4: blasts 10 m/0, 10 m/500 mm, 25 m/2 m: 0 of 20 sent each. The square: the four corners plus both middles score 0 with every set (5 or 6 mics, never only the square); with CH7 cleared (the square plus the right middle) the check flags "without CH8": 484, since dropping the right middle leaves the square; with CH7 and CH8 cleared, 484 of 624, and 20 of 20 frontal blasts sent at (0, 0). The blast check takes 1.0 s with 6 mics (7 sets). 3d-5 `sim thr 4 1 3`: rms 0.016-0.017 mm, residual mean 41-42 ns, and a bias: mean (−0.007, +0.007) mm; `k 4 0.5`: PASS, rms 0.0121 mm, mean within 0.001 mm. A wrong k shows more as a bias than as spread. |
| 2026-11-22 | 3d | Built; PC checks | Six simulated channels on CH3-CH8 with the D1/D2 mics, `sim thr`, the blast and early faults, drop-one in the solve (`measure::solveEvent`, on the board and in the replay), `reslimit` and `dropone` in NVS "cal" and in the `cfg` line, the simulator's drop and blast checks. On the PC: 14 tests pass (new: solveEvent drops a 24 µs late channel and keeps the shot within 0.05 mm; with drop-one off it isn't sent). A PC simulation of the bench tests gives the expected values in 3d. A 3c log still replays to the board's positions (a `cfg` without `lim=`/`drop=` reads as 1500 ns, no drop-one). The firmware builds. Run 3d-0 to 3d-5. |
| 2026-11-21 | 3a | 3a-5, no sensor: the search stops | **PASS (user report).** `temp` says it stopped looking after 3 tries; `temp scan` looks 3 more times. |
| 2026-11-21 | 3c | 3c-1 and 3c-5 again, `sim fault none`, logged | **PASS.** `sim random 200`: 200 of 200 solved and converged, position error rms 0.0140 mm, worst 0.0315 mm, mean (−0.0006, −0.0004) mm; rms residual mean 17 ns, worst 54 ns (the PC simulation: 0.015, 0.039, 16 and 57). `tools/replay` on the log: 200 within 1500 ns, the board's positions to the printed digit (worst 0.0000 mm). 3c done. |
| 2026-11-21 | 3c | 3c-0 to 3c-6, bare NodeMCU, `k 0.33333` | **PASS (user report).** From the logs: `mic sim`'s blast check warns as expected (484 of 624 scored, smallest residual 0, 106 ms). The solve takes 278 µs mean, 453 µs longest (`stat`, 1032 events): slower than the 20-100 µs guessed (float division and square root are slow on the ESP32), still well under 1 ms; drop-one (3d) will take about 7 times that. The replayed logs were made with 3c-4's `sim fault late 7` still on (200 + 200 + 100 shots, rms residual about 5.8 µs, position error 4.4 mm rms, none sent): the replay agreed (none within 1500 ns). It differed from the board by up to 0.0073 mm, from the ESP32's fused multiply-add; built with `-mfma -ffp-contract=fast` it matches to the printed digit in 498 of 500 shots, the other 2 one 0.001 mm step off (a last-bit difference, perhaps in the square root, or a rounding boundary). The replay's build command now has those flags. |
| 2026-11-21 | 3c | Built; PC checks | The calibration in NVS "cal" (mics with h, k and offset per channel), the solver on every event, `shot`/`cfg`/`rec` lines, shots to the app within `reslimit`, `testshot`, `stat`'s solver line, the blast check on `mic`, the simulator's h and position check, `tools/replay`. On the PC: 13 tests pass (new: CRC-32, `cfg` and `rec` lines read back bit for bit, the blast check giving the study's numbers). A log written in the firmware's format replays to the same positions; `--h 0` gives 0.38 mm rms and `--temp 19` 0.16 mm, as the simulation predicts for 3c-2 and 3c-3. The firmware builds (RAM 13 %, flash 56 %). Run 3c-0 to 3c-6. |
| 2026-11-21 | 3a | Built: the search stops | At the user's request, the air task no longer looks for a sensor once a minute for ever: 3 tries (boot, 1 and 2 min), then it stops until `temp scan`. Run 3a-5. |
| 2026-11-21 | 3a | 3a-1 recheck: `sim at 0 0 1`, `temp`, no sensor | **PASS (user report).** The run's first line now gives the firmware's air (18.0 °C, 342.74 m/s), the same as the event lines; `temp` says no sensor on J5 and the assumed values. 3a-1 done. |
| 2026-11-21 | 3a | 3a-2, the sync pin on GPIO 12: bare NodeMCU, `mask 0`, `span 300`, `deadtime 30` | **PASS (user report).** All four self-tests and both `sim raw` runs pass, 0 late starts. `selftest 1050 200`: 200 of 200 pairs, 95 × 1000 ns and 105 × 1100 ns (52 % at 1100), mean 1052.5 ns (rounding alone ±3.5 ns); L by quarter of its DMA block 43/41/52/64; L and H in different blocks 4 times (about 1.1 expected; split pairs were measured right). `sim raw 0 100000 200000 300000 100 1500`: every line at its set time (+0 ns vs set), t0 differences exactly +100000/+200000/+300000 ns, CH8 (unit 1) not a sample off. A mistyped `sim raw 100000 200000 300000 0` was +0 on every line too. The sync loopback works on GPIO 12. The printed t0 of −499.9 ns for a 1500 ns gap shows a stored k of 0.3333, not the default 1/3 (harmless; k moves to the calibration in 3c). |
| 2026-11-21 | 3b | PC: `pio test -e native`, 9 tests | **PASS.** Speed of sound: 160 cases, within 5e-7 m/s of `model.py`. BMP280 datasheet example exact. Noise-free 5 mm grid, h 10.5 and 21.5 mm, D1/D2 layout: 3362 points within 0.00008 mm, at most 6 iterations, never the second start. 50 ns noise per channel, 4000 shots at each of 5 points: the rms error matches the reported σ within 2 % (0.010 mm in the centre, 0.019 mm at (80, 80)), bias under 0.0003 mm. Vectors: 310 fits and 100 drop-one cases agree with Python to 0.00006 mm (residuals 0.26 ns, τ 0.11 ns, σ 0.005 %); every late or early channel found; all 60 outside points converged to the truth; none of 60 blasts scored. 0.35 µs per solve on this PC. **Found on the way:** the final tiny step was dropped when the cost didn't fall (float can't resolve it at the minimum), leaving up to 0.0008 mm; the step is now always taken. |
| 2026-11-21 | 3a | 3a-1, `sim at 0 0 1`, no sensor | **PASS (user report).** The event's air line: 18.0 °C, 50 %RH, 1013.2 hPa, each "(assumed)", c 342.74 m/s, as expected. But the run's first line still showed the simulator's own air (20.0 °C, 343.4 m/s, its old linear formula), different from the firmware's. Harmless without a solver, confusing all the same: the simulator now takes the firmware's air by default (`sim temp <C>` to differ on purpose). Recheck: the first line of `sim at 0 0 1` says "air 18.0 °C (the firmware's, 342.74 m/s)". `temp` still to check. |
| 2026-11-21 | 3a | Built, not tested yet | The air sensor: BME280/BMP280 on I2C (GPIO 14/15) or DS18B20 (GPIO 14), found at boot and once a minute while none answers; T, RH, p and the speed of sound (Cramer) on event lines and `temp`. Checked on the PC: the BMP280 datasheet example (25.08 °C exact; 100653.25 Pa, as an exact integer recomputation) and the Cramer reference values to 0.001 m/s. The sync loopback on GPIO 12. Run 3a-1 to 3a-4. |
| 2026-11-21 | 2 | 2d without a DS18B20 | **PASS (user report).** `temp` says 18.0 °C assumed; event lines end in "air 18.0 °C (assumed)". With a sensor: when the host board arrives (real-board test 5). Step 2's simulated tests are done. |
| 2026-11-21 | 2 | 2c retest after the relink fix: double shots at `deadtime 1`, `selftest 1050 200`, `sim random 100` | **All PASS (user report); 2c done.** No second shot lost or partial; the self-test and random shots unchanged by the fix in the capture's main path. |
| 2026-11-21 | 2 | 2c retest, `span 300`: double shots, `deadtime 30` then `deadtime 1` | `deadtime 30`: **PASS** (20 ignored inside the dead time). `deadtime 1` (1.4 ms in effect): FAIL, 7 MISSED and **13 lost: a capture bug.** The first ring block after a relink was never scanned (its ring neighbour is from before the freeze, so it was skipped), a 200 µs blind spot after every event. Here the relink came ~2.3-2.5 ms after the first edge, so each second shot landed in the spares (MISSED) or in that block (lost). Fixed: that block is scanned against the last spare block's last sample, and a window opened there starts at it (no pre-block, the spare's sample as the state before). A shot found in a spare finished after the relink also gets its dead time. |
| 2026-11-21 | 2 | 2c retest: double shots, `deadtime 30` then `deadtime 1`, then verbose | `deadtime 30`: **PASS**, 20 of 20 second shots ignored inside the dead time. `deadtime 1`: FAIL (14 MISSED, 5 partial, 1 lost). Verbose showed why: the stored mic span was still 600 mm (step 1 test 8), so the window covered 2.4-2.6 ms and the simulator's lines stayed high 1.62 ms (every line falls at the end of the run, which was as long as the window allowed). (1) A second shot 2.5 ms later sometimes rose inside the first window and was merged in, unreported ("lost"): **the dead time is now never shorter than the window.** (2) A missed shot's lines outlasted the restarted 1 ms dead time, so their falls still opened a partial window: the simulator now ends each run 10 µs after the shot's last rise. Rerun with `span 300`. |
| 2026-11-21 | 2 | 2c: `sim fault double 2500`, `sim at 0 0 20` (the stored dead time was still 1 ms from step 1's flood test) | **FAIL, two bugs, the capture's main duty held:** all 20 second shots (2.5 ms after the first, while the ring was frozen) were reported `MISSED`, none lost. (1) Capture: in 9 of 20 a junk event followed, all lines already high at its start and no rising edges: the missed shot's remainder opened a window after the unfreeze. Fixed: a freeze that saw a missed shot restarts the dead time when the ring is relinked. Open: those windows started 4.8-4.9 ms after the first edge, while the second shot's lines should have been high only from 2.5 to ~3.7 ms; a verbose rerun shows where the edges really are. (2) Simulator: it counted those as "measured"; now a second shot is measured (with t0s), MISSED, ignored (dead time), lost, or a partial event (FAIL). The first run read `sim at 0 0 20` as 1 shot: a typo corrected with backspace reached the firmware as a backspace character; backspace now edits the line. |
| 2026-11-21 | 2 | 2c: `sim fault missing 8` and `sim fault honly 8`, `sim at 30 30 20` each | **PASS both.** CH8 had no t0 in all 20 shots, as set (no edges; H without L: t0 needs both, so none from H alone); CH6/CH7 timed as without the fault, every shot seen. Step 4 rejects shots without a t0 on every channel. |
| 2026-11-21 | 2 | 2c: `sim fault late 7`, `sim at 30 30 20` | **PASS.** CH7 +23998.4 ns against the true arrival in every shot (24 µs less one 25 ns tick), the others within the ticks; against the edges sent all within the limit. CH7, the nearest mic, still came first, 223 µs ahead of CH5 instead of 247: step 4's fit residual must reject such shots. `offset clear` logged a NOT_FOUND error per key never stored (harmless); it now removes only stored keys. |
| 2026-11-21 | 2 | 2c: `sim offset 6 500`, `sim random 50`; then `offset 6 500`, `sim random 50` | **PASS both.** A 500 ns delay on CH6 alone: CH6 +500.8 ns against the true arrival (others ~0), still PASS against the edges sent. With the firmware's offset set to match: CH6 −0.8 ns, all within the ticks (worst 20-33 ns). The offset setting takes a channel's own delay out exactly. |
| 2026-11-21 | 2 | 2c: `sim random 500`, app connected, renames during it | **PASS.** 500 of 500 seen, nothing missed or late through the flash writes. Against the edges sent: rms 54.6/52.0/53.1 ns (56 predicted for independent edges; L and H of a channel are a fixed gap apart, which lowers it a little), means +2.3/+0.5/−0.7 (±7.5 allowed), worst 125 (limit 197). Against the true arrival: means within ±0.3 ns, worst 30-36: the generator's ticks only. Still in 2c: offsets and faults. |
| 2026-11-21 | 2 | 2b: `sim model sine`, `sim random 100` (k 1/3) | **PASS; 2b done.** Against the edges sent, with unrelated sample phases at last: rms 56.5/58.3/55.4 ns for a predicted 56, means −4.7/+1.0/−6.2 (±17 allowed), worst 108-125 (limit 197): the capture's error is the 100 ns rounding and nothing else. Against the true arrival: mean ~0, worst 28-29 ns. The straight-line formula leaves about −76000/a³ ns on a sine rise (a = amplitude at the mic): −29 ns at the far corner at amp 40, comparable to the generator's 25 ns ticks: about 0.01 mm. Optional: `sim amp 15` shows the formula's limit (several hundred ns far away). |
| 2026-11-21 | 2 | 2b: `k 0.25`, `sim at 60 -30 50` | PASS against the edges sent (limit 180 ns for this k). Against the true arrival −31.7/−20.6/+44.1 ns, as predicted to the ns: the linear model trips L 1.91 ns per mm of distance late, so k = 0.25 instead of 1/3 leaves 0.48 ns per mm of distance difference (−93.7/−38.9/+32.0 mm: −44.7/−18.6/+15.3), plus the ticks' +14/−2/+29. Small here (≤ ~100 ns, 0.03 mm) because amp 40 rises fast (t2 − t1 0.5-1.2 µs); slower real pulses make k matter more: step 3 fits it. |
| 2026-11-21 | 2 | 2b: `sim at 60 -30 100` | **PASS.** t0 differences match the geometry by hand (mics 181.2/87.5/142.3/213.2 mm, 343.4 m/s: −273.0, −113.3, +93.2 µs vs CH5). Against the edges sent: means +4.0/+1.3/+1.0 ns, rms 67/59/17, worst 100 (limit 197); at a fixed point each channel takes only 2-3 values (the phase pattern between channels is fixed), so the ~56 ns rms needs random points (2c). Against the true arrival: +14/−2/+32 ns, constant: the generator's 25 ns ticks (up to ~42 ns on a difference), not the firmware; the linear formula with k = 1/3 adds nothing. |
| 2026-11-21 | 2 | 2b: `sim at 0 0 100` | PASS, but a trivial case: at the centre all four mics are equidistant, so every channel gets the same edges at the same sample phase and every error is exactly 0 (not the ~56 ns rms that unrelated phases give). t2 − t1 800/900 ns in about equal numbers, for a true 851 ns (148.5 mm, amp 26.9): as the model says. The informative one is `sim at 60 -30`. |
| 2026-11-21 | 2 | 2a: `sim raw 0 100000 200000 300000 100 1500` | **PASS; 2a done.** 100 of 100 runs exact on every line: L at the set times to the sample, H 1500 ns later, t0 differences exactly +100000/+200000/+300000 ns, across DMA block boundaries. Generator, unit sync, 4-channel capture and t0 arithmetic confirmed. |
| 2026-11-21 | 2 | 2a: self-test on the new generator (`selftest`, `selftest 1050 100`, `selftest 0`, `selftest 1000000`) | **PASS**, 0 late starts, no wrong pulse lengths in 160 pairs (so the lines fall at the end of each run, as meant). 1000 and 0 ns exact every time. 1050 ns: 49 × 1000, 51 × 1100, mean 1051.0 (rounding alone ±5.0). 1 ms at 25 ns ticks: 20 of 20 exact, all split across blocks — the case that failed in step 1 before the count reset. Still in 2a: `sim raw` at different times. |
| 2026-11-21 | 2 | 2a: `sim raw 0 0 0 0 200`, bare NodeMCU | **PASS.** 200 of 200 runs, 0 late starts, nothing missed. Every line of CH5-CH8 at the same sample as CH5 L in every run, H exactly 1000 ns after L; t0 −333.3 ns on all four (k = 1/3), differences 0. CH8 (MCPWM unit 1) never a sample off CH5-CH7 (unit 0) in 200 runs at random sample phases: the units start within about 1.5 ns (95 %), so no correction is needed. Still to run in 2a: the self-test on the new generator, `sim raw` at different times. |
| 2026-11-21 | 2 | Built, not tested yet | t0 and its settings, the pulse generator (self-test moved onto it), the simulator on CH5-CH8, the DS18B20 task. Run 2a-2d in order; 2a also re-checks the self-test on the new generator. |
| 2026-11-21 | 1 | Host board v0.3 map (no test yet) | The pins moved: every channel's L and H, IR_EN 16 → 27, DQ 17 → 14 (`config.h`). CH8, and so the self-test, is now on GPIO 4 (L) and 16 (H); the self-test pins follow the map. Nothing else in the capture depends on which GPIO a line is on. Retest: test 3 (self-test on the new pins), test 14 (every line, jumper from GPIO 4), and test 15 once. |
| 2026-11-20 | 1 | Extra and break-it tests after the boundary, summary and serial fixes | All pass (user report). Pin map of CH1/CH2 (GPIO 36, 39, 34, 35) confirmed with jumpers from GPIO 19, so all 16 lines are now checked except CH5-CH7, which go through the same routing. Flood with a finger on CH1, `deadtime 1`, `verbose on`: test shots and heartbeat reach the app, commands work. Step 1 closed; step 2 waits for the sensor boards. |
| 2026-11-20 | 1 | Break-it: `mask 03` floating, then CH1 jumpered, then a finger on CH1 with `verbose on`, app connected | Floating inputs alone stay quiet (no events). Jumper GPIO 19 → CH1: CH1 had edges with CH8, PASS (other lines are a note, not a failure). Finger: a stream of events, heartbeat kept coming, **test shots didn't**: `loop()` printed every queued event and blocked on the UART once the 2 KB buffer was full, so the button was sampled too rarely to see a short press. Events are now printed only while they fit in the serial buffer (the rest wait in the queue, or count as not printed), transitions only with room to spare. Retest 15. |
| 2026-11-20 | 1 | `selftest 1050 100` | PASS: 54 at 1000 ns, 46 at 1100, mean 1046.0 (rounding alone: ±5 ns over 100), L in all four quarters, 0 split across blocks (expected 0.5). Correct, but the summary read as wrong ("0 exact", mean ≠ set). It now says that a gap between two samples can only read as either, gives the expected share, the mean's rounding error (flags a mean more than 3× off: pulses locked to the sample clock) and the expected number of split pairs. |
| 2026-11-20 | 1 | Span 600, long gap | **Capture bug found:** `CH8 -/0, CH8 L was already high at window start`. The activity scan looked at each block on its own, so an edge exactly between two blocks (low all through one, high all through the next) was invisible; the window then opened at the next activity (H, ≥ 2 blocks later) and started after L. About 1 in 2000 edges; only possible to hit once pulses landed at random points in a block. The scan now folds in the previous block's last sample. `stat` counts events with a line already high at window start (should stay 0). |
| 2026-11-20 | 1 | Extra test 11 (long gaps) | 500 µs and 750 µs (12.5 ns ticks): 20 of 20 each, exact, all pairs split across blocks, L in all four quarters: PASS. 1 ms (25 ns ticks): only the first pulse pair of each run came out (1 of 20, 1 of 100), and that one was exact: the MCPWM one-shot restart from the peak failed at that prescale, not the capture. The count is now reset to 0 (soft sync) before each start. Retest 11. |
| 2026-11-20 | 1 | Phase check, app connected, 20 + 100 + 300 pulses, 2 renames, 31 test shots | 1000 ns: 20 exact. 1050 ns with the random start: 37 % then 51 % rounded up (189 of 400 = 47 %), mean 1050.7 over 300: the capture resolves gaps finer than a sample when averaged. All within one sample, nothing missed. But every pulse landed at the same point in its DMA block (timestamps …4698): block boundaries, the trigger block's end and long gaps were untested; extra tests 9-13 added. |
| 2026-11-20 | 1 | Retest 4 (MCPWM generator), app connected, 20 + 300 pulses, 2 renames, 30 test shots | **Pass.** 1000 ns: 20 of 20 exact. 1050 ns: 300 of 300 at 1000 or 1100, none further off, nothing missed, through heartbeats, renames and shots; the 4100 ns pulses are gone, so they were the CPU generator, not the capture. Mean 1018 instead of ~1050: the generator, sample clock and CPU share one crystal, so the pulses started at almost the same point within a sample (timestamps all …856); random 0-200 ns start delay added. Step 1 done. |
| 2026-11-20 | 1 | Retest 3, app connected, 200 pulses, 2 renames, 3 test shots | With `memw` the read-back ranges were narrow (≤ 267 ns), and showed the CPU generator really making ~150 ns gaps for a set 1000 ns while it read its pins back; the capture agreed (100/200 ns). At three heartbeat times the read-back said ~150 ns and the capture 4100 ns: undecided which is wrong. A CPU generator that changes when checked can't settle that, so the self-test now uses MCPWM (hardware-timed gap). Retest. |
| 2026-11-20 | 1 | Retest 2, app connected, 400 pulses, 2 renames, 18 test shots | Checker passed a broken generator: gaps captured 100-200 ns for a set 1000 ns, yet PASS. Without a barrier, the read-back loop held the write of L up by ~0.85 µs, and each edge's range was then ~1 µs wide, so every gap fit. Fixed with `memw` after each pin write; ranges over 300 ns no longer count as verified, and ≥ 95 % must be. Every 40th pulse (2 s: the heartbeat's BLE notification) the generator is ~3 µs late. Time base still right: 10.000 MHz, 420 events, interrupt 9.0 %, longest freeze 1.0 ms. Retest. |
| 2026-11-20 | 1 | Retest, app connected, 400 pulses, 2 renames, 11 test shots | Time base fixed: 10.000 MHz after 420 events, timestamps 50.0 ms apart. Interrupt 9.3 %, longest freeze 1.0 ms, nothing missed. Generator held up 3 times (5000 ns at the pins): captured 5000 ns each time. One FAIL from the checker, not the capture: timing H from L's read-back made the pin gap ~1060 ns, captured as a 40/60 mix of 1000 and 1100 (mean ~1060, sub-sample consistent); one read-back 60 ns late under BLE gave "pins 1058, captured 900". The check now brackets each edge between reads and times H from L's write again. Retest. |
| 2026-11-20 | 1 | Bare NodeMCU-32S, USB, app connected for item 6 | Sample rate 10.000 MHz; interrupt 9.8 % of core 1, longest 22 µs. Self-test exact to the sample at 0, 500, 1000, 10 000 and 100 000 ns; pulses 11.0/10.0 µs. Pin map: jumpers on GPIO 32 (CH3 L) and 25 (CH4 L) at the same sample as CH8 L. Span 600 mm: 2400 µs. Freeze: no wrong windows, longest 1.6 ms, nothing missed. 119 KB heap, 104 KB DMA free. **Two faults, both outside the sampling:** (1) gaps of 0-7200 ns instead of 1000 on the first pulse of a run and right after BLE indications and renames: the generator ran from flash (cache misses) and its pin writes waited behind bus traffic; moved to IRAM, and the capture is now checked against the pins' read-back. (2) Time base lost ~3 blocks per event (9.996 MHz after 849 events; pulses 50 ms apart read 49.4 ms): spare blocks finished after unfreezing weren't counted; fixed. Retest. |

## Open questions

- Sample rate: 10 MHz to start; 20 MHz with a bigger ring if 100 ns turns out too coarse
  on the loudest channel (t2 − t1 ≈ 0.3 µs).
- Dead time vs rapid-fire disciplines. 30 ms was a starting value, not derived. A fast pellet at
  10 m can land inside the muzzle blast's dead time (PLAN phase 2, item 4, with the numbers).
