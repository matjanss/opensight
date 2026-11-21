// OpenSights - sensor simulator (measurement step 2).
//
// A simulated shot: the shot position, the mic positions and a speed of sound give the true
// arrival at each mic; a trip model gives when OUT_L and OUT_H go high after it; the pulse
// generator (pulsegen.cpp) makes those edges in hardware, 25 ns steps, on CH5-CH8's own
// pins, and the capture reads them back like a real event. The simulator knows the edges it
// sent, so it checks:
//   - the capture and the arrival times: t0 from the captured edges against t0 from the
//     edges sent, as differences to the first channel. Only the 100 ns sampling is
//     between them, so the error stays within (1 + 2k) samples (plus a tick for the second
//     MCPWM unit), about 56 ns rms;
//   - the t0 model: t0 from the edges sent against the true arrival. Zero for the linear
//     trip model with k = 1/3 (up to the 25 ns steps); the sine model shows what the
//     straight-line formula leaves on a curved rise;
//   - the solver (step 3): the solved position against the true one. Its own verdict, as
//     it also depends on the firmware's calibration matching the simulated sensors (mic
//     positions, offsets, k, air temperature): `sim mic`, `sim offset` and `sim temp` make
//     them differ, for the calibration test.
// `sim raw` sends fixed edges instead, to check the generator itself: equal times on all
// channels show any offset between the two MCPWM units.
//
// Trip model: the pulse rises like A sin(pi t / 2T) (or its straight start, A pi t / 2T),
// T = 12 us; OUT_L trips at 1, OUT_H at 4 (k = 1/3); A = amp * 100 mm / distance.
#include "simulate.h"
#include "airtemp.h"
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
static const double   VH_OVER_VL = 4.0;        // k = 1 / (4 - 1)
static const double   LATE_NS    = 24000;      // half-cycle-late trip
static const int      MAX_COUNT  = 2000;
static const double   POS_LIMIT_MM = 0.1;  // sampling alone: ~0.015 mm rms, ~0.045 worst

// ---- Settings (RAM; `sim` shows them) ----

static float micX[N], micY[N];
static float trueOffsetNs[N] = {};
static float simTempC = NAN;      // NAN: the firmware's air temperature
static float simAmp = 40.0f;      // pulse amplitude at 100 mm, in units of the OUT_L threshold
static bool  sineModel = false;
enum Fault : uint8_t { F_NONE, F_LATE, F_MISSING, F_HONLY, F_DOUBLE };
static Fault    fault = F_NONE;
static int      faultCh = 0;        // 0..N-1
static uint32_t doubleUs = 2500;

static bool micsSet = false;
static void defaultMics() {
  for (int i = 0; i < N; i++) { micX[i] = micDefaultX(F + i); micY[i] = micDefaultY(F + i); }
  micsSet = true;
}

static float simTemp() { return isnan(simTempC) ? airTemp().c : simTempC; }

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
  Stat     raw[N][2];              // M_RAW: each line vs CH5 L
  uint32_t noT0[N], extraT0[N], rawMissing[N][2];
  uint32_t second[5];              // double: measured, MISSED, ignored (dead time), lost, partial
  uint32_t posN, posUnsolved;      // solved positions; shots with every t0 but no position
  double   posSx, posSy, posSq, posWorst;
} run;

bool simActive() { return run.active; }

bool simTruth(float* x, float* y) {
  if (!run.active || run.mode == M_RAW || !run.sent) return false;
  *x = run.shot.x;
  *y = run.shot.y;
  return true;
}

static uint16_t simChannelMask() { return ((1 << N) - 1) << F; }
static uint16_t simLineMask() {
  uint16_t m = 0;
  for (int i = 0; i < N; i++) m |= 3 << (2 * (F + i));
  return m;
}

static double speedMmPerNs(float tempC) { return (331.3 + 0.606 * tempC) * 1e-6; }

static uint32_t maxRiseTicks() { return run.periodTicks - HOLD_TICKS; }

