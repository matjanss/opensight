// OpenSights - sensor simulator (measurement steps 2-3).
//
// A simulated shot: the shot position, the mic positions and a speed of sound give the true
// arrival at each mic; a trip model gives when OUT_L and OUT_H go high after it; the pulse
// generator (pulsegen.cpp) makes those edges in hardware, 25 ns steps, on CH3-CH8's own
// pins, and the capture reads them back like a real event. The simulator knows the edges it
// sent, so it checks:
//   - the capture and the arrival times: t0 from the captured edges against t0 from the
//     edges sent, as differences to the first channel. Only the 100 ns sampling is
//     between them, so the error stays within (1 + 2k) samples (plus a tick for the second
//     MCPWM unit), about 56 ns rms;
//   - the t0 model: t0 from the edges sent against the true arrival. Zero for the linear
//     trip model with k = 1/3 (up to the 25 ns steps); the sine model shows what the
//     straight-line formula leaves on a curved rise;
//   - the position (step 3c): the solver's shot against the point it was sent to. With the
//     firmware's mics, air, offsets and model the same as the simulator's (`mic sim`), only
//     the sampling is left: about 0.013 mm rms, 0.06 mm worst with 6 mics (a PC
//     simulation). With a late or early channel, drop-one must leave that one out.
// Faults: a channel late or early by a half cycle (24 us), its edges missing or H only,
// a second shot, or a blast: a point source metres in front, which must never score.
// `sim raw` sends fixed edges instead, to check the generator itself: equal times on all
// channels show any offset between the two MCPWM units.
//
// Trip model: the pulse rises like A sin(pi t / 2T) (or its straight start, A pi t / 2T),
// T = 12 us; OUT_L trips at L, OUT_H at H (default 1 and 4: k = L / (H - L) = 1/3; `sim
// thr`); A = amp * 100 mm / distance (a blast: amp, every channel alike).
#include "simulate.h"
#include "airtemp.h"
#include "calib.h"
#include "capture.h"
#include "config.h"
#include "esp_cpu.h"
#include "esp_timer.h"
#include "pulsegen.h"

static const int      N          = SIM_CHANNELS;
static const int      F          = SIM_FIRST_CHANNEL;
static const uint32_t HZ         = 40000000;   // 25 ns ticks: 16 bits hold 1.6 ms
static const double   TICK_NS    = 1e9 / HZ;
static const uint32_t LEAD_TICKS = 800;        // 20 us from the start to the first rise
static const uint32_t HOLD_TICKS = 400;        // every line stays high >= 10 us
static const double   RISE_NS    = 12000;      // quarter period of the simulated pulse
static const double   LATE_NS    = 24000;      // a half-cycle-late (or early) trip
static const int      MAX_COUNT  = 2000;

// ---- Settings (RAM; `sim` shows them) ----

static float micX[N], micY[N], micH[N];
static float trueOffsetNs[N] = {};
static float simTempC = NAN;   // NAN: the firmware's own air (airtemp.h); else `sim temp`
static float simAmp = 40.0f;      // pulse amplitude at 100 mm, in units of the default OUT_L level
static bool  sineModel = false;
static float thrL[N], thrH[N];    // trip levels per channel, same units (`sim thr`)
enum Fault : uint8_t { F_NONE, F_LATE, F_EARLY, F_MISSING, F_HONLY, F_DOUBLE, F_BLAST };
static const char* const FAULT_TEXT[] = {"none", "late", "early", "missing", "honly", "double", "blast"};
static Fault    fault = F_NONE;
static int      faultCh = 0;        // 0..N-1
static uint32_t doubleUs = 2500;
static float    blastM = 10, blastOffMm = 0;   // F_BLAST: metres in front, mm off the axis

static bool micsSet = false;
static void defaultMics() {
  for (int i = 0; i < N; i++) {
    micX[i] = MIC_LAYOUT_D2[i % 6][0];
    micY[i] = MIC_LAYOUT_D2[i % 6][1];
    micH[i] = MIC_H_DEFAULT_MM;
    thrL[i] = 1;
    thrH[i] = 4;
  }
  micsSet = true;
}

static double trueK(int i) { return thrL[i] / (thrH[i] - thrL[i]); }

// ---- A run ----

enum Mode : uint8_t { M_RAW, M_POINT, M_RANDOM };

struct Shot {
  float    x, y;
  bool     on[N][2];
  uint32_t rise[N][2];    // ticks from the start of the run
  double   geo[N];        // true arrival: distance / speed of sound, ns
};

struct Stat {
  uint32_t n, outside;
  double   sum, sumSq, maxAbs;
  void add(double e, double limit) {
    n++;
    sum += e;
    sumSq += e * e;
    if (fabs(e) > maxAbs) maxAbs = fabs(e);
    if (fabs(e) > limit) outside++;
  }
};

