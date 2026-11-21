// OpenSights measurement core - the blast check (PLAN.md D1/D2): the muzzle blasts of
// tools/layout_study.py through the solver. A blast is a point source 10 or 25 m in front of
// the frame, 0-3 m off the axis (a neighbouring lane); it "scores" if the fit converges to a
// point inside the scoring area within the residual limit. With the mics on (or close to)
// one circle, many do: a flat wavefront fits the circle's centre.
// Portable: no Arduino, no IDF; also built on the PC.
#pragma once
#include "solver.h"

namespace measure {

static const int BLASTS = 624;   // 2 distances x 13 offsets x 24 directions, as the study

struct BlastResult {
  int   scored;       // of BLASTS
  float smallestNs;   // the lowest rms residual of a converged fit inside the area (INFINITY: none)
};

// `base` gives the mics, sigmas and c (its t0 are ignored); `mask` the mics to fit.
// `yield`, if given, is called every 100 solves (the target lets its idle task run).
BlastResult blastCheck(const SolveInput& base, uint8_t mask, float limitNs, float halfMm,
                       void (*yield)() = nullptr);

}  // namespace measure
