// OpenSights - tests of the portable measurement core on the PC: `pio test -e native`.
// MEASUREMENT.md step 3b lists what they check. The vectors come from tools/model.py, the
// reference implementation of MODEL.md.
#include <unity.h>

#include <chrono>
#include <cmath>
#include <cstdio>
#include <fstream>
#include <random>
#include <sstream>
#include <string>
#include <vector>

#include "air.h"
#include "bme280_comp.h"
#include "model.h"
#include "solver.h"

using namespace measure;

void setUp() {}
void tearDown() {}

// PLAN D1/D2: CH1 bottom left, CH2 bottom right, CH3 top right slid 40 mm, CH4 top left,
// CH5 bottom middle, CH6 right middle.
static const float D2[6][2] = {{-105, -105}, {105, -105}, {65, 105}, {-105, 105}, {0, -105}, {105, 0}};

static SolveInput layout(float h, float sigma, float cMs) {
  SolveInput in = {};
  in.n = 6;
  for (int i = 0; i < 6; i++) {
    in.mic[i] = {D2[i][0], D2[i][1], h};
    in.sigmaNs[i] = sigma;
  }
  in.c = mmPerNs(cMs);
  return in;
}

static void shoot(SolveInput& in, float x, float y, float tau) {
  for (int i = 0; i < in.n; i++) in.t0[i] = arrivalAt(in.mic[i], x, y, tau, in.c);
}

// ---- Speed of sound, BME280 compensation, arrival time ----

static void test_sound_speed_reference() {
  TEST_ASSERT_DOUBLE_WITHIN(0.001, 331.448, soundSpeedMs(0, 0, 101325));
  TEST_ASSERT_DOUBLE_WITHIN(0.001, 343.359, soundSpeedMs(20, 0, 101325));
  TEST_ASSERT_DOUBLE_WITHIN(0.001, 343.987, soundSpeedMs(20, 50, 101325));
  TEST_ASSERT_DOUBLE_WITHIN(0.001, 342.740, soundSpeedMs(18, 50, 101325));
  TEST_ASSERT_DOUBLE_WITHIN(0.001, 347.127, soundSpeedMs(25, 50, 101325));
}

static void test_sound_speed_vectors() {
  std::ifstream f(VECTOR_DIR "/air.txt");
  TEST_ASSERT_TRUE_MESSAGE(f.good(), "test/vectors/air.txt missing: run tools/model.py");
  std::string line;
  int n = 0;
  double worst = 0;
  while (std::getline(f, line)) {
    if (line.empty() || line[0] == '#') continue;
    double t, rh, p, c;
    TEST_ASSERT_EQUAL(4, sscanf(line.c_str(), "%lf %lf %lf %lf", &t, &rh, &p, &c));
    worst = std::fmax(worst, std::fabs(soundSpeedMs(t, rh, p) - c));
    n++;
  }
  printf("speed of sound: %d cases, worst %.2e m/s\n", n, worst);
  TEST_ASSERT_TRUE(n > 100);
  TEST_ASSERT_TRUE(worst < 0.001);
}

static void test_bme280_datasheet_example() {
  // BMP280 datasheet, section 3.12: the same T and P formulas as the BME280.
  Bme280Calib c = {27504, 26435, -1000, 36477, -10685, 3024, 2855, 140, -7, 15500, -14600, 6000,
                   0, 0, 0, 0, 0, 0};
  int32_t tFine;
  TEST_ASSERT_EQUAL_INT32(2508, bme280CompT(c, 519888, tFine));   // 25.08 degC
  TEST_ASSERT_EQUAL_INT32(128422, tFine);
  uint32_t p = bme280CompP(c, 415148, tFine);   // 100653.27 Pa (rounded in the datasheet)
  TEST_ASSERT_UINT32_WITHIN(8, 25767236, p);
}

static void test_arrival_t0() {
  TEST_ASSERT_FLOAT_WITHIN(1e-3f, -266.667f, arrivalT0(0, 800, 1.0f / 3, 0));
  TEST_ASSERT_FLOAT_WITHIN(1e-3f, 1000 - 0.5f * 600 - 120, arrivalT0(1000, 1600, 0.5f, 120));
}

// ---- The solver on its own ----

static void test_grid_noise_free() {
  int cases = 0, maxIt = 0, second = 0;
  float worst = 0;
  for (float h : {10.5f, 21.5f}) {
    SolveInput in = layout(h, 50, 343.99f);
    for (float x = -100; x <= 100.01f; x += 5)
      for (float y = -100; y <= 100.01f; y += 5) {
        shoot(in, x, y, 1000);
        Solution s = solve(in);
        TEST_ASSERT_TRUE(s.converged);
        float e = std::hypot(s.x - x, s.y - y);
        if (e > worst) worst = e;
        if (s.iterations > maxIt) maxIt = s.iterations;
        if (s.starts > 1) second++;
        TEST_ASSERT_FLOAT_WITHIN(1.0f, 1000.0f, s.tauNs);
        cases++;
      }
  }
  printf("noise-free grid: %d points, worst %.5f mm, at most %d iterations, %d needed the "
         "second start\n", cases, worst, maxIt, second);
  TEST_ASSERT_TRUE(worst < 0.001f);
  TEST_ASSERT_TRUE(maxIt <= SOLVE_MAX_ITER);
}

