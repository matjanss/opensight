// OpenSights measurement core - the position solver: see solver.h and ../../MODEL.md.
#include "solver.h"

namespace measure {
namespace {

// One fit's data, in ranges (mm) relative to the first arrival among the mics used.
struct Work {
  int     n;
  uint8_t mask;
  float   mx[MAX_MICS], my[MAX_MICS], h2[MAX_MICS];
  float   s[MAX_MICS];   // c (t0_i - t0_first)
  float   w[MAX_MICS];   // relative weights (sigmaRef / sigma_i)^2
  float   wsum;
};

inline bool usedBit(const Work& k, int i) { return (k.mask >> i) & 1; }

// Distances, the best range offset D for this point (weighted mean of s - d) and the
// weighted cost sum w (d + D - s)^2.
float evalAt(const Work& k, float x, float y, float* d, float& D) {
  float swr = 0;
  for (int i = 0; i < k.n; i++) {
    const float dx = x - k.mx[i], dy = y - k.my[i];
    d[i] = sqrtf(dx * dx + dy * dy + k.h2[i]);
    if (d[i] < 1e-3f) d[i] = 1e-3f;   // a mic in the paper plane, the point on top of it
    if (usedBit(k, i)) swr += k.w[i] * (k.s[i] - d[i]);
  }
  D = swr / k.wsum;
  float cost = 0;
  for (int i = 0; i < k.n; i++)
    if (usedBit(k, i)) {
      const float r = d[i] + D - k.s[i];
      cost += k.w[i] * r * r;
    }
  return cost;
}

// The normal matrix A = sum w J J^T (upper triangle: a11 a12 a13 a22 a23 a33) and
// g = sum w J r, J = (dd/dx, dd/dy, 1), at (x, y) with its D.
void normal(const Work& k, float x, float y, const float* d, float D, float* A, float* g) {
  for (int j = 0; j < 6; j++) A[j] = 0;
  g[0] = g[1] = g[2] = 0;
  for (int i = 0; i < k.n; i++) {
    if (!usedBit(k, i)) continue;
    const float jx = (x - k.mx[i]) / d[i], jy = (y - k.my[i]) / d[i];
    const float r = d[i] + D - k.s[i], w = k.w[i];
    A[0] += w * jx * jx; A[1] += w * jx * jy; A[2] += w * jx;
    A[3] += w * jy * jy; A[4] += w * jy;      A[5] += w;
    g[0] += w * jx * r;  g[1] += w * jy * r;  g[2] += w * r;
  }
}

// Inverse of a symmetric 3x3 (same packing as A); false if singular.
bool invert3(const float* A, float* inv) {
  const float a11 = A[0], a12 = A[1], a13 = A[2], a22 = A[3], a23 = A[4], a33 = A[5];
  const float c11 = a22 * a33 - a23 * a23, c12 = a13 * a23 - a12 * a33, c13 = a12 * a23 - a13 * a22;
  const float det = a11 * c11 + a12 * c12 + a13 * c13;
  const float scale = a11 * a22 * a33;
  if (!(fabsf(det) > 1e-7f * fabsf(scale)) || !(scale > 0)) return false;   // mics in a line
  const float c22 = a11 * a33 - a13 * a13, c23 = a12 * a13 - a11 * a23, c33 = a11 * a22 - a12 * a12;
  inv[0] = c11 / det; inv[1] = c12 / det; inv[2] = c13 / det;
  inv[3] = c22 / det; inv[4] = c23 / det; inv[5] = c33 / det;
  return true;
}

// Levenberg-Marquardt from (x, y). True if converged; x, y hold the last accepted point.
bool runFrom(const Work& k, float& x, float& y, int& iterations) {
  float d[MAX_MICS], D;
  float cost = evalAt(k, x, y, d, D);
  float lambda = 1e-3f;
  for (int it = 0; it < SOLVE_MAX_ITER; it++) {
    iterations++;
    float A[6], g[3], inv[6];
    normal(k, x, y, d, D, A, g);
    float Ad[6] = {A[0] * (1 + lambda), A[1], A[2], A[3] * (1 + lambda), A[4], A[5] * (1 + lambda)};
    if (!invert3(Ad, inv)) return false;
    const float sx = -(inv[0] * g[0] + inv[1] * g[1] + inv[2] * g[2]);
    const float sy = -(inv[1] * g[0] + inv[3] * g[1] + inv[4] * g[2]);
    const float nx = x + sx, ny = y + sy;
    if (!(fabsf(nx) < SOLVE_FAR_MM && fabsf(ny) < SOLVE_FAR_MM)) return false;
    const bool small = sx * sx + sy * sy < SOLVE_DONE_MM * SOLVE_DONE_MM;
    float nd[MAX_MICS], nD;
    const float ncost = evalAt(k, nx, ny, nd, nD);
    // A tiny step is taken even if the cost doesn't drop: at the minimum the cost is flat
    // to float's precision, while the step (from the gradient) is still accurate.
    if (ncost <= cost || small) {
      x = nx;
      y = ny;
      cost = ncost;
      D = nD;
      for (int i = 0; i < k.n; i++) d[i] = nd[i];
      lambda = fmaxf(lambda * 0.1f, 1e-6f);
      if (small) return true;
    } else {
      lambda *= 10;
    }
  }
  return false;
}

}  // namespace

Solution solve(const SolveInput& in, uint8_t mask) {
  Solution out = {};
  out.dropped = -1;
  Work k;
  k.n = in.n < MAX_MICS ? in.n : MAX_MICS;
  k.mask = mask & (uint8_t)((1u << k.n) - 1);
  int used = 0, first = -1;
  float sigmaRef = 0;
  for (int i = 0; i < k.n; i++) {
    if (!usedBit(k, i)) continue;
    if (!(in.sigmaNs[i] > 0)) { k.mask &= ~(1 << i); continue; }
    used++;
    if (first < 0 || in.t0[i] < in.t0[first]) first = i;
    if (sigmaRef == 0 || in.sigmaNs[i] < sigmaRef) sigmaRef = in.sigmaNs[i];
  }
  out.used = k.mask;
  if (used < 3 || !(in.c > 0)) return out;

  k.wsum = 0;
  for (int i = 0; i < k.n; i++) {
    k.mx[i] = in.mic[i].x;
    k.my[i] = in.mic[i].y;
    k.h2[i] = in.mic[i].h * in.mic[i].h;
    k.s[i] = in.c * (in.t0[i] - in.t0[first]);
    const float q = in.sigmaNs[i] > 0 ? sigmaRef / in.sigmaNs[i] : 0;
    k.w[i] = q * q;
    if (usedBit(k, i)) k.wsum += k.w[i];
  }

  int iterations = 0;
  float x = 0.5f * k.mx[first], y = 0.5f * k.my[first];
  out.starts = 1;
  bool ok = runFrom(k, x, y, iterations);
  if (!ok) {
    out.starts = 2;
    x = 0;
    y = 0;
    ok = runFrom(k, x, y, iterations);
  }
  out.converged = ok;
  out.iterations = (uint8_t)iterations;

  float d[MAX_MICS], D;
  evalAt(k, x, y, d, D);
  float ss = 0;
  for (int i = 0; i < k.n; i++) {
    out.resNs[i] = (k.s[i] - d[i] - D) / in.c;
    if (usedBit(k, i)) ss += out.resNs[i] * out.resNs[i];
  }
  out.rmsNs = sqrtf(ss / used);
  out.x = x;
  out.y = y;
  out.tauNs = in.t0[first] + D / in.c;

  // Covariance (sigmaRef c)^2 A^-1 at the solution; x and y's block.
  float A[6], g[3], inv[6];
  normal(k, x, y, d, D, A, g);
  if (invert3(A, inv)) {
    const float s2 = (sigmaRef * in.c) * (sigmaRef * in.c);
    const float cxx = s2 * inv[0], cxy = s2 * inv[1], cyy = s2 * inv[3];
    out.sx = sqrtf(cxx);
    out.sy = sqrtf(cyy);
    out.rho = out.sx > 0 && out.sy > 0 ? cxy / (out.sx * out.sy) : 0;
    const float m = 0.5f * (cxx + cyy), q = 0.5f * (cxx - cyy);
    out.major = sqrtf(m + sqrtf(q * q + cxy * cxy));
  }
  return out;
}

Solution solveDropOne(const SolveInput& in, float limitNs, uint8_t mask) {
  Solution full = solve(in, mask);
  int used = 0;
  for (int i = 0; i < MAX_MICS; i++) used += (full.used >> i) & 1;
  if ((full.converged && full.rmsNs <= limitNs) || used < 5) return full;
  Solution best = {};
  bool have = false;
  for (int i = 0; i < in.n && i < MAX_MICS; i++) {
    if (!((full.used >> i) & 1)) continue;
    Solution s = solve(in, full.used & ~(1 << i));
    s.dropped = (int8_t)i;
    // A converged fit beats one that didn't; then the lower residual.
    if (!have || (s.converged && !best.converged) ||
        (s.converged == best.converged && s.rmsNs < best.rmsNs)) {
      best = s;
      have = true;
    }
  }
  return have ? best : full;
}

}  // namespace measure