static struct {
  bool     active;
  Mode     mode;
  uint32_t count, sent, seen, notSeen, unexpected, late, skipped;
  uint32_t spacingMs, nextAt, periodTicks, doubleWaitUs;
  uint32_t missedAtStart, missedAtFire;
  bool     pending1, pending2, closed;
  bool     fired2, second2Event, second2Good;   // double: second shot fired, its event, had t0s
  float    px, py;                 // M_POINT
  uint32_t rawRiseNs[N];           // M_RAW: L rise per channel, ns from the first
  uint32_t rawGapNs;
  Shot     shot;
  Fault    fault;
  int      faultCh;
  double   limitNs;                // capture error limit
  Stat     err[N], model[N];       // t0: vs the edges sent, vs the true arrival
  Stat     raw[N][2];              // M_RAW: each line vs the first sim channel's L
  uint32_t noT0[N], extraT0[N], rawMissing[N][2];
  // The position check (3c): the solver's shot against the truth.
  bool     posCounts;              // part of PASS: the firmware solves the simulator's world
  const char* posWhyNot;
  float    posLimitMm;             // the worst error allowed (more with a channel dropped)
  int      expectDrop;             // the channel drop-one must leave out, -1: none
  uint32_t sentToApp, dropRight, dropWrong, dropNone;
  Stat     posErr, posRes;         // mm; the fit's rms residual, ns
  double   posDx, posDy;           // sums, for the mean error
  uint32_t posUnsolved, posNotConverged;
  uint32_t second[5];              // double: measured, MISSED, ignored (dead time), lost, partial
} run;

bool simActive() { return run.active; }

bool simMic(int ch, float* x, float* y, float* h) {
  if (ch < F || ch >= F + N) return false;
  if (!micsSet) defaultMics();
  *x = micX[ch - F];
  *y = micY[ch - F];
  *h = micH[ch - F];
  return true;
}

// The k the firmware uses on a simulated channel (0..N-1), and the largest of them.
static double simK(int i) { return calibCfg().ch[F + i].k; }
static double simKMax() {
  double k = 0;
  for (int i = 0; i < N; i++) if (simK(i) > k) k = simK(i);
  return k;
}

// Why the position check can't be part of PASS: the firmware's world differs from the
// simulator's on purpose (or by mistake). nullptr: it counts. A blast has no position, but
// whether one scores counts only with the same mics too.
static const char* positionMismatch(Fault fl) {
  const measure::Cfg& cfg = calibCfg();
  for (int i = 0; i < N; i++) {
    const measure::ChannelCfg& ch = cfg.ch[F + i];
    if (!ch.mic || fabsf(ch.x - micX[i]) > 0.005f || fabsf(ch.y - micY[i]) > 0.005f)
      return "the firmware's mics are not the simulator's (`mic sim`)";
    if (fabsf(ch.h - micH[i]) > 0.005f) return "the firmware's mic heights are not the simulator's";
    if (fl == F_BLAST) continue;
    if (fabsf(ch.offsetNs - trueOffsetNs[i]) > 0.5f) return "the firmware's offsets are not the simulator's";
    if (!sineModel && fabsf(ch.k - trueK(i)) > 0.001f)
      return "the firmware's k is not the simulator's (L / (H - L) per channel: `sim`, `k`)";
  }
  if (!isnan(simTempC)) return "`sim temp` gives the simulator other air than the firmware's";
  if (fl == F_BLAST) return nullptr;
  if (sineModel) return "the sine model (t0's formula is exact for the linear one only)";
  if (fl == F_DOUBLE) return "the double-shot fault";
  if ((fl == F_LATE || fl == F_EARLY) && !cfg.dropOne) return "a late or early channel with drop-one off";
  return nullptr;
}

static uint16_t simChannelMask() { return ((1 << N) - 1) << F; }
static uint16_t simLineMask() {
  uint16_t m = 0;
  for (int i = 0; i < N; i++) m |= 3 << (2 * (F + i));
  return m;
}

// The simulator's air: the firmware's (sensor or assumed), or its temperature set by
// `sim temp` (humidity and pressure stay the firmware's).
static Air simAir() {
  Air a = air();
  if (!isnan(simTempC)) {
    a.tC = simTempC;
    a.c = airSoundSpeed(a.tC, a.rh, a.hPa);
  }
  return a;
}
static double speedMmPerNs(const Air& a) { return a.c * 1e-6; }

static uint32_t maxRiseTicks() { return run.periodTicks - HOLD_TICKS; }

static double uniform(double lo, double hi);

// The edges of a shot at (x, y), or of a blast (F_BLAST: x and y are NAN). False if they
// don't fit in a run (the window).
static bool makeShot(float x, float y, Shot& s) {
  const double c = speedMmPerNs(simAir());
  const double scale = 2 * RISE_NS / M_PI;
  const bool blast = run.fault == F_BLAST;
  double t[N][2];
  double first = 1e18;
  double bx = 0, by = 0;
  if (blast) {
    const double phi = uniform(0, 2 * M_PI);
    bx = blastOffMm * cos(phi);
    by = blastOffMm * sin(phi);
    x = y = NAN;
  }
  s.x = x;
  s.y = y;
  for (int i = 0; i < N; i++) {
    double d;
    if (blast) {
      const double dx = micX[i] - bx, dy = micY[i] - by, dz = blastM * 1000.0 + micH[i];
      d = sqrt(dx * dx + dy * dy + dz * dz);
    } else {
      d = sqrt((x - micX[i]) * (x - micX[i]) + (y - micY[i]) * (y - micY[i]) + micH[i] * micH[i]);
      if (d < 1) d = 1;
    }
    s.geo[i] = d / c;
    double a = blast ? simAmp : simAmp * 100.0 / d;
    double tl, th;
    bool lOn = true, hOn = true;
    if (sineModel) {
      lOn = a > thrL[i];
      hOn = a > thrH[i];
      tl = lOn ? scale * asin(thrL[i] / a) : 0;
      th = hOn ? scale * asin(thrH[i] / a) : 0;
    } else {
      tl = scale * thrL[i] / a;
      th = scale * thrH[i] / a;
    }
    double late = run.faultCh != i ? 0 : run.fault == F_LATE ? LATE_NS : run.fault == F_EARLY ? -LATE_NS : 0;
    bool gone = (run.fault == F_MISSING || run.fault == F_HONLY) && run.faultCh == i;
    s.on[i][0] = lOn && !gone;
    s.on[i][1] = hOn && !(run.fault == F_MISSING && run.faultCh == i);
    t[i][0] = s.geo[i] + trueOffsetNs[i] + tl + late;
    t[i][1] = s.geo[i] + trueOffsetNs[i] + th + late;
    for (int h = 0; h < 2; h++)
      if (s.on[i][h] && t[i][h] < first) first = t[i][h];
  }
  for (int i = 0; i < N; i++)
    for (int h = 0; h < 2; h++) {
      s.rise[i][h] = 0;
      if (!s.on[i][h]) continue;
      s.rise[i][h] = LEAD_TICKS + (uint32_t)llround((t[i][h] - first) / TICK_NS);
      if (s.rise[i][h] > maxRiseTicks()) return false;
    }
  return true;
}

