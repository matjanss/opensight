// OpenSights - shot position and calibration mode (measurement step 3, see ../MEASUREMENT.md).
//
// The position: the arrival times t0 (arrival.cpp) of every channel that has one, the mic
// positions (NVS, serial `mic`) and the speed of sound from the air temperature go into the
// Gauss-Newton fit in tdoa.h. Coordinates are mm from the card centre, x right, y up, as
// the shooter sees the card (the protocol's shot frame).
//
// Calibration mode (`cal on`): one machine-readable line per event with the raw t1/t2 of
// every channel, the template point the spark was made at (`cal point <x> <y>`, or the
// simulator's true position) and the air temperature. tools/calibrate.py fits the mic
// positions, offsets and k to those lines and prints the commands that store them.
#pragma once
#include "arrival.h"

struct Solution {
  bool        solved;      // a position: >= 3 channels with a t0, the fit converged
  bool        complete;    // every fitted channel had a t0 (otherwise not a shot)
  uint8_t     used;        // channels in the fit
  uint8_t     iterations;
  float       x, y;        // mm from the card centre, x right, y up (seen by the shooter)
  float       residualNs;  // rms over the channels in the fit
  float       airC;        // the temperature the speed of sound came from
  const char* why;         // when not solved
};

// Loads the mic positions from NVS. Call once, before events come in.
void solverBegin();

void solverSolve(const EventResult& r, const Arrival& a, Solution& s);

// The position line under the arrival lines and, in calibration mode, the calibration line.
void solverPrint(const EventResult& r, const Arrival& a, const Solution& s);

float solverMicX(int ch);   // ch 0..7 = CH1..CH8
float solverMicY(int ch);

// Serial commands `mic` and `cal`; false if cmd is not one of them.
bool solverCommand(const char* cmd, const char* line);
void solverHelp();