// The edges of a shot at (x, y). False if they don't fit in a run (the window).
static bool makeShot(float x, float y, Shot& s) {
  const double c = speedMmPerNs(simTemp());
  const double scale = 2 * RISE_NS / M_PI;
  double t[N][2];
  double first = 1e18;
  s.x = x;
  s.y = y;
  for (int i = 0; i < N; i++) {
    double d = hypot(x - micX[i], y - micY[i]);
    if (d < 1) d = 1;
    s.geo[i] = d / c;
    double a = simAmp * 100.0 / d;
    double tl, th;
    bool lOn = true, hOn = true;
    if (sineModel) {
      lOn = a > 1;
      hOn = a > VH_OVER_VL;
      tl = lOn ? scale * asin(1 / a) : 0;
      th = hOn ? scale * asin(VH_OVER_VL / a) : 0;
    } else {
      tl = scale / a;
      th = scale * VH_OVER_VL / a;
    }
    double late = run.fault == F_LATE && run.faultCh == i ? LATE_NS : 0;
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
  double k = arrivalK();
  run.limitNs = (1 + 2 * k) * NS_PER_SAMPLE + TICK_NS + 5;

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
  // Four lines per shot (~450 bytes): 100 ms apart keeps well inside 115200 baud.
  run.spacingMs = dead + 20 > 100 ? dead + 20 : 100;
  run.nextAt = millis() + 100;
  run.missedAtStart = captureMissedCount();
  run.closed = true;

  char pinText[64];
  int n = 0;
  for (int i = 0; i < N; i++)
    n += snprintf(pinText + n, sizeof pinText - n, "%s%d/%d", i ? ", " : "", pins[i][0], pins[i][1]);
  if (rawMode) {
    Serial.printf("sim: %u raw runs on CH%d-CH%d (GPIO %s), L at", (unsigned)count, F + 1, F + N, pinText);
    for (int i = 0; i < N; i++) Serial.printf(" %u", (unsigned)run.rawRiseNs[i]);
    Serial.printf(" ns, H %u ns after L, %.0f ns ticks, every %u ms\n", (unsigned)run.rawGapNs,
                  TICK_NS, (unsigned)run.spacingMs);
    return;
  }
  const char* faultText[] = {"none", "late", "missing", "honly", "double"};
  Serial.printf("sim: %u shots %s on CH%d-CH%d (GPIO %s); %s model, amp %.0f, air %.1f °C%s "
                "(%.1f m/s); firmware k %.4f; fault %s", (unsigned)count,
                m == M_RANDOM ? "at random points on the card" : "at one point", F + 1, F + N,
                pinText, sineModel ? "sine" : "linear", simAmp, simTemp(),
                isnan(simTempC) ? " (the firmware's)" : "", speedMmPerNs(simTemp()) * 1e6, k,
                faultText[run.fault]);
  if (run.fault == F_DOUBLE) Serial.printf(" (second shot %u us later)", (unsigned)run.doubleWaitUs);
  else if (run.fault != F_NONE) Serial.printf(" on CH%d", F + 1 + run.faultCh);
  Serial.printf("; every %u ms\n", (unsigned)run.spacingMs);
}

// ---- Recording ----

static void recordRaw(const EventResult& r) {
  const Shot& s = run.shot;
  const int refLine = 2 * F;
  char line[200];
  int n = snprintf(line, sizeof line, "  sim raw, ns vs set (from CH%d L):", F + 1);
  if (r.line[refLine].rise < 0) {
    run.rawMissing[0][0]++;
    Serial.println("  sim raw: no CH5 L edge");
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

static void recordShot(const EventResult& r, const Arrival& a, const Solution& sol) {
  const Shot& s = run.shot;
  const double k = arrivalK();
  double expT0[N];
  bool expOn[N], got[N];
  int ref = -1;
  for (int i = 0; i < N; i++) {
    expOn[i] = s.on[i][0] && s.on[i][1];
    expT0[i] = expOn[i] ? s.rise[i][0] * TICK_NS - k * ((double)s.rise[i][1] - s.rise[i][0]) * TICK_NS -
                              arrivalOffsetNs(F + i)
                        : 0;
    got[i] = a.valid & (1 << (F + i));
    if (expOn[i] && !got[i]) run.noT0[i]++;
    if (!expOn[i] && got[i]) run.extraT0[i]++;
    if (ref < 0 && expOn[i] && got[i]) ref = i;
  }
  char line[280];
  int n = snprintf(line, sizeof line, "  sim (%.1f, %.1f) mm, t0 error ns vs sent/vs true:", s.x, s.y);
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
  if (sol.solved) {
    double dx = sol.x - s.x, dy = sol.y - s.y, e = sqrt(dx * dx + dy * dy);
    run.posN++;
    run.posSx += dx;
    run.posSy += dy;
    run.posSq += e * e;
    if (e > run.posWorst) run.posWorst = e;
    n += snprintf(line + n, sizeof line - n, "; pos error %+.3f %+.3f mm", dx, dy);
  } else if (sol.complete) {
    run.posUnsolved++;
    n += snprintf(line + n, sizeof line - n, "; NO POSITION");
  }
  Serial.println(line);
}

void simRecord(const EventResult& r, const Arrival& a, const Solution& s) {
  if (!run.active) return;
  if (run.pending1) run.pending1 = false;
  else if (run.pending2) {
    // The second shot's event. Without a t0 on any channel it isn't a measurement of the
    // shot but a window opened in its middle (its start was missed).
    run.pending2 = false;
    run.second2Event = true;
    run.second2Good = (a.valid & (((1 << N) - 1) << F)) != 0;
    run.seen++;
    if (run.second2Good) recordShot(r, a, s);
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
  else recordShot(r, a, s);
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

// Where the firmware's calibration differs from the simulated sensors (empty if nowhere).
static void calibrationDiffers(char* out, size_t size) {
  int n = 0;
  out[0] = 0;
  for (int i = 0; i < N; i++) {
    if (fabsf(micX[i] - solverMicX(F + i)) > 0.0005f || fabsf(micY[i] - solverMicY(F + i)) > 0.0005f)
      n += snprintf(out + n, size - n, "%sCH%d mic", n ? ", " : "", F + 1 + i);
    if (fabsf(trueOffsetNs[i] - arrivalOffsetNs(F + i)) > 0.05f)
      n += snprintf(out + n, size - n, "%sCH%d offset", n ? ", " : "", F + 1 + i);
  }
  if (fabs(arrivalK() - 1 / (VH_OVER_VL - 1)) > 1e-4)
    n += snprintf(out + n, size - n, "%sk", n ? ", " : "");
  if (fabsf(simTemp() - airTemp().c) > 0.005f)
    n += snprintf(out + n, size - n, "%sair temperature", n ? ", " : "");
}

// The solver's verdict: positions against the true ones.
static void finishPosition() {
  if (run.mode == M_RAW) return;
  if (!run.posN && !run.posUnsolved) {
    Serial.println("sim: position: no shot had every t0, nothing solved");
    return;
  }
  double n = run.posN ? run.posN : 1;
  Serial.printf("sim: position vs true: %u solved, mean error %+.4f %+.4f mm, rms %.4f mm, worst "
                "%.4f mm (limit %.2f), %u with every t0 but no position\n", (unsigned)run.posN,
                run.posSx / n, run.posSy / n, sqrt(run.posSq / n), run.posWorst, POS_LIMIT_MM,
                (unsigned)run.posUnsolved);
  char differs[160];
  calibrationDiffers(differs, sizeof differs);
  if (run.fault == F_LATE || run.fault == F_MISSING || run.fault == F_HONLY) {
    Serial.println("sim: position: not judged with a fault on a channel (step 4 must reject those)");
    return;
  }
  bool pass = run.posUnsolved == 0 && run.posWorst <= POS_LIMIT_MM;
  Serial.printf("sim: position %s", pass ? "PASS" : "FAIL");
  if (differs[0])
    Serial.printf(" (the firmware's calibration differs from the simulated sensors: %s)", differs);
  Serial.println();
}

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
    Serial.printf("sim: CH%d-CH%d are on MCPWM unit 0, CH%d on unit 1: a mean on CH%d that the "
                  "others don't have is an offset between the units\n", F + 1, F + 3, F + 4, F + 4);
    Serial.printf("sim: %s\n", pass ? "PASS" : "FAIL");
    return;
  }

  // The captured t0 difference of two channels is off by the rounding of four edges to the
  // 100 ns samples: rms S * sqrt(2 ((1 + k)^2 + k^2) / 12).
  const double k = arrivalK();
  const double S = NS_PER_SAMPLE;
  const double rmsExp = S * sqrt(2 * ((1 + k) * (1 + k) + k * k) / 12);
  for (int i = 0; i < N; i++) {
    const Stat& st = run.err[i];
    const Stat& md = run.model[i];
    bool faulty = run.fault != F_NONE && run.fault != F_DOUBLE && run.faultCh == i;
    if (st.n) {
      double mean = st.sum / st.n, rms = sqrt(st.sumSq / st.n);
      bool meanOff = fabs(mean) > 3 * rmsExp / sqrt((double)st.n) + 2;
      Serial.printf("sim: CH%d t0 vs sent: %u shots, mean %+.1f ns, rms %.1f (rounding alone "
                    "about %.0f), worst %.0f (limit %.0f)%s\n", F + 1 + i, (unsigned)st.n, mean,
                    rms, rmsExp, st.maxAbs, run.limitNs,
                    meanOff ? "; mean more than 3x the rounding's: a fixed offset" : "");
      Serial.printf("sim: CH%d t0 vs true arrival: mean %+.1f ns, worst %.1f ns%s\n", F + 1 + i,
                    md.sum / md.n, md.maxAbs,
                    faulty && run.fault == F_LATE ? " (the late trip: step 4 must catch it)" : "");
      if (st.outside) pass = false;
    }
    if (run.noT0[i])
      Serial.printf("sim: CH%d: no t0 in %u shots that had one\n", F + 1 + i, (unsigned)run.noT0[i]);
    if (run.extraT0[i])
      Serial.printf("sim: CH%d: a t0 in %u shots without one\n", F + 1 + i, (unsigned)run.extraT0[i]);
    if (run.noT0[i] || run.extraT0[i]) pass = false;
    if (faulty && (run.fault == F_MISSING || run.fault == F_HONLY))
      Serial.printf("sim: CH%d had no t0, as the fault set it (step 4 must reject such shots)\n",
                    F + 1 + i);
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
  Serial.printf("sim: %s\n", pass ? "PASS" : "FAIL");
  finishPosition();
}

// ---- Commands ----

void simHelp() {
  Serial.println("  sim at <x> <y> [n]   n shots at (x, y) mm from the card centre, on CH5-CH8\n"
                 "                       (no sensor boards there!), default 20\n"
                 "  sim random [n]       n shots at random points on the card, default 100\n"
                 "  sim raw <a> <b> <c> <d> [n] [gap]  L of CH5-CH8 at a..d ns, H gap ns later\n"
                 "                       (default 1000), n runs (default 20)\n"
                 "  sim fault none | late <ch> | missing <ch> | honly <ch> | double <us>\n"
                 "  sim model linear|sine, sim amp <n>, sim temp <C>|auto, sim offset <ch> <ns>,\n"
                 "  sim mic <ch> <x> <y>  the simulated sensors; `sim` shows them; sim stop");
}

static void printSettings() {
  if (!micsSet) defaultMics();
  const char* faultText[] = {"none", "late", "missing", "honly", "double"};
  Serial.printf("sim: %s model, amp %.0f, air %.1f °C%s, fault %s", sineModel ? "sine" : "linear",
                simAmp, simTemp(), isnan(simTempC) ? " (the firmware's)" : "", faultText[fault]);
  if (fault == F_DOUBLE) Serial.printf(" %u us", (unsigned)doubleUs);
  else if (fault != F_NONE) Serial.printf(" CH%d", F + 1 + faultCh);
  Serial.println();
  for (int i = 0; i < N; i++)
    Serial.printf("sim: CH%d mic at (%.1f, %.1f) mm, own delay %.0f ns\n", F + 1 + i, micX[i],
                  micY[i], trueOffsetNs[i]);
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
    unsigned long t[N] = {}, n = 20, gap = 1000;
    int got = sscanf(line, "%*s %*s %lu %lu %lu %lu %lu %lu", &t[0], &t[1], &t[2], &t[3], &n, &gap);
    if (got < N) { Serial.println("sim raw <a> <b> <c> <d> [n] [gap ns]"); return true; }
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
    if (!strcmp(word, "none")) fault = F_NONE;
    else if (!strcmp(word, "double") && sscanf(line, "%*s %*s %*s %lu", &us) == 1 && us >= 100 &&
             us <= 100000) {
      fault = F_DOUBLE;
      doubleUs = us;
    } else if (sscanf(line, "%*s %*s %*s %d", &ch) == 1 && simChannelArg(ch) >= 0 &&
               (!strcmp(word, "late") || !strcmp(word, "missing") || !strcmp(word, "honly"))) {
      fault = !strcmp(word, "late") ? F_LATE : !strcmp(word, "missing") ? F_MISSING : F_HONLY;
      faultCh = simChannelArg(ch);
    } else {
      Serial.printf("sim fault none | late <ch> | missing <ch> | honly <ch> (ch %d..%d) | "
                    "double <100..100000 us>\n", F + 1, F + N);
      return true;
    }
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
    if (!strcmp(word, "auto")) simTempC = NAN;
    else if (sscanf(line, "%*s %*s %f", &v) != 1 || v < -20 || v > 60) {
      Serial.println("sim temp <-20..60 C> | auto (the firmware's air temperature)");
      return true;
    } else simTempC = v;
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
    float x, y;
    if (sscanf(line, "%*s %*s %d %f %f", &ch, &x, &y) != 3 || simChannelArg(ch) < 0 ||
        fabsf(x) > 1000 || fabsf(y) > 1000) {
      Serial.printf("sim mic <%d..%d> <x> <y> (mm)\n", F + 1, F + N);
      return true;
    }
    micX[simChannelArg(ch)] = x;
    micY[simChannelArg(ch)] = y;
  } else {
    simHelp();
    return true;
  }
  printSettings();
  return true;
}