static bool makeRawShot(Shot& s) {
  s.x = s.y = 0;
  for (int i = 0; i < N; i++) {
    s.geo[i] = 0;
    s.on[i][0] = s.on[i][1] = true;
    s.rise[i][0] = LEAD_TICKS + (uint32_t)llround(run.rawRiseNs[i] / TICK_NS);
    s.rise[i][1] = s.rise[i][0] + (uint32_t)llround(run.rawGapNs / TICK_NS);
    if (s.rise[i][1] > maxRiseTicks()) return false;
  }
  return true;
}

static void startCommon(Mode m, uint32_t count) {
  if (run.active) { Serial.println("sim: already running (`sim stop`)"); return; }
  if (captureSelftestActive()) { Serial.println("sim: wait for the self-test to finish"); return; }
  if (!micsSet) defaultMics();
  if (count < 1 || count > MAX_COUNT) { Serial.printf("sim: 1..%d shots\n", MAX_COUNT); return; }
  // Every rise, and the end of the run where the lines fall, inside the event window.
  uint64_t windowTicks = ((uint64_t)captureWindowUs() * 1000 - 2 * NS_PER_SAMPLE) / (uint64_t)TICK_NS;
  uint64_t period = LEAD_TICKS + windowTicks;
  if (period > 65534) period = 65534;
  Fault fl = fault;
  int fc = faultCh;
  bool rawMode = m == M_RAW;
  float px = run.px, py = run.py;
  uint32_t rawRise[N], rawGap = run.rawGapNs;
  memcpy(rawRise, run.rawRiseNs, sizeof rawRise);

  memset(&run, 0, sizeof run);
  run.mode = m;
  run.count = count;
  run.periodTicks = (uint32_t)period;
  run.fault = rawMode ? F_NONE : fl;
  run.faultCh = fc;
  run.px = px;
  run.py = py;
  memcpy(run.rawRiseNs, rawRise, sizeof rawRise);
  run.rawGapNs = rawGap;
  run.doubleWaitUs = doubleUs;
  uint32_t runUs = (uint32_t)(period * TICK_NS / 1000) + 20;
  if (run.fault == F_DOUBLE && run.doubleWaitUs < runUs) run.doubleWaitUs = runUs;
  const double k = simKMax();
  run.limitNs = (1 + 2 * k) * NS_PER_SAMPLE + TICK_NS + 5;
  run.posWhyNot = rawMode ? "raw edges are not a shot" : positionMismatch(run.fault);
  run.posCounts = !run.posWhyNot;
  // A late or early channel must be dropped; without it (or its t0) 5 mics are left.
  run.expectDrop = run.fault == F_LATE || run.fault == F_EARLY ? F + run.faultCh : -1;
  run.posLimitMm = run.fault == F_LATE || run.fault == F_EARLY || run.fault == F_MISSING ||
                   run.fault == F_HONLY ? SIM_POS_LIMIT_DROP_MM : SIM_POS_LIMIT_MM;

  // Check the first shot fits before setting anything up.
  Shot probe;
  bool fits = rawMode ? makeRawShot(probe) : makeShot(m == M_POINT ? px : 0, m == M_POINT ? py : 0, probe);
  if (!fits) {
    Serial.printf("sim: the shot doesn't fit in the event window (%u us after the first edge: "
                  "the mic span setting)\n", (unsigned)captureWindowUs());
    return;
  }

  int pins[N][2];
  for (int i = 0; i < N; i++) {
    pins[i][0] = CAPTURE_LINE_PINS[2 * (F + i)];
    pins[i][1] = CAPTURE_LINE_PINS[2 * (F + i) + 1];
  }
  if (!pulsegenCreate(N, pins, HZ, run.periodTicks)) {
    pulsegenDelete();
    captureLinesToInputs(simLineMask());
    Serial.println("sim: could not set up the MCPWM pulse generator");
    return;
  }
  captureTestChannels(simChannelMask());
  run.active = true;
  uint32_t dead = captureDeadtimeMs();
  // Six lines per shot (~1000 bytes): 150 ms apart keeps inside 115200 baud.
  run.spacingMs = dead + 20 > 150 ? dead + 20 : 150;
  run.nextAt = millis() + 100;
  run.missedAtStart = captureMissedCount();
  run.closed = true;

  char pinText[64];
  int n = 0;
  for (int i = 0; i < N; i++)
    n += snprintf(pinText + n, sizeof pinText - n, "%s%d/%d", i ? ", " : "", pins[i][0], pins[i][1]);
  calibPrintCfg();   // so a log of the run has the settings its rec lines name
  if (rawMode) {
    Serial.printf("sim: %u raw runs on CH%d-CH%d (GPIO %s), L at", (unsigned)count, F + 1, F + N, pinText);
    for (int i = 0; i < N; i++) Serial.printf(" %u", (unsigned)run.rawRiseNs[i]);
    Serial.printf(" ns, H %u ns after L, %.0f ns ticks, every %u ms\n", (unsigned)run.rawGapNs,
                  TICK_NS, (unsigned)run.spacingMs);
    return;
  }
  const Air sa = simAir();
  Serial.printf("sim: %u shots %s on CH%d-CH%d (GPIO %s); %s model, amp %.0f, air %.1f °C "
                "(%s, %.2f m/s); firmware k up to %.4f; fault %s", (unsigned)count,
                m == M_RANDOM ? "at random points on the card" : "at one point", F + 1, F + N,
                pinText, sineModel ? "sine" : "linear", simAmp, sa.tC,
                isnan(simTempC) ? "the firmware's" : "sim temp", sa.c, k, FAULT_TEXT[run.fault]);
  if (run.fault == F_DOUBLE) Serial.printf(" (second shot %u us later)", (unsigned)run.doubleWaitUs);
  else if (run.fault == F_BLAST) Serial.printf(" (%.0f m in front, %.0f mm off the axis)", blastM, blastOffMm);
  else if (run.fault != F_NONE) Serial.printf(" on CH%d", F + 1 + run.faultCh);
  Serial.printf("; every %u ms\n", (unsigned)run.spacingMs);
  if (run.fault == F_BLAST)
    Serial.printf(run.posCounts ? "sim: blast check on: none may be sent to the app\n"
                                : "sim: blast check for information only: %s\n", run.posWhyNot);
  else if (run.posCounts)
    Serial.printf("sim: position check on: the firmware has the simulator's mics and air (limit %.2f "
                  "mm)%s\n", run.posLimitMm, run.expectDrop >= 0 ? "; drop-one must leave the faulty channel out" : "");
  else
    Serial.printf("sim: position check for information only: %s\n", run.posWhyNot);
}

