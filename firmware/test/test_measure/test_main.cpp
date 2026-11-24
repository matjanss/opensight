// OpenSights - tests of the portable measurement core on the PC: `pio test -e native`.
// MEASUREMENT.md step 3b lists what they check. The vectors come from tools/model.py, the
// reference implementation of MODEL.md.
#include <unity.h>

#include <chrono>
#include <cmath>
#include <cstdio>
#include <cstring>
#include <fstream>
#include <random>
#include <sstream>
#include <string>
#include <vector>

#include "air.h"
#include "blast.h"
#include "bme280_comp.h"
#include "model.h"
#include "record.h"
#include "solver.h"

using namespace measure;

void setUp() {}
void tearDown() {}

// PLAN D1/D2: CH1 bottom left, CH2 bottom right, CH3 top right 25 mm out along its diagonal, CH4 top left,
// CH5 bottom middle, CH6 right middle.
static const float D2[6][2] = {{-105, -105}, {105, -105}, {122.68f, 122.68f}, {-105, 105}, {0, -105}, {105, 0}};

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

// ---- Log lines (record.h) and the blast check (blast.h) ----

static void test_crc32() {
  TEST_ASSERT_EQUAL_HEX32(0xCBF43926u, crc32("123456789"));   // the standard check value
  TEST_ASSERT_EQUAL_HEX32(0u, crc32(""));
}

static Cfg sampleCfg() {
  Cfg cfg = {};
  for (int c = 0; c < 6; c++)
    cfg.ch[c] = {true, D2[c][0], D2[c][1], 10.5f, 1.0f / 3, -12.345f + c, 50};
  cfg.ch[2].h = 21.5f;
  cfg.ch[4].k = 0.3333f;
  cfg.ch[6].k = 0.25f;   // no mic, but its own k
  cfg.resLimitNs = 1234.5f;
  cfg.dropOne = true;
  cfg.maxLost = -1;   // validity off, as before 4a (test_validity turns it on)
  return cfg;
}

