// OpenSights - the calibration (measurement step 3c, see ../MEASUREMENT.md): per channel its
// mic (x, y, h), its arrival-time factor k and own delay (offset), and its timing error
// (sigma). Stored in NVS (namespace "cal"); k and the offsets were in "measure" until 3c and
// are carried over once. Also the solver's policy (`reslimit`, `dropone`) and the validity
// limits (`valid`, step 4a), which the cfg line carries too. Serial commands `mic`, `k`,
// `offset`, `sigma`, `reslimit`, `dropone`, `valid`, `cal`.
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

// Calibration mode (`cal on`): events are marked with the template point (`cal pt`) and not
// sent to the app.
bool calibMode();
bool calibPoint(float* x, float* y);   // false: no template point set

// The calibration record (`cal id`): its id, and whether it is stale (a calibrated value
// changed since). False: none.
bool calibRecord(uint32_t* id, bool* stale);
void calibPrintStatus();   // `cal`, `stat`

// The record's details for the app (Calibration, PROTOCOL.md): false if there is none.
bool calibRecordInfo(uint32_t* date, uint16_t* points, uint16_t* sparks, float* resNs);

// From the app (uplink.cpp), each checked by the caller (proto::settingsValid,
// proto::calibrationValid), stored, and the `cfg` line printed once.
void calibSetMode(bool on);                          // `cal on|off`
void calibSetPoint(bool have, float x, float y);     // `cal pt x y`, `cal pt -`
// The policy and validity fields of `p` (resLimitNs ... curtainHiNs); its channels unused.
void calibSetPolicy(const measure::Cfg& p);
// Every channel at once, and a calibration record for them (not stale): `cal ch` x8, `cal id`.
void calibSetChannels(const measure::ChannelCfg ch[8], uint32_t date, uint16_t points,
                      uint16_t sparks, float resNs);

// `mic`, `k`, `offset`, `sigma`, `reslimit`, `dropone`, `valid`, `cal`.
bool calibCommand(const char* cmd, const char* line);
void calibHelp();
