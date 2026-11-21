# Measurement firmware: plan and bench results

How the firmware turns the comparator lines of the acoustic sensor boards into shots. The
hardware and the design decisions behind it (mics, preamp, thresholds, error budget) are in
the project summary doc ("Electronic shooting target – project summary"); the sensor board
is in `hardware/acoustic-sensor/`. This file is the firmware's plan: it changes in the same
commit as the code, and every bench test result goes into [Bench results](#bench-results).

## Scope

- 4 corner mics to start, up to 8 channels (the host board has 8 connectors). Each
  channel gives two comparator lines: OUT_L (low threshold, trips first) and OUT_H (high
  threshold, a few µs later). Lines idle low and go high while SIG is above the threshold.
- **No IR light curtain** for now; it comes later. IR_IN (GPIO 13) is unused and IR_EN
  (GPIO 27) is held low.
- Results go out over **serial** (115200) until step 5. The BOOT test shot stays the shot
  source until step 3 has a solver. No protocol change before step 5.
- Accuracy target after calibration: 0.2–0.5 mm (1σ). Every 1 µs of timing error is about
  0.37 mm, so the firmware must add well under 0.1 µs.

Without the curtain, two cases stay uncovered until it is added: a shot through an
existing hole (almost no crack: missed), and a blast from a neighbouring lane that fits as a
hit about 10 mm from centre on the centre lines (only absolute level and edge sharpness can
reject it). Logging every rejected event matters more because of this.

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

Other: IR_IN 13, IR_EN 27, DQ 14 (DS18B20, 4.7 k pull-up), STAT 2 (blue LED + panel LED),
BOOT 0. Spare: 5 (the capture clock loopback), 12, 15 (the pulse generator's sync loopback). Empty connectors are held low by 100 k pull-downs. A breadboard
setup uses the same map, so nothing changes when the host board arrives.

## Pipeline

| Stage | Runs on | Does | Output |
|---|---|---|---|
| I2S parallel capture | DMA | Samples the 16 lines at ~10 MHz into a ring of ~64 KB (~3 ms) | 16-bit samples |
| Activity scan | DMA end-of-block interrupt (IRAM) | ORs each finished block; on activity, marks the event window and freezes the ring around it | Event window |
| Edge extraction | Measurement task, core 1 | First rising edge per line in the window, plus pulse length | t1 (L), t2 (H) per channel |
| Arrival times | Measurement task | t0 = t1 − k·(t2 − t1) − offset per channel | t0 per channel |
| Position solver | Measurement task | Time-difference fit over 4–8 mics, speed of sound from the DS18B20 | x, y in mm, residual |
| Validity checks | Measurement task | Residual, loudness pattern, edge sharpness, dead time | Shot or reject reason |
| Shot buffer | `loop()` (existing) | Indicates shots to the app, resends after a link drop | Protocol v2 shot |

Tasks: NimBLE runs on core 0, and so does the DS18B20 housekeeping task (once a minute).
Arduino's `loop()` runs on core 1; it stays the light
10 ms loop (button, LED, BLE requests, shot buffer). The measurement task runs on core 1 at
a higher priority and hands finished shots to `loop()` through a queue. The DMA interrupt
and the code it calls live in IRAM, because NVS writes (name, PIN) stall the flash cache.

Code: `capture.cpp/.h` (I2S, ring, activity scan, edge extraction, output, commands, the
self-test), `event.h` (the event result), `pulsegen.cpp/.h` (MCPWM pulse pairs for the
self-test and the simulator), `arrival.cpp/.h` (t0), `simulate.cpp/.h` (the sensor
simulator), `airtemp.cpp/.h` (DS18B20), later `solver.cpp/.h`. Pins and timings go in `config.h`; a channel mask says which connectors
are fitted.

## Steps

Each step ends with a bench test the user runs. The next step starts only when it passes,
so a problem shows up in the step that caused it.

| # | Step | Status |
|---|---|---|
| 1 | Capture and raw dump | done on the v0.2 map; on the v0.3 map: retest 3 (self-test) and 14 (pin map) |
| 2 | Arrival times | built; 2a and 2b pass, 2c-2d to run; real-board tests wait for the boards |
| 3 | Solver and calibration | planned |
| 4 | Validity checks | planned |
| 5 | Protocol and app | planned |
| 6 | Wi-Fi and MQTT | planned |

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
  the previous event was still being read.
- Edge extraction: XOR with the previous sample, then the changed bits; stop once every
  line has its first rising edge. Times in ns from the window start; block numbers from
  the DMA descriptors.
- Dead time after an event, 30 ms to start (a setting).
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
  Both timers take their count from edges on GPIO 15 (looped back inside the chip): a sync
  edge sets both stopped counts to 0, both timers are started, a second edge sets both to 0
  again at the same moment. All from IRAM with interrupts off, by register (the driver's
  start runs from flash); the rises come ≥ 20 µs later, and every run that took longer to
  start is counted as a late start. The self-test now runs on it (1 channel).
- **Simulator** (`simulate.cpp`, `sim ...`): 4 mics on CH5-CH8 (GPIO 22/23, 19/21, 17/18,
  4/16; CH1/CH2 are input-only) at the corners of a 210 mm square, a speed of sound from a
  simulated temperature (20 °C), and a trip model: the pulse rises like A·sin(πt/2T)
  (`sim model sine`) or its straight start (`linear`, default), T = 12 µs, OUT_L trips at 1
  and OUT_H at 4, A = amp·100 mm / distance (`sim amp`, default 40). 25 ns ticks; each shot
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

Bench test, simulated (bare NodeMCU, nothing on CH5-CH8 and GPIO 15), `mask 0`:
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

Build:
- TDOA solver: single-precision Gauss–Newton, 3–5 iterations from the first-hit mic,
  speed of sound from the DS18B20. Output x, y and the fit residual.
- Calibration mode: prints raw arrival times per spark. The fit of mic positions, offsets
  and k over 9–16 template points runs on the laptop (a script first, the app in step 5);
  the firmware stores the results in NVS.
- Solved shots replace the BOOT test shot as the shot source (the BOOT test shot stays
  available as a setting).

Bench test: spark through a template at 9–16 known points, then at other known points.
Done when: positions are within 0.2–0.5 mm after calibration.

### 4. Validity checks

Build: in order, all channels inside the window (a channel with no edge: reject); acoustic pulse shape (t2 − t1 in range, not
identical on all channels); fit residual ≤ ~1.5 µs; inside the scoring area; loudness
matches distance; edge sharpness; outside dead time. Every event is logged with its reject
reason.

Bench test: on the range, with other lanes firing. Done when: no false shots from blasts
or electrical spikes, and real shots are accepted.

### 5. Protocol and app

Raw-event message (t1/t2 or t0 per channel, fit residual, accept/reject reason, and
`MISSED` events so the shooter is told), calibration-mode switch, reading and writing
calibration data. **The mic span becomes an app setting** (now serial `span`), with the
frame geometry; once the calibration (step 3) knows the mic positions, the app can fill
it in from them. Follows the repo's protocol
rule (PROTOCOL.md, `protocol.h`, `protocol.rs` in one commit). The shot message stays as it
is.

### 6. Wi-Fi and MQTT

FUTURE_ARCHITECTURE.md step 2. Check the GPIO 36/39 erratum here (see Risks).

## Risks

- **I2S parallel capture on IDF 5:** register-level, untested here; settled by the step 1
  self-test. Fallback: MCPWM capture.
- **Relinking descriptors while the DMA runs (ring freeze):** assumes the DMA reads a
  descriptor's next link when it gets to it, as esp32-camera relies on. If not, the
  self-test shows wrong or repeated windows; fallback: stop and restart the DMA.
- **GPIO 36/39 (CH1):** ESP32 erratum: spurious pulses on these pins while Wi-Fi or ADC1
  is active. Not an issue with BLE only; check again in step 6.
- **The two MCPWM units starting together:** both take their count from one edge on
  GPIO 15, but their prescalers may not line up: up to a tick (25 ns) between CH5-CH7 and
  CH8. Settled by test 2a; if it is more, or not stable, the simulator corrects for a
  fixed offset or drops to 3 channels.
- **Edge shape, mic polarity, half-cycle-late trips, range noise:** can't be settled on
  paper; all show up in the step 2 dump.

## Bench results

Newest first. Date, step, setup, what was measured, and what it changed.

| Date | Step | Setup | Result |
|---|---|---|---|
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
- Dead time vs rapid-fire disciplines.
