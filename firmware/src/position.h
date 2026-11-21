// OpenSights - the shot position on the board (measurement step 3c, see ../MEASUREMENT.md):
// every event is solved in loop() with the measurement core (lib/measure), printed as a
// `shot` line and logged as a `rec` line, and sent to the app when the fit is good enough.
#pragma once
#include "event.h"
#include "solver.h"

struct Position {
  bool              solved;   // tried: 4 or more channels with a mic and both edges
  uint8_t           mask;     // bit c: CH(c+1) in the fit
  measure::Solution s;
  bool              sent;     // to the app (converged, rms residual within reslimit)
};

// Loads reslimit and testshot (NVS "measure"). Call once at boot.
void positionBegin();

// One event: solve, print the `shot` and `rec` lines, send. From captureService (loop()).
void positionProcess(const EventResult& r, Position& p, uint32_t nowMs);

// The BOOT button's random test shot (`testshot on|off`).
bool positionTestShot();

// For `mic`: 624 blasts (point sources 10 and 25 m in front, 0-3 m off the axis, as in
// tools/layout_study.py) through the solver, with every mic and with each one dropped. It
// warns if any fits a hit in the scoring area within reslimit (the mics too close to one
// circle, PLAN.md D1/D2).
void positionBlastCheck();

void positionPrintStatus();   // `stat`
bool positionCommand(const char* cmd, const char* line);   // `reslimit`, `testshot`
void positionHelp();
