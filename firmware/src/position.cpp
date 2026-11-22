// OpenSights - the shot position on the board: see position.h.
#include "position.h"
#include <Preferences.h>
#include "airtemp.h"
#include "blast.h"
#include "calib.h"
#include "config.h"
#include "esp_cpu.h"
#include "esp_timer.h"
#include "shots.h"
#include "simulate.h"

static Preferences prefs;
static bool testShot = true;

// For `stat`.
static uint32_t nSolved = 0, nConverged = 0, nSent = 0, nTooFew = 0, nDropped = 0;
static uint32_t nVerdict[measure::V_COUNT] = {};
static uint64_t cyclesSum = 0;
static uint32_t cyclesMax = 0;

void positionBegin() {
  prefs.begin("measure");
  testShot = prefs.getBool("testshot", true);
}

bool positionTestShot() { return testShot; }

// The event as a log record: its edges in ns from its first edge, the air, the settings' id.
static void makeRec(const EventResult& r, measure::Rec& rec) {
  const int ref = eventFirstEdge(r);
  const uint64_t samples = (uint64_t)r.firstTime * SAMPLES_PER_BLOCK + ref;
  const Air a = air();
  rec = {};
  rec.n = r.number;
  rec.timeS = (double)(samples / CAPTURE_SAMPLE_HZ) +
              (double)(samples % CAPTURE_SAMPLE_HZ) / CAPTURE_SAMPLE_HZ;
  rec.cfgId = calibId();
  rec.tC = a.tC;
  rec.rh = a.rh;
  rec.hPa = a.hPa;
  rec.measured[0] = a.tMeasured ? 'T' : '-';
  rec.measured[1] = a.rhMeasured ? 'R' : '-';
  rec.measured[2] = a.pMeasured ? 'P' : '-';
  rec.measured[3] = 0;
  rec.cMs = a.c;
  for (int c = 0; c < CHANNELS; c++) {
    rec.t1[c] = rec.t2[c] = measure::NO_EDGE;
    if (!(r.lines & (3 << (2 * c)))) continue;
    rec.channels |= 1 << c;
    const LineResult& L = r.line[2 * c];
    const LineResult& H = r.line[2 * c + 1];
    if (L.rise >= 0) rec.t1[c] = (L.rise - ref) * (int32_t)NS_PER_SAMPLE;
    if (H.rise >= 0) rec.t2[c] = (H.rise - ref) * (int32_t)NS_PER_SAMPLE;
  }
  // The true point, if known: the simulator's shot, else calibration mode's template point.
  rec.calMode = calibMode();
  rec.hasPt = simTruth(&rec.ptX, &rec.ptY) || (rec.calMode && calibPoint(&rec.ptX, &rec.ptY));
  rec.hasCal = calibRecord(&rec.calId, &rec.calStale);
}

// The channels in `mask`, as " CH5 CH7".
static int channelList(char* out, size_t size, uint8_t mask) {
  int n = 0;
  if (size) out[0] = 0;
  for (int c = 0; c < CHANNELS; c++)
    if ((mask >> c) & 1) n += snprintf(out + n, size > (size_t)n ? size - n : 0, " CH%d", c + 1);
  return n;
}

// Why an event is not a shot, in words (after "[verdict] ").
static void verdictText(char* out, size_t size, const measure::EventFit& f, const measure::Cfg& cfg) {
  char chs[48];
  switch (f.verdict) {
    case measure::V_FEW:
      snprintf(out, size, "%d channel%s with a mic, both edges and a good pulse (4 needed)", f.used,
               f.used == 1 ? "" : "s");
      break;
    case measure::V_TOGETHER:
      snprintf(out, size, "every first edge within %.1f us (limit %.0f): an electrical spike, or "
               "a blast from straight ahead", f.spreadNs / 1000, cfg.togetherNs / 1000);
      break;
    case measure::V_NOFIT: snprintf(out, size, "the fit did not converge"); break;
    case measure::V_RESIDUAL:
      snprintf(out, size, "rms over the %.0f ns limit%s", cfg.resLimitNs,
               f.s.dropped >= 0 ? " even with one channel dropped" : "");
      break;
    case measure::V_LOST:
      channelList(chs, sizeof chs, f.mics & ~f.s.used);
      snprintf(out, size, "%d channels lost (%s), %d allowed", f.lost, chs + 1, cfg.maxLost);
      break;
    case measure::V_OUTSIDE:
      snprintf(out, size, "outside the +-%.0f mm area", cfg.areaMm);
      break;
    default: out[0] = 0;
  }
}

