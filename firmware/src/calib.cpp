// OpenSights - the calibration: see calib.h.
#include "calib.h"
#include <Preferences.h>
#include "config.h"
#include "curtain.h"
#include "position.h"
#include "simulate.h"

static Preferences prefs;
static measure::Cfg cfg;
static uint32_t cfgIdNow = 0;

static const float MIC_MAX_MM = 1000.0f;
static const float H_MAX_MM = 100.0f;
static const float K_MAX = 2.0f;
static const float OFFSET_MAX_NS = 100000.0f;

static const float RES_LIMIT_MAX_NS = 100000.0f;

// Stored per channel as one blob, so a channel's values change together.
struct Stored {
  uint8_t version;   // STORED_VERSION
  uint8_t mic;
  float   x, y, h, k, offsetNs, sigmaNs;
};
static const uint8_t STORED_VERSION = 1;

static void key(char* k, int ch) { snprintf(k, 8, "ch%d", ch + 1); }

// The calibration record (PLAN D5): which values tools/calibrate.py fitted, when, from
// what. Any change to a calibrated value after it marks it stale, until the next `cal id`.
struct CalRecord {
  uint8_t  version;   // CALREC_VERSION
  uint8_t  stale;
  uint16_t points, sparks;
  uint32_t id;        // measure::calibrationId of the values it fitted
  uint32_t date;      // yyyymmdd, from the PC (the target has no clock)
  float    resNs;     // the fit's rms residual
};
static const uint8_t CALREC_VERSION = 1;
static CalRecord rec = {};
static bool haveRec = false;
static bool started = false;   // past calibBegin: a save is a change
static bool calMode = false;
static bool havePt = false;
static float ptX = 0, ptY = 0;

static void saveRec() { prefs.putBytes("calrec", &rec, sizeof rec); }

static void markStale() {
  if (!started || !haveRec || rec.stale) return;
  rec.stale = 1;
  saveRec();
  Serial.println("cal: the calibration record is stale now (a calibrated value changed)");
}

static void save(int ch) {
  markStale();
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
    ch.x = c < 6 ? MIC_LAYOUT_D2[c][0] : 0;
    ch.y = c < 6 ? MIC_LAYOUT_D2[c][1] : 0;
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
  // The solver's policy: in "cal" from 3d (`reslimit` was in "measure" in 3c).
  cfg.resLimitNs = measure::RES_LIMIT_DEFAULT_NS;
  cfg.dropOne = prefs.getBool("dropone", true);
  if (prefs.isKey("reslimit")) {
    cfg.resLimitNs = prefs.getFloat("reslimit", measure::RES_LIMIT_DEFAULT_NS);
  } else {
    Preferences old;
    old.begin("measure");
    if (old.isKey("reslimit")) {
      cfg.resLimitNs = old.getFloat("reslimit", measure::RES_LIMIT_DEFAULT_NS);
      old.remove("reslimit");
      prefs.putFloat("reslimit", cfg.resLimitNs);
    }
    old.end();
  }
  if (!(cfg.resLimitNs > 0 && cfg.resLimitNs <= RES_LIMIT_MAX_NS))
    cfg.resLimitNs = measure::RES_LIMIT_DEFAULT_NS;
  // Validity (step 4a): 0 (or -1 for lost) is off.
  cfg.maxLost = prefs.getChar("lost", measure::MAX_LOST_DEFAULT);
  cfg.areaMm = prefs.getFloat("area", measure::AREA_DEFAULT_MM);
  cfg.togetherNs = prefs.getFloat("together", measure::TOGETHER_DEFAULT_NS);
  cfg.pulseMaxNs = prefs.getFloat("pulsemax", measure::PULSE_MAX_DEFAULT_NS);
  // The curtain's gate (step 4b): off until a curtain is fitted.
  if (prefs.isKey("irhi")) {
    cfg.curtainLoNs = prefs.getFloat("irlo", 0);
    cfg.curtainHiNs = prefs.getFloat("irhi", 0);
  }
  haveRec = prefs.isKey("calrec") && prefs.getBytes("calrec", &rec, sizeof rec) == sizeof rec &&
            rec.version == CALREC_VERSION;
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
  started = true;
}

const measure::Cfg& calibCfg() { return cfg; }

