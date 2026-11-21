// OpenSights - sensor simulator (measurement step 2, see ../MEASUREMENT.md): shots at known
// positions, made as hardware-timed pulse pairs on CH5-CH8 and captured like real ones, so
// the capture, the arrival times and the solver (step 3) can be checked against the truth.
// See simulate.cpp.
#pragma once
#include "arrival.h"
#include "solver.h"

bool simActive();

// The true position of the shot being simulated (not for `sim raw`); false if there is none.
bool simTruth(float* x, float* y);

// From captureService, for every event while the simulator runs (after it is printed).
void simRecord(const EventResult& r, const Arrival& a, const Solution& s);

// From captureService: fires the next shot when it is time.
void simService(uint32_t nowMs);

// The serial command `sim ...`; false if cmd is not "sim".
bool simCommand(const char* cmd, const char* line);
void simHelp();