void positionProcess(const EventResult& r, Position& p, uint32_t nowMs) {
  measure::Rec rec;
  makeRec(r, rec);
  const measure::Cfg& cfg = calibCfg();
  const uint32_t c0 = esp_cpu_get_cycle_count();
  const measure::EventFit f = measure::solveEvent(cfg, rec);
  const uint32_t cycles = esp_cpu_get_cycle_count() - c0;
  p = {};
  p.mask = f.mask;
  p.solved = f.solved;
  p.s = f.s;
  p.verdict = f.verdict;
  if (f.verdict < measure::V_COUNT) nVerdict[f.verdict]++;

  char line[420];
  char why[160];
  int n;
  if (f.solved) {
    nSolved++;
    cyclesSum += cycles;
    if (cycles > cyclesMax) cyclesMax = cycles;
    if (f.s.converged) nConverged++;
    if (f.s.dropped >= 0) nDropped++;
    const measure::Solution& s = f.s;
    n = snprintf(line, sizeof line, "  shot %u: x %+.3f y %+.3f mm, rms %.0f ns, 1 sd %.3f mm "
                 "(long axis), %u iterations%s, res ns", (unsigned)r.number, s.x, s.y, s.rmsNs,
                 s.major, s.iterations, s.starts > 1 ? " (2 starts)" : "");
    for (int c = 0; c < CHANNELS; c++)
      if (f.mask & (1 << c))
        n += snprintf(line + n, sizeof line - n, " CH%d %+.0f%s", c + 1, s.resNs[c],
                      c == s.dropped ? " (dropped)" : "");
  } else {
    nTooFew++;
    n = snprintf(line, sizeof line, "  shot %u -", (unsigned)r.number);
  }
  if (f.shapeBad) {
    char chs[48];
    channelList(chs, sizeof chs, f.shapeBad);
    n += snprintf(line + n, sizeof line - n, "; pulse shape out of range on%s (t2 - t1 not within "
                  "0-%.0f ns: left out)", chs, cfg.pulseMaxNs);
  }
  const char* tag = measure::verdictName(f.verdict);
  if (!f.good) {
    verdictText(why, sizeof why, f, cfg);
    n += snprintf(line + n, sizeof line - n, "; not sent [%s]: %s", tag, why);
  } else if (rec.calMode) {
    n += snprintf(line + n, sizeof line - n, "; not sent [%s]: calibration mode", tag);
  } else {
    const uint32_t seq = shotAdd(lroundf(f.s.x * 100), lroundf(f.s.y * 100), nowMs);
    p.sent = true;
    nSent++;
    n += snprintf(line + n, sizeof line - n, "; sent as shot %u [%s]", (unsigned)seq, tag);
    if (f.lost == 1) n += snprintf(line + n, sizeof line - n, " (with a channel lost)");
    else if (f.lost > 1) n += snprintf(line + n, sizeof line - n, " (with %d channels lost)", f.lost);
  }
  Serial.println(line);
  char recLine[400];
  measure::formatRec(recLine, sizeof recLine, rec);
  Serial.println(recLine);
}

// ---- The blast check ----

static void letIdleRun() { vTaskDelay(1); }   // core 1's idle task, between blasts

static void printBlastLine(const char* what, int scored, float smallest) {
  if (isinf(smallest))
    Serial.printf("mic:   %-14s %3d of %d scored (none fits a point in the scoring area)\n", what,
                  scored, measure::BLASTS);
  else
    Serial.printf("mic:   %-14s %3d of %d scored, smallest residual of a fit in the area %.1f us%s\n",
                  what, scored, measure::BLASTS, smallest / 1000, scored ? "  <-- WARNING" : "");
}

