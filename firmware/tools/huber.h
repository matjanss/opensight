// OpenSights - a Huber fit on top of lib/measure's solver, for study only (app TODO "The
// solver: Huber weights and mic health"; MEASUREMENT 4d). Not in the firmware: the model
// rule (MODEL.md) holds until the study says it helps and the user decides.
//
// Huber by iteratively reweighted least squares: each round gives mic i the weight
//   w_i = 1 if |r_i| <= c sigma_i, else c sigma_i / |r_i|
// and solves again with sigma_i / sqrt(w_i) (the solver weights by 1 / sigma^2), until the
// weights settle. Then the two steps: with 5 or more mics and exactly one mic far out
// (|r_i| > drop sigma_i), a refit without it; if that doesn't clearly lower the rms, the
// full drop-one search (solveDropOne) instead. Two or more far out: nothing dropped, the
// shot marked uncertain. (`solveHuber`: the TODO's two steps.)
//
// Two others to compare: `solveDropSigma`, today's drop-one started by the rms against the
// rms the mics' sigmas lead one to expect instead of the fixed 1500 ns; and
// `solveHuberSearch`, Huber with that drop-one search whenever a mic is far out.
#pragma once
#include <math.h>

#include "solver.h"

namespace measure {

struct HuberOpts {
  float c = 2.0f;         // the soft zone starts at c sigma
  float drop = 5.0f;      // a mic beyond this many sigma is "far out"
  int   rounds = 20;      // reweighting rounds at most
  bool  refit = true;     // step 2: drop one far-out mic
  float improve = 0.5f;   // the refit counts if its rms is under this times the Huber fit's
};

struct HuberFit {
  Solution s;                // the final fit (s.dropped: the mic left out, -1 if none)
  float    w[MAX_MICS];      // each mic's final weight (0: dropped, or not in the fit)
  float    z[MAX_MICS];      // each mic's residual in sigmas, in the Huber fit over all
  int      rounds;           // reweighting rounds
  int      lm;               // LM iterations over every solve
  int      far;              // mics far out in the Huber fit over all
  bool     refitted;         // step 2 dropped the one far-out mic by its weight
  bool     fellBack;         // ... the refit didn't help: the full drop-one search
  bool     uncertain;        // two or more far out: nothing dropped
};

// IRLS over `mask` with the mics' own sigmas as the scale.
inline Solution huberIrls(const SolveInput& in, uint8_t mask, const HuberOpts& o, float* w,
                          int* rounds, int* lm) {
  SolveInput q = in;
  for (int i = 0; i < MAX_MICS; i++) w[i] = (mask >> i) & 1 ? 1.0f : 0.0f;
  Solution s = solve(q, mask);
  *lm += s.iterations;
  *rounds = 0;
  for (int r = 0; r < o.rounds && s.converged; r++) {
    float change = 0;
    for (int i = 0; i < in.n; i++) {
      if (!((s.used >> i) & 1)) continue;
      const float a = fabsf(s.resNs[i]), lim = o.c * in.sigmaNs[i];
      const float nw = a <= lim ? 1.0f : lim / a;
      change = fmaxf(change, fabsf(nw - w[i]));
      w[i] = nw;
      q.sigmaNs[i] = in.sigmaNs[i] / sqrtf(nw);
    }
    if (change < 1e-3f) break;
    ++*rounds;
    s = solve(q, mask);
    *lm += s.iterations;
  }
  return s;
}

inline HuberFit solveHuber(const SolveInput& in, uint8_t mask, const HuberOpts& o) {
  HuberFit h = {};
  h.s = huberIrls(in, mask, o, h.w, &h.rounds, &h.lm);
  h.s.dropped = -1;
  int used = 0, farMic = -1;
  for (int i = 0; i < in.n; i++) {
    if (!((h.s.used >> i) & 1)) continue;
    used++;
    h.z[i] = h.s.resNs[i] / in.sigmaNs[i];
    if (fabsf(h.z[i]) > o.drop) {
      h.far++;
      farMic = i;
    }
  }
  if (!o.refit || used < 5 || !h.s.converged || h.far == 0) return h;
  if (h.far >= 2) {
    h.uncertain = true;
    return h;
  }
  // One far out: its weight names it. Refit without it; it must clearly lower the rms.
  float w2[MAX_MICS];
  int rounds2 = 0;
  Solution r = huberIrls(in, h.s.used & ~(1 << farMic), o, w2, &rounds2, &h.lm);
  if (r.converged && r.rmsNs < o.improve * h.s.rmsNs) {
    r.dropped = (int8_t)farMic;
    h.s = r;
    for (int i = 0; i < MAX_MICS; i++) h.w[i] = w2[i];
    h.refitted = true;
    return h;
  }
  // A good mic took the low weight (a high-leverage bad one elsewhere): search them all.
  Solution d = solveDropOne(in, 0, h.s.used);
  h.lm += d.iterations;
  h.s = d;
  for (int i = 0; i < MAX_MICS; i++) h.w[i] = (d.used >> i) & 1 ? 1.0f : 0.0f;
  h.fellBack = true;
  return h;
}

// The rms residual healthy mics give, from their sigmas: with m mics and 3 unknowns the
// residuals keep (m - 3) / m of the noise's variance.
inline float expectedRmsNs(const SolveInput& in, uint8_t mask) {
  int m = 0;
  float s2 = 0;
  for (int i = 0; i < in.n; i++)
    if ((mask >> i) & 1) {
      m++;
      s2 += in.sigmaNs[i] * in.sigmaNs[i];
    }
  return m > 3 ? sqrtf(s2 / m * (m - 3) / m) : 0;
}

// Drop-one started when the rms is over `trigger` times the expected rms. With `guard`: if
// the fit without the worst mic is still over the trigger (for its own mics), two or more
// are off and one drop can't fix it: the fit over all is kept, and `uncertain` set.
inline Solution solveDropSigma(const SolveInput& in, uint8_t mask, float trigger, bool guard = false,
                               bool* uncertain = nullptr) {
  Solution s = solveDropOne(in, trigger * expectedRmsNs(in, mask), mask);
  if (uncertain) *uncertain = false;
  if (guard && s.dropped >= 0 && s.rmsNs > trigger * expectedRmsNs(in, s.used)) {
    s = solve(in, mask);
    if (uncertain) *uncertain = true;
  }
  return s;
}

// Huber; if a mic is far out (and 5 or more), every mic left out in turn (a Huber fit each),
// the lowest rms kept if it is under `improve` times the Huber fit's. `uncertain`: a mic still
// far out at the end.
inline HuberFit solveHuberSearch(const SolveInput& in, uint8_t mask, const HuberOpts& o) {
  HuberOpts plain = o;
  plain.refit = false;
  HuberFit h = solveHuber(in, mask, plain);
  int used = __builtin_popcount(h.s.used);
  if (h.far && used >= 5 && h.s.converged) {
    Solution best = {};
    float bw[MAX_MICS] = {};
    bool have = false;
    for (int i = 0; i < in.n; i++) {
      if (!((h.s.used >> i) & 1)) continue;
      float w2[MAX_MICS];
      int rounds = 0;
      Solution r = huberIrls(in, h.s.used & ~(1 << i), o, w2, &rounds, &h.lm);
      r.dropped = (int8_t)i;
      if (r.converged && (!have || r.rmsNs < best.rmsNs)) {
        best = r;
        for (int j = 0; j < MAX_MICS; j++) bw[j] = w2[j];
        have = true;
      }
    }
    if (have && best.rmsNs < o.improve * h.s.rmsNs) {
      h.s = best;
      for (int j = 0; j < MAX_MICS; j++) h.w[j] = bw[j];
      h.refitted = true;
    }
  }
  int farLeft = 0;
  for (int i = 0; i < in.n; i++)
    if (((h.s.used >> i) & 1) && fabsf(h.s.resNs[i] / in.sigmaNs[i]) > o.drop) farLeft++;
  h.uncertain = farLeft > 0;
  return h;
}

}  // namespace measure
