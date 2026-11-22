// OpenSights - sensor simulator (measurement step 2, see ../MEASUREMENT.md): shots at known
// positions, made as hardware-timed pulse pairs on CH3-CH8 and captured like real ones, so
// the capture and the arrival times can be checked against the truth. See simulate.cpp.
#pragma once
#include "arrival.h"
#include "position.h"

bool simActive();

// From captureService, for every event while the simulator runs (after it is printed and
// solved): checks the capture, t0 and the position against the shot it sent.
void simRecord(const EventResult& r, const Arrival& a, const Position& p);

// The simulator's mic on channel ch (0-based, CH1 = 0), for `mic sim`; false if ch isn't
// one of its channels.
bool simMic(int ch, float* x, float* y, float* h);

// From captureService: fires the next shot when it is time.
void simService(uint32_t nowMs);

// The serial command `sim ...`; false if cmd is not "sim".
bool simCommand(const char* cmd, const char* line);
void simHelp();