static void test_cfg_round_trip() {
  const Cfg cfg = sampleCfg();
  char line[640];
  formatCfg(line, sizeof line, cfg);
  printf("%s\n", line);
  Cfg back;
  uint32_t id = 0;
  TEST_ASSERT_TRUE(parseCfg(line, back, &id));
  TEST_ASSERT_EQUAL_HEX32(cfgId(cfg), id);
  for (int c = 0; c < MAX_MICS; c++) {
    TEST_ASSERT_EQUAL(cfg.ch[c].mic, back.ch[c].mic);
    if (!cfg.ch[c].mic) continue;
    // Bit for bit: %.9g gives every float back exactly.
    TEST_ASSERT_TRUE(cfg.ch[c].x == back.ch[c].x && cfg.ch[c].y == back.ch[c].y &&
                     cfg.ch[c].h == back.ch[c].h && cfg.ch[c].k == back.ch[c].k &&
                     cfg.ch[c].offsetNs == back.ch[c].offsetNs && cfg.ch[c].sigmaNs == back.ch[c].sigmaNs);
  }
  TEST_ASSERT_TRUE(back.resLimitNs == cfg.resLimitNs && back.dropOne);
  TEST_ASSERT_TRUE(back.maxLost == -1 && back.areaMm == 0 && back.togetherNs == 0 && back.pulseMaxNs == 0);
  TEST_ASSERT_TRUE(back.curtainLoNs == 0 && back.curtainHiNs == 0 && strstr(line, " ir=-"));
  // The validity limits (4a) round-trip too, and change the cfg's id but not the calibration's.
  Cfg v = cfg;
  v.maxLost = 1;
  v.areaMm = 105;
  v.togetherNs = 20000;
  v.pulseMaxNs = 10000.5f;
  char vline[640];
  formatCfg(vline, sizeof vline, v);
  printf("%s\n", vline);
  TEST_ASSERT_TRUE(parseCfg(vline, back, &id));
  TEST_ASSERT_TRUE(back.maxLost == 1 && back.areaMm == 105 && back.togetherNs == 20000 &&
                   back.pulseMaxNs == 10000.5f);
  TEST_ASSERT_NOT_EQUAL(cfgId(cfg), cfgId(v));
  TEST_ASSERT_EQUAL_HEX32(calibrationId(cfg), calibrationId(v));
  // A 3c line (before `lim=` and `drop=`): 1500 ns, no drop-one, as 3c had.
  TEST_ASSERT_TRUE(parseCfg("cfg id=C2127541 CH1=- CH2=- CH3=- CH4=- CH5=-105,-105,10.5,0.333333343,0,50 "
                            "CH6=105,-105,10.5,0.333333343,0,50 CH7=105,105,10.5,0.333333343,0,50 "
                            "CH8=-105,105,10.5,0.333333343,0,50", back, &id));
  TEST_ASSERT_TRUE(back.resLimitNs == 1500 && !back.dropOne && back.ch[4].mic && !back.ch[0].mic);
  TEST_ASSERT_TRUE(back.maxLost == -1 && back.areaMm == 0 && back.togetherNs == 0 && back.pulseMaxNs == 0);
  // With text before it (a monitor's timestamp), and a line end.
  std::string withPrefix = std::string("12:00:01.123 > ") + line + "\r\n";
  TEST_ASSERT_TRUE(parseCfg(withPrefix.c_str(), back, &id));
  // A changed digit no longer matches its id.
  std::string bad = line;
  bad[bad.find("CH2=") + 5] ^= 1;
  TEST_ASSERT_FALSE(parseCfg(bad.c_str(), back, &id));
  // Any setting changes the id.
  Cfg other = cfg;
  other.ch[3].h = 10.6f;
  TEST_ASSERT_NOT_EQUAL(cfgId(cfg), cfgId(other));
  TEST_ASSERT_NOT_EQUAL(calibrationId(cfg), calibrationId(other));
  // The calibration's id leaves the policy out; the cfg's doesn't.
  other = cfg;
  other.resLimitNs = 2000;
  other.dropOne = false;
  TEST_ASSERT_NOT_EQUAL(cfgId(cfg), cfgId(other));
  TEST_ASSERT_EQUAL_HEX32(calibrationId(cfg), calibrationId(other));
  std::string fields = std::string(line).substr(strlen("cfg id=12345678 "));
  fields = fields.substr(0, fields.find(" lim="));
  TEST_ASSERT_EQUAL_HEX32(crc32(fields.c_str()), calibrationId(cfg));
}