bool calibMode() { return calMode; }
bool calibPoint(float* x, float* y) {
  *x = ptX;
  *y = ptY;
  return havePt;
}
bool calibRecord(uint32_t* id, bool* stale) {
  *id = rec.id;
  *stale = rec.stale || rec.id != measure::calibrationId(cfg);
  return haveRec;
}

void calibPrintStatus() {
  uint32_t id;
  bool stale;
  if (!calibRecord(&id, &stale)) {
    Serial.printf("cal: no calibration record (nominal values); values id %08X", (unsigned)measure::calibrationId(cfg));
  } else {
    Serial.printf("cal: record %08X of %04u-%02u-%02u, %u points, %u sparks, residual %.0f ns: %s",
                  (unsigned)id, (unsigned)(rec.date / 10000), (unsigned)(rec.date / 100 % 100),
                  (unsigned)(rec.date % 100), rec.points, rec.sparks, rec.resNs,
                  stale ? "STALE (a mic, h, k, offset or sigma changed since)" : "in use");
  }
  Serial.printf("; calibration mode %s", calMode ? "ON (nothing goes to the app)" : "off");
  if (havePt) Serial.printf(", template point (%.3f, %.3f) mm", ptX, ptY);
  Serial.println();
}
uint32_t calibId() { return cfgIdNow; }

bool calibRecordInfo(uint32_t* date, uint16_t* points, uint16_t* sparks, float* resNs) {
  *date = haveRec ? rec.date : 0;
  *points = haveRec ? rec.points : 0;
  *sparks = haveRec ? rec.sparks : 0;
  *resNs = haveRec ? rec.resNs : 0;
  return haveRec;
}

void calibSetMode(bool on) {
  calMode = on;
  calibPrintStatus();
}

void calibSetPoint(bool have, float x, float y) {
  havePt = have;
  if (have) {
    ptX = x;
    ptY = y;
  }
  calibPrintStatus();
}

void calibSetPolicy(const measure::Cfg& p) {
  cfg.resLimitNs = p.resLimitNs;
  cfg.dropOne = p.dropOne;
  cfg.maxLost = p.maxLost;
  cfg.areaMm = p.areaMm;
  cfg.togetherNs = p.togetherNs;
  cfg.pulseMaxNs = p.pulseMaxNs;
  cfg.curtainLoNs = p.curtainLoNs;
  cfg.curtainHiNs = p.curtainHiNs;
  prefs.putFloat("reslimit", cfg.resLimitNs);
  prefs.putBool("dropone", cfg.dropOne);
  prefs.putChar("lost", cfg.maxLost);
  prefs.putFloat("area", cfg.areaMm);
  prefs.putFloat("together", cfg.togetherNs);
  prefs.putFloat("pulsemax", cfg.pulseMaxNs);
  if (cfg.curtainHiNs > 0) {
    prefs.putFloat("irlo", cfg.curtainLoNs);
    prefs.putFloat("irhi", cfg.curtainHiNs);
  } else {
    for (const char* k : {"irlo", "irhi"})
      if (prefs.isKey(k)) prefs.remove(k);
  }
  curtainApply();
  changed();
}