// ---- Recording ----

static void recordRaw(const EventResult& r) {
  const Shot& s = run.shot;
  const int refLine = 2 * F;
  char line[200];
  int n = snprintf(line, sizeof line, "  sim raw, ns vs set (from CH%d L):", F + 1);
  if (r.line[refLine].rise < 0) {
    run.rawMissing[0][0]++;
    Serial.printf("  sim raw: no CH%d L edge\n", F + 1);
    return;
  }
  for (int i = 0; i < N; i++)
    for (int h = 0; h < 2; h++) {
      if (i == 0 && h == 0) continue;
      int l = 2 * (F + i) + h;
      if (r.line[l].rise < 0) {
        run.rawMissing[i][h]++;
        n += snprintf(line + n, sizeof line - n, "  CH%d %c -", F + 1 + i, h ? 'H' : 'L');
        continue;
      }
      double e = (double)(r.line[l].rise - r.line[refLine].rise) * NS_PER_SAMPLE -
                 ((double)s.rise[i][h] - s.rise[0][0]) * TICK_NS;
      run.raw[i][h].add(e, NS_PER_SAMPLE + TICK_NS + 5);
      n += snprintf(line + n, sizeof line - n, "  CH%d %c %+.0f", F + 1 + i, h ? 'H' : 'L', e);
    }
  Serial.println(line);
}

static void recordShot(const EventResult& r, const Arrival& a, const Position& p) {
  const Shot& s = run.shot;
  double expT0[N];
  bool expOn[N], got[N];
  int ref = -1;
  for (int i = 0; i < N; i++) {
    expOn[i] = s.on[i][0] && s.on[i][1];
    expT0[i] = expOn[i] ? s.rise[i][0] * TICK_NS - simK(i) * ((double)s.rise[i][1] - s.rise[i][0]) * TICK_NS -
                              calibCfg().ch[F + i].offsetNs
                        : 0;
    got[i] = a.valid & (1 << (F + i));
    if (expOn[i] && !got[i]) run.noT0[i]++;
    if (!expOn[i] && got[i]) run.extraT0[i]++;
    if (ref < 0 && expOn[i] && got[i]) ref = i;
  }
  char line[260];
  const bool blast = run.fault == F_BLAST;
  int n = blast ? snprintf(line, sizeof line, "  sim blast, t0 error ns vs sent/vs true:")
                : snprintf(line, sizeof line, "  sim (%.1f, %.1f) mm, t0 error ns vs sent/vs true:", s.x, s.y);
  for (int i = 0; i < N; i++) {
    if (i == ref) continue;
    if (!expOn[i] || !got[i] || ref < 0) {
      n += snprintf(line + n, sizeof line - n, "  CH%d %s", F + 1 + i,
                    !expOn[i] && !got[i] ? "- (as sent)" : expOn[i] ? "MISSING" : "UNEXPECTED");
      continue;
    }
    double e = (a.t0[F + i] - a.t0[F + ref]) - (expT0[i] - expT0[ref]);
    double m = (expT0[i] - expT0[ref]) - (s.geo[i] - s.geo[ref]);
    run.err[i].add(e, run.limitNs);
    run.model[i].add(m, 1e18);
    n += snprintf(line + n, sizeof line - n, "  CH%d %+.0f/%+.0f%s", F + 1 + i, e, m,
                  fabs(e) > run.limitNs ? " (!)" : "");
  }
  if (p.sent) run.sentToApp++;
  if (blast) {
    n += snprintf(line + n, sizeof line - n, "; %s", p.sent ? "SENT to the app (!)" : "not sent");
    Serial.println(line);
    return;
  }
  if (p.solved) {
    const int dropped = p.s.dropped;
    if (run.expectDrop >= 0 && dropped == run.expectDrop) run.dropRight++;
    else if (dropped >= 0) run.dropWrong++;
    else if (run.expectDrop >= 0) run.dropNone++;
    if (dropped >= 0) n += snprintf(line + n, sizeof line - n, "; CH%d dropped", dropped + 1);
  }
  if (!p.solved) {
    run.posUnsolved++;
    n += snprintf(line + n, sizeof line - n, "; not solved");
  } else if (!p.s.converged) {
    run.posNotConverged++;
    n += snprintf(line + n, sizeof line - n, "; did not converge");
  } else {
    const double dx = p.s.x - s.x, dy = p.s.y - s.y, e = sqrt(dx * dx + dy * dy);
    run.posErr.add(e, run.posLimitMm);
    run.posRes.add(p.s.rmsNs, 1e18);
    run.posDx += dx;
    run.posDy += dy;
    n += snprintf(line + n, sizeof line - n, "; position error %.3f mm%s", e,
                  run.posCounts && e > run.posLimitMm ? " (!)" : "");
  }
  Serial.println(line);
}