static void test_rec_round_trip() {
  Rec r = {};
  r.n = 2161;
  r.timeS = 876.1903407;
  r.cfgId = 0x1A2B3C4D;
  r.tC = 18;
  r.rh = 50;
  r.hPa = 1013.25f;
  strcpy(r.measured, "T--");
  r.cMs = (float)soundSpeedMs(18, 50, 101325);
  for (int c = 0; c < MAX_MICS; c++) r.t1[c] = r.t2[c] = NO_EDGE;
  r.channels = 0xF3;   // CH1, CH2, CH5-CH8
  const int32_t t1[8] = {0, 100000, 0, 0, 200000, 300000, 412300, NO_EDGE};
  const int32_t t2[8] = {1500, 101500, 0, 0, 201000, NO_EDGE, 413900, NO_EDGE};
  for (int c = 0; c < MAX_MICS; c++)
    if (r.channels & (1 << c)) { r.t1[c] = t1[c]; r.t2[c] = t2[c]; }
  char line[400];
  formatRec(line, sizeof line, r);
  printf("%s\n", line);
  Rec b;
  TEST_ASSERT_TRUE(parseRec((std::string("  ") + line + "\n").c_str(), b));
  TEST_ASSERT_EQUAL_UINT32(r.n, b.n);
  TEST_ASSERT_EQUAL_HEX32(r.cfgId, b.cfgId);
  TEST_ASSERT_EQUAL_HEX8(r.channels, b.channels);
  TEST_ASSERT_EQUAL_STRING("T--", b.measured);
  TEST_ASSERT_TRUE(r.cMs == b.cMs);   // exact
  TEST_ASSERT_DOUBLE_WITHIN(1e-7, r.timeS, b.timeS);
  for (int c = 0; c < MAX_MICS; c++) {
    TEST_ASSERT_EQUAL_INT32(r.t1[c], b.t1[c]);
    TEST_ASSERT_EQUAL_INT32(r.t2[c], b.t2[c]);
  }
  // The solver's input: CH1, CH2, CH5 have a mic and both edges; CH6 has no H edge, CH7 no
  // mic, CH8 no edges, CH3/CH4 were not fitted.
  const Cfg cfg = sampleCfg();
  SolveInput in;
  TEST_ASSERT_EQUAL_HEX8(0x13, solveInput(cfg, b, in));
  TEST_ASSERT_TRUE(in.t0[4] == arrivalT0(200000.0f, 201000.0f, 0.3333f, cfg.ch[4].offsetNs));
  TEST_ASSERT_TRUE(in.c == mmPerNs(r.cMs));
  TEST_ASSERT_FALSE(b.hasPt || b.calMode || b.hasCal);   // "cal=-"
  // The true point, calibration mode and the record.
  r.hasPt = true;
  r.ptX = -75.0f;
  r.ptY = 25.5f;
  r.calMode = true;
  r.hasCal = true;
  r.calId = 0xDEADBEEF;
  r.calStale = true;
  formatRec(line, sizeof line, r);
  printf("%s\n", line);
  TEST_ASSERT_TRUE(parseRec(line, b));
  TEST_ASSERT_TRUE(b.hasPt && b.ptX == -75.0f && b.ptY == 25.5f && b.calMode && b.hasCal &&
                   b.calId == 0xDEADBEEF && b.calStale);
  // Not a rec line, or one with a field missing.
  TEST_ASSERT_FALSE(parseRec("evt 2161 at 876.1903407 s", b));
  TEST_ASSERT_FALSE(parseRec("rec n=1 t=2 cfg=00000000 T=18 RH=50 p=1013", b));
}

// solveEvent: the policy the target applies (drop-one, the limit), as the replay does.
static void test_solve_event() {
  Cfg cfg = sampleCfg();
  for (int c = 0; c < 6; c++) cfg.ch[c].offsetNs = 0;
  cfg.resLimitNs = 1500;
  const float c = (float)soundSpeedMs(18, 50, 101325);
  // A shot at (31, -47): its edges (k 1/3, so t2 - t1 = 3 * the comparator delay), CH3 24 us late.
  SolveInput truth = layout(10.5f, 50, c);
  truth.mic[2].h = 21.5f;
  shoot(truth, 31, -47, 0);
  Rec r = {};
  r.cMs = c;
  r.channels = 0x3F;
  for (int i = 0; i < MAX_MICS; i++) r.t1[i] = r.t2[i] = NO_EDGE;
  float first = 1e30f;
  for (int i = 0; i < 6; i++) first = fminf(first, truth.t0[i]);
  for (int i = 0; i < 6; i++) {
    const float k = cfg.ch[i].k, delay = 600;   // t1 = t0 + delay, t2 = t1 + delay / k
    r.t1[i] = (int32_t)lroundf(truth.t0[i] - first + delay + (i == 2 ? 24000 : 0));
    r.t2[i] = r.t1[i] + (int32_t)lroundf(delay / k);
  }
  EventFit f = solveEvent(cfg, r);
  TEST_ASSERT_TRUE(f.solved && f.good);
  TEST_ASSERT_EQUAL_INT(2, f.s.dropped);
  TEST_ASSERT_FLOAT_WITHIN(0.05f, 31, f.s.x);
  TEST_ASSERT_FLOAT_WITHIN(0.05f, -47, f.s.y);
  cfg.dropOne = false;
  f = solveEvent(cfg, r);
  TEST_ASSERT_TRUE(f.solved && !f.good);
  TEST_ASSERT_EQUAL_INT(-1, f.s.dropped);
  TEST_ASSERT_TRUE(f.s.rmsNs > 4000);
  // Three channels: not solved.
  r.channels = 0x07;
  f = solveEvent(cfg, r);
  TEST_ASSERT_FALSE(f.solved);
  TEST_ASSERT_EQUAL_INT(3, f.used);
}