static void test_noise_matches_uncertainty() {
  const float sigma = 50;
  std::mt19937 rng(7);
  std::normal_distribution<float> noise(0, sigma);
  const float pts[][2] = {{0, 0}, {50, -30}, {80, 80}, {-90, 60}, {-95, -95}};
  for (auto& p : pts) {
    SolveInput in = layout(10.5f, sigma, 343.99f);
    shoot(in, p[0], p[1], 500);
    Solution ref = solve(in);
    const float t[6] = {in.t0[0], in.t0[1], in.t0[2], in.t0[3], in.t0[4], in.t0[5]};
    const int N = 4000;
    double sx = 0, sy = 0, mx = 0, my = 0;
    for (int k = 0; k < N; k++) {
      for (int i = 0; i < 6; i++) in.t0[i] = t[i] + noise(rng);
      Solution s = solve(in);
      TEST_ASSERT_TRUE(s.converged);
      double ex = s.x - p[0], ey = s.y - p[1];
      sx += ex * ex; sy += ey * ey; mx += ex; my += ey;
    }
    double rx = std::sqrt(sx / N), ry = std::sqrt(sy / N);
    printf("noise %.0f ns at (%.0f, %.0f): rms error x %.4f y %.4f mm, reported %.4f %.4f "
           "(major %.4f), bias %+.4f %+.4f\n", sigma, p[0], p[1], rx, ry, ref.sx, ref.sy,
           ref.major, mx / N, my / N);
    TEST_ASSERT_DOUBLE_WITHIN(0.10 * ref.sx, ref.sx, rx);
    TEST_ASSERT_DOUBLE_WITHIN(0.10 * ref.sy, ref.sy, ry);
  }
}

static void test_too_few_mics() {
  SolveInput in = layout(10.5f, 50, 343.99f);
  shoot(in, 10, 20, 0);
  TEST_ASSERT_FALSE(solve(in, 0x03).converged);   // 2 mics
  Solution s = solve(in, 0x07);                    // 3 mics: exact, no residual
  TEST_ASSERT_TRUE(s.converged);
  TEST_ASSERT_FLOAT_WITHIN(0.001f, 10, s.x);
  TEST_ASSERT_FLOAT_WITHIN(0.001f, 20, s.y);
}

// ---- The shared vectors ----

struct Case {
  std::string name, kind;
  float limit;
  uint8_t mask;
  SolveInput in;
  bool ok;
  int dropped;
  float x, y, tau, rms, res[MAX_MICS], sx, sy, rho, major;
};

static std::vector<Case> readCases() {
  std::vector<Case> out;
  std::ifstream f(VECTOR_DIR "/solve.txt");
  TEST_ASSERT_TRUE_MESSAGE(f.good(), "test/vectors/solve.txt missing: run tools/model.py");
  std::string line;
  while (std::getline(f, line)) {
    if (line.empty() || line[0] == '#') continue;
    std::istringstream s(line);
    Case c = {};
    std::string maskHex, bar;
    double cc;
    s >> c.name >> c.kind >> c.limit >> c.in.n >> maskHex >> cc;
    c.mask = (uint8_t)std::stoul(maskHex, nullptr, 16);
    c.in.c = (float)cc;
    for (int i = 0; i < c.in.n; i++) s >> c.in.mic[i].x >> c.in.mic[i].y >> c.in.mic[i].h;
    for (int i = 0; i < c.in.n; i++) s >> c.in.t0[i];
    for (int i = 0; i < c.in.n; i++) s >> c.in.sigmaNs[i];
    s >> bar;
    TEST_ASSERT_EQUAL_STRING("|", bar.c_str());
    int ok;
    s >> ok;
    c.ok = ok;
    if (ok) {
      s >> c.dropped >> c.x >> c.y >> c.tau >> c.rms;
      for (int i = 0; i < c.in.n; i++) s >> c.res[i];
      s >> c.sx >> c.sy >> c.rho >> c.major;
    }
    TEST_ASSERT_FALSE_MESSAGE(s.fail(), c.name.c_str());
    out.push_back(c);
  }
  return out;
}

