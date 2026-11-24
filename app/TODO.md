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
  (Asked for 2026-11-22.)
