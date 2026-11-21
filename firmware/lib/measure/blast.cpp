// OpenSights measurement core - the blast check: see blast.h.
#include "blast.h"

namespace measure {

BlastResult blastCheck(const SolveInput& base, uint8_t mask, float limitNs, float halfMm,
                       void (*yield)()) {
  BlastResult out = {0, INFINITY};
  SolveInput in = base;
  const double c = base.c;   // mm/ns
  int done = 0;
  for (int zi = 0; zi < 2; zi++) {
    const double z = zi ? 25000 : 10000;
    for (int li = 0; li <= 12; li++) {
      const double lat = 250.0 * li;
      for (int ai = 0; ai < 24; ai++) {
        const double phi = ai * 15 * M_PI / 180;
        const double sx = lat * cos(phi), sy = lat * sin(phi);
        // Times in double (some 10^7 ns), then relative to the first, in float.
        double t[MAX_MICS] = {}, first = INFINITY;
        for (int i = 0; i < base.n && i < MAX_MICS; i++) {
          if (!((mask >> i) & 1)) continue;
          const double dx = base.mic[i].x - sx, dy = base.mic[i].y - sy, dz = z + base.mic[i].h;
          t[i] = sqrt(dx * dx + dy * dy + dz * dz) / c;
          if (t[i] < first) first = t[i];
        }
        for (int i = 0; i < MAX_MICS; i++) in.t0[i] = ((mask >> i) & 1) ? (float)(t[i] - first) : 0;
        const Solution s = solve(in, mask);
        if (s.converged && fabsf(s.x) <= halfMm && fabsf(s.y) <= halfMm) {
          if (s.rmsNs < out.smallestNs) out.smallestNs = s.rmsNs;
          if (s.rmsNs <= limitNs) out.scored++;
        }
        if (yield && ++done % 100 == 0) yield();
      }
    }
  }
  return out;
}

}  // namespace measure