void calibSetChannels(const measure::ChannelCfg ch[8], uint32_t date, uint16_t points,
                      uint16_t sparks, float resNs) {
  for (int c = 0; c < CHANNELS; c++) {
    cfg.ch[c] = ch[c];
    save(c);
  }
  rec = {CALREC_VERSION, 0, points, sparks, measure::calibrationId(cfg), date, resNs};
  haveRec = true;
  saveRec();
  changed();
  calibPrintStatus();
}

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
                 "  mic default | sim    the D1/D2 layout on CH1-CH6 | the simulator's on CH3-CH8\n"
                 "  mic clear <ch>       no mic on that channel\n"
                 "  k [ch] [value]       arrival time t0 = t1 - k (t2 - t1) - offset: k, every\n"
                 "                       channel or one (stored; `k` lists them)\n"
                 "  offset [ch] [ns]     a channel's own delay, taken off its t0 (stored);\n"
                 "                       `offset` lists them, `offset clear` zeroes all\n"
                 "  reslimit [ns]        a shot goes to the app if its fit's rms residual is\n"
                 "                       within this (stored, default 1500)\n"
                 "  dropone on|off       over the limit, the best fit with one channel left\n"
                 "                       out (stored, default on)\n"
                 "  valid                the validity limits (stored; MODEL.md \"Validity\"):\n"
                 "  valid lost <n>|off   mic channels that may be lost (no edge, pulse, dropped), 1\n"
                 "  valid area <mm>|off  the hole within +-this of the centre, 105\n"
                 "  valid together <us>|off  every first edge within this: a spike, 20\n"
                 "  valid pulse <us>|off t2 - t1 up to this, or the channel is left out, 10\n"
                 "  valid default        all four back to their defaults\n"
                 "  curtain              the IR curtain: its gate, IR_IN, its counts\n"
                 "  curtain on|off       the gate: a shot needs a dip 45-100 us after it; on\n"
                 "                       also lights the IR LEDs (stored, default off)\n"
                 "  curtain window <lo> <hi>  the gate's window, us after the shot (turns it on)\n"
                 "  sigma [ch] [ns]      a channel's timing error, 1 sigma: the fit's weight\n"
                 "                       (stored, default 50; `sigma` lists them)\n"
                 "  cal                  the calibration record and calibration mode\n"
                 "  cal on|off           calibration mode: events are marked with the template\n"
                 "                       point and not sent to the app\n"
                 "  cal pt <x> <y> | -   the template point the next sparks are at (mm)\n"
                 "  cal ch <ch> <x> <y> <h> <k> <offset> <sigma>  a channel's values at once\n"
                 "  cal id <id> <yyyymmdd> <points> <sparks> <res ns>  store the record\n"
                 "                       (tools/calibrate.py prints it); cal clear");
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
  // The blast check takes about a second with 6 mics: only for a whole layout, or asked for.
  if (!sub[0] || !strcmp(sub, "default") || !strcmp(sub, "sim")) positionBlastCheck();
  else Serial.println("mic: `mic` runs the blast check");
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

static bool policyCommand(const char* cmd, const char* line) {
  if (!strcmp(cmd, "reslimit")) {
    float v;
    if (sscanf(line, "%*s %f", &v) == 1) {
      if (!(v > 0 && v <= RES_LIMIT_MAX_NS)) {
        Serial.printf("reslimit <1..%.0f ns>\n", RES_LIMIT_MAX_NS);
        return true;
      }
      cfg.resLimitNs = v;
      prefs.putFloat("reslimit", v);
      Serial.printf("reslimit %.0f ns\n", cfg.resLimitNs);
      changed();
    } else {
      Serial.printf("reslimit %.0f ns\n", cfg.resLimitNs);
    }
    return true;
  }
  // dropone
  char word[8] = "";
  sscanf(line, "%*s %7s", word);
  if (!strcmp(word, "on") || !strcmp(word, "off")) {
    cfg.dropOne = !strcmp(word, "on");
    prefs.putBool("dropone", cfg.dropOne);
    Serial.printf("drop-one %s\n", cfg.dropOne ? "on" : "off");
    changed();
  } else if (word[0]) {
    Serial.println("dropone on|off");
  } else {
    Serial.printf("drop-one %s\n", cfg.dropOne ? "on" : "off");
  }
  return true;
}

static void printValid() {
  Serial.printf("valid: lost ");
  if (cfg.maxLost < 0) Serial.print("off");
  else Serial.printf("%d", cfg.maxLost);
  Serial.print(", area ");
  if (cfg.areaMm > 0) Serial.printf("+-%.0f mm", cfg.areaMm);
  else Serial.print("off");
  Serial.print(", together ");
  if (cfg.togetherNs > 0) Serial.printf("%.1f us", cfg.togetherNs / 1000);
  else Serial.print("off");
  Serial.print(", pulse ");
  if (cfg.pulseMaxNs > 0) Serial.printf("%.1f us", cfg.pulseMaxNs / 1000);
  else Serial.print("off");
  Serial.printf("; reslimit %.0f ns, drop-one %s; curtain ", cfg.resLimitNs, cfg.dropOne ? "on" : "off");
  if (cfg.curtainHiNs > 0) Serial.printf("%.0f-%.0f us\n", cfg.curtainLoNs / 1000, cfg.curtainHiNs / 1000);
  else Serial.println("off");
}

