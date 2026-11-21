// OpenSights measurement core - the log lines: see record.h and ../../MODEL.md "Log lines".
#include "record.h"
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

namespace measure {
namespace {

// Appends with snprintf, keeping n at the length the whole text would have.
int add(char* out, size_t size, int n, const char* fmt, ...) __attribute__((format(printf, 4, 5)));
int add(char* out, size_t size, int n, const char* fmt, ...) {
  va_list ap;
  va_start(ap, fmt);
  const size_t at = (size_t)n < size ? (size_t)n : size;
  int m = vsnprintf(out + at, size - at, fmt, ap);
  va_end(ap);
  return n + (m > 0 ? m : 0);
}

int formatFields(char* out, size_t size, const Cfg& cfg) {
  int n = 0;
  if (size) out[0] = 0;
  for (int c = 0; c < MAX_MICS; c++) {
    const ChannelCfg& ch = cfg.ch[c];
    n = add(out, size, n, "%sCH%d=", c ? " " : "", c + 1);
    if (!ch.mic) n = add(out, size, n, "-");
    else n = add(out, size, n, "%.9g,%.9g,%.9g,%.9g,%.9g,%.9g", ch.x, ch.y, ch.h, ch.k,
                 ch.offsetNs, ch.sigmaNs);
  }
  return n;
}

// The next space-separated token of s from *p, as "key=value": false at the end.
bool nextField(const char*& p, char* key, size_t keySize, const char*& value) {
  while (*p == ' ' || *p == '\t') p++;
  if (!*p || *p == '\r' || *p == '\n') return false;
  const char* eq = p;
  while (*eq && *eq != '=' && *eq != ' ') eq++;
  size_t k = (size_t)(eq - p);
  if (k >= keySize) k = keySize - 1;
  memcpy(key, p, k);
  key[k] = 0;
  value = *eq == '=' ? eq + 1 : eq;
  p = value;
  while (*p && *p != ' ' && *p != '\t' && *p != '\r' && *p != '\n') p++;
  return true;
}

// "CH5" -> 4, or -1.
int channelOf(const char* key) {
  if (strncmp(key, "CH", 2) || key[2] < '1' || key[2] > '0' + MAX_MICS || key[3]) return -1;
  return key[2] - '1';
}

}  // namespace

uint32_t crc32(const char* s) {
  uint32_t crc = 0xFFFFFFFFu;
  for (; *s; s++) {
    crc ^= (uint8_t)*s;
    for (int b = 0; b < 8; b++) crc = (crc >> 1) ^ (0xEDB88320u & (0u - (crc & 1)));
  }
  return ~crc;
}

uint32_t cfgId(const Cfg& cfg) {
  char fields[512];
  formatFields(fields, sizeof fields, cfg);
  return crc32(fields);
}

int formatCfg(char* out, size_t size, const Cfg& cfg) {
  char fields[512];
  formatFields(fields, sizeof fields, cfg);
  return snprintf(out, size, "cfg id=%08X %s", (unsigned)crc32(fields), fields);
}

bool parseCfg(const char* line, Cfg& cfg, uint32_t* id) {
  const char* p = strstr(line, "cfg id=");
  if (!p) return false;
  p += 7;
  char* end;
  const unsigned long given = strtoul(p, &end, 16);
  if (end != p + 8 || *end != ' ') return false;
  p = end + 1;
  // The fields as printed (up to the line end) are what the id was computed over.
  char fields[512];
  size_t len = strcspn(p, "\r\n");
  if (len >= sizeof fields) return false;
  memcpy(fields, p, len);
  fields[len] = 0;
  if (crc32(fields) != (uint32_t)given) return false;

  Cfg out = {};
  int seen = 0;
  const char* q = fields;
  char key[8];
  const char* value;
  while (nextField(q, key, sizeof key, value)) {
    const int c = channelOf(key);
    if (c < 0) return false;
    seen |= 1 << c;
    if (value[0] == '-' && (value[1] == 0 || value[1] == ' ')) continue;   // "CH7=-": no mic
    ChannelCfg& ch = out.ch[c];
    float v[6];
    const char* s = value;
    for (int i = 0; i < 6; i++) {
      v[i] = strtof(s, &end);
      if (end == s || (i < 5 && *end != ',')) return false;
      s = end + 1;
    }
    ch = {true, v[0], v[1], v[2], v[3], v[4], v[5]};
  }
  if (seen != (1 << MAX_MICS) - 1) return false;
  cfg = out;
  if (id) *id = (uint32_t)given;
  return true;
}

int formatRec(char* out, size_t size, const Rec& r) {
  int n = snprintf(out, size, "rec n=%u t=%.7f cfg=%08X T=%.2f RH=%.1f p=%.2f meas=%.3s c=%.9g",
                   (unsigned)r.n, r.timeS, (unsigned)r.cfgId, r.tC, r.rh, r.hPa, r.measured, r.cMs);
  if (n < 0) return n;
  for (int c = 0; c < MAX_MICS; c++) {
    if (!((r.channels >> c) & 1)) continue;
    n = add(out, size, n, " CH%d=", c + 1);
    for (int e = 0; e < 2; e++) {
      const int32_t t = e ? r.t2[c] : r.t1[c];
      n = t == NO_EDGE ? add(out, size, n, "-") : add(out, size, n, "%ld", (long)t);
      if (!e) n = add(out, size, n, ",");
    }
  }
  return n;
}

bool parseRec(const char* line, Rec& rec) {
  const char* p = strstr(line, "rec n=");
  if (!p) return false;
  Rec r = {};
  for (int c = 0; c < MAX_MICS; c++) r.t1[c] = r.t2[c] = NO_EDGE;
  strcpy(r.measured, "---");
  unsigned got = 0;   // bit per required field
  char key[8];
  const char* value;
  char* end;
  while (nextField(p, key, sizeof key, value)) {
    if (!strcmp(key, "rec")) continue;
    const int c = channelOf(key);
    if (c >= 0) {
      r.channels |= 1 << c;
      const char* s = value;
      for (int e = 0; e < 2; e++) {
        int32_t t = NO_EDGE;
        if (*s == '-') {
          end = (char*)s + 1;
        } else {
          t = (int32_t)strtol(s, &end, 10);
          if (end == s) return false;
        }
        (e ? r.t2 : r.t1)[c] = t;
        if (!e && *end != ',') return false;
        s = end + 1;
      }
      continue;
    }
    if (!strcmp(key, "n")) { r.n = (uint32_t)strtoul(value, &end, 10); got |= 1; }
    else if (!strcmp(key, "t")) { r.timeS = strtod(value, &end); got |= 2; }
    else if (!strcmp(key, "cfg")) { r.cfgId = (uint32_t)strtoul(value, &end, 16); got |= 4; }
    else if (!strcmp(key, "T")) { r.tC = strtof(value, &end); got |= 8; }
    else if (!strcmp(key, "RH")) { r.rh = strtof(value, &end); got |= 16; }
    else if (!strcmp(key, "p")) { r.hPa = strtof(value, &end); got |= 32; }
    else if (!strcmp(key, "c")) { r.cMs = strtof(value, &end); got |= 64; }
    else if (!strcmp(key, "meas")) {
      for (int i = 0; i < 3; i++) r.measured[i] = value[i] && value[i] != ' ' ? value[i] : '-';
      r.measured[3] = 0;
    }
    // Unknown keys are skipped: later firmware may add fields.
  }
  if (got != 127) return false;
  rec = r;
  return true;
}

uint8_t solveInput(const Cfg& cfg, const Rec& rec, SolveInput& in) {
  uint8_t mask = 0;
  in.n = MAX_MICS;
  in.c = mmPerNs(rec.cMs);
  for (int c = 0; c < MAX_MICS; c++) {
    const ChannelCfg& ch = cfg.ch[c];
    in.mic[c] = {ch.x, ch.y, ch.h};
    in.sigmaNs[c] = ch.sigmaNs;
    in.t0[c] = 0;
    if (!ch.mic || !((rec.channels >> c) & 1) || rec.t1[c] == NO_EDGE || rec.t2[c] == NO_EDGE)
      continue;
    in.t0[c] = arrivalT0((float)rec.t1[c], (float)rec.t2[c], ch.k, ch.offsetNs);
    mask |= 1 << c;
  }
  return mask;
}

}  // namespace measure
