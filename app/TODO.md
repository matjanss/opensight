# App: to do

Wishes and known gaps in the desktop app, not yet planned into a release. The bigger app
work (the calibration wizard, step 5d) is in `../PLAN.md` phase 3; protocol v3's raw events,
status and settings are in (step 5c).

## Protocol v3 (step 5c leftovers)

- **Unscored hits: resolve them.** A hole or a missed shot is listed as "not scored". The
  shooter (or range officer) should be able to place it on the card by hand after checking
  the paper, or mark it a miss, so the total is complete.
- **Settings editor.** The sensor's settings (span, dead time, validity limits, the curtain's
  window) are shown read-only; the protocol can write them (Settings).
- **Calibration templates.** The wizard has the 4 × 4 grid built in
  (`assets/templates/grid-4x4.json`); choosing another template file, and the sparks per
  point, in the window.
- **Fit from a log file.** `calib::logline` reads the firmware's and the app's logs: a
  "Calibrate from a log…" button would fit sparks recorded earlier (as `calibrate.py` does).
- **An events list** at the range: the recent events with time, verdict, position and
  residual, beside the counters (chosen against for 5c: counters only).

## The card

- **Overlapping shots: click shows them all.** Clicking on the card opens the info popup
  of one shot: the nearest within the hit radius, the newest when several are equally
  near (`shot_at` in `main.rs`). When several shots overlap under the pointer (a tight
  group, two shots through the same hole), the click should list all of them: shot
  number, score and position for each, newest first. Picking one from the list opens its
  info as now. A single shot under the pointer opens its popup directly, as now.
  (Asked for 2026-11-22.)