// `valid`: the validity limits (MODEL.md "Validity", MEASUREMENT 4a).
static bool validCommand(const char* line) {
  char what[12] = "", arg[12] = "";
  const int n = sscanf(line, "%*s %11s %11s", what, arg);
  if (n < 1) {
    printValid();
    return true;
  }
  if (!strcmp(what, "default")) {
    cfg.maxLost = measure::MAX_LOST_DEFAULT;
    cfg.areaMm = measure::AREA_DEFAULT_MM;
    cfg.togetherNs = measure::TOGETHER_DEFAULT_NS;
    cfg.pulseMaxNs = measure::PULSE_MAX_DEFAULT_NS;
    for (const char* k : {"lost", "area", "together", "pulsemax"})
      if (prefs.isKey(k)) prefs.remove(k);   // a missing key would log an error
    printValid();
    changed();
    return true;
  }
  const bool off = !strcmp(arg, "off");
  char* end;
  const float v = strtof(arg, &end);
  const bool num = n == 2 && end != arg && *end == 0;
  if (!strcmp(what, "lost") && (off || (num && v >= 0 && v <= 4 && v == (int)v))) {
    cfg.maxLost = off ? -1 : (int8_t)v;
    prefs.putChar("lost", cfg.maxLost);
  } else if (!strcmp(what, "area") && (off || (num && v >= 10 && v <= 1000))) {
    cfg.areaMm = off ? 0 : v;
    prefs.putFloat("area", cfg.areaMm);
  } else if (!strcmp(what, "together") && (off || (num && v > 0 && v <= 100))) {
    cfg.togetherNs = off ? 0 : v * 1000;
    prefs.putFloat("together", cfg.togetherNs);
  } else if (!strcmp(what, "pulse") && (off || (num && v > 0 && v <= 100))) {
    cfg.pulseMaxNs = off ? 0 : v * 1000;
    prefs.putFloat("pulsemax", cfg.pulseMaxNs);
  } else {
    Serial.println("valid [default] | lost <0..4>|off | area <10..1000 mm>|off | together "
                   "<us>|off | pulse <us>|off");
    return true;
  }
  printValid();
  if (!strcmp(what, "area")) Serial.println("valid: `mic` runs the blast check for the new area");
  changed();
  return true;
}

static bool sigmaCommand(const char* line) {
  int ch = 0;
  float v = NAN;
  const int w = words(line);   // sigma | sigma <ns> | sigma <ch> <ns>
  if (w == 2 && sscanf(line, "%*s %f", &v) != 1) v = NAN;
  if (w == 3 && (sscanf(line, "%*s %d %f", &ch, &v) != 2 || !channelArg(ch))) return true;
  if (w > 1) {
    if (w > 3 || !(v >= 1 && v <= 100000)) { Serial.println("sigma [ch] <1..100000 ns>"); return true; }
    for (int c = 0; c < CHANNELS; c++)
      if (!ch || c == ch - 1) {
        cfg.ch[c].sigmaNs = v;
        save(c);
      }
  }
  char out[200];
  int n = snprintf(out, sizeof out, "sigma ns:");
  for (int c = 0; c < CHANNELS; c++)
    n += snprintf(out + n, sizeof out - n, "  CH%d %.1f", c + 1, cfg.ch[c].sigmaNs);
  Serial.println(out);
  if (w > 1) changed();
  return true;
}

