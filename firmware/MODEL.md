# The measurement model

The source of truth for how the target turns comparator edges into a shot position. It
is mirrored by the C++ core in `lib/measure/` (on the target, and on the PC for tests and
replay) and by `tools/model.py`, the reference implementation that writes the shared test
vectors. A Rust implementation (the app's calibration fit, PLAN D5) will follow.

**The model rule** (CLAUDE.md): a change to the model touches this file, `lib/measure/`
and `tools/model.py` in one commit, with the vectors made again
(`python3 firmware/tools/model.py`) and `pio test -d firmware -e native` passing.

The decisions behind it are in `../PLAN.md` (D3 the model, D8 the air, D12 the solver).

The log lines (below) are mirrored by `lib/measure/record.h` and `tools/replay.cpp`, not by
`tools/model.py`; a change to them touches both, and the tests in `pio test -e native`.

## Units and coordinates

- **Positions:** mm from the frame centre, in the paper plane. x points right, y up, as
  seen by the shooter.
- **Times:** ns. Arrival times count from any common origin. On the target that is the
  event's first edge.
- **The speed of sound c:** m/s in settings and output, mm/ns inside the solver
  (1 m/s = 10⁻⁶ mm/ns).

## Mics

Each mic i has:

| Value | Meaning | Where it comes from |
|---|---|---|
| xᵢ, yᵢ | Its position in the paper plane, mm | The frame's nominal layout, then the calibration fit |
| hᵢ | The mic port's height behind the paper plane, mm (10.5, or 21.5 with the sensor board flipped) | A setting per mic, measured with a calliper. Not fitted |
| kᵢ | The arrival-time factor of its comparator pair | The calibration fit (default 1/3) |
| offsetᵢ | Its channel's own delay, ns | The calibration fit (default 0) |
| σᵢ | Its timing error, 1σ, ns | The calibration's residuals (default 50 ns until then) |

The default layout (PLAN D1/D2), h = 10.5 mm:

| Channel | Mic | x, y (mm) |
|---|---|---|
| CH1 | bottom left | −105, −105 |
| CH2 | bottom right | 105, −105 |
| CH3 | top right, slid 40 mm along the top edge | 65, 105 |
| CH4 | top left | −105, 105 |
| CH5 | bottom middle | 0, −105 |
| CH6 | right middle | 105, 0 |

No set of mics left after dropping one may lie on one circle: a blast's flat wavefront
would then fit as a centre hit (PLAN D1, D2).

## Arrival

From a channel's first rising edge on OUT_L (t1) and on OUT_H (t2):

  t0 = t1 − k · (t2 − t1) − offset

For a signal that rises in a straight line, k = VL / (VH − VL) takes out the comparators'
delay, which grows the slower the rise (k = 1/3 for VH = 4·VL). The real edge is curved by
the mic port's resonance, so k is fitted per channel. `measure::arrivalT0` in `model.h`.

## Speed of sound

Cramer (1993), from the air temperature T (°C), relative humidity RH (%) and pressure p
(Pa), with 400 ppm CO₂. Water vapour comes from RH through the saturation vapour pressure
and the enhancement factor (Davis 1992, as Cramer uses them). `measure::soundSpeedMs` in
`air.h`; the formula is written out there.

| T | RH | p | c |
|---|---|---|---|
| 0 °C | 0 % | 101 325 Pa | 331.448 m/s |
| 20 °C | 0 % | 101 325 Pa | 343.359 m/s |
| 18 °C | 50 % | 101 325 Pa | 342.740 m/s |
| 20 °C | 50 % | 101 325 Pa | 343.987 m/s |
| 25 °C | 50 % | 101 325 Pa | 347.127 m/s |

Near 18 °C: +0.62 m/s per °C, +0.011 m/s per % RH, +0.0006 m/s per hPa.

## Propagation

Subsonic projectiles and air guns: the crack starts at the hole (x, y), in the paper plane,
at the emission time τ. It reaches mic i at

  t0ᵢ = τ + dᵢ / c,   dᵢ = √((x − xᵢ)² + (y − yᵢ)² + hᵢ²)

Supersonic projectiles are out of scope (PLAN D3).

## Solver

Unknowns: x, y and τ. Mic i's residual is rᵢ = t0ᵢ − τ − dᵢ / c (positive: the mic heard it
late). The solver minimises Σ wᵢ rᵢ² over the mics used, with wᵢ = 1 / σᵢ².
`measure::solve` in `solver.h`.

