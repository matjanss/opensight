// OpenSights measurement core - the position solver (../../MODEL.md "Solver", PLAN.md D12).
// Portable: no Arduino, no IDF; also built on the PC.
//
// Unknowns: the hole (x, y) and the emission time tau. Mic i's residual is
//   r_i = t0_i - tau - d_i / c,   d_i = sqrt((x - x_i)^2 + (y - y_i)^2 + h_i^2),
// fitted by weighted least squares (weights 1 / sigma_i^2) with Levenberg-Marquardt in
// single precision. Internally every time becomes a range in mm relative to the first
// arrival, s_i = c (t0_i - t0_first), and tau a range offset D = c (tau - t0_first), so no
// value is much over 1000 and float's 7 digits leave about 0.0001 mm.
#pragma once
#include "model.h"

namespace measure {

static const int   SOLVE_MAX_ITER = 15;      // linear solves per start
static const float SOLVE_DONE_MM  = 0.001f;  // a step this small: converged
static const float SOLVE_FAR_MM   = 2000;    // a point this far from the centre: no solution

struct SolveInput {
  int   n;                    // mics 0..n-1 (at most MAX_MICS)
  Mic   mic[MAX_MICS];
  float t0[MAX_MICS];         // arrival times, ns, any common origin
  float sigmaNs[MAX_MICS];    // 1-sigma timing error per mic, ns (> 0; the calibration's)
  float c;                    // speed of sound, mm/ns
};

struct Solution {
  bool    converged;          // a step under SOLVE_DONE_MM, within SOLVE_MAX_ITER
  uint8_t iterations;         // linear solves, over all starts
  uint8_t starts;             // 1, or 2 if the first start failed
  uint8_t used;               // bit i: mic i was in the fit
  int8_t  dropped;            // solveDropOne: the mic left out, -1 if none
  float   x, y;               // mm
  float   tauNs;              // emission time, on t0's clock
  float   rmsNs;              // rms residual over the mics used
  float   resNs[MAX_MICS];    // per mic, every mic with a t0 (also the dropped one)
  float   sx, sy, rho;        // 1-sigma uncertainty of x and y (mm), their correlation
  float   major;              // 1-sigma along the error ellipse's long axis, mm
};

// Fits the mics in `mask` (bit i = mic i; at least 3, and 4 or more to have a residual).
// Starts halfway between the first-hit mic and the centre; if that does not converge,
// again from the centre.
Solution solve(const SolveInput& in, uint8_t mask = 0xFF);

// The drop-one search (the mechanism; the policy is the caller's): the fit over `mask`; if
// its rms residual is over limitNs and it used 5 or more mics, the best fit with one of
// them left out (lowest rms residual), whatever that residual is. `dropped` says which.
Solution solveDropOne(const SolveInput& in, float limitNs, uint8_t mask = 0xFF);

}  // namespace measure