static bool calCommand(const char* line) {
  char sub[8] = "";
  sscanf(line, "%*s %7s", sub);
  if (!strcmp(sub, "on") || !strcmp(sub, "off")) {
    calMode = !strcmp(sub, "on");
  } else if (!strcmp(sub, "pt")) {
    float x, y;
    char dash[4] = "";
    if (sscanf(line, "%*s %*s %f %f", &x, &y) == 2 && fabsf(x) <= MIC_MAX_MM && fabsf(y) <= MIC_MAX_MM) {
      ptX = x;
      ptY = y;
      havePt = true;
    } else if (sscanf(line, "%*s %*s %3s", dash) == 1 && !strcmp(dash, "-")) {
      havePt = false;
    } else {
      Serial.println("cal pt <x> <y> (mm), or `cal pt -`");
      return true;
    }
  } else if (!strcmp(sub, "id")) {
    unsigned long id, date, points, sparks;
    float res;
    if (sscanf(line, "%*s %*s %lx %lu %lu %lu %f", &id, &date, &points, &sparks, &res) != 5) {
      Serial.println("cal id <id> <yyyymmdd> <points> <sparks> <res ns> (as tools/calibrate.py prints)");
      return true;
    }
    const uint32_t now = measure::calibrationId(cfg);
    if ((uint32_t)id != now) {
      Serial.printf("cal: the board's values (id %08X) are not the fit's (%08X): send the fit's "
                    "`cal ch` lines again, then this one\n", (unsigned)now, (unsigned)id);
      return true;
    }
    rec = {CALREC_VERSION, 0, (uint16_t)points, (uint16_t)sparks, now, (uint32_t)date, res};
    haveRec = true;
    saveRec();
  } else if (!strcmp(sub, "ch")) {
    // One channel's calibrated values in one line, as tools/calibrate.py prints them.
    int ch;
    float x, y, h, k, off, sig;
    if (sscanf(line, "%*s %*s %d %f %f %f %f %f %f", &ch, &x, &y, &h, &k, &off, &sig) != 7 ||
        ch < 1 || ch > CHANNELS || !(fabsf(x) <= MIC_MAX_MM && fabsf(y) <= MIC_MAX_MM) ||
        !(h >= 0 && h <= H_MAX_MM) || !(k >= 0 && k <= K_MAX) || !(fabsf(off) <= OFFSET_MAX_NS) ||
        !(sig >= 1 && sig <= 100000)) {
      Serial.println("cal ch <ch> <x> <y> <h> <k> <offset ns> <sigma ns>");
      return true;
    }
    cfg.ch[ch - 1] = {true, x, y, h, k, off, sig};
    save(ch - 1);
    Serial.printf("cal: CH%d (%.4f, %.4f) mm, h %.2f mm, k %.5f, offset %.2f ns, sigma %.1f ns\n", ch,
                  x, y, h, k, off, sig);
    changed();
    return true;
  } else if (!strcmp(sub, "clear")) {
    if (prefs.isKey("calrec")) prefs.remove("calrec");
    haveRec = false;
    rec = {};
  } else if (sub[0]) {
    Serial.println("cal [on|off | pt <x> <y> | ch ... | id ... | clear]");
    return true;
  }
  calibPrintStatus();
  return true;
}

// `curtain`: the IR curtain's gate (MODEL.md "Validity", MEASUREMENT 4b).
static bool curtainCommand(const char* line) {
  char what[12] = "";
  float lo = 0, hi = 0;
  const int n = sscanf(line, "%*s %11s %f %f", what, &lo, &hi);
  if (n < 1) {
    curtainPrintStatus();
    return true;
  }
  if (!strcmp(what, "on")) {
    lo = measure::CURTAIN_LO_DEFAULT_NS / 1000;
    hi = measure::CURTAIN_HI_DEFAULT_NS / 1000;
  } else if (!strcmp(what, "off")) {
    lo = hi = 0;
  } else if (strcmp(what, "window") || n != 3 || !(lo >= 0 && hi > lo && hi <= 2000)) {
    Serial.println("curtain [on|off | window <lo> <hi>] (us after the shot, 0 <= lo < hi <= 2000)");
    return true;
  }
  cfg.curtainLoNs = lo * 1000;
  cfg.curtainHiNs = hi * 1000;
  if (hi > 0) {
    prefs.putFloat("irlo", cfg.curtainLoNs);
    prefs.putFloat("irhi", cfg.curtainHiNs);
  } else {
    for (const char* k : {"irlo", "irhi"})
      if (prefs.isKey(k)) prefs.remove(k);
  }
  curtainApply();
  curtainPrintStatus();
  changed();
  return true;
}

bool calibCommand(const char* cmd, const char* line) {
  if (!strcmp(cmd, "curtain")) return curtainCommand(line);
  if (!strcmp(cmd, "sigma")) return sigmaCommand(line);
  if (!strcmp(cmd, "cal")) return calCommand(line);
  if (!strcmp(cmd, "reslimit") || !strcmp(cmd, "dropone")) return policyCommand(cmd, line);
  if (!strcmp(cmd, "valid")) return validCommand(line);
  if (!strcmp(cmd, "mic")) return micCommand(line);
  if (!strcmp(cmd, "k")) return kCommand(line);
  if (!strcmp(cmd, "offset")) return offsetCommand(line);
  return false;
}
