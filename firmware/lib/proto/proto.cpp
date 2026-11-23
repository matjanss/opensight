// OpenSights - the BLE protocol's messages: see proto.h and PROTOCOL.md.
#include "proto.h"
#include <math.h>
#include <string.h>

namespace proto {

// Little-endian writer and reader over a fixed buffer.
namespace {
struct W {
  uint8_t* p;
  void u8(uint8_t v) { *p++ = v; }
  void u16(uint16_t v) { u8(v & 0xFF); u8(v >> 8); }
  void u32(uint32_t v) { for (int i = 0; i < 4; i++) u8((v >> (8 * i)) & 0xFF); }
  void u64(uint64_t v) { for (int i = 0; i < 8; i++) u8((v >> (8 * i)) & 0xFF); }
  void i32(int32_t v) { u32((uint32_t)v); }
  void f32(float v) { uint32_t b; memcpy(&b, &v, 4); u32(b); }
  void zero(int n) { while (n-- > 0) u8(0); }
};
struct R {
  const uint8_t* p;
  uint8_t u8() { return *p++; }
  uint16_t u16() { uint16_t v = p[0] | (p[1] << 8); p += 2; return v; }
  uint32_t u32() { uint32_t v = 0; for (int i = 0; i < 4; i++) v |= (uint32_t)p[i] << (8 * i); p += 4; return v; }
  uint64_t u64() { uint64_t v = 0; for (int i = 0; i < 8; i++) v |= (uint64_t)p[i] << (8 * i); p += 8; return v; }
  int32_t i32() { return (int32_t)u32(); }
  float f32() { uint32_t b = u32(); float v; memcpy(&v, &b, 4); return v; }
  void skip(int n) { p += n; }
};
}  // namespace

void encodeShot(const Shot& s, uint8_t out[SHOT_LEN]) {
  W w{out};
  w.u32(s.seq); w.u32(s.boot); w.u32(s.time); w.i32(s.x); w.i32(s.y);
  w.u32(s.event); w.u32(s.cal); w.u16(s.sigma); w.u8(s.kind); w.u8(s.flags);
}

bool decodeShot(const uint8_t* in, size_t len, Shot& s) {
  if (len != SHOT_LEN) return false;
  R r{in};
  s.seq = r.u32(); s.boot = r.u32(); s.time = r.u32(); s.x = r.i32(); s.y = r.i32();
  s.event = r.u32(); s.cal = r.u32(); s.sigma = r.u16(); s.kind = r.u8(); s.flags = r.u8();
  return true;
}

void encodeEvent(const Event& e, uint8_t out[EVENT_LEN]) {
  W w{out};
  w.u8(e.kind); w.u8(e.verdict); w.u8(e.acoustic); w.u8((uint8_t)e.curtain);
  w.u32(e.boot); w.u32(e.seq); w.u32(e.n); w.u32(e.time); w.u64(e.tNs);
  w.u32(e.cfg); w.u32(e.cal);
  w.u8(e.flags); w.u8(e.channels); w.u8(e.fitted); w.u8((uint8_t)e.dropped);
  w.i32(e.x); w.i32(e.y);
  w.f32(e.rmsNs); w.f32(e.sigmaMm); w.f32(e.tC); w.f32(e.rh); w.f32(e.hPa); w.f32(e.c);
  w.u8(e.measured); w.u8(e.dips); w.zero(2);
  for (int i = 0; i < 8; i++) w.i32(e.t1[i]);
  for (int i = 0; i < 8; i++) w.i32(e.t2[i]);
  for (int i = 0; i < 4; i++) w.i32(e.dip[i]);
  for (int i = 0; i < 4; i++) w.i32(e.dipLen[i]);
  w.f32(e.ptX); w.f32(e.ptY);
}

bool decodeEvent(const uint8_t* in, size_t len, Event& e) {
  if (len != EVENT_LEN) return false;
  R r{in};
  e.kind = r.u8(); e.verdict = r.u8(); e.acoustic = r.u8(); e.curtain = (int8_t)r.u8();
  e.boot = r.u32(); e.seq = r.u32(); e.n = r.u32(); e.time = r.u32(); e.tNs = r.u64();
  e.cfg = r.u32(); e.cal = r.u32();
  e.flags = r.u8(); e.channels = r.u8(); e.fitted = r.u8(); e.dropped = (int8_t)r.u8();
  e.x = r.i32(); e.y = r.i32();
  e.rmsNs = r.f32(); e.sigmaMm = r.f32(); e.tC = r.f32(); e.rh = r.f32(); e.hPa = r.f32(); e.c = r.f32();
  e.measured = r.u8(); e.dips = r.u8(); r.skip(2);
  for (int i = 0; i < 8; i++) e.t1[i] = r.i32();
  for (int i = 0; i < 8; i++) e.t2[i] = r.i32();
  for (int i = 0; i < 4; i++) e.dip[i] = r.i32();
  for (int i = 0; i < 4; i++) e.dipLen[i] = r.i32();
  e.ptX = r.f32(); e.ptY = r.f32();
  return e.dips <= 4;
}

void encodeStatus(const Status& s, uint8_t out[STATUS_LEN]) {
  W w{out};
  w.u32(s.boot); w.u32(s.cfg); w.u32(s.cal);
  w.u8(s.flags); w.u8(s.measured); w.u8(s.mics); w.u8(0);
  w.f32(s.ptX); w.f32(s.ptY); w.f32(s.tC); w.f32(s.rh); w.f32(s.hPa); w.f32(s.c);
  w.u32(s.events); w.u32(s.missed); w.u32(s.shots);
}

bool decodeStatus(const uint8_t* in, size_t len, Status& s) {
  if (len != STATUS_LEN) return false;
  R r{in};
  s.boot = r.u32(); s.cfg = r.u32(); s.cal = r.u32();
  s.flags = r.u8(); s.measured = r.u8(); s.mics = r.u8(); r.skip(1);
  s.ptX = r.f32(); s.ptY = r.f32(); s.tC = r.f32(); s.rh = r.f32(); s.hPa = r.f32(); s.c = r.f32();
  s.events = r.u32(); s.missed = r.u32(); s.shots = r.u32();
  return true;
}

void encodeSettings(const Settings& s, uint8_t out[SETTINGS_LEN]) {
  W w{out};
  w.u8(LAYOUT); w.u8(s.dropOne ? 1 : 0); w.u8((uint8_t)s.maxLost); w.u8(s.mask);
  w.f32(s.resLimitNs); w.f32(s.areaMm); w.f32(s.togetherNs); w.f32(s.pulseMaxNs);
  w.f32(s.curtainLoNs); w.f32(s.curtainHiNs);
  w.u16(s.spanMm); w.u16(s.deadtimeMs); w.zero(4);
}

bool decodeSettings(const uint8_t* in, size_t len, Settings& s) {
  if (len != SETTINGS_LEN || in[0] != LAYOUT) return false;
  R r{in + 1};
  const uint8_t drop = r.u8();
  if (drop > 1) return false;
  s.dropOne = drop == 1;
  s.maxLost = (int8_t)r.u8(); s.mask = r.u8();
  s.resLimitNs = r.f32(); s.areaMm = r.f32(); s.togetherNs = r.f32(); s.pulseMaxNs = r.f32();
  s.curtainLoNs = r.f32(); s.curtainHiNs = r.f32();
  s.spanMm = r.u16(); s.deadtimeMs = r.u16();
  return true;
}

void encodeCalibration(const Calibration& c, uint8_t out[CALIBRATION_LEN]) {
  W w{out};
  w.u8(LAYOUT); w.u32(c.date); w.f32(c.resNs); w.u16(c.points); w.u16(c.sparks);
  for (int i = 0; i < 8; i++) {
    const measure::ChannelCfg& ch = c.ch[i];
    w.u8(ch.mic ? 1 : 0);
    w.f32(ch.x); w.f32(ch.y); w.f32(ch.h); w.f32(ch.k); w.f32(ch.offsetNs); w.f32(ch.sigmaNs);
  }
}

bool decodeCalibration(const uint8_t* in, size_t len, Calibration& c) {
  if (len != CALIBRATION_LEN || in[0] != LAYOUT) return false;
  R r{in + 1};
  c.date = r.u32(); c.resNs = r.f32(); c.points = r.u16(); c.sparks = r.u16();
  for (int i = 0; i < 8; i++) {
    measure::ChannelCfg& ch = c.ch[i];
    const uint8_t mic = r.u8();
    if (mic > 1) return false;
    ch.mic = mic == 1;
    ch.x = r.f32(); ch.y = r.f32(); ch.h = r.f32(); ch.k = r.f32(); ch.offsetNs = r.f32(); ch.sigmaNs = r.f32();
  }
  return true;
}

bool decodeRequest(const uint8_t* in, size_t len, Request& q) {
  if (len != REQUEST_LEN) return false;
  R r{in};
  q.boot = r.u32();
  q.next = r.u32();
  return true;
}

bool decodeClear(const uint8_t* in, size_t len, Request& q) {
  if (len != CLEAR_LEN || in[0] != CLEAR) return false;
  R r{in + 1};
  q.boot = r.u32();
  q.next = r.u32();
  return true;
}

// The serial commands' limits (calib.cpp, capture.cpp), so a write from the app can't set
// what the serial console couldn't.
bool settingsValid(const Settings& s, uint16_t spanMaxMm) {
  const bool off = s.curtainLoNs == 0 && s.curtainHiNs == 0;
  return s.resLimitNs > 0 && s.resLimitNs <= 100000 &&
         s.maxLost >= -1 && s.maxLost <= 4 &&
         (s.areaMm == 0 || (s.areaMm >= 10 && s.areaMm <= 1000)) &&
         (s.togetherNs == 0 || (s.togetherNs > 0 && s.togetherNs <= 100000)) &&
         (s.pulseMaxNs == 0 || (s.pulseMaxNs > 0 && s.pulseMaxNs <= 100000)) &&
         (off || (s.curtainLoNs >= 0 && s.curtainHiNs > s.curtainLoNs && s.curtainHiNs <= 2000000)) &&
         s.spanMm >= 50 && s.spanMm <= spanMaxMm &&
         s.deadtimeMs >= 1 && s.deadtimeMs <= 1000;
}

bool calibrationValid(const Calibration& c) {
  int mics = 0;
  for (int i = 0; i < 8; i++) {
    const measure::ChannelCfg& ch = c.ch[i];
    if (!(fabsf(ch.x) <= 1000 && fabsf(ch.y) <= 1000 && ch.h >= 0 && ch.h <= 100 && ch.k >= 0 &&
          ch.k <= 2 && fabsf(ch.offsetNs) <= 100000 && ch.sigmaNs >= 1 && ch.sigmaNs <= 100000))
      return false;
    if (ch.mic) mics++;
  }
  return mics >= 4 && c.resNs >= 0 && isfinite(c.resNs);
}

void eventFromRec(const measure::Rec& rec, const measure::EventFit& f, Event& e) {
  e = {};
  e.kind = KIND_SHOT;
  e.verdict = f.verdict;
  e.acoustic = f.acoustic;
  e.curtain = f.curtain;
  e.n = rec.n;
  e.tNs = (uint64_t)llround(rec.timeS * 1e9);
  e.cfg = rec.cfgId;
  e.cal = rec.hasCal ? rec.calId : 0;
  e.flags = (rec.calMode ? EV_CAL_MODE : 0) | (rec.hasCal && rec.calStale ? EV_CAL_STALE : 0) |
            (rec.hasPt ? EV_HAS_PT : 0) | (rec.hasIr ? EV_CURTAIN_ON : 0) | (f.solved ? EV_SOLVED : 0);
  e.channels = rec.channels;
  e.fitted = f.solved ? f.s.used : 0;
  e.dropped = f.solved ? f.s.dropped : -1;
  if (f.solved && isfinite(f.s.x) && isfinite(f.s.y)) {
    e.x = lroundf(f.s.x * 100);
    e.y = lroundf(f.s.y * 100);
    e.rmsNs = f.s.rmsNs;
    e.sigmaMm = f.s.major;
  } else {
    e.rmsNs = e.sigmaMm = NAN;
  }
  e.tC = rec.tC; e.rh = rec.rh; e.hPa = rec.hPa; e.c = rec.cMs;
  e.measured = (rec.measured[0] == 'T' ? MEAS_T : 0) | (rec.measured[1] == 'R' ? MEAS_RH : 0) |
               (rec.measured[2] == 'P' ? MEAS_P : 0);
  for (int i = 0; i < 8; i++) {
    e.t1[i] = rec.t1[i] == measure::NO_EDGE ? NO_VALUE : rec.t1[i];
    e.t2[i] = rec.t2[i] == measure::NO_EDGE ? NO_VALUE : rec.t2[i];
  }
  e.dips = rec.hasIr ? rec.nDips : 0;
  for (int i = 0; i < 4; i++) {
    e.dip[i] = i < e.dips ? rec.dipNs[i] : 0;
    e.dipLen[i] = i < e.dips ? (rec.dipLenNs[i] == measure::NO_EDGE ? NO_VALUE : rec.dipLenNs[i]) : 0;
  }
  e.ptX = rec.hasPt ? rec.ptX : 0;
  e.ptY = rec.hasPt ? rec.ptY : 0;
}

bool recFromEvent(const Event& e, measure::Rec& rec) {
  if (e.kind != KIND_SHOT) return false;
  rec = {};
  rec.n = e.n;
  rec.timeS = (double)(e.tNs / 1000000000ULL) + (double)(e.tNs % 1000000000ULL) / 1e9;
  rec.cfgId = e.cfg;
  rec.tC = e.tC; rec.rh = e.rh; rec.hPa = e.hPa; rec.cMs = e.c;
  rec.measured[0] = e.measured & MEAS_T ? 'T' : '-';
  rec.measured[1] = e.measured & MEAS_RH ? 'R' : '-';
  rec.measured[2] = e.measured & MEAS_P ? 'P' : '-';
  rec.measured[3] = 0;
  rec.channels = e.channels;
  for (int i = 0; i < 8; i++) {
    rec.t1[i] = e.t1[i] == NO_VALUE ? measure::NO_EDGE : e.t1[i];
    rec.t2[i] = e.t2[i] == NO_VALUE ? measure::NO_EDGE : e.t2[i];
  }
  rec.hasPt = e.flags & EV_HAS_PT;
  rec.ptX = e.ptX; rec.ptY = e.ptY;
  rec.calMode = e.flags & EV_CAL_MODE;
  rec.hasCal = e.cal != 0;
  rec.calId = e.cal;
  rec.calStale = e.flags & EV_CAL_STALE;
  rec.hasIr = e.flags & EV_CURTAIN_ON;
  rec.nDips = e.dips;
  for (int i = 0; i < e.dips && i < measure::MAX_DIPS; i++) {
    rec.dipNs[i] = e.dip[i];
    rec.dipLenNs[i] = e.dipLen[i] == NO_VALUE ? measure::NO_EDGE : e.dipLen[i];
  }
  return true;
}

}  // namespace proto