- **Ranges:** inside, every time becomes a range relative to the first arrival among the
  mics used, sᵢ = c (t0ᵢ − t0_first), and τ a range offset D = c (τ − t0_first). The residual
  in range is d_i + D − s_i. For a given (x, y), the best D is the weighted mean of sᵢ − dᵢ;
  the solver sets it so at every point.
- **Method:** Levenberg–Marquardt on (x, y, D): the normal matrix A = Σ wᵢ Jᵢ Jᵢᵀ with
  Jᵢ = ((x − xᵢ)/dᵢ, (y − yᵢ)/dᵢ, 1), its diagonal times (1 + λ), λ from 10⁻³, divided by 10
  after a step that lowers the cost and multiplied by 10 after one that doesn't. Single
  precision on the target.
- **Start:** halfway between the first-hit mic and the frame centre. If that does not
  converge, again from the centre.
- **Convergence:** a step in (x, y) under 0.001 mm, which is taken even if the cost does not
  drop (at the minimum, float cannot resolve the change). At most 15 linear solves per
  start. A point more than 2000 mm from the centre, or a singular matrix (mics in a line):
  not converged.
- **Outside the area:** no clamping. The solver reports where the fit lands; validity
  decides (MEASUREMENT step 4).
- **Mics:** at least 3; 4 or more to have a residual.

### Outputs

| Output | Meaning |
|---|---|
| x, y | mm |
| τ | ns, on the arrival times' clock |
| rms | √(Σ rᵢ² / n) over the n mics used, ns |
| rᵢ | ns, per mic, also for a mic not used (its misfit against the solution) |
| converged, iterations, starts | as above |
| used, dropped | which mics were in the fit; the one drop-one left out (−1: none) |
| σx, σy, ρ | the 1σ uncertainty of x and y and their correlation |
| major | 1σ along the long axis of the error ellipse, mm |

The uncertainty is the covariance A⁻¹ at the solution, with the weights in absolute
units (wᵢ = 1 / (c σᵢ)² in mm⁻²). It uses the σᵢ given, not the shot's own residual: with
2–3 degrees of freedom that would be too noisy. It is checked by simulation: with 50 ns of
noise per channel, the rms error matches it within 10 % (`pio test -e native`).

### Drop-one

`measure::solveDropOne`: the mechanism. The policy, when to use it and whether to accept
the result, is the caller's (MEASUREMENT steps 3d and 4).

1. Fit with all the mics given.
2. If that converged with rms ≤ the limit, or it used fewer than 5 mics, that is the
   result.
3. Otherwise fit once with each mic left out, and take the best: a converged fit over one
   that didn't, then the lowest rms. Its `dropped` says which mic. Its rms may still be
   over the limit.

## Validity

`measure::solveEvent` judges every event after its fit (MEASUREMENT steps 4a and 4b). The
verdict is `ok` (a shot: it goes to the app) or the first of these checks it fails, in
this order:

| Verdict | When | Limit (`cfg` key, firmware default) |
|---|---|---|
| `few` | Fewer than 4 channels to fit (a mic, both edges, a pulse shape in range) | – |
| `together` | Every first edge (t1) of the channels to fit within the limit: an electrical spike, or a blast from straight ahead | `tog`, 20 000 ns. A real shot spreads over 126 µs or more anywhere in the frame (D2 layout; 147 µs with the corner mic out on the diagonal) |
| `nofit` | The fit did not converge | – |
| `residual` | rms residual over the limit, after drop-one if it is on | `lim`, 1500 ns |
| `lost` | More of the event's mic channels left out of the final fit than allowed: no edge, a pulse shape out of range, or dropped | `lost`, 1 |
| `outside` | The hole beyond ±area in x or y | `area`, 105 mm (the mics' edge: a hole further out would hit the frame) |
| `curtain` | The curtain's gate is on and no dip rises within its window after the emission time (the fit's τ) | `ir`, 45 000–100 000 ns (the sheet 15 mm behind the paper at 150–350 m/s, PLAN D9); off until a curtain is fitted |