// A rec of a shot at (x, y) on the sample cfg's six mics: comparator delay 600 ns, so
// t2 - t1 = 600 / k; `lateCh` (or -1) 24 us late.
static Rec shotRec(const Cfg& cfg, float x, float y, int lateCh = -1) {
  const float c = (float)soundSpeedMs(18, 50, 101325);
  SolveInput truth = layout(10.5f, 50, c);
  truth.mic[2].h = 21.5f;
  shoot(truth, x, y, 0);
  Rec r = {};
  r.cMs = c;
  r.channels = 0x3F;
  for (int i = 0; i < MAX_MICS; i++) r.t1[i] = r.t2[i] = NO_EDGE;
  float first = 1e30f;
  for (int i = 0; i < 6; i++) first = fminf(first, truth.t0[i]);
  for (int i = 0; i < 6; i++) {
    const float k = cfg.ch[i].k, delay = 600;
    r.t1[i] = (int32_t)lroundf(truth.t0[i] - first + delay + (i == lateCh ? 24000 : 0));
    r.t2[i] = r.t1[i] + (int32_t)lroundf(delay / k);
  }
  return r;
}

// The verdict (MODEL.md "Validity"): each check, in its order.
static void test_validity() {
  Cfg cfg = sampleCfg();
  for (int c = 0; c < 6; c++) cfg.ch[c].offsetNs = 0;
  cfg.resLimitNs = 1500;
  cfg.maxLost = MAX_LOST_DEFAULT;
  cfg.areaMm = AREA_DEFAULT_MM;
  cfg.togetherNs = TOGETHER_DEFAULT_NS;
  cfg.pulseMaxNs = PULSE_MAX_DEFAULT_NS;
  // A clean shot.
  Rec r = shotRec(cfg, 31, -47);
  EventFit f = solveEvent(cfg, r);
  TEST_ASSERT_EQUAL_STRING("ok", verdictName(f.verdict));
  TEST_ASSERT_TRUE(f.good && f.lost == 0 && f.mics == 0x3F && f.spreadNs > 126000);
  // One channel late: dropped, one lost, still a shot.
  r = shotRec(cfg, 31, -47, 2);
  f = solveEvent(cfg, r);
  TEST_ASSERT_TRUE(f.good && f.s.dropped == 2 && f.lost == 1);
  // ... and another without its H edge: two lost.
  r.t2[4] = NO_EDGE;
  f = solveEvent(cfg, r);
  TEST_ASSERT_EQUAL_STRING("lost", verdictName(f.verdict));
  TEST_ASSERT_TRUE(f.s.dropped == 2 && f.lost == 2 && f.s.converged && f.s.rmsNs < 1500);
  // With validity off (a log from before 4a) that is a shot.
  Cfg off = cfg;
  off.maxLost = -1;
  TEST_ASSERT_TRUE(solveEvent(off, r).good);
  // A pulse too long, and one with H before L: left out for their shape.
  r = shotRec(cfg, 31, -47);
  r.t2[4] = r.t1[4] + 15000;
  f = solveEvent(cfg, r);
  TEST_ASSERT_TRUE(f.good && f.shapeBad == 0x10 && f.lost == 1 && f.mask == 0x2F);
  r.t2[4] = r.t1[4] - 100;
  f = solveEvent(cfg, r);
  TEST_ASSERT_TRUE(f.good && f.shapeBad == 0x10 && f.lost == 1);
  off = cfg;
  off.pulseMaxNs = 0;   // off: the 15 us pulse is used (and its t0 is far off)
  r.t2[4] = r.t1[4] + 15000;
  f = solveEvent(off, r);
  printf("pmax off, CH5 t0 5 us early: rms %.0f ns, dropped %d, at (%.2f, %.2f) for (31, -47)\n", f.s.rmsNs, f.s.dropped, f.s.x, f.s.y);
  TEST_ASSERT_TRUE(f.shapeBad == 0 && f.mask == 0x3F);
  // Every channel at once: an electrical spike.
  for (int i = 0; i < 6; i++) {
    r.t1[i] = 0;
    r.t2[i] = 300;
  }
  f = solveEvent(cfg, r);
  TEST_ASSERT_EQUAL_STRING("together", verdictName(f.verdict));
  TEST_ASSERT_TRUE(f.solved && f.spreadNs == 0);
  // Outside the area, inside the frame.
  r = shotRec(cfg, 112, -20);
  f = solveEvent(cfg, r);
  TEST_ASSERT_EQUAL_STRING("outside", verdictName(f.verdict));
  TEST_ASSERT_FLOAT_WITHIN(0.05f, 112, f.s.x);
  // Late on two channels: drop-one can take out only one.
  r = shotRec(cfg, 31, -47, 2);
  r.t1[5] += 24000;
  r.t2[5] += 24000;
  f = solveEvent(cfg, r);
  TEST_ASSERT_EQUAL_STRING("residual", verdictName(f.verdict));
  // Too few.
  r = shotRec(cfg, 31, -47);
  r.channels = 0x07;
  f = solveEvent(cfg, r);
  TEST_ASSERT_EQUAL_STRING("few", verdictName(f.verdict));
  TEST_ASSERT_TRUE(!f.solved && !f.good);
  // The names read back.
  for (int i = 0; i < V_COUNT; i++) {
    Verdict v;
    TEST_ASSERT_TRUE(parseVerdict(verdictName((Verdict)i), &v) && v == i);
  }
  Verdict v;
  TEST_ASSERT_FALSE(parseVerdict("good", &v));
}

