// OpenSights - the shot position on the board (measurement step 3c, see ../MEASUREMENT.md):
// every event is solved in loop() with the measurement core (lib/measure), printed as a
// `shot` line and logged as a `rec` line, and sent to the app when the fit is good enough.
#pragma once
#include "event.h"
#include "record.h"

struct Position {
  bool              solved;   // tried: 4 or more channels with a mic, both edges and a good pulse
  uint8_t           mask;     // bit c: CH(c+1) in the fit
  measure::Solution s;
  measure::Verdict  verdict;  // MODEL.md "Validity"
  bool              sent;     // to the app (verdict ok, not calibration mode)
  // The curtain (4b), judged apart from the acoustic checks.
  measure::Verdict  acoustic;    // the verdict without the curtain
  int8_t            curtain;     // 1: a dip in the window, 0: none, -1: not judged
  float             dipAfterNs;  // the dip in (or nearest to) the window, after the shot; NAN: none
};

// Loads reslimit and testshot (NVS "measure"). Call once at boot.
void positionBegin();

// One event: solve, judge, print the `shot` and `rec` lines, send. From captureService (loop()).
void positionProcess(const EventResult& r, Position& p, uint32_t nowMs);

// The BOOT button's random test shot (`testshot on|off`).
bool positionTestShot();

// For `mic`: 624 blasts (point sources 10 and 25 m in front, 0-3 m off the axis, as in
// tools/layout_study.py) through the solver, with every mic and with each one dropped. It
// warns if any fits a hit inside the validity area (`valid area`) within reslimit (the mics
// too close to one circle, PLAN.md D1/D2).
void positionBlastCheck();

void positionPrintStatus();   // `stat`
bool positionCommand(const char* cmd, const char* line);   // `reslimit`, `testshot`
void positionHelp();
