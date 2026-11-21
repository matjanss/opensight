// OpenSights - arrival times (measurement step 2): see arrival.h.
#include "arrival.h"
#include <Preferences.h>
#include "airtemp.h"
#include "model.h"

static Preferences prefs;
static float kFactor = ARRIVAL_K_DEFAULT;
static float offsetNs[CHANNELS] = {};

static const float K_MAX = 2.0f;
static const float OFFSET_MAX_NS = 100000.0f;

static void offsetKey(char* key, int ch) { snprintf(key, 8, "off%d", ch + 1); }

void arrivalBegin() {
  prefs.begin("measure");
  kFactor = prefs.getFloat("k", ARRIVAL_K_DEFAULT);
  if (!(kFactor >= 0 && kFactor <= K_MAX)) kFactor = ARRIVAL_K_DEFAULT;
  for (int c = 0; c < CHANNELS; c++) {
    char key[8];
    offsetKey(key, c);
    offsetNs[c] = prefs.getFloat(key, 0.0f);
    if (!(fabsf(offsetNs[c]) <= OFFSET_MAX_NS)) offsetNs[c] = 0;
  }
}

float arrivalK() { return kFactor; }
float arrivalOffsetNs(int ch) { return ch >= 0 && ch < CHANNELS ? offsetNs[ch] : 0; }

void arrivalCompute(const EventResult& r, Arrival& a) {
  const int ref = eventFirstEdge(r);
  a.valid = 0;
  for (int c = 0; c < CHANNELS; c++) {
    a.t0[c] = 0;
    a.gap[c] = 0;
    if (!(r.lines & (3 << (2 * c)))) continue;
    const LineResult& L = r.line[2 * c];
    const LineResult& H = r.line[2 * c + 1];
    if (L.rise < 0 || H.rise < 0) continue;
    float t1 = (float)(L.rise - ref) * NS_PER_SAMPLE;
    float t2 = (float)(H.rise - ref) * NS_PER_SAMPLE;
    a.t0[c] = measure::arrivalT0(t1, t2, kFactor, offsetNs[c]);
    a.gap[c] = (int32_t)(H.rise - L.rise) * (int32_t)NS_PER_SAMPLE;
    a.valid |= 1 << c;
  }
}

void arrivalPrint(const EventResult& r, const Arrival& a) {
  char line[256];
  int n = snprintf(line, sizeof line, "  t0 ns (t2-t1):");
  int first = -1;
  for (int c = 0; c < CHANNELS; c++) {
    if (!(r.lines & (3 << (2 * c)))) continue;
    if (a.valid & (1 << c)) {
      n += snprintf(line + n, sizeof line - n, "  CH%d %.1f (%ld)", c + 1, a.t0[c], (long)a.gap[c]);
      if (first < 0) first = c;
    } else {
      n += snprintf(line + n, sizeof line - n, "  CH%d -", c + 1);
    }
  }
  Serial.println(line);

  n = 0;
  if (first >= 0 && (a.valid & ~(1 << first))) {
    n = snprintf(line, sizeof line, "  t0 minus CH%d:", first + 1);
    for (int c = first + 1; c < CHANNELS; c++)
      if (a.valid & (1 << c))
        n += snprintf(line + n, sizeof line - n, "  CH%d %+.1f", c + 1, a.t0[c] - a.t0[first]);
    n += snprintf(line + n, sizeof line - n, "   ");
  } else {
    n = snprintf(line, sizeof line, "  ");
  }
  airFormat(line + n, sizeof line - n, air());
  Serial.println(line);
}

void arrivalHelp() {
  Serial.printf("  k [value]            arrival time t0 = t1 - k (t2 - t1) - offset: k (stored;\n"
                "                       now %.4f, default %.4f)\n"
                "  offset [ch] [ns]     a channel's own delay, taken off its t0 (stored);\n"
                "                       `offset` alone lists them, `offset clear` zeroes all\n",
                kFactor, ARRIVAL_K_DEFAULT);
}

bool arrivalCommand(const char* cmd, const char* line) {
  if (!strcmp(cmd, "k")) {
    float v;
    if (sscanf(line, "%*s %f", &v) == 1) {
      if (!(v >= 0 && v <= K_MAX)) { Serial.printf("k 0..%.1f\n", K_MAX); return true; }
      kFactor = v;
      prefs.putFloat("k", kFactor);
    }
    Serial.printf("k %.4f (VH = %.2f VL for a signal rising in a straight line)\n", kFactor,
                  kFactor > 0 ? 1 + 1 / kFactor : 0.0f);
    return true;
  }
  if (!strcmp(cmd, "offset")) {
    int ch;
    float v;
    char word[8] = "";
    if (sscanf(line, "%*s %7s", word) == 1 && !strcmp(word, "clear")) {
      for (int c = 0; c < CHANNELS; c++) {
        char key[8];
        offsetKey(key, c);
        offsetNs[c] = 0;
        if (prefs.isKey(key)) prefs.remove(key);   // remove() logs an error for a missing key
      }
    } else {
      int got = sscanf(line, "%*s %d %f", &ch, &v);
      if (got == 2) {
        if (ch < 1 || ch > CHANNELS || !(fabsf(v) <= OFFSET_MAX_NS)) {
          Serial.printf("offset <1..%d> <-%.0f..%.0f ns>\n", CHANNELS, OFFSET_MAX_NS, OFFSET_MAX_NS);
          return true;
        }
        char key[8];
        offsetKey(key, ch - 1);
        offsetNs[ch - 1] = v;
        prefs.putFloat(key, v);
      } else if (got == 1) {
        Serial.println("offset <ch> <ns>, or `offset` to list, `offset clear`");
        return true;
      }
    }
    char out[160];
    int n = snprintf(out, sizeof out, "offsets ns:");
    for (int c = 0; c < CHANNELS; c++)
      n += snprintf(out + n, sizeof out - n, "  CH%d %.1f", c + 1, offsetNs[c]);
    Serial.println(out);
    return true;
  }
  return false;
}
