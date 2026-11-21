// OpenSights - sensor simulator (measurement step 2, see ../MEASUREMENT.md): shots at known
// positions, made as hardware-timed pulse pairs on CH5-CH8 and captured like real ones, so
// the capture and the arrival times can be checked against the truth. See simulate.cpp.
#pragma once
#include "arrival.h"

bool simActive();

// From captureService, for every event while the simulator runs (after it is printed).
void simRecord(const EventResult& r, const Arrival& a);

// From captureService: fires the next shot when it is time.
void simService(uint32_t nowMs);

// The serial command `sim ...`; false if cmd is not "sim".
bool simCommand(const char* cmd, const char* line);
void simHelp();
