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
- **Calibration templates.** The wizard has the 4 × 4 grid built in
  (`assets/templates/grid-4x4.json`); choosing another template file, and the sparks per
  point, in the window.

## Shots

- **Undo shot / too many shots on a card.** Normally a card takes x shots, then a new
  card goes in. If the shooter fires too many holes in one card (or a stray shot is not
  meant for this series), the app needs a way to take a shot out of the series: "Undo
  shot" on the newest shot, and removing any shot from the list. Which shot ISSF drops
  (user's question, 2026-11-27; check the current rulebook text before building, this is
  from memory): for excess shots in a series or match, the General Technical Rules
  (6.3.8, "Excess shots") say the shot of the highest value is not counted, and a
  penalty of 2 points (in some events per excess shot) is deducted, so a shooter gains
  nothing from it. It is the highest-value shot, not the last one fired, because on paper
  there is no telling the order. With a sensor the order is known, so the app can offer
  both: undo the last one (a mistake, no penalty), and "excess shots" (ISSF: the highest
  scores go, with the penalty). Open points: whether the rule applies per series or per
  card, and the penalty for the discipline; keep the removed shot in the event log (a
  `manual` line, as the resolve window does) and greyed in the list, so nothing is lost.

## Interface cleanup (the user, 2026-11-27)

The interface is getting cluttered. A cleanup of the main window, in these parts. Think it
through before building; the UI tests (`egui_kittest`) follow each change.

- **One window, two app modes.** The "shoot series" view and the "count cards" window become
  one window, with no separate window. At the top of the column right of the target card
  sits a banner, the "app mode": a plain row of buttons, "Shoot" and "Count" (or "Shoot
  series" and "Count cards"). A click changes both the card view and the panels below the
  banner, like browser tabs. The "Count card" button in the status bar goes.
- **Shoot mode, the right column:**
  - last shot info;
  - the series shots;
  - "Clear series" as a button (it is text now);
  - "Card photo" becomes a button, "Take Photo";
  - the "Shots per ring" panel goes (it moves to the card details, below).
- **Take Photo.** Opens the window with the QR code, as now. It closes when the photo is
  uploaded, and the main view comes back (as now). What changes after that, in the series
  panel:
  - the "photo's details" drop-down goes. A button "Card Details", under the shot table
    next to "Remove Photo", opens a new window with all the detail;
  - the details window gets: the stats and the notifications (warnings), the shot x, y and
    hole − shot columns (the extended table as it is now), and the shots per ring;
  - the table in the main view has fewer columns (without those);
  - "Keep the photo" stays as it is;
  - "Clear series", "Take another photo" and "Open the folder" become buttons;
  - "Remove from the card" becomes a button, "Remove Photo".
- **Count mode, the right column:** not yet described (the user, 2026-11-27: to follow).

## The solver: Huber weights and mic health (to discuss again)

**Note (2026-11-27, MEASUREMENT 4d): on synthetic shots Huber weights don't seem to add
value.** Against one mic 0.5–3 µs off they help a little (not the worst shots), against a
big fault they are much worse than today's drop-one (1.9 mm at 24 µs, today 0.021). What did
help was today's drop-one started earlier: when the rms is over 3–5 times what the mics'
sigmas predict, instead of over 1500 ns (one mic 1 µs off: 0.13 → 0.03–0.07 mm; 3 µs: 0.41 →
0.017 mm; healthy shots untouched). It needs sigmas from the calibration, so it waits for
the real logs of 7-3 and 7-4 (`tools/replay --alt dropS:3`, `dropS:5`); if it holds up
there, it goes into the firmware as a setting, off by default (a model change). The idea
below is kept as it was.

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
