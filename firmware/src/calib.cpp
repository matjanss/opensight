// OpenSights - the calibration: see calib.h.
#include "calib.h"
#include <Preferences.h>
#include "config.h"
#include "position.h"
#include "simulate.h"

static Preferences prefs;
static measure::Cfg cfg;
static uint32_t cfgIdNow = 0;

static const float MIC_MAX_MM = 1000.0f;
static const float H_MAX_MM = 100.0f;
static const float K_MAX = 2.0f;
static const float OFFSET_MAX_NS = 100000.0f;

// The D1/D2 layout (PLAN.md): CH1 bottom left, CH2 bottom right, CH3 top right slid 40 mm
// along the top edge, CH4 top left, CH5 bottom middle, CH6 right middle.
static const float DEFAULT_XY[6][2] = {
  {-105, -105}, {105, -105}, {65, 105}, {-105, 105}, {0, -105}, {105, 0},
};

// Stored per channel as one blob, so a channel's values change together.
struct Stored {
  uint8_t version;   // STORED_VERSION
  uint8_t mic;
  float   x, y, h, k, offsetNs, sigmaNs;
};
static const uint8_t STORED_VERSION = 1;

static void key(char* k, int ch) { snprintf(k, 8, "ch%d", ch + 1); }

static void save(int ch) {
  const measure::ChannelCfg& c = cfg.ch[ch];
  Stored s = {STORED_VERSION, c.mic, c.x, c.y, c.h, c.k, c.offsetNs, c.sigmaNs};
  char k[8];
  key(k, ch);
  prefs.putBytes(k, &s, sizeof s);
}

static void changed() {
  cfgIdNow = measure::cfgId(cfg);
  calibPrintCfg();
}

static void setDefaultMics() {
  for (int c = 0; c < CHANNELS; c++) {
    measure::ChannelCfg& ch = cfg.ch[c];
    ch.mic = c < 6;
    ch.x = c < 6 ? DEFAULT_XY[c][0] : 0;
    ch.y = c < 6 ? DEFAULT_XY[c][1] : 0;
    ch.h = MIC_H_DEFAULT_MM;
  }
}

static bool plausible(const Stored& s) {
  return s.version == STORED_VERSION && fabsf(s.x) <= MIC_MAX_MM && fabsf(s.y) <= MIC_MAX_MM &&
         s.h >= 0 && s.h <= H_MAX_MM && s.k >= 0 && s.k <= K_MAX &&
         fabsf(s.offsetNs) <= OFFSET_MAX_NS && s.sigmaNs > 0 && s.sigmaNs < 1e6f;
}

void calibBegin() {
  prefs.begin("cal");
  setDefaultMics();
  for (int c = 0; c < CHANNELS; c++) {
    cfg.ch[c].k = ARRIVAL_K_DEFAULT;
    cfg.ch[c].offsetNs = 0;
    cfg.ch[c].sigmaNs = SIGMA_DEFAULT_NS;
  }
  bool any = false;
  for (int c = 0; c < CHANNELS; c++) {
    char k[8];
    key(k, c);
    Stored s;
    if (prefs.isKey(k) && prefs.getBytes(k, &s, sizeof s) == sizeof s && plausible(s)) {
      cfg.ch[c] = {s.mic != 0, s.x, s.y, s.h, s.k, s.offsetNs, s.sigmaNs};
      any = true;
    }
  }
  if (!any) {
    // First boot with 3c: carry k and the offsets over from "measure" (step 2), once.
    Preferences old;
    old.begin("measure");
    bool moved = false;
    if (old.isKey("k")) {
      float k = old.getFloat("k", ARRIVAL_K_DEFAULT);
      if (k >= 0 && k <= K_MAX)
        for (int c = 0; c < CHANNELS; c++) cfg.ch[c].k = k;
      old.remove("k");
      moved = true;
    }
    for (int c = 0; c < CHANNELS; c++) {
      char k[8];
      snprintf(k, sizeof k, "off%d", c + 1);
      if (!old.isKey(k)) continue;
      float v = old.getFloat(k, 0);
      if (fabsf(v) <= OFFSET_MAX_NS) cfg.ch[c].offsetNs = v;
      old.remove(k);
      moved = true;
    }
    old.end();
    for (int c = 0; c < CHANNELS; c++) save(c);
    Serial.printf("cal: no calibration stored yet: the default mics (D1/D2 on CH1-CH6)%s\n",
                  moved ? ", k and the offsets carried over from step 2" : "");
  }
  changed();
}