// The curtain's gate (4b, MODEL.md "Validity"): a dip rising 45-100 us after the emission
// time, judged apart from the acoustic checks and checked last.
static void test_curtain() {
  Cfg cfg = sampleCfg();
  for (int c = 0; c < 6; c++) cfg.ch[c].offsetNs = 0;
  cfg.resLimitNs = 1500;
  cfg.maxLost = MAX_LOST_DEFAULT;
  cfg.areaMm = AREA_DEFAULT_MM;
  cfg.togetherNs = TOGETHER_DEFAULT_NS;
  cfg.pulseMaxNs = PULSE_MAX_DEFAULT_NS;
  // The gate off, no dips (every log before 4b): ok, the curtain not judged.
  Rec r = shotRec(cfg, 31, -47);
  EventFit f = solveEvent(cfg, r);
  TEST_ASSERT_TRUE(f.good && f.curtain == -1 && f.dip == -1 && isnan(f.dipAfterNs));
  const float tau = f.s.tauNs;   // the emission time, ns from the first edge (before it)
  TEST_ASSERT_TRUE(tau < -100000);
  // The gate on, and no curtain in the record: no dip, so not a shot.
  cfg.curtainLoNs = CURTAIN_LO_DEFAULT_NS;
  cfg.curtainHiNs = CURTAIN_HI_DEFAULT_NS;
  f = solveEvent(cfg, r);
  TEST_ASSERT_EQUAL_STRING("curtain", verdictName(f.verdict));
  TEST_ASSERT_TRUE(!f.good && f.acoustic == V_OK && f.curtain == 0 && f.dip == -1);
  // A dip 70 us after the emission: a shot.
  r.hasIr = true;
  r.nDips = 1;
  r.dipNs[0] = (int32_t)lroundf(tau + 70000);
  r.dipLenNs[0] = 34500;
  f = solveEvent(cfg, r);
  TEST_ASSERT_TRUE(f.good && f.curtain == 1 && f.dip == 0);
  TEST_ASSERT_FLOAT_WITHIN(1, 70000, f.dipAfterNs);
  // Too early, too late: not a shot, the dip still given.
  r.dipNs[0] = (int32_t)lroundf(tau + 30000);
  f = solveEvent(cfg, r);
  TEST_ASSERT_TRUE(f.verdict == V_CURTAIN && f.curtain == 0 && f.dip == 0);
  TEST_ASSERT_FLOAT_WITHIN(1, 30000, f.dipAfterNs);
  r.dipNs[0] = (int32_t)lroundf(tau + 120000);
  TEST_ASSERT_TRUE(solveEvent(cfg, r).verdict == V_CURTAIN);
  // The window's edges count as in it.
  r.dipNs[0] = (int32_t)lroundf(tau + 45000 + 2);
  TEST_ASSERT_TRUE(solveEvent(cfg, r).good);
  r.dipNs[0] = (int32_t)lroundf(tau + 100000 - 2);
  TEST_ASSERT_TRUE(solveEvent(cfg, r).good);
  // Two dips, one late, one in the window: the one in the window.
  r.nDips = 2;
  r.dipNs[0] = (int32_t)lroundf(tau + 150000);
  r.dipNs[1] = (int32_t)lroundf(tau + 50000);
  r.dipLenNs[1] = NO_EDGE;
  f = solveEvent(cfg, r);
  TEST_ASSERT_TRUE(f.good && f.dip == 1);
  TEST_ASSERT_FLOAT_WITHIN(1, 50000, f.dipAfterNs);
  // The gate off: a shot whatever the dips, the nearest still given.
  Cfg off = cfg;
  off.curtainLoNs = off.curtainHiNs = 0;
  f = solveEvent(off, r);
  TEST_ASSERT_TRUE(f.good && f.curtain == -1 && f.dip == 1);
  // An acoustic reject keeps its verdict; the curtain is judged apart (PLAN D9).
  Rec out = shotRec(cfg, 112, -20);
  f = solveEvent(cfg, out);
  out.hasIr = true;
  out.nDips = 1;
  out.dipNs[0] = (int32_t)lroundf(f.s.tauNs + 70000);
  out.dipLenNs[0] = 30000;
  f = solveEvent(cfg, out);
  TEST_ASSERT_TRUE(f.verdict == V_OUTSIDE && f.acoustic == V_OUTSIDE && f.curtain == 1);
  // Too few channels: no emission time, so the curtain is not judged.
  out.channels = 0x07;
  f = solveEvent(cfg, out);
  TEST_ASSERT_TRUE(f.verdict == V_FEW && f.curtain == -1);

  // The rec line: a rise before the first edge, a fall not seen.
  r.dipNs[0] = -150300;
  r.dipLenNs[0] = 34500;
  r.dipNs[1] = 73400;
  r.dipLenNs[1] = NO_EDGE;
  r.cfgId = 0x1A2B3C4D;
  r.cMs = 342.7f;
  char line[400];
  formatRec(line, sizeof line, r);
  printf("%s\n", line);
  TEST_ASSERT_NOT_NULL(strstr(line, " ir=-150300/34500,73400/-"));
  Rec b;
  TEST_ASSERT_TRUE(parseRec(line, b));
  TEST_ASSERT_TRUE(b.hasIr && b.nDips == 2 && b.dipNs[0] == -150300 && b.dipLenNs[0] == 34500 &&
                   b.dipNs[1] == 73400 && b.dipLenNs[1] == NO_EDGE);
  TEST_ASSERT_TRUE(parseRec((std::string(line) + "\r\n").c_str(), b) && b.nDips == 2);   // as read from a log
  r.nDips = 0;   // on, but no dip
  formatRec(line, sizeof line, r);
  TEST_ASSERT_NOT_NULL(strstr(line, " ir=-"));
  TEST_ASSERT_TRUE(parseRec(line, b) && b.hasIr && b.nDips == 0);
  TEST_ASSERT_TRUE(parseRec((std::string(line) + "\n").c_str(), b) && b.hasIr && b.nDips == 0);
  r.hasIr = false;   // off: no ir field
  formatRec(line, sizeof line, r);
  TEST_ASSERT_NULL(strstr(line, " ir="));
  TEST_ASSERT_TRUE(parseRec(line, b) && !b.hasIr);

  // The cfg line: the window round-trips, changes the cfg's id but not the calibration's.
  char cline[640];
  formatCfg(cline, sizeof cline, cfg);
  printf("%s\n", cline);
  TEST_ASSERT_NOT_NULL(strstr(cline, " ir=45000,100000"));
  Cfg back;
  uint32_t id;
  TEST_ASSERT_TRUE(parseCfg(cline, back, &id));
  TEST_ASSERT_TRUE(back.curtainLoNs == 45000 && back.curtainHiNs == 100000);
  TEST_ASSERT_NOT_EQUAL(cfgId(cfg), cfgId(off));
  TEST_ASSERT_EQUAL_HEX32(calibrationId(cfg), calibrationId(off));
}

