// OpenSights - replay a serial log through the measurement core on the PC (MEASUREMENT.md
// steps 3c-4b). Reads the `cfg` and `rec` lines a target printed (lib/measure/record.h),
// solves and judges every event again with lib/measure (measure::solveEvent), exactly as the
// target did, and prints the shots and verdicts. If the log has the target's own `shot`
// lines, it compares positions and verdicts; if the events carry their true point (`pt`:
// the simulator's, or calibration mode's), it gives the position error.
//
// Settings can be changed for the replay, to see what they do to the same events:
//   --h <mm>          every mic's height
//   --temp <C>        the air temperature (c again from it and the logged RH and p)
//   --c <m/s>         the speed of sound
//   --without <ch>    leave a channel out (repeatable): e.g. 5 mics instead of 6
//   --reslimit <ns>   the residual limit for "would be sent" (default: the log's cfg)
//   --dropone on|off  drop-one (default: the log's cfg)
//   --lost <n>|off    validity (MODEL.md "Validity"; default: the log's cfg): mic channels
//   --area <mm>|off     that may be lost, the area, the together limit, the longest pulse
//   --together <us>|off
//   --pulse <us>|off
//   --curtain <lo>,<hi>|off  the curtain's gate, us after the shot (4b; default: the log's
//                     cfg). An event logged with the curtain off has no dips: with a gate
//                     here it is no shot
//   --fit-c           fit one factor on every event's speed of sound (the least total
//                     residual) and say what air temperature that means
//   --quiet           the summary only
//
// Build (from firmware/):
//   g++ -std=gnu++17 -O2 -mfma -ffp-contract=fast -I lib/measure -o tools/replay
//       tools/replay.cpp lib/measure/solver.cpp lib/measure/record.cpp      (one line)
// The ESP32 fuses a*b + c into one instruction (one rounding); -mfma -ffp-contract=fast does
// the same on an x86-64 PC, so a replay matches the board (to the printed 0.001 mm). Leave
// -mfma out on ARM.
// Run:  tools/replay [options] logs/device-monitor-*.log
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include <algorithm>
#include <map>
#include <vector>

#include "air.h"
#include "record.h"

using namespace measure;

namespace {

struct Options {
  float h = NAN, tempC = NAN, cMs = NAN, resLimitNs = NAN;
  int dropOne = -1;   // -1: the cfg's
  int maxLost = -2;   // -2: the cfg's, -1: off
  float areaMm = NAN, togetherNs = NAN, pulseMaxNs = NAN;   // NAN: the cfg's, 0: off
  float curtainLoNs = NAN, curtainHiNs = NAN;               // NAN: the cfg's, 0, 0: off
  uint8_t without = 0;
  bool quiet = false, fitC = false;
  bool changed() const {
    return !isnan(h) || !isnan(tempC) || !isnan(cMs) || !isnan(resLimitNs) || dropOne >= 0 || without ||
           maxLost != -2 || !isnan(areaMm) || !isnan(togetherNs) || !isnan(pulseMaxNs) ||
           !isnan(curtainHiNs);
  }
};

// One event of the log, with the settings it is replayed with.
struct Item {
  Cfg   cfg;
  Rec   rec;
  bool  board;   // the board's own shot line came with it, with a position
  float boardX, boardY;
  bool  boardHasVerdict;   // ... and its verdict tag (4a: "[ok]")
  Verdict boardVerdict;
};

void usage() {
  fprintf(stderr, "usage: replay [--h mm] [--temp C] [--c m/s] [--without ch]... "
                  "[--reslimit ns] [--dropone on|off] [--lost n|off] [--area mm|off] "
                  "[--together us|off] [--pulse us|off] [--curtain lo,hi|off] [--fit-c] [--quiet] log...\n");
  exit(2);
}

// The total squared rms residual over the events that converge with every channel, with
// every c scaled.
double totalResidual(const std::vector<Item>& items, double scale) {
  double sum = 0;
  for (const Item& it : items) {
    Rec r = it.rec;
    r.cMs = (float)(r.cMs * scale);
    const EventFit f = solveEvent(it.cfg, r);
    if (f.solved && f.s.converged && f.s.dropped < 0) sum += (double)f.s.rmsNs * f.s.rmsNs;
  }
  return sum;
}

// The temperature at which air of (t, rh, p) has its c times `scale`.
double tempForScale(double t, double rh, double pPa, double scale) {
  const double want = soundSpeedMs(t, rh, pPa) * scale;
  double lo = t - 40, hi = t + 40;
  for (int i = 0; i < 60; i++) {
    const double mid = 0.5 * (lo + hi);
    (soundSpeedMs(mid, rh, pPa) < want ? lo : hi) = mid;
  }
  return 0.5 * (lo + hi);
}

}  // namespace

