// OpenSights - replay a serial log through the measurement core on the PC (MEASUREMENT.md
// step 3c). Reads the `cfg` and `rec` lines a target printed (lib/measure/record.h), solves
// every event again with lib/measure, exactly as the target did, and prints the shots. If the
// log has the target's own `shot` lines, it compares them.
//
// Settings can be changed for the replay, to see what they do to the same events:
//   --h <mm>          every mic's height
//   --temp <C>        the air temperature (c again from it and the logged RH and p)
//   --c <m/s>         the speed of sound
//   --without <ch>    leave a channel out (repeatable): e.g. 5 mics instead of 6
//   --reslimit <ns>   the residual limit for "would be sent" (default: the log's cfg)
//   --dropone on|off  drop-one (default: the log's cfg)
//   --quiet           the summary only
//
// Build (from firmware/):
//   g++ -std=gnu++17 -O2 -mfma -ffp-contract=fast -I lib/measure -o tools/replay
//       tools/replay.cpp lib/measure/solver.cpp lib/measure/record.cpp      (one line)
// The ESP32 fuses a*b + c into one instruction (one rounding); -mfma -ffp-contract=fast does
// the same on an x86-64 PC, so a replay matches the board (to the printed 0.001 mm). Leave -mfma out on ARM.
// Run:  tools/replay [options] logs/device-monitor-*.log
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include <map>
#include <string>
#include <vector>

#include "air.h"
#include "record.h"

using namespace measure;

namespace {

struct Options {
  float h = NAN, tempC = NAN, cMs = NAN, resLimitNs = NAN;
  int dropOne = -1;   // -1: the cfg's
  uint8_t without = 0;
  bool quiet = false;
  bool changed() const {
    return !isnan(h) || !isnan(tempC) || !isnan(cMs) || !isnan(resLimitNs) || dropOne >= 0 || without;
  }
};

struct Totals {
  int recs = 0, noCfg = 0, tooFew = 0, solved = 0, converged = 0, withinLimit = 0, dropped = 0;
  int compared = 0, boardOnly = 0;
  double worstDiff = 0, sumDiff2 = 0;
};

void usage() {
  fprintf(stderr, "usage: replay [--h mm] [--temp C] [--c m/s] [--without ch]... "
                  "[--reslimit ns] [--dropone on|off] [--quiet] log...\n");
  exit(2);
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
    else if (!strcmp(a, "--dropone")) {
      const char* v = value();
      if (strcmp(v, "on") && strcmp(v, "off")) usage();
      opt.dropOne = !strcmp(v, "on");
    }
    else if (!strcmp(a, "--without")) {
      int ch = atoi(value());
      if (ch < 1 || ch > MAX_MICS) usage();
      opt.without |= 1 << (ch - 1);
    } else if (!strcmp(a, "--quiet")) opt.quiet = true;
    else if (a[0] == '-') usage();
    else files.push_back(a);
  }
  if (files.empty()) usage();

