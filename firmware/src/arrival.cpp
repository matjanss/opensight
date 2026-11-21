// OpenSights - arrival times (measurement step 2): see arrival.h.
#include "arrival.h"
#include "airtemp.h"
#include "calib.h"
#include "model.h"

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
    const measure::ChannelCfg& ch = calibCfg().ch[c];
    a.t0[c] = measure::arrivalT0(t1, t2, ch.k, ch.offsetNs);
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
