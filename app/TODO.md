# App: to do

Wishes and known gaps in the desktop app, not yet planned into a release. The bigger app
work (the calibration wizard, step 5d) is in `../PLAN.md` phase 3; protocol v3's raw events,
status and settings are in (step 5c).

## Protocol v3 (step 5c leftovers)

- **Unscored hits: resolved at the end of the series, with the card photo.** Now each hit
  is resolved on its own, whenever the shooter clicks it. A better workflow (the user,
  2026-11-26; think it through before building) does it once, at the end of the series,
  together with the check against the paper:
  1. shoot the series (the unscored hits only listed, as now);
  2. upload the card photo(s) and correct the scores against the paper;
  3. confirm the hits still not scored and place them on the card.

  The photo helps with step 3: a hole no shot claims (`Processed::holes`) is likely an
  unscored hit's, so the app can offer "the photo's hole at x, y" for each, and the
  shooter only confirms (or places it by hand, or calls it a miss, as now). Open points:
  what "correct the scores" means for a shot whose paper score differs (take the paper's?
  keep both?), several unscored hits against several unclaimed holes (which goes where),
  and whether the series counts as finished only once every hit is resolved.
- **Settings editor.** The sensor's settings (span, dead time, validity limits, the curtain's
  window) are shown read-only, but for the curtain's switch (MEASUREMENT 5f); the protocol
  can write them all (Settings, `Command::WriteSettings`).
- **Calibration templates.** The wizard has the 4 × 4 grid built in
  (`assets/templates/grid-4x4.json`); choosing another template file, and the sparks per
  point, in the window.
- **Fit from a log file.** `calib::logline` reads the firmware's and the app's logs: a
  "Calibrate from a log…" button would fit sparks recorded earlier (as `calibrate.py` does).

## The card

- **Overlapping shots: click shows them all.** Clicking on the card opens the info popup
  of one shot: the nearest within the hit radius, the newest when several are equally
  near (`shot_at` in `main.rs`). When several shots overlap under the pointer (a tight
  group, two shots through the same hole), the click should list all of them: shot
  number, score and position for each, newest first. Picking one from the list opens its
  info as now. A single shot under the pointer opens its popup directly, as now.
  (Asked for 2026-11-22.) Include the hits resolved by hand: a "same hole" one lies exactly
  on its shot.

## The solver: Huber weights and mic health (to discuss again)

An idea from 2026-11-26, not decided. It touches the firmware solver (`lib/measure/solver.cpp`,
PLAN D12), the protocol and the app; it is listed here because the visible result (the mic
health flags) is in the app. Go over it with the user before planning it.

- **What:** the solver's LM fit minimises a Huber loss instead of plain least squares
  (iteratively reweighted: each iteration sets a mic's weight to 1 if |rᵢ| ≤ c·σᵢ, else
  c·σᵢ / |rᵢ|, then takes the usual weighted LM step). One solver, not a second algorithm.
  Riemannian LM was considered and brings nothing: x, y and D are flat, unconstrained
  unknowns, so it reduces to the same step.
- **Why:** a moderately off mic (0.5–3 µs: a drifted threshold making k wrong, a changed
  edge shape from a blocked port or a drifting high-pass, noise on the edge, a moved mic,
  sound through the frame) stays under the 1500 ns rms limit, so drop-one never starts, yet
  it moves the hit (1 µs ≈ 0.34 mm of path; with 6 mics roughly 0.1–0.3 mm, a tenth in
  decimal scoring). Huber caps its pull. It helps only with 5 or more mics: with 4 there is
  one redundant measurement and a bad mic can't be told from a good one. It does nothing
  for errors common to all channels (the speed of sound, calibration in general).
- **Two steps on the ESP:**
  1. Huber fit over all mics → x, y, τ and a final weight and residual per mic.
  2. Exactly one mic far out (|r| above about 5 σ, weight under about 0.3) and at least 5 mics:
     refit without it, `dropped` = that mic (the weights name the suspect: one refit instead
     of the N-subset search). Two mics far out: drop nothing, mark the shot uncertain. If the
     refit's rms does not clearly improve (a high-leverage bad mic near the edges or corners
     can push the low weight onto a good one), fall back to the full drop-one search as now.

  Three zones per channel: under ~2 σ full weight; ~2–5 σ down-weighted ("moderately
  unhealthy", counted for health); over ~5 σ dropped for that shot. Cost: a few extra
  iterations and at most one refit, microseconds a shot.
- **Mic health in the app:** the shot message carries each channel's final weight and
  residual (a protocol change: PROTOCOL.md, `protocol.h`, `protocol.rs` in one commit,
  probably a `PROTOCOL_VERSION` bump). The app (or a server later) keeps per mic: the bias
  (mean of rᵢ/σᵢ), the spread (rms of rᵢ/σᵢ) and the share of shots in the soft zone or
  dropped. It flags a mic that is down-weighted structurally, e.g. "CH3 down-weighted in 30 %
  of the last 50 shots: check the sensor", "CH5 late by 0.8 µs on average: recalibrate or
  check the mount", "CH2 noisy: check the cable". Statistics reset on recalibration or a
  sensor swap. Only shots that say something count: converged, inside the area, 5+ mics.
  Reading the pattern: a constant bias = offset, threshold ratio or mic position; a bias
  depending on where the shot lands = sound through the frame, reflections, a mic position
  off in one direction; random scatter = noise, a connector, an unstable stage.
- **Not too sensitive:** Huber doesn't make the position more sensitive to calibration; the
  threshold and the flags are what can be. So:
  - σᵢ comes from the calibration fit's residuals per channel (jitter plus what the
    calibration doesn't capture), not from the comparator jitter alone; one common σ until
    calibration exists.
  - At 2 σ about 5 % of a healthy mic's shots land in the soft zone: flag a change from the
    baseline (5 % → 25 %), not any down-weighting.
  - The ~5 σ drop limit is for gross faults only.
  - Optional: a running robust σ (median absolute residual) per channel, but compare against
    the calibrated σ for the flag, or a slowly failing mic raises its own threshold.
  - A mic position error gives a bias that depends on where the shots land: a tight central
    group shows a steady bias on one mic. Judge bias over shots spread across the card.
- **Also:** the 1σ ellipse from the weighted JᵀWJ (slightly optimistic when weights are cut);
  the residual limit recomputed over the weighted residuals and retuned; log the soft
  weights and the drops separately.
- **Order:** (1) a Huber option in `tools/replay`, compared with drop-one on logged events and
  the simulator's faults, the 2 σ / 5 σ / flag levels tuned there; (2) if it helps, into the
  firmware solver; (3) the per-channel weights and residuals in the protocol; (4) the health
  view in the app.
