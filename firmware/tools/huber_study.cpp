// OpenSights - the Huber study (MEASUREMENT 4d, app TODO "The solver: Huber weights and mic
// health"): synthetic shots with known truth, solved by lib/measure's own solver four ways,
// to see whether Huber weights help against a moderately off mic before anything goes into
// the firmware.
//
//   ls      plain least squares over every mic
//   drop    today's firmware: solveDropOne with the 1500 ns limit (dropOne on), its verdict
//   huber   Huber weights (tools/huber.h), no drop
//   huber2  Huber, then the TODO's two steps: one far-out mic dropped by its weight (a refit),
//           the full drop-one search if that refit doesn't help, nothing dropped if two are far
//   dropS   drop-one started when the rms is over --trigger times the rms the sigmas give
//           (instead of the fixed 1500 ns)
//   dropSg  dropS with a guard: still over the trigger after the drop (two or more off), the
//           fit over all kept and the shot marked uncertain
//   huberS  Huber, and when a mic is far out the drop-one search (a Huber fit each)
// Every method's shot is judged as the firmware does: its rms residual (over the mics in the
// final fit, unweighted) within 1500 ns, at most one mic lost, inside the area. `unc` counts
// the shots huber2/huberS mark uncertain (a mic still far out): `sent-unc` is what would be
// sent if those were held back.
//
// Each case: N shots uniform over the card (+-100 mm), the D1/D2 mics (h 5.5 mm), Gaussian
// timing noise per mic, and 0, 1 or 2 mics with a bias (each shot a random mic, a random
// sign). The fit's sigma is the noise unless the case says otherwise (a calibration that
// got the noise wrong).
//
// Build (from firmware/):
//   g++ -std=gnu++17 -O2 -mfma -ffp-contract=fast -I lib/measure -o tools/huber_study
//       tools/huber_study.cpp lib/measure/solver.cpp      (one line; no -mfma on ARM)
// Run:  tools/huber_study [--n shots] [--c sigmas] [--drop sigmas] [--trigger x] [--seed s]
//                         [--only text]
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include <algorithm>
#include <random>
#include <string>
#include <vector>

#include "huber.h"

using namespace measure;

namespace {

// The firmware's default layout (PLAN D1/D2, `mic default`): CH1-CH6.
const Mic D1D2[6] = {{-105, -105, 5.5f}, {105, -105, 5.5f}, {122.68f, 122.68f, 5.5f},
                     {-105, 105, 5.5f},  {0, -105, 5.5f},   {105, 0, 5.5f}};
const float C_MM_PER_NS = 342.74f * 1e-6f;   // 18 °C, 50 % RH
const float LIMIT_NS = 1500, AREA_MM = 105;

struct Case {
  std::string name;
  uint8_t mask;      // the mics in the fit (bits of D1D2)
  float noiseNs;     // the true timing noise, 1 sigma
  float sigmaNs;     // the fit's sigma (the calibration's)
  int   bad;         // mics with a bias, 0-2
  float biasNs;
};

enum Method { LS, DROP, DROPS, DROPSG, HUBER, HUBER2, HUBERS, METHODS };
const char* METHOD_NAME[METHODS] = {"ls", "drop", "dropS", "dropSg", "huber", "huber2", "huberS"};

struct Tally {
  int n = 0, conv = 0, ok = 0;          // shots, converged, verdict ok (would be sent)
  int okSure = 0;                       // ... and not uncertain
  std::vector<double> errSure;          // the error of those
  std::vector<double> err;              // position error of converged shots, mm
  std::vector<double> errOk;            // ... of the shots sent
  int dropRight = 0, dropWrong = 0;     // a drop, of a bad mic / of a healthy one
  int healthy = 0, healthySoft = 0;     // healthy mic-shots in the fit, down-weighted (w < 0.999)
  double badW = 0;                      // the bad mics' mean weight (Huber)
  int badN = 0;
  int refit = 0, fellBack = 0, uncertain = 0;
  long lm = 0;                          // LM iterations
};

double pct(std::vector<double> v, double p) {
  if (v.empty()) return NAN;
  std::sort(v.begin(), v.end());
  return v[std::min(v.size() - 1, (size_t)(p * v.size()))];
}
double rms(const std::vector<double>& v) {
  if (v.empty()) return NAN;
  double s = 0;
  for (double e : v) s += e * e;
  return sqrt(s / v.size());
}

}  // namespace

