// OpenSights - shot position and calibration mode (measurement step 3): see solver.h.
#include "solver.h"
#include <Preferences.h>
#include "airtemp.h"
#include "capture.h"
#include "config.h"
#include "simulate.h"
#include "tdoa.h"

static Preferences prefs;
static float micX[CHANNELS], micY[CHANNELS];

static const float MIC_MAX_MM = 1000.0f;

static void micKeys(char* kx, char* ky, int ch) {
  snprintf(kx, 8, "mx%d", ch + 1);
  snprintf(ky, 8, "my%d", ch + 1);
}

void solverBegin() {
  prefs.begin("measure");
  for (int c = 0; c < CHANNELS; c++) {
    char kx[8], ky[8];
    micKeys(kx, ky, c);
    micX[c] = prefs.getFloat(kx, micDefaultX(c));
    micY[c] = prefs.getFloat(ky, micDefaultY(c));
    if (!(fabsf(micX[c]) <= MIC_MAX_MM && fabsf(micY[c]) <= MIC_MAX_MM)) {
      micX[c] = micDefaultX(c);
      micY[c] = micDefaultY(c);
    }
  }
}

float solverMicX(int ch) { return ch >= 0 && ch < CHANNELS ? micX[ch] : 0; }
float solverMicY(int ch) { return ch >= 0 && ch < CHANNELS ? micY[ch] : 0; }

static uint8_t fittedChannels(const EventResult& r) {
  uint8_t m = 0;
  for (int c = 0; c < CHANNELS; c++)
    if (r.lines & (3 << (2 * c))) m |= 1 << c;
  return m;
}

void solverSolve(const EventResult& r, const Arrival& a, Solution& s) {
  s = {};
  AirTemp air = airTemp();
  s.airC = air.c;
  const uint8_t fitted = fittedChannels(r);
  float mx[TDOA_MAX_MICS], my[TDOA_MAX_MICS], t0[TDOA_MAX_MICS];
  int n = 0;
  for (int c = 0; c < CHANNELS; c++) {
    if (!(fitted & (1 << c)) || !(a.valid & (1 << c))) continue;
    mx[n] = micX[c];
    my[n] = micY[c];
    t0[n] = a.t0[c];
    n++;
    s.used |= 1 << c;
  }
  s.complete = s.used == fitted;
  if (n < 3) { s.why = "fewer than 3 channels with a t0"; return; }
  const float speed = soundSpeedMmPerNs(air.c);
  TdoaResult t = tdoaSolve(n, mx, my, t0, speed);
  s.iterations = t.iterations;
  if (!t.converged) { s.why = "the fit did not converge"; return; }
  s.solved = true;
  s.x = t.x;
  s.y = t.y;
  s.residualNs = t.rmsMm / speed;
}

// ---- Calibration mode ----

struct PointStat {
  uint32_t n;
  double   sx, sy, sxx, syy, worst;
  void add(double dx, double dy) {
    n++;
    sx += dx; sy += dy;
    sxx += dx * dx; syy += dy * dy;
    double e = sqrt(dx * dx + dy * dy);
    if (e > worst) worst = e;
  }
};

static bool      calOn = false;
static bool      pointSet = false;
static float     pointX, pointY;
static PointStat pointStat;

static void printPointStat() {
  if (!pointSet) return;
  const PointStat& p = pointStat;
  if (!p.n) {
    Serial.printf("cal point (%+.3f, %+.3f): no solved sparks yet\n", pointX, pointY);
    return;
  }
  double mx = p.sx / p.n, my = p.sy / p.n;
  double sdx = p.n > 1 ? sqrt(fmax(0, (p.sxx - p.n * mx * mx) / (p.n - 1))) : 0;
  double sdy = p.n > 1 ? sqrt(fmax(0, (p.syy - p.n * my * my) / (p.n - 1))) : 0;
  Serial.printf("cal point (%+.3f, %+.3f): %u sparks, mean error %+.3f %+.3f mm (%.3f mm off), "
                "spread (1 sd) %.3f %.3f mm, worst %.3f mm\n", pointX, pointY, (unsigned)p.n, mx,
                my, sqrt(mx * mx + my * my), sdx, sdy, p.worst);
}

// The firmware's calibration, for tools/calibrate.py's starting values.
static void printCalConfig() {
  char line[400];
  int n = snprintf(line, sizeof line, "cal config k=%.5f", arrivalK());
  for (int c = 0; c < CHANNELS; c++)
    n += snprintf(line + n, sizeof line - n, " CH%d=%.3f,%.3f,%.1f", c + 1, micX[c], micY[c],
                  arrivalOffsetNs(c));
  Serial.println(line);
}

// One line per event: the raw edges (not t0: the fit finds k and the offsets itself), where
// the spark was made and the air temperature. Times in ns from the event's first edge.
static void printCalLine(const EventResult& r, const Solution& s) {
  const int ref = eventFirstEdge(r);
  float tx, ty;
  bool truth = simTruth(&tx, &ty);
  if (!truth && pointSet) { tx = pointX; ty = pointY; truth = true; }
  char line[320];
  int n = snprintf(line, sizeof line, "cal evt=%u", (unsigned)r.number);
  if (truth) n += snprintf(line + n, sizeof line - n, " pt=%.3f,%.3f", tx, ty);
  else n += snprintf(line + n, sizeof line - n, " pt=-");
  n += snprintf(line + n, sizeof line - n, " air=%.2f", s.airC);
  for (int c = 0; c < CHANNELS; c++) {
    if (!(r.lines & (3 << (2 * c)))) continue;
    n += snprintf(line + n, sizeof line - n, " CH%d=", c + 1);
    for (int h = 0; h < 2; h++) {
      const LineResult& L = r.line[2 * c + h];
      if (L.rise >= 0) n += snprintf(line + n, sizeof line - n, "%d", (L.rise - ref) * (int)NS_PER_SAMPLE);
      else n += snprintf(line + n, sizeof line - n, "-");
      if (!h) n += snprintf(line + n, sizeof line - n, ",");
    }
  }
  Serial.println(line);
}

