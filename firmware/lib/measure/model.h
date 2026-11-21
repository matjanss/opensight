// OpenSights measurement core - the propagation model (../../MODEL.md is its source of
// truth; tools/model.py is the reference implementation, with the shared test vectors).
// Portable: no Arduino, no IDF; also built on the PC.
//
// Frame coordinates: mm from the frame centre, x to the right, y up, as seen by the shooter.
// Times: ns. The speed of sound: mm/ns (m/s * 1e-6).
#pragma once
#include <math.h>
#include <stdint.h>

namespace measure {

static const int MAX_MICS = 8;

// One mic: its position in the paper plane and its height behind the paper (the mic port's
// distance from the paper plane: 10.5 mm, 21.5 mm with the sensor board flipped).
struct Mic {
  float x, y, h;   // mm
};

// Speed of sound in mm/ns from m/s.
inline float mmPerNs(float mPerS) { return mPerS * 1e-6f; }

// Distance from a hole at (x, y) in the paper plane to a mic.
inline float micDistance(const Mic& m, float x, float y) {
  const float dx = x - m.x, dy = y - m.y;
  return sqrtf(dx * dx + dy * dy + m.h * m.h);
}

// The arrival time at a mic of a crack made at (x, y) at time tau (ns), c in mm/ns.
inline float arrivalAt(const Mic& m, float x, float y, float tau, float c) {
  return tau + micDistance(m, x, y) / c;
}

// A channel's arrival time from its two comparator edges (MODEL.md "Arrival"): t1 = OUT_L's
// first rise, t2 = OUT_H's, both ns; k per channel; offset = the channel's own delay, ns.
inline float arrivalT0(float t1, float t2, float k, float offsetNs) {
  return t1 - k * (t2 - t1) - offsetNs;
}

}  // namespace measure