- **The pulse shape** is checked per channel before the fit, in `measure::solveInput`: with
  `pmax` (firmware default 10 000 ns), a channel is left out unless 0 ≤ t2 − t1 ≤ `pmax`.
  A long t2 − t1 means a pulse barely over the high threshold, whose t0 the straight-line
  model gets wrong. It counts as lost, so with `lost` 1 one bad pulse costs a channel but
  not the shot.
- **Off:** `tog`, `area` and `pmax` 0, `lost` −1. A `cfg` line without these keys (before
  4a) means off, so an old log replays with the verdicts it had then: a shot was good if
  it converged within `lim`.
- **Why `lost` 1:** with 6 mics every set of 5 rejects blasts (D2), while two lost channels
  leave 4, which may not.
- **The curtain** (4b) is checked last, and judged apart from the acoustic checks, so each
  defence's rejections can be counted on their own (PLAN D9). `measure::EventFit` gives
  the verdict without it (`acoustic`), the curtain's result (`curtain`: 1 a dip in the
  window, 0 none, −1 not judged: the gate off, or no converged fit to time it against), and
  the dip in the window, else the one nearest to it (`dip`, and `dipAfterNs`: its rise
  after τ). The window's edges count as in it. Both are in the emission's clock: a dip's
  rise is logged in ns from the event's first edge, as the edges are, and τ is the fit's,
  with the channels' offsets taken off.
- **A dip with no event** (no acoustic event within 1.5 ms of it) is a shot through an
  existing hole: the firmware prints it as a `dip` line (see Log lines). It has no
  position, so protocol v2 can't send it.
- **Still to come:** loudness and edge sharpness (4c, from real pulses); the curtain's own
  delay (its amplifier and comparator), once measured on the bench.

## Log lines