int main(int argc, char** argv) {
  Options opt;
  std::vector<const char*> files;
  for (int i = 1; i < argc; i++) {
    const char* a = argv[i];
    auto value = [&]() -> const char* {
      if (i + 1 >= argc) usage();
      return argv[++i];
    };
    if (!strcmp(a, "--h")) opt.h = strtof(value(), nullptr);
    else if (!strcmp(a, "--temp")) opt.tempC = strtof(value(), nullptr);
    else if (!strcmp(a, "--c")) opt.cMs = strtof(value(), nullptr);
    else if (!strcmp(a, "--reslimit")) opt.resLimitNs = strtof(value(), nullptr);
    else if (!strcmp(a, "--lost")) {
      const char* v = value();
      opt.maxLost = !strcmp(v, "off") ? -1 : atoi(v);
      if (opt.maxLost < -1 || opt.maxLost > 8) usage();
    } else if (!strcmp(a, "--area") || !strcmp(a, "--together") || !strcmp(a, "--pulse")) {
      const char* v = value();
      const float f = !strcmp(v, "off") ? 0 : strtof(v, nullptr) * (strcmp(a, "--area") ? 1000 : 1);
      if (!(f >= 0)) usage();
      (!strcmp(a, "--area") ? opt.areaMm : !strcmp(a, "--together") ? opt.togetherNs : opt.pulseMaxNs) = f;
    } else if (!strcmp(a, "--curtain")) {
      const char* v = value();
      float lo = 0, hi = 0;
      if (strcmp(v, "off") && (sscanf(v, "%f,%f", &lo, &hi) != 2 || !(lo >= 0 && hi > lo))) usage();
      opt.curtainLoNs = lo * 1000;
      opt.curtainHiNs = hi * 1000;
    } else if (!strcmp(a, "--dropone")) {
      const char* v = value();
      if (strcmp(v, "on") && strcmp(v, "off")) usage();
      opt.dropOne = !strcmp(v, "on");
    } else if (!strcmp(a, "--without")) {
      int ch = atoi(value());
      if (ch < 1 || ch > MAX_MICS) usage();
      opt.without |= 1 << (ch - 1);
    } else if (!strcmp(a, "--fit-c")) opt.fitC = true;
    else if (!strcmp(a, "--quiet")) opt.quiet = true;
    else if (a[0] == '-') usage();
    else files.push_back(a);
  }
  if (files.empty()) usage();

  // Read every event, with its cfg (as the options change it).
  std::map<uint32_t, Cfg> cfgs;
  std::map<uint32_t, int> missingIds;
  std::vector<Item> items;
  int recs = 0, noCfg = 0, dipLines = 0;
  for (const char* path : files) {
    FILE* f = fopen(path, "r");
    if (!f) {
      perror(path);
      return 1;
    }
    char line[2048];
    // The target prints an event's `shot` line just before its `rec` line.
    bool haveBoard = false, havePos = false, haveVerdict = false;
    unsigned boardN = 0;
    float boardX = 0, boardY = 0;
    Verdict boardVerdict = V_OK;
    while (fgets(line, sizeof line, f)) {
      Cfg cfg;
      uint32_t id;
      if (parseCfg(line, cfg, &id)) {
        cfgs[id] = cfg;
        continue;
      }
      if (strstr(line, "dip n=")) {   // 4b: a curtain dip with no acoustic event
        dipLines++;
        continue;
      }
      const char* sl = strstr(line, "shot ");
      unsigned n;
      float x, y;
      char dash;
      if (sl && (sscanf(sl, "shot %u: x %f y %f", &n, &x, &y) == 3 ||
                 sscanf(sl, "shot %u %c", &n, &dash) == 2)) {
        haveBoard = true;
        havePos = sscanf(sl, "shot %u: x %f y %f", &n, &x, &y) == 3;
        boardN = n;
        boardX = havePos ? x : 0;
        boardY = havePos ? y : 0;
        // The verdict tag, "[ok]" (4a); a log from before has none.
        haveVerdict = false;
        const char* tag = strrchr(sl, '[');
        char name[16];
        if (tag && sscanf(tag, "[%15[a-z]]", name) == 1) haveVerdict = parseVerdict(name, &boardVerdict);
        continue;
      }
      Item it = {};
      if (!parseRec(line, it.rec)) continue;
      recs++;
      const bool mine = haveBoard && boardN == it.rec.n;
      it.board = mine && havePos;
      it.boardHasVerdict = mine && haveVerdict;
      it.boardVerdict = boardVerdict;
      haveBoard = false;
      it.boardX = boardX;
      it.boardY = boardY;
      auto c = cfgs.find(it.rec.cfgId);
      if (c == cfgs.end()) {
        noCfg++;
        missingIds[it.rec.cfgId]++;
        continue;
      }
      it.cfg = c->second;
      for (int ch = 0; ch < MAX_MICS; ch++) {
        if (opt.without & (1 << ch)) it.cfg.ch[ch].mic = false;
        if (!isnan(opt.h)) it.cfg.ch[ch].h = opt.h;
      }
      if (!isnan(opt.resLimitNs)) it.cfg.resLimitNs = opt.resLimitNs;
      if (opt.dropOne >= 0) it.cfg.dropOne = opt.dropOne;
      if (opt.maxLost != -2) it.cfg.maxLost = (int8_t)opt.maxLost;
      if (!isnan(opt.areaMm)) it.cfg.areaMm = opt.areaMm;
      if (!isnan(opt.togetherNs)) it.cfg.togetherNs = opt.togetherNs;
      if (!isnan(opt.pulseMaxNs)) it.cfg.pulseMaxNs = opt.pulseMaxNs;
      if (!isnan(opt.curtainHiNs)) {
        it.cfg.curtainLoNs = opt.curtainLoNs;
        it.cfg.curtainHiNs = opt.curtainHiNs;
      }
      if (!isnan(opt.cMs)) it.rec.cMs = opt.cMs;
      else if (!isnan(opt.tempC))
        it.rec.cMs = (float)soundSpeedMs(opt.tempC, it.rec.rh, it.rec.hPa * 100.0);
      items.push_back(it);
    }
    fclose(f);
  }

  // The speed of sound fitted over every event: one factor, by golden-section search.
  double scale = 1;
  if (opt.fitC && !items.empty()) {
    double lo = 0.97, hi = 1.03;
    const double g = (sqrt(5.0) - 1) / 2;
    double a = hi - g * (hi - lo), b = lo + g * (hi - lo);
    double fa = totalResidual(items, a), fb = totalResidual(items, b);
    for (int i = 0; i < 40; i++) {
      if (fa < fb) {
        hi = b; b = a; fb = fa;
        a = hi - g * (hi - lo);
        fa = totalResidual(items, a);
      } else {
        lo = a; a = b; fa = fb;
        b = lo + g * (hi - lo);
        fb = totalResidual(items, b);
      }
    }
    scale = 0.5 * (lo + hi);
  }

  int tooFew = 0, solved = 0, converged = 0, good = 0, dropped = 0, compared = 0, withPt = 0;
  int verdicts[V_COUNT] = {}, verdictsCompared = 0, verdictsDiffer = 0;
  int curtain[2][2] = {}, curtainNot = 0, withDips = 0;   // [acoustic ok][a dip in the window]
  double worstDiff = 0, sumDiff2 = 0, worstPt = 0, sumPt2 = 0;
  for (const Item& it : items) {
    Rec rec = it.rec;
    rec.cMs = (float)(rec.cMs * scale);
    const EventFit f = solveEvent(it.cfg, rec);
    verdicts[f.verdict]++;
    if (f.curtain >= 0) curtain[f.acoustic == V_OK][f.curtain]++;
    else curtainNot++;
    if (rec.hasIr) withDips++;
    bool differs = false;
    if (it.boardHasVerdict) {
      verdictsCompared++;
      differs = it.boardVerdict != f.verdict;
      if (differs) verdictsDiffer++;
    }
    if (!f.solved) {
      tooFew++;
      if (!opt.quiet) {
        printf("replay %u -: %d channels with a mic, both edges and a good pulse [%s]", (unsigned)rec.n,
               f.used, verdictName(f.verdict));
        if (differs) printf("; board [%s]", verdictName(it.boardVerdict));
        printf("\n");
      }
      continue;
    }
    const Solution& s = f.s;
    solved++;
    if (s.converged) converged++;
    if (s.dropped >= 0) dropped++;
    if (f.good) good++;
    if (!opt.quiet) {
      printf("replay %u: x %+.3f y %+.3f mm, rms %.0f ns, 1 sd %.3f mm, %u iterations, res ns",
             (unsigned)rec.n, s.x, s.y, s.rmsNs, s.major, s.iterations);
      for (int c = 0; c < MAX_MICS; c++)
        if (f.mask & (1 << c)) printf(" CH%d %+.0f%s", c + 1, s.resNs[c], c == s.dropped ? " (dropped)" : "");
      if (f.dip >= 0) printf(" dip %+.1f us%s", f.dipAfterNs / 1000, f.curtain == 0 ? " (outside)" : "");
      printf(" [%s]", verdictName(f.verdict));
      if (differs) printf("; board [%s]", verdictName(it.boardVerdict));
    }
    if (it.board && s.converged) {
      // The board prints 3 decimals: compare as printed.
      char bx[24], by[24];
      snprintf(bx, sizeof bx, "%+.3f", s.x);
      snprintf(by, sizeof by, "%+.3f", s.y);
      const double d = hypot(atof(bx) - it.boardX, atof(by) - it.boardY);
      compared++;
      sumDiff2 += d * d;
      worstDiff = std::max(worstDiff, d);
      if (!opt.quiet) printf("; board %+.3f %+.3f (%.3f mm off)", it.boardX, it.boardY, d);
    }
    if (rec.hasPt && f.good) {
      const double e = hypot(s.x - rec.ptX, s.y - rec.ptY);
      withPt++;
      sumPt2 += e * e;
      worstPt = std::max(worstPt, e);
      if (!opt.quiet) printf("; %.3f mm from pt", e);
    }
    if (!opt.quiet) printf("\n");
  }

  printf("replay: %d events, %d solved (%d converged, %d shots, %d with a channel dropped), %d "
         "with too few channels\n", recs, solved, converged, good, dropped, tooFew);
  printf("replay: verdicts:");
  for (int v = 0; v < V_COUNT; v++) printf(" %s %d", verdictName((Verdict)v), verdicts[v]);
  printf("\n");
  if (withDips || dipLines || curtain[0][0] + curtain[0][1] + curtain[1][0] + curtain[1][1])
    printf("replay: curtain (4b): acoustic ok with a dip in the window %d, without %d; acoustic reject "
           "with %d, without %d; %d not judged (gate off, or no fit); %d events logged with the "
           "curtain on; %d `dip` lines (a dip with no event)\n", curtain[1][1], curtain[1][0],
           curtain[0][1], curtain[0][0], curtainNot, withDips, dipLines);
  if (verdictsCompared)
    printf("replay: against the board's verdicts: %d compared, %d differ%s\n", verdictsCompared,
           verdictsDiffer, opt.changed() || opt.fitC ? " (settings changed for the replay: differences expected)" : "");
  if (noCfg) {
    printf("replay: %d events without their cfg line in the log:", noCfg);
    for (auto& m : missingIds) printf(" %08X (%d)", (unsigned)m.first, m.second);
    printf(" (type `mic` on the target while logging, or start a `sim` run: both print it)\n");
  }
  if (compared)
    printf("replay: against the board's shot lines (as printed, 0.001 mm steps): %d compared, "
           "rms %.4f mm, worst %.4f mm%s\n", compared, sqrt(sumDiff2 / compared), worstDiff,
           opt.changed() || opt.fitC ? " (settings changed for the replay: differences expected)" : "");
  if (withPt)
    printf("replay: against the true points (pt): %d good shots, error rms %.4f mm, worst %.4f mm\n",
           withPt, sqrt(sumPt2 / withPt), worstPt);
  if (opt.fitC && !items.empty()) {
    // In terms of temperature, at the middle event's logged air.
    const Rec& mid = items[items.size() / 2].rec;
    const double t = tempForScale(mid.tC, mid.rh, mid.hPa * 100.0, scale);
    printf("replay: the speed of sound fits x%.6f the logged one (%.2f m/s at the middle event): "
           "as if the air were %.2f °C instead of the logged %.2f °C (%+.2f °C)\n", scale,
           mid.cMs * scale, t, mid.tC, t - mid.tC);
  }
  return 0;
}