static SolveInput micsAt(const float (*xy)[2], int n, float h) {
  SolveInput in = {};
  in.n = n;
  for (int i = 0; i < n; i++) {
    in.mic[i] = {xy[i][0], xy[i][1], h};
    in.sigmaNs[i] = 50;
  }
  in.c = mmPerNs((float)soundSpeedMs(18, 50, 101325));
  return in;
}

static void test_blast_check() {
  // The study (PLAN D1/D2, h 0, 9 starts): the square scores 484 of 624; one corner moved
  // 25 mm out along its diagonal none (smallest residual 6.9 us; slid 40 mm along the edge,
  // before 2026-11-24: 9.1 us); D2 none, with every mic or any one dropped.
  const float SQ[4][2] = {{-105, -105}, {105, -105}, {105, 105}, {-105, 105}};
  BlastResult sq = blastCheck(micsAt(SQ, 4, 10.5f), 0x0F, 1500, 100);
  BlastResult d1 = blastCheck(micsAt(D2, 4, 10.5f), 0x0F, 1500, 100);
  printf("blasts scored: square %d (smallest %.0f ns), D1 corners %d (smallest %.0f ns)\n",
         sq.scored, sq.smallestNs, d1.scored, d1.smallestNs);
  TEST_ASSERT_TRUE(sq.scored > 300);
  TEST_ASSERT_EQUAL_INT(0, d1.scored);
  TEST_ASSERT_TRUE(d1.smallestNs > 3 * 1500);
  // D2, over the validity area the firmware checks (105 mm, 4a).
  const SolveInput d2 = micsAt(D2, 6, 10.5f);
  BlastResult all = blastCheck(d2, 0x3F, 1500, AREA_DEFAULT_MM);
  printf("D2: every mic %d (smallest %.0f ns);", all.scored, all.smallestNs);
  TEST_ASSERT_EQUAL_INT(0, all.scored);
  for (int i = 0; i < 6; i++) {
    BlastResult b = blastCheck(d2, 0x3F & ~(1 << i), 1500, AREA_DEFAULT_MM);
    printf(" without CH%d %d (%.0f ns)", i + 1, b.scored, b.smallestNs);
    TEST_ASSERT_EQUAL_INT(0, b.scored);
  }
  printf("\n");
  // The square plus a bottom middle: dropping the middle leaves the square (the study: 484).
  const float SQ5[5][2] = {{-105, -105}, {105, -105}, {105, 105}, {-105, 105}, {0, -105}};
  BlastResult sq5 = blastCheck(micsAt(SQ5, 5, 10.5f), 0x0F, 1500, 100);
  TEST_ASSERT_EQUAL_INT(sq.scored, sq5.scored);
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
  RUN_TEST(test_crc32);
  RUN_TEST(test_cfg_round_trip);
  RUN_TEST(test_rec_round_trip);
  RUN_TEST(test_solve_event);
  RUN_TEST(test_validity);
  RUN_TEST(test_curtain);
  RUN_TEST(test_blast_check);
  RUN_TEST(test_time_per_solve);
  return UNITY_END();
}