void simRecord(const EventResult& r, const Arrival& a, const Position& p) {
  if (!run.active) return;
  if (run.pending1) run.pending1 = false;
  else if (run.pending2) {
    // The second shot's event. Without a t0 on any channel it isn't a measurement of the
    // shot but a window opened in its middle (its start was missed).
    run.pending2 = false;
    run.second2Event = true;
    run.second2Good = (a.valid & (((1 << N) - 1) << F)) != 0;
    run.seen++;
    if (run.second2Good) recordShot(r, a, p);
    else Serial.println("  sim: an event with no first edges: a window opened in the middle of "
                        "the second shot");
    return;
  } else {
    run.unexpected++;
    Serial.println("  sim: an event with no shot sent");
    return;
  }
  run.seen++;
  if (run.mode == M_RAW) recordRaw(r);
  else recordShot(r, a, p);
}

// ---- Firing ----

static void closeShot() {
  if (run.closed) return;
  run.closed = true;
  if (run.pending1) {
    run.notSeen++;
    Serial.printf("sim: shot %u was not seen\n", (unsigned)run.sent);
  }
  if (run.fired2) {
    bool missed = captureMissedCount() != run.missedAtFire;
    if (run.second2Event && run.second2Good) run.second[0]++;
    else if (run.second2Event) {
      run.second[4]++;
      Serial.printf("sim: the second shot of %u came out as a partial event%s\n",
                    (unsigned)run.sent, missed ? " after its MISSED" : "");
    } else if (missed) run.second[1]++;
    else if (run.doubleWaitUs < captureDeadUs()) run.second[2]++;
    else {
      run.second[3]++;
      Serial.printf("sim: the second shot of %u was lost (after the dead time, not reported)\n",
                    (unsigned)run.sent);
    }
  }
  run.pending1 = run.pending2 = false;
  run.fired2 = run.second2Event = run.second2Good = false;
}

static double uniform(double lo, double hi) { return lo + (hi - lo) * (esp_random() / 4294967296.0); }

static void finish();

void simService(uint32_t now) {
  if (!run.active) return;
  if ((int32_t)(now - run.nextAt) < 0) return;
  closeShot();
  if (run.sent >= run.count) { finish(); return; }
  if (!captureIdle()) return;   // wait for the previous event to finish

  Shot& s = run.shot;
  bool ok = run.mode == M_RAW ? makeRawShot(s)
          : run.mode == M_POINT ? makeShot(run.px, run.py, s)
          : makeShot(uniform(-SIM_CARD_MM, SIM_CARD_MM), uniform(-SIM_CARD_MM, SIM_CARD_MM), s);
  if (!ok) {
    run.skipped++;
    run.sent++;
    run.nextAt = now + run.spacingMs;
    return;
  }
  // Lines fall together at the end of the run: end it soon after the last rise, so a shot
  // isn't kept high longer than a real comparator pulse needs to be.
  uint32_t last = LEAD_TICKS;
  for (int i = 0; i < N; i++)
    for (int h = 0; h < 2; h++)
      if (s.on[i][h] && s.rise[i][h] > last) last = s.rise[i][h];
  pulsegenSetPeriod(last + HOLD_TICKS + 1);
  for (int i = 0; i < N; i++)
    pulsegenSet(i, s.rise[i][0], s.rise[i][1], s.on[i][0], s.on[i][1]);
  // At a random point in a DMA block and in a sample, as real sound comes (step 1).
  delayMicroseconds(esp_random() % (2 * BLOCK_US));
  uint32_t wait = esp_random() % (2 * NS_PER_SAMPLE * getCpuFrequencyMhz() / 1000);
  uint32_t c0 = esp_cpu_get_cycle_count();
  while (esp_cpu_get_cycle_count() - c0 < wait) {}
  run.missedAtFire = captureMissedCount();
  int64_t t0 = esp_timer_get_time();
  if (pulsegenFire() >= LEAD_TICKS * TICK_NS) run.late++;
  run.pending1 = true;
  if (run.fault == F_DOUBLE) {
    while (esp_timer_get_time() - t0 < run.doubleWaitUs) {}
    if (pulsegenFire() >= LEAD_TICKS * TICK_NS) run.late++;
    run.pending2 = true;
    run.fired2 = true;
  }
  run.closed = false;
  run.sent++;
  run.nextAt = now + run.spacingMs;
}

