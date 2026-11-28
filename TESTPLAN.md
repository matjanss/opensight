# OpenSights: test and calibration plan

From the first board on the bench to a calibrated target on the range, and the routine that
keeps it calibrated. For each test: what it checks, why it matters for the accuracy, what
to expect and what to do when it doesn't.

This plan puts the existing tests (MEASUREMENT.md step 7, PLAN phases 1–2, the host board's
`TODO.md`) in one order and adds what was missing. Results still go into MEASUREMENT.md's
"Bench results" (the measurement rule); this file says what to run and what to expect.

**Contents:** [1 The chain](#1-the-chain-and-what-each-link-costs) ·
[2 The route](#2-the-route) · [3 Equipment](#3-equipment-and-ground-rules) ·
[A Sensor boards](#a-each-sensor-board-alone) · [B Host board](#b-the-host-board-and-the-esp32) ·
[C Boards and host together](#c-boards-and-host-together-no-frame-yet) ·
[D The frame](#d-the-frame-and-the-holders) · [E Noise floor](#e-the-noise-floor) ·
[F Calibration](#f-calibration-and-verification) · [G Systematic errors](#g-systematic-errors) ·
[H Validity checks](#h-the-validity-checks-with-real-sounds) · [I IR curtain](#i-the-ir-curtain) ·
[J The range](#j-the-range) · [K Field routine](#k-field-routine) ·
[L Fault finding](#l-fault-finding-table) · [M Sheets](#m-record-sheets)

---

## 1. The chain, and what each link costs

A shot is found from the **differences** in the time its crack reaches six mics. So the
only thing that matters is how well each mic's arrival time is measured, and how well we
know where the mics are and how fast sound travels.

```
 pellet hits paper           air              mic board (x6)                host board + ESP32            app
 ┌───────────────┐    ┌──────────────┐   ┌─────────────────────────┐   ┌──────────────────────────┐   ┌────────┐
 │ crack at the  │ →  │ c from T, RH │ → │ MEMS mic → ×12 preamp   │ → │ 10 MHz capture of L, H   │ → │ card,  │
 │ hole, in the  │    │ (BME280)     │   │ → 2 comparators:        │   │ → t1, t2 → t0 → solver   │   │ score  │
 │ paper plane   │    │ ~343 m/s     │   │   OUT_L (low) OUT_H     │   │ → verdict (+ IR curtain) │   │        │
 └───────────────┘    └──────────────┘   └─────────────────────────┘   └──────────────────────────┘   └────────┘
   D11 ground truth     E: speed (G)       A: analog, polarity, gain     B, C: pins, timing, noise       J: card photos
```

**The edge, and why there are two comparators.** The crack rises over a few µs. A comparator
trips later on a quiet crack than on a loud one (walk). Two thresholds see the same edge
twice, and the line through both points is run back to where the edge starts:

```
  SIG
   ^                                      ╱╲
   │ VTH_H = VMID+400 mV ─ ─ ─ ─ ─ ─ ─ ─╱─ ─ ─ ─  OUT_H rises at t2
   │                                   ╱
   │ VTH_L = VMID+100 mV ─ ─ ─ ─ ─ ─ ╱─ ─ ─ ─ ─ ─  OUT_L rises at t1
   │                               ╱
   │ ____________________________╱ ← the true arrival t0 = t1 − k·(t2 − t1) − offset
   └──────────────────────────────────────────────────→ time
```

k is 1/3 for a straight edge. The mic's port resonance curves the edge, so the real k is
nearer 1 and differs per board (PLAN D3). Offset is each channel's own fixed delay.

**The mics** (PLAN D1/D2, `MIC_LAYOUT_D2`; mm from the card centre; the frame is 250 × 250,
the scoring area 200 × 200). The top-right mic is moved out along the diagonal so that a
blast from the firing line can't fit as a hit:

```
   y ↑
     │  CH4 ●                                 ● CH3  (122.7, 122.7)
     │ (−105, 105)                          ╱        the square's corner (105, 105),
     │                                    ╱           moved out 25 mm on the diagonal
     │                                  ● CH6 (105, 0)
  ───┼─────────────────────────────────────→ x
     │
     │  CH1 ●          ● CH5          ● CH2
       (−105,−105)   (0,−105)       (105,−105)
```

Channel numbers as `mic default` lists them: check with `mic` before the first spark.

**What an error in each link costs in position.** One µs of timing error is 0.34 mm of sound
path. With six mics it moves the shot by about **0.21 mm** (1σ, long axis, ring area; 0.35 mm
near the corners). So 0.2 mm needs about 0.9 µs per channel, and 0.5 mm about 2.3 µs.

| Error source | Size | Effect on a shot | Who removes it | Tested in |
|---|---|---|---|---|
| Sampling (100 ns ticks) | 0.03–0.06 µs | about 0.01 mm | nothing needed | C4 |
| Noise on the edges (random) | to find out | 0.21 mm per µs | six mics average it; it is the floor | E1 |
| Fixed delay per channel (comparator offset, threshold, cable) | up to a few µs | 0.2–0.4 mm per µs | calibration: offset | F1 |
| k, the edge's curve | 1/3 assumed, ~1 real | 0.1–0.2 mm (summary) | calibration: k per channel | F3 |
| Edge shape changing with loudness | 2–12 µs build-up | **the largest left after calibration** | dual thresholds; limited by physics | G1 |
| Trip one half-cycle late | ~24 µs | ~8 mm (an outlier) | polarity right; drop-one | A5, E1 |
| Mic positions (±0.1 mm) | set by holders | about 1 : 1 | calibration fit | D2, F1 |
| Mic height h (5.5 mm nominal) | a holder gap adds to it | a small, steady bias | calliper per mic (not fitted) | D3 |
| Speed of sound: 1 °C | 0.6 m/s | 0.15 mm rms | BME280 (needs ±0.3 °C) | B6, G2 |
| Speed of sound: 30 → 60 % RH | 0.3 m/s | 0.08 mm rms | BME280 | B6 |
| Template holes (the ground truth) | to measure | adds 1 : 1, **invisible to the sparks' own verification** | measure it; the range cards (J1) | D4, J1 |
| Spark is not a pellet | unknown | a bias that sparks can't show | real shots against cards | J1 |
| Filter capacitors ±10 % | ≤0.17 µs | ≤0.06 mm, mostly fixed | offset | – |

Two lines of this table drive the plan. **Noise** (E1) says whether 0.2–0.5 mm is possible at
all, since calibration can't remove it. **Edge shape versus loudness** (G1) and **the
spark against the pellet** (J1) are what calibration can't fully remove, and they decide
what accuracy the range will see.

---

## 2. The route

Each gate must pass before the next stage. A failure sends you back to the stage named in
the last column.

| Stage | Question it answers | Gate to pass | Else go to |
|---|---|---|---|
| **A** each sensor board | Is each channel healthy, and is the polarity right? | SIG goes up first; thresholds set; gains match | A (rework) |
| **B** host board and ESP32 | Do the pins, rails, air sensor and radio behave? | `selftest` PASS; no false events at rest | B |
| **C** together, no frame | Does each mic land on the right channel without noise? | all six channels respond; sim PASS on the real board | A, B |
| **D** the frame | Are mics where the model says, and is sound only coming through the air? | positions and h measured; template checked; no frame-borne trips | D |
| **E** noise floor | How good can it be? | long axis ≤ 0.5 mm (best ≤ 0.2) from 50 sparks | A (thresholds, gain) |
| **F** calibration | How good is it after calibration? | 1σ per axis ≤ 0.5 mm; no point's bias over 0.5 mm | E, G |
| **G** systematic errors | Does it hold with temperature, loudness, a board swap, a transport? | the numbers in G | F |
| **H** validity | Are blasts and noise refused, real shots kept? | no false hit on the test sounds | H |
| **I** IR curtain | Does the gate see every pellet and nothing else? | every dropped pellet seen | I |
| **J** the range | Real shots against scanned cards, with other lanes firing | to be set after F (see J) | G, H, I |
| **K** field routine | Is it still calibrated today? | the field check inside its limit | F |

**Stop rules.** Two failures stop everything:
1. **SIG goes down first** on the first board (A5): the circuit is wired the wrong way.
   Stop before ordering or building more boards.
2. **E1's long axis is over 0.5 mm** with the thresholds set right: the goal can't be met by
   calibration. Look at the analog side (gain, thresholds, mic port) before calibrating.

---

## 3. Equipment and ground rules

| Item | Used for |
|---|---|
| Oscilloscope, 2+ channels, 20 MHz or more | SIG, OUT_L, OUT_H, rails, ripple |
| Multimeter with mV range | VMID, thresholds, current |
| Piezo lighter (spark) and 0.5 mm pencil leads | the sound source; the spark gap sits **at the paper plane** |
| Fixed spark stand | E (same point, 50 times) |
| The template (4 × 4 grid ±25, ±75 mm; check point; 3 × 3 verification) | known points, on the card holder |
| Calliper (0.02 mm) | h per mic, mic positions, template |
| Reference thermometer (±0.2 °C) and a hygrometer | the air sensor (B6, G2) |
| Heater or open window | G2 |
| USB power bank and a mains USB supply | B (supply noise), J6 |
| One spare sensor board, set up as in A | G4 |
| The PC with the app, `probe`, `tools/bench.py`, `pio` | everything |
| A clap, a balloon, a cap gun or a loud click, a phone | H |
| A scanner (1200 dpi) or the phone photo workflow (C5) | J1 |
| The IR curtain parts, a pellet tube | I |

**Ground rules.**
- **Log everything.** Serial logs go to `firmware/logs/` (`--filter log2file`); the app writes
  its event logs. Note the first and last event number `n` of each test; `bench.py --n A-B`
  needs them. Name logs `YYYY-MM-DD-<test>.log`.
- **Change one thing at a time**, and write down the `cfg` id after every change. A new
  `cfg` id means the calibration is stale.
- **Don't touch RV1 after the gains are matched** (A4): a gain change shifts the timing.
- **Spark rules:** gap at the paper plane; the same lighter and gap within a series; quiet
  room (no fans, no phones near the cable); wait 1 s between sparks (dead time 30 ms is
  not the limit, the room's echo is).
- **Record sheets** are in [M](#m-record-sheets).

---

## A. Each sensor board alone

| | |
|---|---|
| **Tests** | One mic channel: the MEMS mic, the ×6.3–17.7 preamp with three 16 kHz high-pass poles, two comparators |
| **Role** | Turns a sound edge into two digital edges. Everything downstream measures what this board does to the edge |
| **Impact** | The largest: threshold levels set the walk and k; gain sets the edge's steepness; polarity decides whether a trip is on the first lobe (right) or a half-cycle late (24 µs, 8 mm) |
| **Setup** | Board on 3.3 V through J1 (or plugged into the host board). Scope ground on TP5 (GND). Meter's black lead on VMID (TP4) so it reads offsets directly |

| # | Do | Expect, and why | If not |
|---|---|---|---|
| **A1** | Look at the board: mic orientation (pin-1 dot), solder, no bridges, the 0.5 mm port hole open. Power it with a current limit: 3.3 V | About 3 mA per board. A short or a much higher draw means a fault. The mic's pads were corrected on 23 Nov: check that its pin-1 mark sits on the VDD pad | Don't power it further; find the short |
| **A2** | VMID at TP4 to GND | 1.64 V ± 30 mV (half the supply). No adjustment: it comes from two resistors, and every threshold hangs from it | Wrong resistor or a short |
| **A3** | SIG (TP1) at rest, scope on AC, 1 ms/div | Within ±10 mV of VMID, a few mV peak to peak in a quiet room. A big sine is oscillation; a 50 Hz pattern is hum. Noise decides how low VTH_L can go (the design: 100 mV is about 900 times the estimated 0.1 mV rms) | Fix before anything else: cable, ground, decoupling |
| **A4** | RV1 to one end (over 25 turns), back 12–13 turns: mid-travel, about ×12. Then RV2: VTH_L = VMID + 100 mV (TP2); RV3: VTH_H = VMID + 400 mV (TP3) | 100 mV is about 90 dB at the mic, 400 mV about 102 dB. They sit in the first, steep part of the edge, below the amplifier's clipping (about 115 dB). The 1 : 4 ratio gives k = 1/3 for a straight edge | Wiper range is VMID + 63…698 mV: outside it a resistor is wrong |
| **A5** | **Polarity.** A spark or pencil lead 50–100 mm from the mic. Scope SIG, OUT_L, OUT_H | OUT_L and OUT_H low at rest, pulse on the spark, OUT_L first. **SIG's first movement goes up.** Why: the comparators trip on a rise. If the first half-cycle goes down, they trip on the second, 24 µs late | **STOP rule 1:** do the first board alone, before more boards are made |
| **A6** | Same spark at 50 mm and at 100 mm, 5 sparks each. Read t2 − t1 on the scope | From about 0.3 µs (loud, near) up to a few µs (quiet, far). Over a single spark: OUT_L, OUT_H, then both fall. No second trip from ringing | A second trip: VTH_L too low or the ring too loud; raise RV2 and RV3 together |
| **A7** | Write the board's sheet ([M1](#m1-sensor-board-sheet)): number, VMID, VTH_L, VTH_H, RV1 turns, t2 − t1 at 50 and 100 mm | Sensor boards from one order should be within about 10 % in t2 − t1 at the same distance and gain | A board far off in t2 − t1 has a different edge shape (bad mic, part): keep it as a spare |

**Why A matters most.** Everything in the budget after A is calibrated or averaged, except
what the edge itself does. A board that rings, clips early or has a noisy SIG gives a noisy
t1, and no later stage can recover that.

---

## B. The host board and the ESP32

| | |
|---|---|
| **Tests** | The NodeMCU on its sockets, the 3.3 V sensor supply, the eight connectors with their 470 Ω and 100 k, the capture pins, the air sensor, LED and button, the IR parts' supplies |
| **Role** | Gives the boards clean power, and samples the 16 lines at 10 MHz (100 ns) into a DMA ring |
| **Impact** | Supply ripple moves thresholds (a timing error); a wrong pin map loses a channel; the clock's accuracy (10 ppm of a 1.6 ms window is 16 ns) is negligible |

**Assembly order (README "Notes for assembly"):** passives and sockets, then U3 and U4 into
sockets (not soldered), the two 1×19 NodeMCU headers **last**. Set JP1 to 3.3 V for a TLC372.

| # | Do | Expect, and why | If not |
|---|---|---|---|
| **B1** | Before the NodeMCU: check for shorts 5 V–GND, 3V3–GND. Then USB 5 V on, panel switch on, no NodeMCU | 5 V after the diode a little under 5 V. The LP2950 gives 3.3 V. U4's pin 8 at 3.3 V (JP1) | Fix before inserting the module |
| **B2** | Ripple of the 3.3 V sensor supply, scope AC, 20 MHz limit: with no boards, then with six boards, then with Bluetooth connected and shots flowing | A few mV or less. Each sensor board adds its own RC (3.4 kHz, R21/C10). Supply noise reaches the thresholds directly | Add decoupling; try the mains supply against the power bank |
| **B3** | Insert the NodeMCU. `pio run -t upload`. Open the port with DTR and RTS released (clone CP2102). Type `help`, `stat` | The firmware prints its settings. The channel mask must include every fitted channel (`mask`) | – |
| **B4** | MEASUREMENT step 1 **test 3** (`selftest`, `selftest 1050 200`, `selftest 0`, `selftest 100000`) and **test 14** (every pin on the v0.3 map) | PASS as in the v0.2 results. Every L and H line appears on its own pin. This is **MEASUREMENT 7-2, first half**. The 10 MHz capture is what limits timing to ±100 ns, so this proves the instrument | A wrong channel: compare with `CAPTURE_LINE_PINS` and the schematic. Pins 34–39 have no internal pull-down: the 100 k pull-downs are what hold CH1 and CH2 low |
| **B5** | Panel LED and BOOT: short press makes a test shot, 2 s hold gives pairing mode, 10 s hold resets the PIN. Pair the app (`pairing`, then app or `bluetoothctl`) | As CLAUDE.md. A new board has a new MAC and name | – |
| **B6** | **Air sensor on J5.** BME280: chip ID 0x60 (a BMP280 reads 0x58 and has no humidity). Put it in the air between the mics, then next to the reference thermometer and hygrometer for 20 min | MEASUREMENT 3a-3 and 3a-4. Temperature within 0.3 °C of the reference, humidity within a few %. 1 °C of error costs 0.15 mm. The sensor must not read the board: IR LEDs and the regulator warm it, so keep it on its cable, up to 50 cm | If it reads warm, move it. A DS18B20 works on the same J5 (RH then assumed 50 %) |
| **B7** | **Rest noise.** Boards connected, no frame, nothing makes sound. `stat` and the log for 30 min, with Bluetooth connected, then with the IR curtain emitter on (`curtain on`) | Zero events at rest. Radio bursts and IR-LED current must not cross into the comparators. An event with no sound is a false trigger the range can't afford | Look at the ground path (NT1, AGND for the receiver), cable routing, and the thresholds |

---

## C. Boards and host together (no frame yet)

| | |
|---|---|
| **Tests** | The whole electrical chain: sensor board → cable → host → capture → t0 → solver |
| **Role** | Shows channels are not swapped, the cables don't pick up noise, and the firmware's own timing is clean on the real board |
| **Impact** | A swapped channel gives a position error of many mm. A noisy cable adds jitter that no calibration removes |

| # | Do | Expect, and why | If not |
|---|---|---|---|
| **C1** | Plug board 1 into CH1. A spark next to it. `verbose on` | Only CH1 has edges, both L and H, L first. Repeat for each of CH1–CH6 (**MEASUREMENT 7-2, second half**) | Another channel fires: crossed cable or a pin swap |
| **C2** | A spark near the board, then tap the board and cable | The spark gives clean edges. Tapping (microphonics) may trip a channel, but only a hard knock should | A cable that triggers on a light touch: shorten, shield or twist it |
| **C3** | Hold a phone or a fan near the cables, with and without Bluetooth traffic | No events | Re-route; the 470 Ω series resistors and the cable's ground pair help |
| **C4** | Unplug all boards, run the simulator: `valid default`, `mic sim`, `sim random 160` (CH3–CH8 drive their own inputs) | PASS, about 0.013–0.015 mm rms as in the bare-board bench. This is the firmware's own accuracy on the real host board: everything from the pins on. Anything more means the host board adds noise | Check B2 and the capture clock; read MEASUREMENT 3d |
| **C5** | Reset to the real layout: `mic default`, plug the boards back in, `sim` off | Layout and `cfg` as CLAUDE.md | – |

After C the electronics are known good. From here, errors come from the frame, the air and
the model.

---

## D. The frame and the holders

| | |
|---|---|
| **Tests** | The six holders, the aluminium backbone, the mic positions and heights, the template, and sound that goes through the frame instead of the air |
| **Role** | Puts each mic where the solver thinks it is, at 5.5 mm above the paper plane |
| **Impact** | A mic position error moves a shot about 1 : 1. The calibration fits x and y, so only the start must be close. **h is not fitted**: a gap under a board adds to it |

```
   mic board, side view (stands on its long edge)            top view, paper plane

        │ board 1.0 mm │                                       ●  mic (x, y)
        │      ○ port  │ ┐                                      \
        │              │ │ h = 5.5 mm  (port to paper plane)     \  d = √(ρ² + h²)
 ═══════╧══════════════╧═┘══════════  paper plane                  ●  spark / hole (ρ in the plane)
```

| # | Do | Expect, and why | If not |
|---|---|---|---|
| **D1** | Mount the boards: the board's bottom edge on the paper plane, port clear, the clamp strips only on the board's ends. Check the port with a torch | Port unblocked; no glue or clamp over it. Sound enters through a 0.5 mm hole | – |
| **D2** | Measure each mic's x, y from the card centre with the calliper, enter them with `mic <ch> x y h` (or in the app's Settings) | Within about 0.3 mm of the layout. The fit will refine x and y; a start closer than a mm helps it converge | A mic far off: fix the holder, not the number |
| **D3** | Measure each mic's **h** (port to paper plane) with the calliper, enter it | 5.5 mm if the edge sits on the paper. A gap adds to it. If the holder leaves 1 mm, h is 6.5 mm. This is a setting, not a fit (PLAN D3) | Different holders may differ: set each |
| **D4** | **The template.** Measure the template's holes against a precise reference (a scan of a printed card, or the calliper on pairs) | Holes within **0.05 mm** of the stated positions. The calibration places the mics to fit these points, so any error here lands in the mic positions and cannot be seen by later sparks from the same template. Only real shots against scanned cards (J1) can see it | A hole off by 0.1 mm or more: re-make the template, or measure the true positions and put them in the template file |
| **D5** | Put the template on the card holder, so its coordinates are card coordinates | The paper plane and the template plane are the same plane (the spark gap at the paper) | – |
| **D6** | **Sound through the frame.** Knock the frame and the holders with a knuckle, a pencil, and the spark lighter held against the frame. Look for events | Ideally none; at worst, events with a bad residual that the verdict refuses (`residual`). Sound through aluminium reaches every mic at once (the verdict `together`). Metal carries sound at ~5 km/s, so it arrives almost immediately: it would look like an early channel | If a holder couples sound: rubber or foam between holder and frame |
| **D7** | A **balance of gain**: a spark at the exact centre, 10 times; read each channel's first-lobe amplitude | Within about 10 % between channels, at 2–3× VTH_H or more (clipping at the peak is fine). Adjust **RV1** only, once, then lock it | This is the last time RV1 is touched; record in [M1](#m1-sensor-board-sheet) |

---

## E. The noise floor

| | |
|---|---|
| **Tests** | **Repeatability**: the same spark at the same point, 50 times (MEASUREMENT 7-3) |
| **Role** | Finds the random error of each channel and of the solved position |
| **Impact** | **The one error calibration can't remove.** It decides whether 0.2–0.5 mm is reachable at all. Do it first, before spending days on calibration |

**Setup:** the frame with six mics, the nominal layout (no calibration needed), the spark in
a fixed stand at the centre. 50 sparks, 1 s apart.

| # | Do | Expect, and why | Pass | If not |
|---|---|---|---|---|
| **E1** | `bench.py repeat LOG --n A-B` | Per channel: its own timing noise from about 50 ns (the sampling) to a few hundred ns. Position scatter close to what the noise predicts (it will be a mm or so away from the true centre until calibrated: only the scatter counts here) | Long axis ≤ **0.5 mm** (1σ); best ≤ 0.2 mm; no outliers | An outlier 10–40 µs late: polarity or thresholds (A5). Over 0.5 mm: STOP rule 2 |
| **E2** | Same at one off-centre point and at one near a corner, 20 sparks each | The scatter grows towards the corners (0.35 mm per µs against 0.21) | Within 1.7× the centre's | A channel with a high spread: its gain or threshold |
| **E3** | The same 50 sparks **with** Bluetooth traffic, then **with** `curtain on` | No change in the noise | Within 20 % | Radio or IR current coupling (B7) |

**Reading the result.** If the position scatter is bigger than the noise predicts, there is a
second source: the spark itself (spark-to-spark shape), or the frame. Try a pencil lead
instead of the lighter to see which.

---

## F. Calibration and verification

| | |
|---|---|
| **Tests** | The calibration wizard (4 × 4 grid, 10 sparks per point, 160 sparks) and verification at nine other points (MEASUREMENT 7-4) |
| **Role** | Finds each mic's position, each channel's offset and k, from sparks at known points. The record is stored on the target; every shot carries its ID |
| **Impact** | Removes the fixed errors of the budget. What remains is noise (E), the edge's change with loudness (G1), and the ground truth (D4, J1) |

```
 calibration grid (16 points × 10 sparks)      verification grid (9 points × 10 sparks)
   · · · ·    ±25 and ±75 mm                        ·   ·   ·     0 and ±50 mm
   · · · ·    check point: centre                   ·   ·   ·     (not the same points)
   · · · ·                                          ·   ·   ·
   · · · ·
```

Verifying at **other** points matters: a model fitted to its own points always looks good.

| # | Do | Expect, and why | Pass | If not |
|---|---|---|---|---|
| **F1** | **Calibrate** with the app: "Calibrate…", the 4 × 4 grid, 10 sparks per point. Redo any point that is flagged. Write the record | The fit reports an rms. A good fit has each point's error after the fit near the noise (E1). The wizard flags outliers. The record gets an ID; the sensor holds it | fit rms near E1's noise; no point with a large error | A point with a large error: its template hole (D4), or a double or stray spark |
| **F2** | **Verify.** Calibration mode on; `cal pt` at each of 9 points; 10 sparks each. `bench.py verify LOG --n A-B --drop` | Bias per point, rms, 1σ per axis, worst. `--drop` leaves each channel out in turn: shows what each mic adds, and that the middle mics matter near the edges (D2) | **1σ per axis ≤ 0.5 mm** (best ≤ 0.2 mm); **no point's bias over 0.5 mm** | A bias at the corners: h wrong (D3), or k. A bias at one edge: a mic position |
| **F3** | **Is a straight line right for k?** `bench.py kcheck LOG --n C-D` on the 160 calibration sparks | Each board's k and its uncertainty. k is near 1, not 1/3. CURVED flags a channel whose rise isn't straight over the range used. Boards' k differing by more than their uncertainty is normal | rms left by the straight line under about 300 ns | Over 300 ns: a curved arrival model is the next step (MEASUREMENT 4c) |
| **F4** | **Five against six mics.** Replay the verification with 5 mics (`--drop`) | The sixth mic helps near the edges and gives the drop-one fallback (PLAN D2) | Six better than five at the corners | – |
| **F5** | **Field check at the centre:** "Field check…" in the app, 5 sparks | A mean within the wizard's limit (0.5 mm); with a good calibration it should be much less | Mean ≤ half the limit | Redo the calibration |
| **F6** | Write the result into MEASUREMENT.md Bench results (noise floor and error, per axis), with the `cfg` id | This is the first number that says how good the system is | – | – |

**Order inside F:** F1 → F2 → F3 → F4 → F5. Keep the logs: they are regression tests for the
Rust fit and the solver (`tools/replay`).

---

## G. Systematic errors

What E and F can't show: things that change over time, with the air, with loudness and with
parts.

| # | Test | Why it matters | Do | Expect | Pass |
|---|---|---|---|---|---|
| **G1** | **Loudness** | The edge shape changes with how loud the crack is. A spark is one loudness; a pellet another. If k varies with amplitude, a quiet shot lands elsewhere than a loud one | Sparks at one point, first as they are, then made quieter with a layer of tape or foam over the spark gap (a few dB at a time, as long as events pass the verdict) | A position that **moves with the loudness** is the edge-shape error. Note the shift per dB | The shift between loud and quiet is under 0.3 mm. A bigger one means the arrival model needs the amplitude (4c) |
| **G2** | **Temperature** (MEASUREMENT 7-5) | The solver takes c from the BME280. A bias in the air reading shifts all distances | F2 at room temperature. Then warm or cool the room by at least 4 °C, wait 30 min, repeat F2 **with the same calibration**. `bench.py verify A.log B.log` | The bias moves by ≤ 0.1 mm; 1σ within 20 % of the first. 1 °C error would cost 0.15 mm, so a bigger shift means the sensor reads the board | Both ≤ limits |
| **G3** | **Warm-up** | The electronics warm up in the first hour. Offsets and thresholds drift with temperature | Power on cold. Sparks at the centre every 5 min for 90 min (10 each) | The centre's mean stays within 0.1 mm | If it drifts, warm up before the field check |
| **G4** | **A board swap** (MEASUREMENT 7-6) | Boards fail. Is a quick refit enough, or does a swap need a full calibration? | Replace CH4's board with the spare (set up as A). Run F2's verification with the **old** calibration. `bench.py swap LOG --ch 4` | The error grows before the refit; after refitting only that channel's k and offset, back within 20 % of F2's rms | Then "swap needs only that channel's k and offset" is true; write the two serial lines |
| **G5** | **A transport** | The summary says to recalibrate after every transport. Is that needed? | After F, take the frame down, pack it, mount it again, run F5 and F2's centre row without recalibrating | A shift tells you how much the mounting moves the mics. This sets the field routine in K | Shift under 0.2 mm: the field check is enough after a move |
| **G6** | **Day to day** | The calibration should hold for weeks | The same F5 check on 3 different days, same room | A shift under 0.1 mm | – |
| **G7** | **h's weight** | Shows how much the calliper values matter | Change one mic's h by 1 mm in the settings, verify (no refit), and then return it | The error that comes from a 1 mm h error. It tells how careful D3 must be | Written down; no pass/fail |

---

## H. The validity checks, with real sounds

| | |
|---|---|
| **Tests** | The verdicts (`ok`, `few`, `together`, `nofit`, `residual`, `lost`, `outside`, `curtain`) and the limits in the `cfg` line |
| **Role** | Refuses what isn't a shot: blasts from other lanes, noise, a bad channel |
| **Impact** | A false hit puts a wrong score on someone's card; a refused shot is a missed one. The solver can't tell without these checks |

The simulator tested these with faults. Now use real sounds, each 20 times, and record the
verdict from the `rec` lines.

| # | Sound | Expect | Why |
|---|---|---|---|
| **H1** | Spark inside the 200 × 200 area | `ok`, the right position | The normal case |
| **H2** | Spark in the frame, outside the scoring area (the `area` limit is 105 mm) | `outside` | Blast fits end up here |
| **H3** | A clap or balloon pop 2–3 m in front of the card, then to the side | Never `ok`. A distant wavefront is almost flat, and with the mics' arrangement it doesn't fit as a point on the card (the residual) | This is the layout's job (PLAN D1/D2): the test of blast rejection |
| **H4** | A loud click from 10 m, if the room allows | Never `ok` | Closer to the firing line |
| **H5** | Two sparks 1–5 ms apart | `together`, or the second `MISSED`; never a wrong position | Overlapping events |
| **H6** | A spark with one board unplugged (CH5), then one board with its gain down | `ok` with that channel dropped (flag bit 0) | One bad channel must not lose the shot |
| **H7** | A moderately off mic: change one `offset` by 1 µs, shoot 10 | Position moves by about 0.2–0.3 mm and no drop | The study in MEASUREMENT 4d: moderate errors are not caught. Decide there whether the firmware change is needed |
| **H8** | The tap on the frame (D6) | not `ok` | Frame-borne sound |

Every refused event stays in the log with its verdict, so the thresholds and limits can be
tuned later from real data (the summary's "log everything").

---

## I. The IR curtain

| | |
|---|---|
| **Tests** | The emitter, the BPW34F strip, the MCP6002 receiver, the comparator (TLC372 at 3.3 V, or LM393 at 5 V), and the dip's timing |
| **Role** | Confirms an event with a light dip 45–100 µs after the emission, and reports a dip with no sound as a shot through an existing hole (PLAN D9) |
| **Impact** | It is independent of the acoustic checks. It also gives a second, **timed** confirmation, so a late or early dip means a false event |

A parallel track: only the host board is needed. The summary says to do this first.

| # | Do | Expect, and why | If not |
|---|---|---|---|
| **I1** | One LED and one photodiode at 10 cm and 60 cm, under range lighting. Look at TP6 and the comparator output | A clear dip. Range lighting (flicker, lamp drivers) must not look like a dip | Raise the high-pass to 8–10 kHz (summary) |
| **I2** | The threshold RV1: turn it up slowly while dropping pellets through the beam. Note the TP6 voltage at which dips stop | Dips seen until near the signal's peak. If they stop near 2.1–2.3 V, it is the TLC372's input range: fit an LM393 and set JP1 to 5 V. **Never a push-pull part at 5 V** | Set RV1 well below the stop point, above the noise |
| **I3** | U3: the MCP6002 against the MCP602, same pellets. TP4's level with the LEDs on | The dip's timing scatter with each; the MCP602's inputs stop at 2.1 V | Keep the better one |
| **I4** | **Dip timing on real shots.** `curtain on`. The dip's delay after the event | The window is 45–100 µs; the receiver adds 3–8 µs. Slow air-pistol pellets are near the window's 100 µs edge already | Set `curtain window` from the measured delays |
| **I5** | A pellet through the beam without sound (no shot): a `dip` line | Reported as a shot through an existing hole | – |
| **I6** | `curtain on` during E3 and during H | Does not disturb the mics | B7 |

---

## J. The range

| | |
|---|---|
| **Tests** | Real shots, 25 m indoor, with other lanes firing (PLAN phase 2) |
| **Role** | The only test of what the system is for. Sparks can't show a bias between a spark and a pellet |
| **Impact** | Decides the accuracy claim, the false-trigger rate, the dead time and the thresholds |

**Set the pass levels after F.** The sparks' result is the best the system can do. Expect the
range to be somewhat worse, and let the first day show by how much.

| # | Do | Expect, and why | Pass |
|---|---|---|---|
| **J1** | **Ground truth.** Shoot cards of 5–10 shots; photograph them (card photos, PLAN D13, CARDPHOTO.md) or scan at 1200 dpi (0.02 mm per pixel) with the card holder's registration marks. Compare each shot with its hole in the app's table (OpenSights against Paper) | A systematic difference means the spark and the pellet disagree (or the template, D4). Random scatter is the system's accuracy. Use 100 or more shots over several cards | Position error and scatter against the goal; record both |
| **J2** | **Spark against pellet.** Compare J1's mean error per area of the card with the spark verification (F2) at the same places | The same bias on every card is systematic: the pellet's crack doesn't start where the spark does (a different h, or a different edge). A bias that grows toward the edges points at h | – |
| **J3** | **False triggers with other lanes firing**, for an hour or more. Count events with no shot of yours, for the acoustic checks and the curtain **separately** | Zero false hits. This tests whether an air-gun-only version can do without the curtain (summary's first open question) | Zero `ok` without a shot |
| **J4** | **Shots through an existing hole.** 10 of them | Reported as a `hole` (a dip with no event): scored by hand, not lost | None lost |
| **J5** | **Rapid fire and the dead time.** The blast arrives at d/c and opens an event; the pellet lands at d/v. At 30 ms dead time a fast pellet (150 m/s) at 10 m falls in the dead time and is `MISSED`; 8.4 m is the limit at 150 m/s, 10.5 m at 170 m/s (PLAN, a calculation). Shoot at 10 and 25 m at the speeds in use; count `MISSED`; then shorten the dead time until echoes and ringing are rejected | No `MISSED` at 10 m, or a shorter dead time. The test also says how many 30 ms windows another lane's blast takes from you | `MISSED` rate near zero |
| **J6** | **Thresholds and the high-pass corner** (20, 25 or 30 kHz). Tune from the logged events: raise VTH_L until the false triggers stop; keep VTH_H at 4× | The trade: blast rejection against the crack's amplitude. **Any threshold change shifts the timing, so recalibrate** | The false triggers gone, no real shot lost |
| **J7** | **A full range day on the power bank.** Field routine K from start to finish | Battery life, ripple (B2 on the bank), no resets, every shot sent. Pull the plug once mid-session (MEASUREMENT 5e-8): no shot lost | No resets, no lost shots |

---

## K. Field routine

How to keep the system calibrated in daily use. It also doubles as the plan for the
operator.

| When | What | Limit | If it fails |
|---|---|---|---|
| **Setting up** | Mount the frame; check no holder or cable is loose and every port is clear | – | – |
| **Warm up** | Power on before the first check (G3 says how long) | – | – |
| **Air** | Check the app shows a measured T and RH, not "assumed"; the BME280 in the air between the mics, not on the host board | within 0.3 °C of a reference thermometer, if one is at hand | Fix the sensor; assumed air costs up to 0.15 mm per °C |
| **Field check** | "Field check…": 5 sparks at the marked check point (the centre) | Mean inside the limit. Start at 0.5 mm; set it to about 2× the error in F5 once known | Redo the calibration |
| **After a transport or a rebuild** | Field check first. If it is out, calibrate (the 4 × 4 grid, about 160 sparks, 15 min) | per G5 | – |
| **After a board swap** | Refit that channel only (`bench.py swap`, or a few points), then a field check | per G4 | Full calibration |
| **After changing thresholds, gain, h, or a mic position** | Calibrate again: the `cfg` id changed, the calibration is stale (a flag on every shot) | – | – |
| **Each day's end** | Save the calibration record and the logs; copy the app's logs | – | – |
| **Every month** | One verification (F2's centre row) and a look at per-mic health (a mic down-weighted in many shots) | – | – |

**Rules of thumb.**
- A field check mean over 0.5 mm: don't shoot a match until recalibrated.
- A single channel always flagged `lost`: look at that board (A), not the calibration.
- A new place (another range) means a new field check; another temperature is covered by
  the BME280.

---

## L. Fault finding table

| Symptom | Likely cause | Look at |
|---|---|---|
| One channel 10–40 µs late | trip on the second half-cycle: polarity or VTH_L too high | A5, A4 |
| Position biased toward one mic | that mic's offset or position, or its h | F2, D2, D3 |
| Position error that grows with distance from the centre | k wrong, or c wrong (T) | F3, G2 |
| Errors worse in a warm room | the air sensor reads the board | B6, G2 |
| Quiet shots land elsewhere than loud ones | edge changes with loudness | G1 |
| Events with no sound | supply or radio noise, a loose cable | B7, C3 |
| Every shot marked `residual` | a bad channel, or the calibration is stale | `cfg` id; F |
| Many `together` | sound through the frame | D6 |
| Shots `MISSED` at close range | dead time too long | J5 |
| Dips seen with LEDs off, or no dips | the curtain's threshold or the receiver's input range | I2 |
| Field check suddenly bad | a holder moved; a mic knocked | G5, D2 |

---

## M. Record sheets

### M1. Sensor board sheet

Copy for each board.

| Item | Value |
|---|---|
| Board number / channel | |
| Date, tester | |
| Current at 3.3 V (A1) | mA |
| VMID (A2) | V |
| SIG at rest, noise p-p (A3) | mV |
| RV1 turns from the end (A4, set at D7) | |
| VTH_L − VMID (A4) | mV |
| VTH_H − VMID (A4) | mV |
| First SIG movement up? (A5) | yes / no |
| t2 − t1 at 50 mm / 100 mm (A6) | µs / µs |
| Centre spark first-lobe amplitude (D7) | mV |
| Mic h, measured (D3) | mm |
| k, offset from F1 | / ns |

### M2. Test log header

Each log and each line in Bench results gets: date, test number, what was set up, the `cfg`
id, first and last event `n`, air (T, RH, p), result, what it changed.

### M3. Numbers to write down in MEASUREMENT.md when done

| Number | From | Meaning |
|---|---|---|
| Noise per channel (ns) | E1 | the floor |
| Position scatter, centre and corner | E1, E2 | the floor in mm |
| 1σ per axis and bias per point after calibration | F2 | the system's accuracy with sparks |
| Each board's k and its uncertainty | F3 | whether a straight line is right |
| Shift between loud and quiet | G1 | edge shape against loudness |
| Shift per °C | G2 | air model |
| Warm-up drift | G3 | when the field check is valid |
| Swap error before and after | G4 | what a swap needs |
| Shift after a transport | G5 | how often to calibrate |
| Spark against pellet bias; error against paper | J1, J2 | the real accuracy |
| False hits per hour, `MISSED` rate | J3, J5 | the validity checks and the dead time |