  std::map<uint32_t, Cfg> cfgs;
  std::map<uint32_t, int> missingIds;
  Totals t;
  float lastLimit = RES_LIMIT_DEFAULT_NS;
  for (const char* path : files) {
    FILE* f = fopen(path, "r");
    if (!f) {
      perror(path);
      return 1;
    }
    char line[2048];
    // The target prints an event's `shot` line just before its `rec` line.
    bool haveBoard = false;
    unsigned boardN = 0;
    float boardX = 0, boardY = 0;
    while (fgets(line, sizeof line, f)) {
      Cfg cfg;
      uint32_t id;
      if (parseCfg(line, cfg, &id)) {
        cfgs[id] = cfg;
        continue;
      }
      const char* sl = strstr(line, "shot ");
      unsigned n;
      float x, y;
      if (sl && sscanf(sl, "shot %u: x %f y %f", &n, &x, &y) == 3) {
        if (haveBoard) t.boardOnly++;
        haveBoard = true;
        boardN = n;
        boardX = x;
        boardY = y;
        continue;
      }
      Rec rec;
      if (!parseRec(line, rec)) continue;
      t.recs++;
      const bool paired = haveBoard && boardN == rec.n;
      if (haveBoard && !paired) t.boardOnly++;
      haveBoard = false;
      auto it = cfgs.find(rec.cfgId);
      if (it == cfgs.end()) {
        t.noCfg++;
        missingIds[rec.cfgId]++;
        continue;
      }
      Cfg use = it->second;
      for (int c = 0; c < MAX_MICS; c++) {
        if (opt.without & (1 << c)) use.ch[c].mic = false;
        if (!isnan(opt.h)) use.ch[c].h = opt.h;
      }
      if (!isnan(opt.resLimitNs)) use.resLimitNs = opt.resLimitNs;
      if (opt.dropOne >= 0) use.dropOne = opt.dropOne;
      if (!isnan(opt.cMs)) rec.cMs = opt.cMs;
      else if (!isnan(opt.tempC)) rec.cMs = (float)soundSpeedMs(opt.tempC, rec.rh, rec.hPa * 100.0);

      const EventFit f = solveEvent(use, rec);
      const uint8_t mask = f.mask;
      if (!f.solved) {
        t.tooFew++;
        if (!opt.quiet) printf("replay %u -: %d channels with a mic and both edges\n", (unsigned)rec.n, f.used);
        continue;
      }
      const Solution& s = f.s;
      t.solved++;
      if (s.converged) t.converged++;
      if (s.dropped >= 0) t.dropped++;
      const bool good = f.good;
      if (good) t.withinLimit++;
      lastLimit = use.resLimitNs;
      if (!opt.quiet) {
        printf("replay %u: x %+.3f y %+.3f mm, rms %.0f ns, 1 sd %.3f mm, %u iterations, res ns",
               (unsigned)rec.n, s.x, s.y, s.rmsNs, s.major, s.iterations);
        for (int c = 0; c < MAX_MICS; c++)
          if (mask & (1 << c)) printf(" CH%d %+.0f%s", c + 1, s.resNs[c], c == s.dropped ? " (dropped)" : "");
        printf("%s", !s.converged ? "; did not converge" : good ? "" : "; over the limit");
      }
      if (paired && s.converged) {
        // The board prints 3 decimals: compare as printed.
        char bx[24], by[24];
        snprintf(bx, sizeof bx, "%+.3f", s.x);
        snprintf(by, sizeof by, "%+.3f", s.y);
        const double d = hypot(atof(bx) - boardX, atof(by) - boardY);
        t.compared++;
        t.sumDiff2 += d * d;
        if (d > t.worstDiff) t.worstDiff = d;
        if (!opt.quiet) printf("; board %+.3f %+.3f (%.3f mm off)", boardX, boardY, d);
      }
      if (!opt.quiet) printf("\n");
    }
    fclose(f);
  }

  printf("replay: %d events, %d solved (%d converged, %d within %.0f ns, %d with a channel "
         "dropped), %d with too few channels\n", t.recs, t.solved, t.converged, t.withinLimit,
         lastLimit, t.dropped, t.tooFew);
  if (t.noCfg) {
    printf("replay: %d events without their cfg line in the log:", t.noCfg);
    for (auto& m : missingIds) printf(" %08X (%d)", (unsigned)m.first, m.second);
    printf(" (type `mic` on the target while logging, or start a `sim` run: both print it)\n");
  }
  if (t.compared)
    printf("replay: against the board's shot lines (as printed, 0.001 mm steps): %d compared, "
           "rms %.4f mm, worst %.4f mm%s\n", t.compared, sqrt(t.sumDiff2 / t.compared),
           t.worstDiff, opt.changed() ? " (settings changed for the replay: differences expected)" : "");
  return 0;
}