// ---- Summary ----

static void finish() {
  closeShot();
  pulsegenDelete();
  captureLinesToInputs(simLineMask());
  captureEndTestChannels();
  run.active = false;

  uint32_t missed = captureMissedCount() - run.missedAtStart;
  Serial.printf("sim: %u shots sent, %u events, %u not seen, %u unexpected, %u possibly missed, "
                "%u late starts%s\n", (unsigned)run.sent, (unsigned)run.seen, (unsigned)run.notSeen,
                (unsigned)run.unexpected, (unsigned)missed, (unsigned)run.late,
                run.skipped ? " (some shots didn't fit the window: skipped)" : "");
  bool pass = run.notSeen == 0 && run.unexpected == 0 && run.late == 0 && run.skipped == 0 &&
              run.seen > 0;
  if (run.fault != F_DOUBLE && missed) pass = false;

  if (run.mode == M_RAW) {
    for (int i = 0; i < N; i++)
      for (int h = 0; h < 2; h++) {
        if (i == 0 && h == 0) continue;
        const Stat& st = run.raw[i][h];
        Serial.printf("sim: CH%d %c vs CH%d L: %u runs, mean %+.1f ns, worst %.0f ns, %u more "
                      "than a sample + a tick off, %u without the edge\n", F + 1 + i, h ? 'H' : 'L',
                      F + 1, (unsigned)st.n, st.n ? st.sum / st.n : 0.0, st.maxAbs,
                      (unsigned)st.outside, (unsigned)run.rawMissing[i][h]);
        if (st.outside || run.rawMissing[i][h]) pass = false;
      }
    Serial.printf("sim: CH%d-CH%d are on MCPWM unit 0, CH%d-CH%d on unit 1: a mean on the second "
                  "group alone is an offset between the units\n", F + 1, F + 3, F + 4, F + N);
    Serial.printf("sim: %s\n", pass ? "PASS" : "FAIL");
    return;
  }

  // The captured t0 difference of two channels is off by the rounding of four edges to the
  // 100 ns samples: rms S * sqrt(2 ((1 + k)^2 + k^2) / 12).
  const double S = NS_PER_SAMPLE;
  for (int i = 0; i < N; i++) {
    const double k = simK(i);
    const double rmsExp = S * sqrt(2 * ((1 + k) * (1 + k) + k * k) / 12);
    const Stat& st = run.err[i];
    const Stat& md = run.model[i];
    bool faulty = run.fault != F_NONE && run.fault != F_DOUBLE && run.fault != F_BLAST && run.faultCh == i;
    if (st.n) {
      double mean = st.sum / st.n, rms = sqrt(st.sumSq / st.n);
      bool meanOff = fabs(mean) > 3 * rmsExp / sqrt((double)st.n) + 2;
      Serial.printf("sim: CH%d t0 vs sent: %u shots, mean %+.1f ns, rms %.1f (rounding alone "
                    "about %.0f), worst %.0f (limit %.0f)%s\n", F + 1 + i, (unsigned)st.n, mean,
                    rms, rmsExp, st.maxAbs, run.limitNs,
                    meanOff ? "; mean more than 3x the rounding's: a fixed offset" : "");
      Serial.printf("sim: CH%d t0 vs true arrival: mean %+.1f ns, worst %.1f ns%s\n", F + 1 + i,
                    md.sum / md.n, md.maxAbs,
                    faulty && run.fault == F_LATE    ? " (the late trip: drop-one must leave it out)"
                    : faulty && run.fault == F_EARLY ? " (the early trip: drop-one must leave it out)"
                                                     : "");
      if (st.outside) pass = false;
    }
    if (run.noT0[i])
      Serial.printf("sim: CH%d: no t0 in %u shots that had one\n", F + 1 + i, (unsigned)run.noT0[i]);
    if (run.extraT0[i])
      Serial.printf("sim: CH%d: a t0 in %u shots without one\n", F + 1 + i, (unsigned)run.extraT0[i]);
    if (run.noT0[i] || run.extraT0[i]) pass = false;
    if (faulty && (run.fault == F_MISSING || run.fault == F_HONLY))
      Serial.printf("sim: CH%d had no t0, as the fault set it (the fit has the other %d)\n",
                    F + 1 + i, N - 1);
  }
  if (run.fault == F_DOUBLE) {
    Serial.printf("sim: second shots %u us after the first: %u measured as their own event, %u "
                  "reported MISSED, %u ignored inside the dead time (%.1f ms in effect), %u lost, %u partial "
                  "events (a window opened in the middle of a shot)\n",
                  (unsigned)run.doubleWaitUs, (unsigned)run.second[0], (unsigned)run.second[1],
                  (unsigned)run.second[2], captureDeadUs() / 1000.0, (unsigned)run.second[3],
                  (unsigned)run.second[4]);
    if (run.second[3] || run.second[4]) pass = false;
  }
  // The position check (a blast: whether any scored).
  const Stat& pe = run.posErr;
  const uint32_t tried = pe.n + run.posUnsolved + run.posNotConverged;
  if (run.fault == F_BLAST) {
    Serial.printf("sim: blasts %.0f m in front, %.0f mm off the axis: %u of %u sent to the app\n",
                  blastM, blastOffMm, (unsigned)run.sentToApp, (unsigned)run.seen);
    if (run.posCounts) {
      Serial.printf("sim: blast check %s (none may score)\n", run.sentToApp ? "FAIL" : "PASS");
      if (run.sentToApp) pass = false;
    } else {
      Serial.printf("sim: blast check not part of PASS: %s\n", run.posWhyNot);
    }
  } else if (tried) {
    Serial.printf("sim: position vs the truth: %u of %u shots solved and converged", (unsigned)pe.n,
                  (unsigned)tried);
    if (pe.n)
      Serial.printf(", error rms %.4f mm, worst %.4f mm, mean (%+.4f, %+.4f) mm; rms residual "
                    "mean %.0f ns, worst %.0f ns", sqrt(pe.sumSq / pe.n), pe.maxAbs, run.posDx / pe.n,
                    run.posDy / pe.n, run.posRes.sum / pe.n, run.posRes.maxAbs);
    Serial.println();
    Serial.printf("sim: %u of %u sent to the app; drop-one: ", (unsigned)run.sentToApp, (unsigned)tried);
    if (run.expectDrop >= 0)
      Serial.printf("CH%d dropped in %u, another channel in %u, none in %u\n", run.expectDrop + 1,
                    (unsigned)run.dropRight, (unsigned)run.dropWrong, (unsigned)run.dropNone);
    else
      Serial.printf("a channel dropped in %u\n", (unsigned)run.dropWrong);
    if (run.posCounts) {
      const bool drops = run.expectDrop >= 0 ? run.dropRight == tried : run.dropWrong == 0;
      const bool ok = pe.n == tried && pe.outside == 0 && run.sentToApp == tried && drops;
      Serial.printf("sim: position check %s (every shot solved and sent, %s, worst error within "
                    "%.2f mm)\n", ok ? "PASS" : "FAIL",
                    run.expectDrop >= 0 ? "the faulty channel dropped" : "none dropped", run.posLimitMm);
      if (!ok) pass = false;
    } else {
      Serial.printf("sim: position check not part of PASS: %s\n", run.posWhyNot);
    }
  }
  Serial.printf("sim: %s\n", pass ? "PASS" : "FAIL");
}

