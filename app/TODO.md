# App: to do

Wishes and known gaps in the desktop app, not yet planned into a release. The bigger app
work (raw events, the calibration wizard, protocol v3) is in `../PLAN.md` phase 3.

## The card

- **Overlapping shots: click shows them all.** Clicking on the card opens the info popup
  of one shot: the nearest within the hit radius, the newest when several are equally
  near (`shot_at` in `main.rs`). When several shots overlap under the pointer (a tight
  group, two shots through the same hole), the click should list all of them: shot
  number, score and position for each, newest first. Picking one from the list opens its
  info as now. A single shot under the pointer opens its popup directly, as now.
  (Asked for 2026-11-22.)
