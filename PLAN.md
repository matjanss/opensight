# From prototype to product: decisions and plan

The decisions that shape the target, viewer and server from here on, why, and what test
settles each; then the order of work. Written 2026-11-21, after a first try of step 3
(solver, calibration mode, fit script) and before the sensor boards arrive. That first try
was set aside and kept as the tag `archive/step3-first-try`. The plan was then revisited
with the user the same day: six mics (D2), the mic height per mic as a setting (D3), a
BME280 for temperature and humidity (D8), and the solver (D12). Step 3 in MEASUREMENT.md is
rewritten from it, on the branch `step3-solver`.

It builds on three sources:

- the project summary doc "Electronic shooting target – project summary" (hardware, error
  budget, error sources, valid-shot detection), as of 19 Nov;
- `FUTURE_ARCHITECTURE.md`: the system;
- `firmware/MEASUREMENT.md`: the measuring steps, each with its bench test.

Where this plan answers one of the summary's open questions, or corrects it, it says so.

**Scope from the summary:** the proof of concept is an indoor 25 m range, mainly air guns
and subsonic ammunition, with other lanes firing. The frame is 250 × 250 mm around a
200 × 200 mm scoring area, with an IR light curtain behind the paper. Supersonic
ammunition is out of scope for now.

Numbers marked *(study)* come from `firmware/tools/layout_study.py`: mics around the
prototype frame (a 210 mm square), 18 °C, a residual limit of 1.5 µs. "Ring area" means
within 80 mm of the centre (the air pistol card's scoring rings are 155.5 mm across);
"scoring area" means the whole 200 × 200 mm. Rerun the study when the frame changes.

## What the first try showed (the archived step 3)

- **Sampling is not the limit.** 100 ns samples give 0.015 mm rms, 0.044 mm worst
  (simulated, PC). 10 MHz stays.
- **The model and the geometry are.** Left out or set wrong, each of these costs more than
  the whole sampling error:

  | Source | Cost in the ring area |
  |---|---|
  | Mic height above the paper (10.5 mm) left out of the model | 0.15 mm mean, up to 0.8 *(study)*; at 5.5 mm (2026-11-24) about a quarter of that (it grows with h²) |
  | Air temperature 1 °C off | 0.15 mm rms, 0.24 worst |
  | Mic positions ~1 mm off (no calibration) | 0.3 mm rms, 0.7 worst |
  | A channel's own delay of 500 ns | 0.09 mm rms, 0.12 worst |
  | The analog side (edge shape, walk, k) | unknown until the boards; the summary expects it to dominate |

- **Correction to the summary's error budget.** It counts "mic out of the paper plane by
  1 mm" as negligible (0.005 mm). But the mic port sits 5.5 mm behind the paper (the
  sensor board's edge on it; 21.5 mm flipped; 10.5 mm was planned before 2026-11-24), and
  the error grows with the square of that height. Over the 200 × 200 mm area it reaches
  1.4 mm at 5.5 mm (0.15 mm mean; 3.9 mm at 10.5 mm), the worst in the corners, close to a
  mic. The model has to include the height (D3). Once it does, the height costs nothing.
- **The scoring area's corners are close to the mics.** Mics at ±105 mm sit 5–7 mm from
  the corners at ±100 mm, so every error grows there (GDOP up to 3.6, against about 1 in
  the ring area). It matters little for scoring, since those corners are outside the rings,
  but a frame a little larger (mics at ±115) would help if the holders allow it.
- **The square layout can't tell a blast from a centre shot.** The summary already ruled
  it out; D1 says where to move the mic.
- The calibration fit works on synthetic logs (0.6 → 0.015 mm). The summary had already
  decided it runs in the app (D5).

## Decisions

### D1 Mic layout: slide one corner mic 40 mm along its edge

The summary decided that the mics must not all sit on one circle. A flat wavefront from
the front (a muzzle blast) fits a dead-centre hit if they do. Moving a mic 40 mm inward
along the diagonal fixes that, but puts it inside the scoring area. The open question was
where else to move it. The study answers it:

| Layout | Blasts scored as hits *(study)* | Smallest blast residual |
|---|---|---|
| Square, all corners | 484 of 624 | 0 |
| One corner turned 20° along the circle | 497 of 624 | 0 |
| Rectangle, isosceles trapezoid, regular pentagon (all on a circle) | 456–500 of 624 | 0 |
| **One corner slid along its edge, 25 / 40 / 60 mm** | **0 of 624** | 3.0 / 9.1 / 15.8 µs |
| One corner 25 mm further out along the diagonal | 0 of 624 | 6.9 µs |

The blasts were point sources 10 or 25 m in front, 0–3 m off the axis, counted when they
fit a point in the 200 × 200 mm scoring area.

- **Decision:** slide the top-right corner mic (CH3) 40 mm along the top edge, to
  (65, 105). It stays on the frame edge and outside the scoring area. The smallest blast
  residual, 9 µs, is 6 times the 1.5 µs limit. 25 mm also works, but with a 3 µs
  margin. Sliding it outward along the diagonal needs room the frame corner may not have.
- **Turning a mic along the circle doesn't help:** it has to change its distance from the
  centre.
- **Cost:** in the ring area, the GDOP goes from 1.03 to 1.05 mean (1.27 worst). The centre
  is no longer immune to speed-of-sound errors: 0.12 mm per 1 % of c instead of 0, which
  makes D8 slightly more important.
- **Two defences:** this covers the weak spot the summary describes (a neighbouring
  lane's blast fitting as a hit about 10 mm from the centre on the centre lines). The
  curtain is the second, independent defence (D9).
- **Firmware:** `mic default` gets this layout. `mic` runs the study's 624 blasts through
  the board's solver, with every mic and with each one dropped, and warns if any scores
  (MEASUREMENT 3c; this replaced a planned "within 25 mm of one circle" test).
- **Test:** a blast fault in the simulator (phase 0), then real blasts on the range (phase
  2).
- **Update (2026-11-22): the corner mic moves outward along the diagonal instead.** At
  (65, 105) the mic stands in front of the card. With the two middle mics (D2), moving it
  out works better:
  - d = 25 mm beyond the square corner, at (122.7, 122.7): no set of 6, 5 or 4 mics
    scores any of the 624 blasts. At (65, 105), two 4-mic sets let 42 and 1 through.
  - d = 15 mm: blasts score only with both middle mics gone.
  - The worst GDOP over the scoring area drops from 2.79 to 1.15.
  - The distance (about 15–25 mm) is decided with the holder. Calibration fits the mic's
    x and y anyway.
  - The holder's principle: `hardware/frame/top-right-holder.svg`. The board is 44 mm
    wide (50 mm was planned before 2026-11-24) and stands across the diagonal, so beyond d ≈ 7 mm the holder overhangs the
    frame corner.
  - **Changed 2026-11-24: d = 25 mm, CH3 at (122.68, 122.68)** (105 + 25/√2), the study's
    cleanest distance. `mic default`, the simulator, `tools/model.py`, the test vectors,
    the native tests and `tools/layout_study.py` (layouts O25, O25+B, O25+BR) use it. If
    the holder needs a shorter d, it is `MIC_LAYOUT_D2` in `config.h` and `LAYOUT_D2` in
    `model.py`; calibration fits the mic's x and y anyway. Checked: MEASUREMENT 7-0b.

### D2 Six mics: four corners (one slid), the bottom middle and the right middle

The summary asks whether a fifth mid-edge mic, for leave-one-out, is worth a spare
channel. It is, and with one condition: the four corners must already be off one circle
(D1). A sixth was then checked too, and is worth it.

| Mics *(study)* | A bad channel (a half-cycle-late trip, noise just before the wavefront) | Blasts scored, with a channel dropped | Speed of sound fitted, GDOP in the ring area |
|---|---|---|---|
| 4, one slid | Detected (residual ≥ 4 µs), but which one can't be told: the shot is lost | – | 9 mean, 184 worst: no |
| 5: square + bottom middle | Found and dropped every time | **484 of 624**: dropping the 5th mic leaves the square | 1.9 |
| **5: one corner slid + bottom middle** | Found and dropped in 553 of 555 | **0 of 624** | 1.7 mean, 4.9 worst |
| 6: one corner slid + left and right middles | Found and dropped every time | 0 of 624 | 1.45 |
| **6: one corner slid + bottom and right middles** | **Found and dropped in 666 of 666** | **0 of 624** | – |

- **Decision (2026-11-21):** six mics. CH1 bottom left (−105, −105), CH2 bottom right
  (105, −105), CH3 top right slid (65, 105), CH4 top left (−105, 105), CH5 bottom middle
  (0, −105), CH6 right middle (105, 0). This is `mic default` (D12). CH3 moves out along
  the diagonal (D1's update, 2026-11-22): at (122.68, 122.68) from 2026-11-24.
- **What the sixth adds** (1σ position error per unit of timing error per channel, ring
  area and whole area, from a covariance calculation with h = 10.5 mm):

  | Mics | Within 50 mm | Whole area, mean | 95 % of the area | Worst |
  |---|---|---|---|---|
  | 5 (slid corner + bottom middle) | 0.76 | 0.86 | 1.21 | 3.07 |
  | 6 (+ right middle) | 0.63 | 0.75 | 0.95 | 2.71 |
  | 6 with the left middle instead | 0.65 | 0.77 | 1.08 | 2.70 |
  | 6 with a second top mic (−20, 105) | 0.76 | 0.82 | 1.19 | 3.07 |

  About 17 % less in the centre, but only for the errors that are independent per
  channel and per shot (edge jitter, echoes). A delay shared by every channel drops out
  (the emission time absorbs it), a channel's constant delay is calibrated, and a
  speed-of-sound error scales every distance alike, so more mics don't help there. The
  bigger gain is drop-one: with one channel dropped, 5 mics are left (2 degrees of
  freedom) instead of 4 (1), so the residual still checks the rest.
- **Test:** record shots with all six on the bench, replay them on the PC with 5 and with
  6 mics (phase 1.3).
- **Holders:** the user designs and prints them (PETG): the slid top-right corner, the
  bottom middle and the right middle (Q2).
- **Sensor boards:** JLC makes 5 per order, so 6 mics take two orders (spares included).
- **The speed of sound** is not fitted per shot on the target (D8): it comes from the
  BME280. Fitting it from the shots stays a diagnostic on the PC, from replayed events.
  Supersonic ammunition would need it per shot, but is out of scope.
- **Rule for any layout:** no set of mics left after dropping one may be on one circle.
  `layout_study.py` checks it (`--drop`).

### D3 The propagation model, written down once

- **Subsonic projectiles and air guns:** the crack starts at the hole, in the paper plane.
  Each mic sits at a height h behind that plane, so the distance is √(ρ² + h²). The
  first try leaves h out; that is the first fix.
- **Where h comes from** (2026-11-24): the port is 5.5 mm from the board's paper edge
  (21.5 mm flipped; the board is 44 × 27 mm). The holders put that edge on the paper
  plane, so **h = 5.5 mm**, the default from 2026-11-24 (`MIC_H_DEFAULT_MM`; 10.5 mm
  before). A holder that leaves a gap adds it to h: measure it with a calliper.
- **h is a setting per mic, not fitted** (2026-11-21). The board is made to be flipped,
  to find which height works best, so h can differ per mic. It is set with a calliper
  value: by serial `mic` in step 3, from the app once protocol v3 carries the calibration
  record. Changing h (or a mic position) marks the calibration as stale, because the
  offsets and k were fitted with the old geometry.
- **The speed of sound** comes from the air temperature, humidity and pressure (Cramer
  1993), measured by the BME280 (D8).
- **k per channel**, as the summary says: the mic's port resonance curves the edge, so k
  comes out nearer 1 than the straight-line 1/3, and differs per board. The first try has
  one global k; the firmware's `k` and the fit become per channel.
- **Supersonic projectiles** are out of scope, as in the summary. If they come in, the
  mics hear the shock wave, whose trace spreads over the paper at c / √(1 − c²/v²), so the
  speed becomes an unknown per shot (5 mics can fit it, D2).
- **Model spec:** `firmware/MODEL.md` becomes the source of truth for the model, mirrored
  by `firmware/lib/measure/` and `firmware/tools/model.py`, later a Rust implementation (D5), with shared test vectors. It works like the
  protocol rule: a model change touches all three in one commit.

### D4 Who does what

| | Target (ESP32) | Viewer (app, Pi, phone) | Range server | Management (admin) |
|---|---|---|---|---|
| Capture, edges, t0, position, validity | **yes** | – | can re-solve from raw data (diagnosis), never silently replaces a shot | – |
| Shot store until acknowledged | **yes**, in flash | – | stores every shot with its raw data and calibration ID | – |
| Card, scoring, display | – | **yes** | serves cards | uploads cards |
| Calibration | measures (`cal` mode), stores and applies the record | – | keeps a copy of every target's record | **runs the fit and the workflow** |
| Settings | applies them (signed later) | none | holds them | edits them |

- **Confirmed 2026-11-21:** the target solves, not a thin target sending raw times.
  Step 3 keeps BLE to the current app; the raw data goes along as a first step towards
  the server.
- **Why the target solves:** it keeps working with BLE only and without a server
  (principle 1), it is the source of truth for shots (principle 2), and it adds no
  latency. The ESP32 has plenty of time: the solve takes microseconds per event.
- **Raw data goes along with every shot:** t1/t2 per channel, curtain time, temperature,
  residual, reject reason and calibration ID. This matches the summary's "log
  everything": the validity checks can then be tuned from real range data, a better model
  can re-solve old shots, and a disputed shot can be checked.

### D5 Calibration: the target measures, the app fits

This was already decided in the summary: the firmware stores results in NVS and sends raw
arrival times in calibration mode, and the fit over the spark points runs in the app. This
plan adds:

- **The fit is Rust,** in a crate the desktop app uses now (a calibration wizard,
  MEASUREMENT step 5). The management UI uses the same crate later (native on the server,
  or wasm).
- **Not in the viewer:** viewers have no settings (FUTURE_ARCHITECTURE principle 5).
  Calibration is an admin job; for the prototype, the desktop app is the admin tool.
- **Not on the ESP:** the model will change several times yet (h, k per channel, maybe
  the speed per shot), and each change there costs a flash cycle. A fit also needs review
  on a screen: the outliers, the error per point, how well each parameter is pinned down.
- **Meanwhile:** a Python fit stays the reference while the model settles (the first
  try's `firmware/tools/calibrate.py`, in the archive tag). It is then ported to Rust
  and checked against the same recorded logs.
- **The calibration record:**
  - ID (a hash of the record), date, target ID, frame type;
  - per mic: x, y, h (set, D3), offset, k;
  - the threshold and gain settings logged at the initial calibration (the summary's
    per-board values);
  - the fit's residual, and how many points and sparks went into it.

  It is stored on the target, in its own NVS namespace, which survives updates, and as a
  copy on the server, so a flash erase or a host board swap can be restored. Every shot
  carries the ID.
- **The template sits on the card holder,** so template coordinates are card coordinates.
  That takes out the holder's offset and rotation too.
- **When to calibrate:** after every transport (summary). Each day starts with a field
  check: a few sparks at one marked check point, recalibrating if they are more than a set
  amount off.
- **Board swap:** a quick re-calibration of that board's offset and k (a few points), if
  test 1.5 shows that's all a swap changes.
- **Identifiability:** with sparks of nearly equal loudness, k is weakly determined,
  because t2 − t1 then grows with distance like the travel time itself. Real sparks vary.
  The fit reports k's uncertainty per channel, and may pull uncertain ones towards the
  common value.
- **The fit models the edge noise** (MEASUREMENT 3e): t1 and t2 are both noisy, so a plain
  least-squares fit of t0 = t1 − k (t2 − t1) biases k low (0.02-0.05 in the simulation).
  Each channel's residual is scaled by √((1 + k)² + k²) (Deming regression). The Rust fit
  must do the same.

### D6 Firmware structure for production

- **Measurement core:** plain C++ with no Arduino or IDF, built and unit-tested on the PC
  (a PlatformIO `native` env). From one event's edges, the curtain time, a calibration
  record and a temperature, it gives t0, the position, the loudness and sharpness per
  channel, and a valid/reject reason. Today that is `arrival.cpp`; the solver and
  step 4's checks join it, untangled from `Serial` and `Preferences`.
- **Replay:** recorded raw events run through the core on the PC. That covers the
  `cal evt=` lines now, the raw-event message later, and every rejected event from the
  range. The same files are test cases for the Rust implementation. Real recordings then
  become regression tests, and the range-tuning loop the summary asks for becomes a PC
  job.
- **Platform:** capture (I2S, DMA, ISR), the pulse generator, the IR input, the air
  temperature, the links, storage, the button and LED.
- **Uplink interface:** BLE now, MQTT later, carrying the same messages (D7).
  `main.cpp`'s shot buffer moves behind it.
- **Shot store in flash:** a ring in its own partition, not 64 shots in RAM. Principle 2
  says no shot is ever lost, and that includes a reboot.
- **Storage:** settings, calibration and identity go in separate NVS namespaces, each with
  a schema version.
- **Serial commands stay a developer interface.** Production configuration comes through
  signed messages (FUTURE_ARCHITECTURE principle 5).

### D7 Messages

One message set for BLE and MQTT, under the protocol rule:

- the shot, as now, plus the calibration ID;
- the raw event: t1/t2 per channel, curtain time, temperature, residual, reject reason,
  flags (dropped channel, `MISSED`). This is the summary's step 5 list;
- calibration mode on/off, and reading and writing the calibration record.

The encoding stays binary little-endian, as now (BLE's MTU is small); the server shows it
as JSON. `PROTOCOL_VERSION` goes up.

**Not in step 3** (2026-11-21): step 3 sends solved shots in the v2 shot message only, so
the app needs no change. Everything else goes to serial, in lines the PC can replay (D12).
Protocol v3 (these messages, plus h per mic from the app) is its own branch, designed for
BLE and MQTT alike.

**Built in step 5** (2026-11-23, `protocol/PROTOCOL.md` v3): the hit (shot, hole, missed)
with its calibration ID and uncertainty, the raw event, Status, Control (calibration mode,
template point), Calibration (h per mic is in it) and Settings, one characteristic each, the
same bytes for MQTT later. The calibration wizard is step 5d.

### D8 Air: temperature, humidity, pressure

- The summary's budget is 0.17 % of c per °C, about 0.2 mm at the edge. The study gives
  0.15 mm rms per °C in the ring area. So the sensor has to read the air between the
  mics to about 0.3 °C. The summary already has it on a cable at the frame, which is
  right; keep it off the host board, which the IR LEDs and the regulator warm.
- **Humidity counts too:** 0.011 m/s per % RH at 18 °C (0.017 at 25 °C), against
  0.62 m/s per °C. 30 → 60 % RH is about 0.08 mm rms in the ring area. Pressure barely
  matters (0.04 m/s from 1013 to 950 hPa).
- **Decision (2026-11-21): a BME280** (temperature, humidity, pressure; I2C) on the host
  board's J5, which becomes a 4-pin connector: SDA on GPIO 14, SCL on GPIO 15 (host board
  README). A DS18B20 still works on the same connector (GPIO 14). The firmware finds
  which one is there; with a DS18B20 or a BMP280 (no humidity), RH is assumed 50 %, and
  with no sensor, 18 °C too.
- The pulse generator's sync loopback moves from GPIO 15 to GPIO 12 (unconnected; a
  strapping pin, driven only after boot).
- **Every event logs T, RH and p** (a few bytes), so a better model can re-solve it.
- **The speed of sound per shot is not fitted on the target.** With 6 mics it could be,
  but it costs precision (one more unknown). Fitting it over many replayed events on the
  PC checks the sensor instead (phase 1.4).

### D9 The IR curtain: the gate, and the shots through an existing hole

The summary's rule stays: a valid acoustic event plus a light dip 45–100 µs later. With D1
and D2 the geometry also rejects blasts, so the two defences are independent. That makes
the summary's open question testable: can an air-gun-only version do without the curtain?
Count each defence's rejections separately on the range (phase 2).

- **A dip with no acoustic event:** a shot through an existing hole. It is reported as a
  shot with no position, for manual scoring, rather than missed.
- **The curtain's bench test comes first,** as the summary says (one LED, one photodiode,
  10 and 60 cm, under range lighting).
- **Timing** (MEASUREMENT 4b): the dip is latched by the MCPWM capture unit, not a GPIO
  interrupt as the summary had it. On this board an interrupt can be 22 µs late (the
  capture's own), a large part of the 55 µs window; the capture latches in hardware at
  12.5 ns on the samples' clock. A dip with no event is printed on serial (a `dip` line)
  until protocol v3 can send a shot without a position.

### D10 Frame sizes and the capture's limits

- The PoC frame (mic span about 300 mm) fits the ring easily, and mic positions are
  already a setting.
- The ring (64 KB, 3.2 ms) holds a mic span up to about 690 mm. A frame larger than about
  480 × 480 mm (the 25 m precision pistol card, the summary's 60 × 60 cm optical option)
  would need a bigger ring (about 104 KB of DMA RAM is free) or 5 MHz sampling (0.03 mm).
  That's a later decision.

### D11 Ground truth for real shots

The summary's open question asks for an evaluation protocol: false accepts, false rejects
and position error.

- Sparks at template points test the system against itself.
- Shots need the real holes: scan the shot cards at ≥ 1200 dpi (0.02 mm per pixel), with
  registration marks on the card holder, and find the hole centres with a small tool.
- False accepts and rejects: every event logged (D4), with its curtain and acoustic
  verdicts, against the shooter's count.

All of this is needed before the range tests (phase 2).

### D12 The solver (2026-11-21)

- **Unknowns:** x, y and the emission time τ. Channel i's residual is
  rᵢ = t0ᵢ − τ − dᵢ / c, with dᵢ = √((x − xᵢ)² + (y − yᵢ)² + hᵢ²). This is the TDOA fit,
  but each channel's error stays its own, which makes weights, per-channel residuals and
  drop-one simple. c is known (D8).
- **Method:** Gauss–Newton with Levenberg–Marquardt damping, in single-precision float
  (the ESP32's FPU; double is software), coordinates in mm from the frame centre, times
  relative to the first arrival.
- **Start:** halfway between the first-hit mic and the centre; if that does not converge,
  again from the centre. Inside the frame the problem is nearly convex (GDOP about 1 in
  the ring area).
- **Convergence:** a step under 0.001 mm, at most 15 iterations. Not converged: no shot,
  logged.
- **Outside the area:** no clamping. The solver reports where it lands; validity decides
  (step 4).
- **Weights:** equal in step 3, but the code takes a σ per channel. The calibration record
  supplies them later (1/σ², from each channel's fit residuals).
- **Outputs:** x, y, τ; the rms residual and each channel's residual; iterations,
  converged, the channels used and dropped; and an **uncertainty per shot**:
  σ²(JᵀJ)⁻¹ with σ the calibrated timing σ (not the shot's own residual: with 2–3 degrees
  of freedom that is too noisy), as a 1σ ellipse (σx, σy, ρ) and its major axis in mm.
- **A bad channel:** the solver has the mechanism: solve with a channel mask, and a
  drop-one search over every subset that leaves out one channel. The policy (when to
  search, what to accept, the residual limit) belongs to validity (step 4); step 3 tests
  the mechanism with the simulator's faults.
- **Where it runs:** a portable measurement core (`firmware/lib/measure/`, no Arduino or
  IDF; D6), unit-tested on the PC with PlatformIO's `native` env. `firmware/MODEL.md` is
  its spec, with test vectors made by a Python reference (`firmware/tools/model.py`), in
  plain text, one case per line, so C++, Python and later Rust read the same files.
- **Replay:** the firmware prints every event as an `evt` line; a PC program feeds such
  logs through the same core.


### D13 Card photos: ground truth and the shooter's view (2026-11-23)

A phone photo of the shot card, matched to the measured shots: first to measure the
system's accuracy on real shots (D11), later to show the shooter the real card under the
app's shots. Steps, tests and results: `app/CARDPHOTO.md`.

- **The photo:** taken with the shooter's own phone, card flat on a table. The app shows a
  QR code with a one-time token and the series; the phone's browser opens a page the app
  serves on the local network, takes the photo and uploads it. Nothing to install on the
  phone, no camera on the target. It needs the phone and the computer on one network: the
  range Wi-Fi, best a separate target-system SSID without client isolation. Later the range
  server serves the same page (FUTURE_ARCHITECTURE).
- **Registration from the printed rings only** (no extra marks: placing marks to < 0.1 mm
  isn't repeatable): the rings are concentric circles of known diameter (genuine ISSF cards,
  Krüger), seen as near-ellipses. One fit over their edge points gives the perspective and
  the lens distortion (radial terms), robust to holes and print. The fit's residual says
  whether the photo is good enough. The print's scale can't be seen in a photo (it trades
  with the camera's distance): the rings are taken at their ISSF size, and the match to the
  shots fits a scale between the two when there are enough shots (C1, C2).
- **The surface under the card:** a hole shows it. Mid-grey or coloured, not black (a black
  table hides the holes in the black) and not white (C2).
- **Rotation:** circles say nothing about the card's rotation in its plane, and the card may
  be turned by 90°, 180° or 270°, plus a small angle in its holder. The printed ring numbers (on the
  print's four axes) give the rotation modulo 90°, the card's square outline where they don't
  show (CARDPHOTO C1); the quarter turn and the small angle come from matching the
  holes to the measured shots (all four tried). Without shots, the printed text and serial
  break the symmetry.
- **The holes:** searched near each measured shot, a 4.5 mm circle fitted to each edge,
  overlapping holes together. The shift and rotation between shots and holes are fitted per
  card: what remains per shot is the sensor's error; the shift and rotation themselves are
  the card's placement in its holder (it slides in; meant to be tight).
- **The serial number** (6 digits on each card) identifies the card: typed in at first,
  read from the photo later.
- **Storage** (a photo is 3–5 MB): kept with the series are the results (registration,
  hole centres with uncertainties, per-shot errors, the card's shift and rotation, the
  serial; a few KB), a small patch per hole (about 15 KB each: a dispute, a later re-fit) and
  a low-resolution rectified card (about 80 KB: the shooter's view). The original photo is
  kept 30 days, or longer when marked for evaluation. About 4 GB a year for a busy range
  instead of 90 GB. Photo metadata (GPS) is stripped on upload.
- **Risks:** the print's own accuracy and the paper's humidity (0.1–0.3 %; the fit has a
  scale term; measure a few cards once against a reference), flatness (a slight warp shows
  in the fit's residual), and the photo method's own precision (tested: the same holes from
  several photos must agree).

## Open questions for you

- **Q1 Which card first?** Answered 2026-11-24: the 10 m air pistol card (10-ring
  11.5 mm), the app's card already. Get it working first, optimise later; the air rifle card
  (10-ring 0.5 mm, the harder target for 0.2–0.5 mm) comes after.
- **Q2 The mic holders:** answered 2026-11-21: yes, the user designs and prints them
  (PETG) for the slid corner and both middle mics (D2). Still open: could the mics move
  out to ±115 mm (D1's corner remark)? 2026-11-22: the top-right mic moves out along the
  diagonal, its distance decided with its holder (D1's update).
- **Q3 The summary doc:** answered 2026-11-24: yes. Laid as suggestions in the doc
  (2026-11-24), for the user to accept: six mics in the intro and the layout bullet (D1/D2),
  the open question on the corner symmetry answered, the sensor board 44 × 27 mm with the
  port 5.5 mm from the paper edge, and the error budget's mic-height row (D3).

## Plan

Each phase ends with tests. The detailed bench steps go into MEASUREMENT.md as each step
starts, as they did so far.

### Phase 0: before the boards (PC and a bare NodeMCU)

This is MEASUREMENT step 3, sub-steps 3a–3e. **Done 2026-11-22** on a bare NodeMCU, all
simulated (the bench results are in MEASUREMENT.md): 6 mics 0.0125 mm rms; a late or early
channel dropped in 100 of 100 shots; 0 blasts scored with D1/D2, 484 of 624 with the square;
calibration from 160 simulated sparks takes the error from 1.0 to 0.014 mm rms; a 2 °C error
recovered as +1.99 °C. Left for the parts: the DS18B20 and BME280 tests (3a-3, 3a-4).

1. The BME280 or DS18B20 on J5, humidity in the speed of sound, the sync loopback on
   GPIO 12 (3a).
2. `firmware/MODEL.md`: the subsonic model with h and k per channel. The measurement core
   split out (D6, D12), the Python reference and shared test vectors, the `native` tests
   (3b).
3. The solver on the board: `mic` with the D1/D2 layout and h per mic, k per channel,
   `cfg` and `rec` lines, solved shots to the app, replay on the PC (3c).
4. Simulator (3d):
   - six channels (CH3–CH8: those pins can drive, and the pulse generator has exactly 6
     operators);
   - h and k per channel in its model;
   - `sim fault blast <distance> <offset>`: a far point source on every channel;
   - noise just before the wavefront on one channel;
   - the D1/D2 layout as its default;
   - the drop-one search and a residual limit, moved forward from step 4.
5. Calibration mode and the Python fit (3e).
6. Tests, all simulated:
   - blasts: 0 scored with the D1/D2 layout, many with the square (the study again, on
     the hardware path);
   - a late or early channel found and dropped with 6 mics, the shot kept;
   - calibration recovers mic positions, offsets and k per channel from simulated sparks
     (h is set);
   - on the PC, a speed of sound fitted over replayed events recovers a simulated
     temperature error.

### Phase 1: the boards arrive (bench)

1. **The summary's initial calibration per board,** and MEASUREMENT step 2's real tests:
   - polarity (before ordering more boards);
   - the edge shape on the scope;
   - t2 − t1 against amplitude: is a straight-line k right, and how far apart are the
     boards' k?
2. **Repeatability:** 50 sparks at one point, giving σ of every t0 difference. This is the
   noise floor of the real system, the one error calibration can't remove, and the number
   that decides whether 0.2–0.5 mm is reachable.
3. **Calibration:** the frame with the D1/D2 layout and the template on the card holder,
   then verification at other known points: within 0.2–0.5 mm? The same shots replayed
   with 5 and with 6 mics show what the sixth adds (D2).
4. **Temperature:** the room several °C warmer or colder, verification again.
5. **Board swap:** replace one sensor board and see what the calibration loses (just that
   channel's offset and k?).
6. **The IR curtain's bench test** (summary), as soon as the parts are there.

### Phase 2: the range (25 m indoor)

1. Real shots against scanned cards (D11).
2. Other lanes firing: false triggers per hour, counted separately for the acoustic checks
   and the curtain (D9).
3. Shots through existing holes: reported, not lost.
4. Rapid fire, as fast as the discipline allows: the dead time.
5. The high-pass corner (20, 25 or 30 kHz) and the thresholds, tuned from the logged
   events (summary open questions).
6. A full range day on the power bank.

### Phase 3: protocol and app (MEASUREMENT step 5)

The raw-event message and calibration over BLE (D7): **done 2026-11-23** (MEASUREMENT
5a–5c, protocol v3, passing over BLE). The Rust fit (`app/calib`, ported from
`calibrate.py` and checked against the same logs) and the calibration wizard with the field
check in the desktop app: **done 2026-11-23** (5d, passing over BLE with the simulator).
The shot store in flash: **done 2026-11-24** (5e: every hit in a 128 KB flash ring across
reboots, the shot request and clear across boots, passing on the bare NodeMCU and over BLE;
a real power cut is the user's test, 5e-8).

### Phase 4: the network (FUTURE_ARCHITECTURE step 2)

The uplink interface with MQTT. A minimal range server: shots with raw data, calibration
copies, re-solving from raw data.

### Phase 5: production hardening

- signed configuration;
- updates that keep the calibration;
- restoring a target from the server's copy after a flash erase;
- cold, heat and a long soak; power for a range day.

## How this changes the existing plans

- **MEASUREMENT step 3** is rewritten from this plan (done 2026-11-21): 3a–3e above. The
  archived first try supplies parts of the solver (`tdoa.h`), the simulator's position
  check and the Python fit.
- **MEASUREMENT step 4:** the residual limit and the drop-one search move forward to step
  3d. The loudness and edge-sharpness checks wait for real pulses (4c). 4a (2026-11-22)
  gives every event a verdict from what can be simulated; 4b adds the curtain (D9).
- **The summary doc:** D1 and D2 answer its open question on breaking the corner
  symmetry, and the mic height corrects its error budget (Q3).