// ---- Commands ----

void simHelp() {
  Serial.println("  sim at <x> <y> [n]   n shots at (x, y) mm from the card centre, on CH3-CH8\n"
                 "                       (no sensor boards there!), default 20\n"
                 "  sim random [n]       n shots at random points on the card, default 100\n"
                 "  sim raw <t3>..<t8> [n] [gap]  L of CH3-CH8 at these ns, H gap ns later\n"
                 "                       (default 1000), n runs (default 20)\n"
                 "  sim fault none | late <ch> | early <ch> | missing <ch> | honly <ch> |\n"
                 "                       double <us> | blast <m> <mm off the axis>\n"
                 "  sim model linear|sine, sim amp <n>, sim temp <C>|air, sim offset <ch> <ns>,\n"
                 "  sim thr <ch> <L> <H>  a channel's trip levels (default 1 4: k = L / (H - L))\n"
                 "  sim mic <ch> <x> <y> [h]  the simulated mics (default D1/D2, h 10.5 mm); `sim`\n"
                 "                       shows them; `mic sim` gives the firmware the same; sim stop");
}

static void printSettings() {
  if (!micsSet) defaultMics();
  const Air sa = simAir();
  Serial.printf("sim: %s model, amp %.0f, air %.1f °C (%s, %.2f m/s), fault %s",
                sineModel ? "sine" : "linear", simAmp, sa.tC,
                isnan(simTempC) ? "the firmware's" : "sim temp", sa.c, FAULT_TEXT[fault]);
  if (fault == F_DOUBLE) Serial.printf(" %u us", (unsigned)doubleUs);
  else if (fault == F_BLAST) Serial.printf(" %.0f m in front, %.0f mm off the axis", blastM, blastOffMm);
  else if (fault != F_NONE) Serial.printf(" CH%d", F + 1 + faultCh);
  Serial.println();
  for (int i = 0; i < N; i++)
    Serial.printf("sim: CH%d mic at (%.1f, %.1f) mm, h %.1f mm, own delay %.0f ns, trips at %g/%g "
                  "(k %.4f)\n", F + 1 + i, micX[i], micY[i], micH[i], trueOffsetNs[i], thrL[i],
                  thrH[i], trueK(i));
}

static int simChannelArg(int ch) { return ch >= F + 1 && ch <= F + N ? ch - F - 1 : -1; }

