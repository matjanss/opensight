# App: to do

Wishes and known gaps in the desktop app, not yet planned into a release. The bigger app
work (the calibration wizard, step 5d) is in `../PLAN.md` phase 3; protocol v3's raw events,
status and settings are in (step 5c).

## Protocol v3 (step 5c leftovers)

- **Unscored hits by hand: the check with a sensor.** Resolving by hand is in (branch
  `app-unscored`), passing its unit tests and the headless UI test (`resolve_hits_by_hand`,
  2026-11-26). Still to run, when the NodeMCU is on USB again: real holes and missed shots
  over BLE. Curtain on (`curtain on`) with `sim dip 60`, then `sim fault silent` (expect a
  hole per shot, "not scored") and `sim fault double` (expect a MISSED hit); resolve one of
  each in the app, check the `manual` lines in the day's log, then disconnect and reconnect:
  the series stays as resolved (the decisions live in memory with the series, so an app
  restart loses both).
- **Unscored hits: the card photo's suggestion.** With a card photo, a hole no shot claims
  (`Processed::holes`) is likely the unscored hit's: the resolve window could offer "the
  photo's hole at x, y".
- **Settings editor.** The sensor's settings (span, dead time, validity limits, the curtain's
  window) are shown read-only; the protocol can write them (Settings).
- **The IR curtain on or off, in the settings.** A switch for whether a shot needs the
  curtain's confirmation (PLAN D9, step 4b). The first tests run without the curtain, to
  find out whether the acoustic checks alone tell real shots from other sounds. The firmware
  already supports it over BLE, with no change needed: Settings with the curtain's window at
  0, 0 switch the gate off (and the emitter, IR_EN), a window (45–100 µs by default) switches
  it on, and the setting is kept in NVS. The gate is off by default. The app writes Settings
  whole, so the switch keeps the other values as read, and remembers the last window to turn
  it back on. Do this first, ahead of the rest of the settings editor. (Asked for
  2026-11-24.)
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