The target logs every event on serial so it can be solved again on the PC (`tools/replay`,
later the app's calibration fit). `lib/measure/record.h` writes and reads both lines, and
`measure::solveInput` turns them into the solver's input, on the target and in the replay
alike. One line each, fields `key=value` separated by spaces; text before the line (a
monitor's timestamp) is ignored, and so are unknown keys (later firmware may add some).

**`cfg`**, the settings, printed at boot, after every change, by `mic` and at the start of
a `sim` run:

```
cfg id=0262A982 CH1=-105,-105,10.5,0.333333343,0,50 ... CH7=- CH8=- lim=1500 drop=1 lost=1 area=105 tog=20000 pmax=10000 ir=-
```

- `CHn=x,y,h,k,offset,sigma` (mm, mm, mm, -, ns, ns), or `-` for a channel without a mic.
- `lim`: the residual limit (ns), `drop`: drop-one on (1) or off (0): the policy
  `measure::solveEvent` applies. A line without them (3c) means 1500 and 0.
- `lost`, `area` (mm), `tog` (ns), `pmax` (ns): the validity limits (see Validity). A line
  without them (before 4a) means off.
- `ir=lo,hi` (ns): the curtain's window (4b), or `ir=-` for its gate off. A line without
  it (before 4b) means off.
- Floats with 9 significant digits: they read back to the same bits.
- `id`: the CRC-32 (IEEE, as zlib's) of the text after `id=XXXXXXXX `. A line whose id
  doesn't match its text is rejected.
- The **calibration id** (`measure::calibrationId`, the `cal` record's) is the CRC-32 of
  the channel fields alone, up to ` lim=`: changing the policy or the validity limits
  doesn't make a calibration stale. `tools/calibrate.py` computes it the same way.

**`rec`**, one event:

```
rec n=2161 t=876.1903407 cfg=0262A982 T=18.00 RH=50.0 p=1013.25 meas=--- c=342.739716 CH5=0,1500 CH6=100000,101500 CH8=-,-
```

| Key | Meaning |
|---|---|
| `n` | Event number (from 1 per boot) |
| `t` | The event's first edge, s since the capture started |
| `cfg` | The id of the `cfg` it was solved with |
| `T`, `RH`, `p` | The air: °C, % RH, hPa |
| `meas` | `T`, `R`, `P` where measured, `-` where assumed |
| `c` | The speed of sound used, m/s, 9 digits (the solver's c, not computed again) |
| `CHn=t1,t2` | Each channel fitted in the event: OUT_L's and OUT_H's first rise, ns from the event's first edge, `-` for none |
| `pt=x,y` | The true point, mm, if known: calibration mode's template point (`cal pt`), or the simulator's shot. Optional |
| `mode=cal` | Made in calibration mode (not sent to the app). Optional |
| `cal`, `stale` | The calibration record's id and whether it is stale (`cal=1A2B3C4D stale=0`), or `cal=-` for none |
| `ir=rise/len,...` | The curtain's dips within 1.5 ms of the event (4b, up to 4): each rise in ns from the event's first edge (negative: before it), and its length in ns (`-`: its fall not seen). `ir=-`: the curtain on, no dip. Missing: the curtain off |

The solver's input: t0 = arrivalT0(t1, t2, k, offset) per channel, in float, for every
channel that has a mic, both edges and (with `pmax`) a pulse shape in range; c from `c`.
`measure::solveEvent` then fits them if there are 4 or more (with `drop`, through drop-one
when 5 or more), and gives the verdict (see Validity).

The firmware's `shot` line, just before the `rec` line, ends with the verdict in brackets
(`; sent as shot 12 [ok]`, `; not sent [residual]: ...`), which the replay compares with
its own. With the curtain on, the dip comes before it: `; curtain dip +70.0 us after the
shot (35.0 us long)`.

**`dip`**, a curtain dip that no event claimed (4b), printed 50 ms after it:

```
dip n=3 t=1234.567890123 len=35000
```

`n` counts them from 1 per boot, `t` is its rise (s since the capture started, as `rec`'s
`t`), `len` its length in ns (`-`: not seen). The replay counts them.

**The app's event log** (protocol v3, step 5c) has the same lines, made from the Event
messages and the Calibration and Settings reads (`app/src/eventlog.rs`): `cfg` (identical,
ID included), `rec` (identical), `dip` (with `n` the event's stream number), and a `MISSED`
line for a missed shot. So `tools/replay` reads an app's log like a serial one (bench 5:
38 of 38 `rec` lines identical).

## Test vectors

`test/vectors/`, written by `tools/model.py` (fixed seed: the same files every run). Plain
text, one case per line, fields separated by spaces, `#` starts a comment line. Inputs are
written rounded, and the expected values are computed from the rounded inputs, so every
implementation reads exactly what the reference solved.

**`air.txt`:** `T_degC RH_percent p_Pa c_m_per_s`.

**`solve.txt`:**

```
name kind limit_ns n mask c_mm_per_ns  (x y h)×n  t0_ns×n  sigma_ns×n  |  ok  [dropped x y tau rms res×n sx sy rho major]
```

- `mask`: hex, bit i = mic i used.
- `ok`: 1 if the reference found a solution, then the expected values follow; 0 if
  not (blasts).
- `kind`:
  - `fit`: `solve(mask)` must converge to the expected values;
  - `drop`: `solveDropOne(limit, mask)` likewise, dropping the same mic;
  - `outside`: noise-free points outside the frame. The solver may fail to converge,
    but a converged fit with rms under 10 ns must be the expected point (0.01 mm);
  - `blast`: a far point source. No fit, with or without drop-one, may be a hit (converged,
    rms ≤ limit, inside ±100 mm).

The cases: 230 random points with 50 ns noise on the D1/D2 layout (equal and mixed heights,
some with unequal σ), 40 with the D1 corners only, 40 on the square, 60 outside the frame,
80 with one channel 24 µs late or early, 20 clean shots through drop-one, 60 blasts.

**Agreement** (single precision against double): positions 0.001 mm, residuals and rms
0.5 ns, τ 0.5 ns, σx/σy/major 0.1 %, ρ 0.001. Today's C++ reaches 0.00006 mm, 0.26 ns,
0.11 ns and 0.005 %. Arrival times near 10⁶ ns are float steps of 0.06 ns apart; that is
where the residuals' 0.26 ns comes from (0.0001 mm of range).