bool simCommand(const char* cmd, const char* line) {
  if (strcmp(cmd, "sim")) return false;
  char sub[12] = "", word[12] = "";
  sscanf(line, "%*s %11s %11s", sub, word);
  if (!micsSet) defaultMics();
  if (!sub[0]) { printSettings(); return true; }

  if (!strcmp(sub, "stop")) {
    if (run.active) finish();
    return true;
  }
  if (!strcmp(sub, "at")) {
    float x, y;
    unsigned long n = 20;
    if (sscanf(line, "%*s %*s %f %f %lu", &x, &y, &n) < 2 || fabsf(x) > 500 || fabsf(y) > 500) {
      Serial.println("sim at <x> <y> [n] (mm, +-500)");
      return true;
    }
    if (!run.active) { run.px = x; run.py = y; }
    startCommon(M_POINT, n);
    return true;
  }
  if (!strcmp(sub, "random")) {
    unsigned long n = 100;
    sscanf(line, "%*s %*s %lu", &n);
    startCommon(M_RANDOM, n);
    return true;
  }
  if (!strcmp(sub, "raw")) {
    unsigned long v[N + 2];
    int got = 0;
    int skip = 0;
    sscanf(line, "%*s %*s%n", &skip);   // past "sim raw"
    const char* q = line + skip;
    while (got < N + 2 && *q) {
      char* end;
      unsigned long x = strtoul(q, &end, 10);
      if (end == q) break;
      v[got++] = x;
      q = end;
    }
    if (got < N) {
      Serial.printf("sim raw <t%d>..<t%d> [n] [gap ns]: %d times\n", F + 1, F + N, N);
      return true;
    }
    unsigned long t[N], n = got > N ? v[N] : 20, gap = got > N + 1 ? v[N + 1] : 1000;
    for (int i = 0; i < N; i++) t[i] = v[i];
    if (!run.active) {
      for (int i = 0; i < N; i++) run.rawRiseNs[i] = t[i];
      run.rawGapNs = gap;
    }
    startCommon(M_RAW, n);
    return true;
  }
  if (run.active) { Serial.println("sim: settings can't change while it runs (`sim stop`)"); return true; }
  if (!strcmp(sub, "fault")) {
    int ch = 0;
    unsigned long us = 0;
    float m, off;
    if (!strcmp(word, "none")) fault = F_NONE;
    else if (!strcmp(word, "blast") && sscanf(line, "%*s %*s %*s %f %f", &m, &off) == 2 && m >= 1 &&
             m <= 100 && off >= 0 && off <= 10000) {
      fault = F_BLAST;
      blastM = m;
      blastOffMm = off;
    } else if (!strcmp(word, "double") && sscanf(line, "%*s %*s %*s %lu", &us) == 1 && us >= 100 &&
             us <= 100000) {
      fault = F_DOUBLE;
      doubleUs = us;
    } else if (sscanf(line, "%*s %*s %*s %d", &ch) == 1 && simChannelArg(ch) >= 0 &&
               (!strcmp(word, "late") || !strcmp(word, "early") || !strcmp(word, "missing") ||
                !strcmp(word, "honly"))) {
      fault = !strcmp(word, "late") ? F_LATE : !strcmp(word, "early") ? F_EARLY
            : !strcmp(word, "missing") ? F_MISSING : F_HONLY;
      faultCh = simChannelArg(ch);
    } else {
      Serial.printf("sim fault none | late|early|missing|honly <ch> (ch %d..%d) | "
                    "double <100..100000 us> | blast <1..100 m> <0..10000 mm>\n", F + 1, F + N);
      return true;
    }
  } else if (!strcmp(sub, "thr")) {
    int ch;
    float l, h;
    if (sscanf(line, "%*s %*s %d %f %f", &ch, &l, &h) != 3 || simChannelArg(ch) < 0 || !(l > 0) ||
        !(h > l) || h > 1000) {
      Serial.printf("sim thr <%d..%d> <L> <H> (0 < L < H; default 1 4)\n", F + 1, F + N);
      return true;
    }
    thrL[simChannelArg(ch)] = l;
    thrH[simChannelArg(ch)] = h;
  } else if (!strcmp(sub, "model")) {
    if (!strcmp(word, "linear")) sineModel = false;
    else if (!strcmp(word, "sine")) sineModel = true;
    else { Serial.println("sim model linear|sine"); return true; }
  } else if (!strcmp(sub, "amp")) {
    float v;
    if (sscanf(line, "%*s %*s %f", &v) != 1 || v < 1 || v > 1000) { Serial.println("sim amp <1..1000>"); return true; }
    simAmp = v;
  } else if (!strcmp(sub, "temp")) {
    float v;
    char word[8] = "";
    if (sscanf(line, "%*s %*s %7s", word) == 1 && !strcmp(word, "air")) {
      simTempC = NAN;
    } else if (sscanf(line, "%*s %*s %f", &v) != 1 || v < -20 || v > 60) {
      Serial.println("sim temp <-20..60 C>, or `sim temp air` for the firmware's air");
      return true;
    } else {
      simTempC = v;
    }
  } else if (!strcmp(sub, "offset")) {
    int ch;
    float v;
    if (sscanf(line, "%*s %*s %d %f", &ch, &v) != 2 || simChannelArg(ch) < 0 || fabsf(v) > 100000) {
      Serial.printf("sim offset <%d..%d> <ns>\n", F + 1, F + N);
      return true;
    }
    trueOffsetNs[simChannelArg(ch)] = v;
  } else if (!strcmp(sub, "mic")) {
    int ch;
    float x, y, h = NAN;
    int got = sscanf(line, "%*s %*s %d %f %f %f", &ch, &x, &y, &h);
    if (got < 3 || simChannelArg(ch) < 0 || fabsf(x) > 1000 || fabsf(y) > 1000 ||
        (got == 4 && !(h >= 0 && h <= 100))) {
      Serial.printf("sim mic <%d..%d> <x> <y> [h] (mm)\n", F + 1, F + N);
      return true;
    }
    micX[simChannelArg(ch)] = x;
    micY[simChannelArg(ch)] = y;
    if (got == 4) micH[simChannelArg(ch)] = h;
  } else {
    simHelp();
    return true;
  }
  printSettings();
  return true;
}