static void test_vectors() {
  std::vector<Case> cases = readCases();
  int fits = 0, drops = 0, outside = 0, outsideConv = 0, blasts = 0;
  float worstPos = 0, worstRes = 0, worstTau = 0, worstSig = 0;
  char msg[160];
  for (const Case& c : cases) {
    if (c.kind == "fit" || c.kind == "drop") {
      Solution s = c.kind == "fit" ? solve(c.in, c.mask) : solveDropOne(c.in, c.limit, c.mask);
      snprintf(msg, sizeof msg, "%s: x %.4f y %.4f (want %.4f %.4f), dropped %d (want %d), conv %d",
               c.name.c_str(), s.x, s.y, c.x, c.y, s.dropped, c.dropped, s.converged);
      TEST_ASSERT_TRUE_MESSAGE(c.ok && s.converged, msg);
      TEST_ASSERT_EQUAL_INT_MESSAGE(c.dropped, s.dropped, msg);
      float e = std::hypot(s.x - c.x, s.y - c.y);
      TEST_ASSERT_TRUE_MESSAGE(e < 0.001f, msg);
      worstPos = std::fmax(worstPos, e);
      worstTau = std::fmax(worstTau, std::fabs(s.tauNs - c.tau));
      worstRes = std::fmax(worstRes, std::fabs(s.rmsNs - c.rms));
      for (int i = 0; i < c.in.n; i++) worstRes = std::fmax(worstRes, std::fabs(s.resNs[i] - c.res[i]));
      worstSig = std::fmax(worstSig, std::fabs(s.sx / c.sx - 1));
      worstSig = std::fmax(worstSig, std::fabs(s.sy / c.sy - 1));
      worstSig = std::fmax(worstSig, std::fabs(s.major / c.major - 1));
      TEST_ASSERT_FLOAT_WITHIN_MESSAGE(0.001f, c.rho, s.rho, msg);
      (c.kind == "fit" ? fits : drops)++;
    } else if (c.kind == "outside") {
      // Noise-free points outside the frame: the solver may fail to converge, but a fit
      // that converges with a small residual must be the true point.
      Solution s = solve(c.in, c.mask);
      outside++;
      if (s.converged && s.rmsNs < 10) {
        outsideConv++;
        snprintf(msg, sizeof msg, "%s: x %.4f y %.4f, want %.4f %.4f", c.name.c_str(), s.x, s.y, c.x, c.y);
        TEST_ASSERT_TRUE_MESSAGE(std::hypot(s.x - c.x, s.y - c.y) < 0.01f, msg);
      }
    } else if (c.kind == "blast") {
      // No blast may fit as a hit in the scoring area, with or without drop-one.
      for (int k = 0; k < 2; k++) {
        Solution s = k ? solveDropOne(c.in, c.limit, c.mask) : solve(c.in, c.mask);
        bool hit = s.converged && s.rmsNs <= c.limit && std::fabs(s.x) <= 100 && std::fabs(s.y) <= 100;
        snprintf(msg, sizeof msg, "%s%s: fits as a hit at (%.1f, %.1f), rms %.0f ns", c.name.c_str(),
                 k ? " (drop-one)" : "", s.x, s.y, s.rmsNs);
        TEST_ASSERT_FALSE_MESSAGE(hit, msg);
      }
      blasts++;
    }
  }
  printf("vectors: %d fits, %d drop-one, worst position %.6f mm, residual %.3f ns, tau %.3f ns, "
         "sigma %.2e (relative)\n", fits, drops, worstPos, worstRes, worstTau, worstSig);
  printf("vectors: %d points outside the frame, %d converged (all to the true point); %d blasts, "
         "none scored\n", outside, outsideConv, blasts);
  TEST_ASSERT_TRUE(fits >= 300 && drops >= 100 && outside >= 50 && blasts >= 50);
  // Single precision: arrival times near 1e6 ns are float steps of 0.06 ns apart, so the
  // residuals agree to a few tenths of a ns (0.5 ns is 0.0002 mm of range).
  TEST_ASSERT_TRUE(worstRes < 0.5f);
  TEST_ASSERT_TRUE(worstTau < 0.5f);
  TEST_ASSERT_TRUE(worstSig < 1e-3f);
}

static void test_time_per_solve() {
  SolveInput in = layout(10.5f, 50, 343.99f);
  shoot(in, 37, -61, 0);
  const int N = 20000;
  float sink = 0;
  auto t0 = std::chrono::steady_clock::now();
  for (int k = 0; k < N; k++) {
    in.t0[k % 6] += 0.01f;
    sink += solve(in).x;
  }
  auto t1 = std::chrono::steady_clock::now();
  printf("time per solve on this PC: %.2f us (the ESP32's is measured in 3c) [%g]\n",
         std::chrono::duration<double, std::micro>(t1 - t0).count() / N, sink > 0 ? 1.0 : 0.0);
}

int main(int, char**) {
  UNITY_BEGIN();
  RUN_TEST(test_sound_speed_reference);
  RUN_TEST(test_sound_speed_vectors);
  RUN_TEST(test_bme280_datasheet_example);
  RUN_TEST(test_arrival_t0);
  RUN_TEST(test_grid_noise_free);
  RUN_TEST(test_noise_matches_uncertainty);
  RUN_TEST(test_too_few_mics);
  RUN_TEST(test_vectors);
  RUN_TEST(test_time_per_solve);
  return UNITY_END();
}