int main(int argc, char** argv) {
  int n = 2000;
  unsigned seed = 1;
  HuberOpts ho;
  float trigger = 3;
  const char* only = nullptr;
  for (int i = 1; i < argc; i++) {
    auto v = [&]() { if (i + 1 >= argc) exit(2); return argv[++i]; };
    if (!strcmp(argv[i], "--n")) n = atoi(v());
    else if (!strcmp(argv[i], "--c")) ho.c = strtof(v(), nullptr);
    else if (!strcmp(argv[i], "--drop")) ho.drop = strtof(v(), nullptr);
    else if (!strcmp(argv[i], "--trigger")) trigger = strtof(v(), nullptr);
    else if (!strcmp(argv[i], "--seed")) seed = (unsigned)atoi(v());
    else if (!strcmp(argv[i], "--only")) only = v();
    else { fprintf(stderr, "usage: huber_study [--n shots] [--c sigmas] [--drop sigmas] [--trigger x] [--seed s] [--only text]\n"); return 2; }
  }

  const uint8_t SIX = 0x3F, FIVE = 0x1F /* without the right middle */, FOUR = 0x0F /* corners */;
  std::vector<Case> cases;
  for (float noise : {50.f, 150.f, 300.f}) cases.push_back({"6 mics, healthy", SIX, noise, noise, 0, 0});
  for (float noise : {50.f, 150.f})
    for (float b : {500.f, 1000.f, 2000.f, 3000.f, 5000.f, 10000.f, 24000.f})
      cases.push_back({"6 mics, 1 off", SIX, noise, noise, 1, b});
  for (float b : {1000.f, 3000.f, 10000.f}) cases.push_back({"6 mics, 2 off", SIX, 50, 50, 2, b});
  cases.push_back({"5 mics, healthy", FIVE, 50, 50, 0, 0});
  for (float b : {1000.f, 3000.f, 10000.f}) cases.push_back({"5 mics, 1 off", FIVE, 50, 50, 1, b});
  for (float b : {1000.f, 3000.f}) cases.push_back({"4 mics, 1 off", FOUR, 50, 50, 1, b});
  cases.push_back({"6 mics, healthy, sigma 3x low", SIX, 150, 50, 0, 0});
  cases.push_back({"6 mics, healthy, sigma 3x high", SIX, 50, 150, 0, 0});
  cases.push_back({"6 mics, 1 off, sigma 3x low", SIX, 150, 50, 1, 1000});
  cases.push_back({"6 mics, 1 off, sigma 3x high", SIX, 50, 150, 1, 1000});

  printf("# Huber study: %d shots per case, Huber c %.1f sigma, far out at %.1f sigma, refit if "
         "the rms drops under %.0f %%, dropS at %.1f x the expected rms, seed %u\n\n", n, ho.c, ho.drop,
         ho.improve * 100, trigger, seed);
  printf("| case | noise ns | fit sigma ns | bias us | method | sent %% | err rms mm | 95 %% | worst | "
         "sent: rms | drops right/wrong %% | healthy soft %% | bad w | refit/fallback/uncertain %% | sent-unc %% | sent-unc: rms | LM/shot |\n");
  printf("|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|\n");
  std::mt19937 rng(seed);
  std::uniform_real_distribution<float> uni(-100, 100);
  std::normal_distribution<float> gauss(0, 1);
  for (const Case& cs : cases) {
    if (only && !strstr(cs.name.c_str(), only)) continue;
    std::vector<int> mics;
    for (int i = 0; i < 6; i++)
      if ((cs.mask >> i) & 1) mics.push_back(i);
    Tally t[METHODS];
    for (int k = 0; k < n; k++) {
      const float px = uni(rng), py = uni(rng);
      SolveInput in = {};
      in.n = 6;
      in.c = C_MM_PER_NS;
      bool badMic[6] = {};
      std::vector<int> pool = mics;
      for (int b = 0; b < cs.bad; b++) {
        const int j = std::uniform_int_distribution<int>(0, (int)pool.size() - 1)(rng);
        badMic[pool[j]] = true;
        pool.erase(pool.begin() + j);
      }
      for (int i = 0; i < 6; i++) {
        in.mic[i] = D1D2[i];
        const float sign = std::bernoulli_distribution(0.5)(rng) ? 1.f : -1.f;
        in.t0[i] = 1000 + micDistance(D1D2[i], px, py) / in.c + cs.noiseNs * gauss(rng) +
                   (badMic[i] ? sign * cs.biasNs : 0);
        in.sigmaNs[i] = cs.sigmaNs;
      }
      for (int m = 0; m < METHODS; m++) {
        Tally& tt = t[m];
        Solution s;
        float w[MAX_MICS];
        for (int i = 0; i < MAX_MICS; i++) w[i] = (cs.mask >> i) & 1 ? 1.0f : 0.0f;
        bool uncertain = false;
        if (m == LS || m == DROP || m == DROPS || m == DROPSG) {
          s = m == LS     ? solve(in, cs.mask)
              : m == DROP ? solveDropOne(in, LIMIT_NS, cs.mask)
                          : solveDropSigma(in, cs.mask, trigger, m == DROPSG, &uncertain);
          if (s.dropped >= 0) w[s.dropped] = 0;
          tt.uncertain += uncertain;
          tt.lm += s.iterations;
        } else {
          HuberOpts o = ho;
          o.refit = m == HUBER2;
          const HuberFit h = m == HUBERS ? solveHuberSearch(in, cs.mask, o) : solveHuber(in, cs.mask, o);
          s = h.s;
          for (int i = 0; i < MAX_MICS; i++) w[i] = h.w[i];
          uncertain = h.uncertain;
          tt.refit += h.refitted;
          tt.fellBack += h.fellBack;
          tt.uncertain += h.uncertain;
          tt.lm += h.lm;
        }
        const float verdictRms = s.rmsNs;
        tt.n++;
        if (!s.converged) continue;
        tt.conv++;
        const double e = hypot(s.x - px, s.y - py);
        tt.err.push_back(e);
        const int lost = __builtin_popcount(cs.mask) - __builtin_popcount(s.used);
        const bool ok = verdictRms <= LIMIT_NS && lost <= 1 && fabsf(s.x) <= AREA_MM && fabsf(s.y) <= AREA_MM;
        if (ok) {
          tt.ok++;
          tt.errOk.push_back(e);
          if (!uncertain) {
            tt.okSure++;
            tt.errSure.push_back(e);
          }
        }
        if (s.dropped >= 0) (badMic[s.dropped] ? tt.dropRight : tt.dropWrong)++;
        for (int i : mics) {
          if (badMic[i]) {
            tt.badW += (s.used >> i) & 1 ? w[i] : 0;
            tt.badN++;
          } else if ((s.used >> i) & 1) {
            tt.healthy++;
            if (w[i] < 0.999f) tt.healthySoft++;
          }
        }
      }
    }
    for (int m = 0; m < METHODS; m++) {
      const Tally& tt = t[m];
      const double N = tt.n;
      char bad[16] = "-", soft[16] = "-", steps[32] = "-";
      if (m >= HUBER) {
        snprintf(soft, sizeof soft, "%.1f", tt.healthy ? 100.0 * tt.healthySoft / tt.healthy : 0.0);
        if (tt.badN) snprintf(bad, sizeof bad, "%.2f", tt.badW / tt.badN);
      }
      if (m == DROPSG)
        snprintf(steps, sizeof steps, "- / - / %.1f", 100 * tt.uncertain / N);
      if (m == HUBER2 || m == HUBERS)
        snprintf(steps, sizeof steps, "%.1f / %.1f / %.1f", 100 * tt.refit / N, 100 * tt.fellBack / N,
                 100 * tt.uncertain / N);
      printf("| %s | %.0f | %.0f | %s | %s | %.1f | %.3f | %.3f | %.3f | %.3f | %.1f / %.1f | %s | %s | %s | %.1f | %.3f | %.1f |\n",
             cs.name.c_str(), cs.noiseNs, cs.sigmaNs,
             cs.bad ? std::to_string(cs.biasNs / 1000).substr(0, 4).c_str() : "-", METHOD_NAME[m],
             100 * tt.ok / N, rms(tt.err), pct(tt.err, 0.95),
             tt.err.empty() ? NAN : *std::max_element(tt.err.begin(), tt.err.end()), rms(tt.errOk),
             100 * tt.dropRight / N, 100 * tt.dropWrong / N, soft, bad, steps, 100 * tt.okSure / N,
             rms(tt.errSure), tt.lm / N);
    }
  }
  return 0;
}
