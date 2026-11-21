// OpenSights - the calibration (measurement step 3c, see ../MEASUREMENT.md): per channel its
// mic (x, y, h), its arrival-time factor k and own delay (offset), and its timing error
// (sigma). Stored in NVS (namespace "cal"); k and the offsets were in "measure" until 3c and
// are carried over once. Serial commands `mic`, `k`, `offset`.
//
// Every change prints the `cfg` line (lib/measure/record.h), whose id every `rec` line
// carries, so a log can be replayed with the settings it was made with.
#pragma once
#include "record.h"

// Loads it from NVS (or the defaults: `mic default`, k 1/3, offsets 0). Call once at boot.
void calibBegin();

const measure::Cfg& calibCfg();
uint32_t calibId();
void calibPrintCfg();   // the `cfg` line

bool calibCommand(const char* cmd, const char* line);   // `mic`, `k`, `offset`
void calibHelp();
