// OpenSights - arrival times (measurement step 2, see ../MEASUREMENT.md).
//
// Per channel, from the first rising edges of OUT_L (t1) and OUT_H (t2):
//   t0 = t1 - k * (t2 - t1) - offset
// For a signal that rises in a straight line, both comparators trip a time after the true
// arrival that grows with how slowly it rises; k = VL / (VH - VL) takes that out (k = 1/3
// for VH = 4 VL). The offset is a channel's own delay (from the calibration, step 3).
#pragma once
#include "event.h"

struct Arrival {
  uint8_t valid;            // bit c: CH(c+1) has a t0 (both of its lines rose)
  float   t0[CHANNELS];     // ns from the event's first edge
  int32_t gap[CHANNELS];    // t2 - t1, ns
};

// Loads k and the offsets from NVS. Call once, before events come in.
void arrivalBegin();

void arrivalCompute(const EventResult& r, Arrival& a);

// Two lines under the event line: t0 and t2 - t1 per channel, then the t0 differences to
// the first channel that has one, and the air temperature.
void arrivalPrint(const EventResult& r, const Arrival& a);

float arrivalK();
float arrivalOffsetNs(int ch);   // ch 0..7 = CH1..CH8

// Serial commands `k` and `offset`; false if cmd is not one of them.
bool arrivalCommand(const char* cmd, const char* line);
void arrivalHelp();