void positionBlastCheck() {
  measure::SolveInput in = {};
  const measure::Cfg& cfg = calibCfg();
  uint8_t mics = 0;
  in.n = CHANNELS;
  in.c = measure::mmPerNs(air().c);
  for (int c = 0; c < CHANNELS; c++) {
    in.mic[c] = {cfg.ch[c].x, cfg.ch[c].y, cfg.ch[c].h};
    in.sigmaNs[c] = cfg.ch[c].sigmaNs;
    if (cfg.ch[c].mic) mics |= 1 << c;
  }
  const int n = __builtin_popcount(mics);
  if (n < 4) {
    Serial.printf("mic: %d mics: at least 4 are needed for a shot\n", n);
    return;
  }
  const int64_t t0 = esp_timer_get_time();
  const float half = cfg.areaMm > 0 ? cfg.areaMm : CARD_HALF_MM;
  Serial.printf("mic: blast check (%d point sources 10 and 25 m in front, 0-3 m off the axis; "
                "scored = a fit inside +-%.0f mm within the %.0f ns limit):\n", measure::BLASTS,
                half, cfg.resLimitNs);
  measure::BlastResult b = measure::blastCheck(in, mics, cfg.resLimitNs, half, letIdleRun);
  int bad = b.scored;
  printBlastLine("every mic", b.scored, b.smallestNs);
  if (n >= 5) {
    for (int c = 0; c < CHANNELS; c++) {
      if (!(mics & (1 << c))) continue;
      char what[16];
      snprintf(what, sizeof what, "without CH%d", c + 1);
      b = measure::blastCheck(in, mics & ~(1 << c), cfg.resLimitNs, half, letIdleRun);
      printBlastLine(what, b.scored, b.smallestNs);
      bad += b.scored;
    }
  }
  Serial.printf("mic: %s (%.0f ms)\n",
                bad ? "WARNING: blasts can score as hits: the mics (or the ones left after dropping "
                      "one) are close to one circle (PLAN.md D1/D2)"
                    : n >= 5 ? "no blast scores, with every mic or with any one dropped"
                             : "no blast scores (with 4 mics none can be dropped)",
                (esp_timer_get_time() - t0) / 1000.0);
}

// ---- Status and commands ----

void positionPrintStatus() {
  calibPrintStatus();
  Serial.printf("solver: %u events solved (%u converged, %u sent to the app, %u with a channel "
                "dropped), %u with too few channels; per event %.1f us mean, %.1f us longest\n",
                (unsigned)nSolved, (unsigned)nConverged, (unsigned)nSent, (unsigned)nDropped,
                (unsigned)nTooFew,
                nSolved ? (double)cyclesSum / nSolved / getCpuFrequencyMhz() : 0.0,
                (double)cyclesMax / getCpuFrequencyMhz());
  Serial.printf("solver: reslimit %.0f ns, drop-one %s, test shots %s, settings id %08X\n",
                calibCfg().resLimitNs, calibCfg().dropOne ? "on" : "off", testShot ? "on" : "off",
                (unsigned)calibId());
  Serial.print("verdicts since boot:");
  for (int v = 0; v < measure::V_COUNT; v++)
    Serial.printf(" %s %u", measure::verdictName((measure::Verdict)v), (unsigned)nVerdict[v]);
  Serial.println();
}

void positionHelp() {
  Serial.println("  testshot on|off      BOOT's short press makes a random test shot (stored)");
}

bool positionCommand(const char* cmd, const char* line) {
  if (!strcmp(cmd, "testshot")) {
    char word[8] = "";
    sscanf(line, "%*s %7s", word);
    if (!strcmp(word, "on") || !strcmp(word, "off")) {
      testShot = !strcmp(word, "on");
      prefs.putBool("testshot", testShot);
    } else if (word[0]) {
      Serial.println("testshot on|off");
      return true;
    }
    Serial.printf("test shots %s (BOOT short press)\n", testShot ? "on" : "off");
    return true;
  }
  return false;
}