void solverPrint(const EventResult& r, const Arrival& a, const Solution& s) {
  char line[200];
  int n;
  if (!s.solved) {
    n = snprintf(line, sizeof line, "  pos -: %s", s.why);
  } else {
    n = snprintf(line, sizeof line, "  pos x %+.3f y %+.3f mm, residual %.0f ns rms (%d ch, %u iter)",
                 s.x, s.y, s.residualNs, __builtin_popcount(s.used), s.iterations);
    if (!s.complete) n += snprintf(line + n, sizeof line - n, ", not every channel: not a shot");
    if (calOn && pointSet && !simActive()) {
      double dx = s.x - pointX, dy = s.y - pointY;
      if (s.complete) pointStat.add(dx, dy);
      n += snprintf(line + n, sizeof line - n, "; vs point %+.3f %+.3f mm", dx, dy);
    }
  }
  Serial.println(line);
  if (calOn) printCalLine(r, s);
}

// ---- Commands ----

void solverHelp() {
  Serial.println("  mic [ch x y]         mic position of a channel, mm from the card centre, x\n"
                 "                       right, y up (stored); `mic` lists, `mic default`\n"
                 "  cal on|off           calibration mode: a `cal evt=` line per event for\n"
                 "                       tools/calibrate.py\n"
                 "  cal point <x> <y>    where the next sparks are made (mm); `cal` shows the\n"
                 "                       error statistics of the current point");
}

static void printMics() {
  const uint16_t mask = captureChannelMask();
  char out[400];
  int n = snprintf(out, sizeof out, "mics mm:");
  for (int c = 0; c < CHANNELS; c++)
    n += snprintf(out + n, sizeof out - n, "  CH%d%s (%.3f, %.3f)", c + 1,
                  (mask & (1 << c)) ? "" : "*", micX[c], micY[c]);
  Serial.println(out);
  // The event window must cover the largest distance between two fitted mics.
  float span = 0;
  for (int i = 0; i < CHANNELS; i++)
    for (int j = i + 1; j < CHANNELS; j++)
      if ((mask & (1 << i)) && (mask & (1 << j)))
        span = fmaxf(span, hypotf(micX[i] - micX[j], micY[i] - micY[j]));
  Serial.printf("  (* not fitted) largest distance between fitted mics %.1f mm, mic span setting "
                "%u mm%s\n", span, (unsigned)captureSpanMm(),
                span > captureSpanMm() ? ": TOO SHORT, set `span` to at least the distance" : "");
}

bool solverCommand(const char* cmd, const char* line) {
  if (!strcmp(cmd, "mic")) {
    int ch;
    float x, y;
    char word[12] = "";
    sscanf(line, "%*s %11s", word);
    if (!strcmp(word, "default")) {
      for (int c = 0; c < CHANNELS; c++) {
        char kx[8], ky[8];
        micKeys(kx, ky, c);
        micX[c] = micDefaultX(c);
        micY[c] = micDefaultY(c);
        if (prefs.isKey(kx)) prefs.remove(kx);
        if (prefs.isKey(ky)) prefs.remove(ky);
      }
    } else if (word[0]) {
      if (sscanf(line, "%*s %d %f %f", &ch, &x, &y) != 3 || ch < 1 || ch > CHANNELS ||
          !(fabsf(x) <= MIC_MAX_MM && fabsf(y) <= MIC_MAX_MM)) {
        Serial.printf("mic <1..%d> <x> <y> (mm, +-%.0f), `mic` to list, `mic default`\n", CHANNELS,
                      MIC_MAX_MM);
        return true;
      }
      char kx[8], ky[8];
      micKeys(kx, ky, ch - 1);
      micX[ch - 1] = x;
      micY[ch - 1] = y;
      prefs.putFloat(kx, x);
      prefs.putFloat(ky, y);
    }
    printMics();
    return true;
  }
  if (!strcmp(cmd, "cal")) {
    char word[12] = "";
    sscanf(line, "%*s %11s", word);
    if (!strcmp(word, "on")) {
      calOn = true;
      printCalConfig();
    } else if (!strcmp(word, "off")) {
      printPointStat();
      calOn = false;
      pointSet = false;
    } else if (!strcmp(word, "point")) {
      float x, y;
      if (sscanf(line, "%*s %*s %f %f", &x, &y) != 2 || !(fabsf(x) <= MIC_MAX_MM && fabsf(y) <= MIC_MAX_MM)) {
        Serial.println("cal point <x> <y> (mm)");
        return true;
      }
      printPointStat();   // the previous point's
      if (!calOn) { calOn = true; printCalConfig(); }
      pointSet = true;
      pointX = x;
      pointY = y;
      pointStat = {};
    } else if (word[0]) {
      Serial.println("cal on|off, cal point <x> <y>, cal");
      return true;
    }
    Serial.printf("calibration mode %s", calOn ? "on" : "off");
    if (calOn && pointSet) Serial.printf(", sparks at (%+.3f, %+.3f) mm", pointX, pointY);
    else if (calOn) Serial.print(", no point set (the simulator gives its own)");
    Serial.println();
    if (calOn && !word[0]) printPointStat();
    return true;
  }
  return false;
}