const measure::Cfg& calibCfg() { return cfg; }
uint32_t calibId() { return cfgIdNow; }

void calibPrintCfg() {
  char line[640];
  measure::formatCfg(line, sizeof line, cfg);
  Serial.println(line);
}

static void printMics() {
  for (int c = 0; c < CHANNELS; c++) {
    const measure::ChannelCfg& ch = cfg.ch[c];
    if (ch.mic)
      Serial.printf("mic CH%d: (%.2f, %.2f) mm, h %.2f mm; k %.4f, offset %.1f ns, sigma %.0f ns\n",
                    c + 1, ch.x, ch.y, ch.h, ch.k, ch.offsetNs, ch.sigmaNs);
    else
      Serial.printf("mic CH%d: none; k %.4f, offset %.1f ns\n", c + 1, ch.k, ch.offsetNs);
  }
}

void calibHelp() {
  Serial.println("  mic                  the mics (x, y, h per channel) and the blast check\n"
                 "  mic <ch> <x> <y> [h] a channel's mic, mm from the frame centre (stored)\n"
                 "  mic h [ch] <mm>      the mic height behind the paper, every mic or one\n"
                 "  mic default | sim    the D1/D2 layout on CH1-CH6 | the simulator's on CH5-CH8\n"
                 "  mic clear <ch>       no mic on that channel\n"
                 "  k [ch] [value]       arrival time t0 = t1 - k (t2 - t1) - offset: k, every\n"
                 "                       channel or one (stored; `k` lists them)\n"
                 "  offset [ch] [ns]     a channel's own delay, taken off its t0 (stored);\n"
                 "                       `offset` lists them, `offset clear` zeroes all");
}

// The number of space-separated words in line.
static int words(const char* line) {
  int n = 0;
  for (const char* p = line; *p;) {
    while (*p == ' ') p++;
    if (!*p) break;
    n++;
    while (*p && *p != ' ') p++;
  }
  return n;
}

static bool channelArg(int ch) {
  if (ch >= 1 && ch <= CHANNELS) return true;
  Serial.printf("channel 1..%d\n", CHANNELS);
  return false;
}

static bool micCommand(const char* line) {
  char sub[12] = "";
  sscanf(line, "%*s %11s", sub);
  if (sub[0] && simActive()) {
    Serial.println("mic: wait for the simulator to finish (`sim stop`)");
    return true;
  }
  if (!sub[0]) {
    // just list
  } else if (!strcmp(sub, "default")) {
    setDefaultMics();
    for (int c = 0; c < CHANNELS; c++) save(c);
  } else if (!strcmp(sub, "sim")) {
    for (int c = 0; c < CHANNELS; c++) {
      float x, y, h;
      measure::ChannelCfg& ch = cfg.ch[c];
      ch.mic = simMic(c, &x, &y, &h);
      if (ch.mic) { ch.x = x; ch.y = y; ch.h = h; }
      save(c);
    }
  } else if (!strcmp(sub, "clear")) {
    int ch;
    if (sscanf(line, "%*s %*s %d", &ch) != 1) { Serial.println("mic clear <ch>"); return true; }
    if (!channelArg(ch)) return true;
    cfg.ch[ch - 1].mic = false;
    save(ch - 1);
  } else if (!strcmp(sub, "h")) {
    int ch = 0;
    float v = NAN;
    const int w = words(line);   // mic h <mm> | mic h <ch> <mm>
    bool ok = w == 3 ? sscanf(line, "%*s %*s %f", &v) == 1
            : w == 4 && sscanf(line, "%*s %*s %d %f", &ch, &v) == 2;
    if (!ok || !(v >= 0 && v <= H_MAX_MM)) {
      Serial.printf("mic h [ch] <0..%.0f mm>\n", H_MAX_MM);
      return true;
    }
    if (ch && !channelArg(ch)) return true;
    for (int c = 0; c < CHANNELS; c++)
      if (!ch || c == ch - 1) {
        cfg.ch[c].h = v;
        save(c);
      }
  } else {
    int ch;
    float x, y, h = NAN;
    int got = sscanf(line, "%*s %d %f %f %f", &ch, &x, &y, &h);
    if (got < 3 || !(fabsf(x) <= MIC_MAX_MM && fabsf(y) <= MIC_MAX_MM) ||
        (got == 4 && !(h >= 0 && h <= H_MAX_MM))) {
      Serial.println("mic <ch> <x> <y> [h] (mm), mic h [ch] <mm>, mic default | sim, mic clear <ch>");
      return true;
    }
    if (!channelArg(ch)) return true;
    measure::ChannelCfg& c = cfg.ch[ch - 1];
    if (!c.mic && got < 4) h = MIC_H_DEFAULT_MM;
    c.mic = true;
    c.x = x;
    c.y = y;
    if (!isnan(h)) c.h = h;
    save(ch - 1);
  }
  printMics();
  positionBlastCheck();
  changed();
  return true;
}

