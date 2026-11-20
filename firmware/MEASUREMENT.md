# Measurement firmware: plan and bench results

How the firmware turns the comparator lines of the acoustic sensor boards into shots. The
hardware and the design decisions behind it (mics, preamp, thresholds, error budget) are in
the project summary doc ("Electronic shooting target – project summary"); the sensor board
is in `hardware/acoustic-sensor/`. This file is the firmware's plan: it changes in the same
commit as the code, and every bench test result goes into [Bench results](#bench-results).

## Scope

- 4 corner mics to start, up to 8 channels (host board v0.2 has 8 connectors). Each
  channel gives two comparator lines: OUT_L (low threshold, trips first) and OUT_H (high
  threshold, a few µs later). Lines idle low and go high while SIG is above the threshold.
- **No IR light curtain** for now; it comes later. IR_IN (GPIO 13) is unused and IR_EN
  (GPIO 16) is held low.
- Results go out over **serial** (115200) until step 5. The BOOT test shot stays the shot
  source until step 3 has a solver. No protocol change before step 5.
- Accuracy target after calibration: 0.2–0.5 mm (1σ). Every 1 µs of timing error is about
  0.37 mm, so the firmware must add well under 0.1 µs.

Without the curtain, two cases stay uncovered until it is added: a shot through an
existing hole (almost no crack: missed), and a blast from a neighbouring lane that fits as a
hit about 10 mm from centre on the centre lines (only absolute level and edge sharpness can
reject it). Logging every rejected event matters more because of this.

## Pins (host board v0.2)

| Channel | OUT_L | OUT_H |
|---|---|---|
| CH1 | 36 | 39 |
| CH2 | 34 | 35 |
| CH3 | 32 | 33 |
| CH4 | 25 | 26 |
| CH5 | 27 | 14 |
| CH6 | 4 | 23 |
| CH7 | 22 | 21 |
| CH8 | 19 | 18 |

Other: IR_IN 13, IR_EN 16, DQ 17 (DS18B20, 4.7 k pull-up), STAT 2 (blue LED + panel LED),
BOOT 0. Spare: 5, 12, 15. Empty connectors are held low by 100 k pull-downs. A breadboard
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

Tasks: NimBLE runs on core 0. Arduino's `loop()` also runs on core 1; it stays the light
10 ms loop (button, LED, BLE requests, shot buffer). The measurement task runs on core 1 at
a higher priority and hands finished shots to `loop()` through a queue. The DMA interrupt
and the code it calls live in IRAM, because NVS writes (name, PIN) stall the flash cache.

Code: a new `capture.cpp/.h` (I2S, ring, activity scan, edge extraction), later
`solver.cpp/.h`. Pins and timings go in `config.h`; a channel mask says which connectors
are fitted.

## Steps

Each step ends with a bench test the user runs. The next step starts only when it passes,
so a problem shows up in the step that caused it.

| # | Step | Status |
|---|---|---|
| 1 | Capture and raw dump | done (bench test 5); extra tests (block boundaries, long gaps, short pulses) pending |
| 2 | Arrival times | planned |
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
- Activity scan in the end-of-block interrupt; on activity, the measurement task reads an
  event window that starts one block (200 µs) before the first activity.
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
  the pulse length of each line; quiet lines marked `-`. A verbose mode lists every
  transition of every line in the window (ringing, double trips).
- Serial commands (type `help`): `stat`, `mask <hex>`, `deadtime <ms>` (both stored in
  NVS, namespace `"measure"`), `verbose on|off`, `selftest [ns] [n]`.
- Self-test: MCPWM drives a pulse pair out on CH8's own pins (GPIO 19 = L, 18 = H), OUT_H
  a set gap after OUT_L (12.5 ns steps), both falling together. The I2S reads them back
  on the same pins, so no jumpers are needed; CH8 must have no sensor board plugged in.
  A jumper from GPIO 19 to another input checks that input. The gap is made by the timer
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
   pulse), no extra transitions, PASS. `selftest 1050 200`: a mix of 1000 and 1100,
   about half rounded up, mean close to 1050 (each pulse starts at a random point
   within a sample, as real sound does). Then `selftest 0` (no skew between
   lines: gap 0), `selftest 500`, `selftest 10000`, `selftest 100000`.
4. `verbose on`, `selftest 1000 3`: each CH8 line shows one + and one -, pulse lengths
   about 11 µs (L) and 10 µs (H).
5. Pin map: `mask FF`, a jumper from GPIO 19 to one input at a time, `selftest 1000 3`:
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
    shots during it. About 50 % rounded up, PASS; `stat` after.

Done when: every pulse shows up once, at the right time, on the right line, with BLE busy.

Fallback if I2S parallel capture can't be made to work: MCPWM capture (12.5 ns, 6 inputs,
so three channels with both lines), and revisit the design for 4+ channels.

### 2. Arrival times

Build:
- t0 = t1 − k·(t2 − t1) − offset per channel; k and offset as settings (k = 1/3, offset
  0 to start), stored in NVS under their own keys so firmware updates keep them.
- The dump adds t2 − t1, t0, and the t0 differences to the first channel.
- Read the DS18B20 every few seconds and print the temperature with each event (the solver
  needs it in step 3).

Bench test (sensor boards and host board):
1. Per board, the initial calibration from the project doc (VMID, gain, thresholds).
2. Piezo-lighter spark or snapped pencil lead near one mic: OUT_L before OUT_H, and the
   verbose dump matches the scope on SIG and OUT_L/OUT_H.
3. Mic polarity: SIG must first go up. A half-cycle-late trip shows as about 24 µs extra
   on one channel.
4. Spark at the centre of the mounted frame, 20 times: the four t0 agree within a few
   hundred ns after per-channel offsets.

Done when: centre sparks give consistent t0 on all four channels, polarity is confirmed,
and there are no half-cycle-late trips.

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
- **Edge shape, mic polarity, half-cycle-late trips, range noise:** can't be settled on
  paper; all show up in the step 2 dump.

## Bench results

Newest first. Date, step, setup, what was measured, and what it changed.

| Date | Step | Setup | Result |
|---|---|---|---|
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
