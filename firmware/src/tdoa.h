// OpenSights - the position solver's arithmetic (measurement step 3, see ../MEASUREMENT.md).
// No Arduino dependencies, so it can be tested on a PC.
//
// Sound leaves the shot at (x, y) at an unknown time and reaches mic i at
//   t0_i = T + |p - m_i| / c.
// With every distance in mm (s_i = c t0_i, D = c T) the residual of mic i is
//   r_i = |p - m_i| + D - s_i,
// and Gauss-Newton fits x, y and D to all the mics (3 unknowns: 3 mics are an exact
// solution, 4 or more leave a residual that shows how well the times fit one point).
// Single precision is enough: the coordinates are taken relative to the mics' centroid and
// the times relative to the event's first edge, so no value is much over 1000 and float's
// 7 digits leave about 0.0001 mm.
#pragma once
#include <math.h>
#include <stdint.h>

static const int TDOA_MAX_MICS = 8;

struct TdoaResult {
  bool    converged;   // the last step moved the point less than TDOA_DONE_MM
  uint8_t iterations;
  float   x, y;        // mm, in the mics' frame
  float   rmsMm;       // rms residual over the mics, mm (divide by c for time)
  float   resMm[TDOA_MAX_MICS];
};

static const int   TDOA_MAX_ITER = 10;     // 3-5 needed inside the mics (step 3 model)
static const float TDOA_DONE_MM  = 0.001f;
static const float TDOA_FAR_MM   = 2000;   // a point this far from the mics: no solution

// n mics at (mx, my) mm, arrival times t0 ns (any common origin), c in mm/ns.
// Starts halfway between the first-hit mic and the centroid (at a mic its distance is 0
// and the direction undefined).
inline TdoaResult tdoaSolve(int n, const float* mx, const float* my, const float* t0, float c) {
  TdoaResult out = {};
  if (n < 3 || n > TDOA_MAX_MICS || !(c > 0)) return out;
  float cx = 0, cy = 0;
  for (int i = 0; i < n; i++) { cx += mx[i]; cy += my[i]; }
  cx /= n;
  cy /= n;
  float ux[TDOA_MAX_MICS], uy[TDOA_MAX_MICS], s[TDOA_MAX_MICS];
  int first = 0;
  for (int i = 0; i < n; i++) {
    ux[i] = mx[i] - cx;
    uy[i] = my[i] - cy;
    if (t0[i] < t0[first]) first = i;
  }
  for (int i = 0; i < n; i++) s[i] = c * (t0[i] - t0[first]);
  float x = 0.5f * ux[first], y = 0.5f * uy[first];

  float d[TDOA_MAX_MICS];
  float D = 0;
  auto distances = [&]() {
    D = 0;
    for (int i = 0; i < n; i++) {
      float dx = x - ux[i], dy = y - uy[i];
      d[i] = sqrtf(dx * dx + dy * dy);
      if (d[i] < 1e-3f) d[i] = 1e-3f;
      D += s[i] - d[i];
    }
    D /= n;   // the best D for this point
  };

  for (int it = 1; it <= TDOA_MAX_ITER; it++) {
    distances();
    // Normal equations J^T J delta = -J^T r, J row = ((x - ux)/d, (y - uy)/d, 1).
    float a11 = 0, a12 = 0, a13 = 0, a22 = 0, a23 = 0, a33 = 0, b1 = 0, b2 = 0, b3 = 0;
    for (int i = 0; i < n; i++) {
      float jx = (x - ux[i]) / d[i], jy = (y - uy[i]) / d[i];
      float r = d[i] + D - s[i];
      a11 += jx * jx; a12 += jx * jy; a13 += jx;
      a22 += jy * jy; a23 += jy;      a33 += 1;
      b1 -= jx * r;   b2 -= jy * r;   b3 -= r;
    }
    // Cramer's rule on the symmetric 3x3.
    float c11 = a22 * a33 - a23 * a23, c12 = a13 * a23 - a12 * a33, c13 = a12 * a23 - a13 * a22;
    float det = a11 * c11 + a12 * c12 + a13 * c13;
    if (!(fabsf(det) > 1e-12f)) break;   // the mics in a line, or the point on top of them
    float c22 = a11 * a33 - a13 * a13, c23 = a12 * a13 - a11 * a23;
    float sx = (c11 * b1 + c12 * b2 + c13 * b3) / det;
    float sy = (c12 * b1 + c22 * b2 + c23 * b3) / det;
    x += sx;
    y += sy;
    out.iterations = it;
    if (!(fabsf(x) < TDOA_FAR_MM && fabsf(y) < TDOA_FAR_MM)) break;
    if (sx * sx + sy * sy < TDOA_DONE_MM * TDOA_DONE_MM) { out.converged = true; break; }
  }
  distances();
  float ss = 0;
  for (int i = 0; i < n; i++) {
    out.resMm[i] = d[i] + D - s[i];
    ss += out.resMm[i] * out.resMm[i];
  }
  out.rmsMm = sqrtf(ss / n);
  out.x = x + cx;
  out.y = y + cy;
  return out;
}

// Speed of sound in dry air, mm/ns (331.3 m/s at 0 degC, +0.606 m/s per degC).
inline float soundSpeedMmPerNs(float tempC) { return (331.3f + 0.606f * tempC) * 1e-6f; }