static bool kCommand(const char* line) {
  int ch = 0;
  float v = NAN;
  const int w = words(line);   // k | k <value> | k <ch> <value>
  bool got = w > 1;
  if (w == 2 && sscanf(line, "%*s %f", &v) != 1) v = NAN;
  if (w == 3 && (sscanf(line, "%*s %d %f", &ch, &v) != 2 || !channelArg(ch))) return true;
  if (got) {
    if (w > 3 || !(v >= 0 && v <= K_MAX)) { Serial.printf("k [ch] <0..%.1f>\n", K_MAX); return true; }
    for (int c = 0; c < CHANNELS; c++)
      if (!ch || c == ch - 1) {
        cfg.ch[c].k = v;
        save(c);
      }
  }
  char out[200];
  int n = snprintf(out, sizeof out, "k:");
  for (int c = 0; c < CHANNELS; c++)
    n += snprintf(out + n, sizeof out - n, "  CH%d %.4f", c + 1, cfg.ch[c].k);
  Serial.println(out);
  if (got) changed();
  return true;
}

static bool offsetCommand(const char* line) {
  int ch;
  float v;
  char word[8] = "";
  bool change = false;
  if (sscanf(line, "%*s %7s", word) == 1 && !strcmp(word, "clear")) {
    for (int c = 0; c < CHANNELS; c++) {
      cfg.ch[c].offsetNs = 0;
      save(c);
    }
    change = true;
  } else {
    int got = sscanf(line, "%*s %d %f", &ch, &v);
    if (got == 2) {
      if (!channelArg(ch)) return true;
      if (!(fabsf(v) <= OFFSET_MAX_NS)) {
        Serial.printf("offset <ch> <-%.0f..%.0f ns>\n", OFFSET_MAX_NS, OFFSET_MAX_NS);
        return true;
      }
      cfg.ch[ch - 1].offsetNs = v;
      save(ch - 1);
      change = true;
    } else if (got == 1) {
      Serial.println("offset <ch> <ns>, or `offset` to list, `offset clear`");
      return true;
    }
  }
  char out[200];
  int n = snprintf(out, sizeof out, "offsets ns:");
  for (int c = 0; c < CHANNELS; c++)
    n += snprintf(out + n, sizeof out - n, "  CH%d %.1f", c + 1, cfg.ch[c].offsetNs);
  Serial.println(out);
  if (change) changed();
  return true;
}

bool calibCommand(const char* cmd, const char* line) {
  if (!strcmp(cmd, "mic")) return micCommand(line);
  if (!strcmp(cmd, "k")) return kCommand(line);
  if (!strcmp(cmd, "offset")) return offsetCommand(line);
  return false;
}
