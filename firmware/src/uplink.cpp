// OpenSights - the uplink: see uplink.h.
#include "uplink.h"
#include <math.h>
#include "airtemp.h"
#include "ble_link.h"
#include "calib.h"
#include "capture.h"
#include "config.h"
#include "protocol.h"

static uint32_t bootId;

// ---- Shots ----

static proto::Shot shots[SHOT_BUFFER];
static uint32_t lastSeq = 0;      // 0: no shot yet
static uint32_t sendNext = 1;     // next shot to indicate; lastSeq + 1 when the app has them all
static uint32_t inFlight = 0;     // the shot being indicated
static uint32_t retryAt = 0;
static uint32_t clearedSeq = 0;   // shots up to this one were cleared by the app
static uint32_t lastShotMs = 0;

static uint32_t oldestSeq() {
  uint32_t oldest = lastSeq >= (uint32_t)SHOT_BUFFER ? lastSeq - SHOT_BUFFER + 1 : 1;
  return oldest > clearedSeq ? oldest : clearedSeq + 1;
}

uint32_t uplinkShot(uint8_t kind, int32_t x, int32_t y, uint16_t sigma, uint8_t flags,
                    uint32_t eventSeq, uint32_t nowMs) {
  proto::Shot& s = shots[(++lastSeq) % SHOT_BUFFER];
  uint32_t calId;
  bool stale;
  s = {lastSeq, bootId, nowMs, x, y, eventSeq, calibRecord(&calId, &stale) ? calId : 0, sigma, kind, flags};
  lastShotMs = nowMs;
  return lastSeq;
}

uint32_t uplinkLastShotMs() { return lastShotMs; }

// ---- Events ----

static uint8_t  events[EVENT_BUFFER][proto::EVENT_LEN];
static uint32_t lastEv = 0, evNext = 1, evInFlight = 0, evRetryAt = 0;

static uint32_t oldestEv() { return lastEv >= (uint32_t)EVENT_BUFFER ? lastEv - EVENT_BUFFER + 1 : 1; }

uint32_t uplinkEvent(proto::Event& e, uint32_t nowMs) {
  e.boot = bootId;
  e.seq = ++lastEv;
  e.time = nowMs;
  proto::encodeEvent(e, events[lastEv % EVENT_BUFFER]);
  return lastEv;
}

// The air and the calibration as every message has them.
static uint8_t measuredBits(const Air& a) {
  return (a.tMeasured ? proto::MEAS_T : 0) | (a.rhMeasured ? proto::MEAS_RH : 0) |
         (a.pMeasured ? proto::MEAS_P : 0);
}

// An event with no fit (hole, missed): the air, the settings, the calibration; no edges.
static void bareEvent(proto::Event& e, uint8_t kind, uint64_t tNs, uint32_t n) {
  e = {};
  e.kind = kind;
  e.verdict = e.acoustic = proto::VERDICT_NONE;
  e.curtain = -1;
  e.n = n;
  e.tNs = tNs;
  e.cfg = calibId();
  uint32_t calId;
  bool stale;
  const bool haveCal = calibRecord(&calId, &stale);
  e.cal = haveCal ? calId : 0;
  e.flags = (calibMode() ? proto::EV_CAL_MODE : 0) | (haveCal && stale ? proto::EV_CAL_STALE : 0) |
            (calibCfg().curtainHiNs > 0 ? proto::EV_CURTAIN_ON : 0);
  e.dropped = -1;
  e.rmsNs = e.sigmaMm = NAN;
  const Air a = air();
  e.tC = a.tC;
  e.rh = a.rh;
  e.hPa = a.hPa;
  e.c = a.c;
  e.measured = measuredBits(a);
  for (int i = 0; i < 8; i++) e.t1[i] = e.t2[i] = proto::NO_VALUE;
  float x, y;
  if (calibMode() && calibPoint(&x, &y)) {
    e.flags |= proto::EV_HAS_PT;
    e.ptX = x;
    e.ptY = y;
  }
}

static uint8_t bareShotFlags(const proto::Event& e) {
  return (e.measured != (proto::MEAS_T | proto::MEAS_RH | proto::MEAS_P) ? proto::SHOT_AIR_ASSUMED : 0) |
         (e.flags & proto::EV_CAL_STALE ? proto::SHOT_CAL_STALE : 0);
}

uint32_t uplinkHole(uint64_t tNs, int32_t lenNs, uint32_t nowMs) {
  proto::Event e;
  bareEvent(e, proto::KIND_HOLE, tNs, 0);
  e.dips = 1;   // its rise is the event's time itself
  e.dipLen[0] = lenNs;
  const bool hit = !calibMode();
  if (hit) e.flags |= proto::EV_SENT;
  const uint32_t ev = uplinkEvent(e, nowMs);
  return hit ? uplinkShot(proto::KIND_HOLE, 0, 0, proto::SIGMA_UNKNOWN, bareShotFlags(e), ev, nowMs) : 0;
}

uint32_t uplinkMissed(uint32_t eventN, uint32_t nowMs) {
  proto::Event e;
  bareEvent(e, proto::KIND_MISSED, captureSamplesNow() * (1000000000ULL / CAPTURE_SAMPLE_HZ), eventN);
  const bool hit = !calibMode();
  if (hit) e.flags |= proto::EV_SENT;
  const uint32_t ev = uplinkEvent(e, nowMs);
  return hit ? uplinkShot(proto::KIND_MISSED, 0, 0, proto::SIGMA_UNKNOWN, bareShotFlags(e), ev, nowMs) : 0;
}

// ---- Delivery ----

// One stream's requests and confirmations (shots also take a clear request).
static void takeShotRequests(uint32_t now) {
  if (bleTakeSubscribed(BLE_SHOT)) sendNext = lastSeq + 1;   // a fresh subscription: new shots only
  uint8_t req[16];
  size_t len;
  proto::Request q;
  if (bleTakeRequest(BLE_SHOT, req, &len)) {
    if (proto::decodeRequest(req, len, q)) {
      // From the shot after the app's last one; a different boot means it has none of ours.
      uint32_t from = q.boot == bootId ? q.next : 1;
      sendNext = constrain(from, oldestSeq(), lastSeq + 1);
    } else if (proto::decodeClear(req, len, q) && q.boot == bootId) {
      // The app cleared its series: forget the shots it has (never ones it hasn't seen).
      uint32_t upTo = q.next > lastSeq ? lastSeq : q.next;
      if (upTo > clearedSeq) clearedSeq = upTo;
      Serial.printf("Shots up to %u cleared\n", (unsigned)clearedSeq);
    }
  }
  switch (bleTakeResult(BLE_SHOT)) {
    case 1:  if (sendNext == inFlight) sendNext++; break;   // (unless a request moved it)
    case -1: retryAt = now + SHOT_RETRY_MS; break;
  }
  if (sendNext < oldestSeq()) sendNext = oldestSeq();   // overwritten meanwhile: lost
}

static void takeEventRequests(uint32_t now) {
  if (bleTakeSubscribed(BLE_EVENT)) evNext = lastEv + 1;
  uint8_t req[16];
  size_t len;
  proto::Request q;
  if (bleTakeRequest(BLE_EVENT, req, &len) && proto::decodeRequest(req, len, q)) {
    uint32_t from = q.boot == bootId ? q.next : 1;
    evNext = constrain(from, oldestEv(), lastEv + 1);
  }
  switch (bleTakeResult(BLE_EVENT)) {
    case 1:  if (evNext == evInFlight) evNext++; break;
    case -1: evRetryAt = now + SHOT_RETRY_MS; break;
  }
  if (evNext < oldestEv()) evNext = oldestEv();
}

// Shots first: the score never waits behind the diagnostics.
static void send(uint32_t now) {
  const uint16_t mtu = bleMtu();
  if (sendNext <= lastSeq && bleReady(BLE_SHOT)) {
    if (mtu < MTU_SHOTS || (int32_t)(now - retryAt) < 0) return;
    uint8_t buf[proto::SHOT_LEN];
    proto::encodeShot(shots[sendNext % SHOT_BUFFER], buf);
    if (bleSend(BLE_SHOT, buf, sizeof buf)) inFlight = sendNext;
    else retryAt = now + SHOT_RETRY_MS;
    return;
  }
  if (evNext <= lastEv && bleReady(BLE_EVENT) && mtu >= MTU_EVENTS && (int32_t)(now - evRetryAt) >= 0) {
    if (bleSend(BLE_EVENT, events[evNext % EVENT_BUFFER], proto::EVENT_LEN)) evInFlight = evNext;
    else evRetryAt = now + SHOT_RETRY_MS;
  }
}

// ---- Status, Calibration, Settings ----

static uint8_t lastStatus[proto::STATUS_LEN];
static uint8_t lastCalibration[proto::CALIBRATION_LEN];
static uint8_t lastSettings[proto::SETTINGS_LEN];
static bool    published = false;
static uint32_t nextStatusAt = 0;

static void buildStatus(uint8_t* out) {
  const measure::Cfg& cfg = calibCfg();
  proto::Status s = {};
  s.boot = bootId;
  s.cfg = calibId();
  uint32_t calId;
  bool stale;
  const bool haveCal = calibRecord(&calId, &stale);
  s.cal = haveCal ? calId : 0;
  float x, y;
  const bool pt = calibPoint(&x, &y);
  const Air a = air();
  const uint16_t mtu = bleMtu();
  s.flags = (calibMode() ? proto::ST_CAL_MODE : 0) | (haveCal && stale ? proto::ST_CAL_STALE : 0) |
            (pt ? proto::ST_POINT : 0) | (cfg.curtainHiNs > 0 ? proto::ST_CURTAIN_ON : 0) |
            (mtu && mtu < MTU_EVENTS ? proto::ST_MTU_SMALL : 0) |
            (a.sensor != AIR_NONE ? proto::ST_AIR_SENSOR : 0);
  s.measured = measuredBits(a);
  for (int c = 0; c < 8; c++)
    if (cfg.ch[c].mic) s.mics |= 1 << c;
  s.ptX = pt ? x : 0;
  s.ptY = pt ? y : 0;
  s.tC = a.tC;
  s.rh = a.rh;
  s.hPa = a.hPa;
  s.c = a.c;
  s.events = captureEventCount();
  s.missed = captureMissedCount();
  s.shots = lastSeq;
  proto::encodeStatus(s, out);
}

static void buildCalibration(uint8_t* out) {
  proto::Calibration c = {};
  calibRecordInfo(&c.date, &c.points, &c.sparks, &c.resNs);
  for (int i = 0; i < 8; i++) c.ch[i] = calibCfg().ch[i];
  proto::encodeCalibration(c, out);
}

static void buildSettings(uint8_t* out) {
  const measure::Cfg& cfg = calibCfg();
  const proto::Settings s = {cfg.dropOne, cfg.maxLost, captureMask(), cfg.resLimitNs, cfg.areaMm,
                             cfg.togetherNs, cfg.pulseMaxNs, cfg.curtainLoNs, cfg.curtainHiNs,
                             (uint16_t)captureSpanMm(), (uint16_t)captureDeadtimeMs()};
  proto::encodeSettings(s, out);
}

// Rebuilt every STATUS_PERIOD_MS (or at once after a change from the app); Status is notified
// only when it changed, the others are just kept current for reads.
static void publish(uint32_t now, bool force) {
  if (!force && published && (int32_t)(now - nextStatusAt) < 0) return;
  nextStatusAt = now + STATUS_PERIOD_MS;
  uint8_t st[proto::STATUS_LEN], cal[proto::CALIBRATION_LEN], set[proto::SETTINGS_LEN];
  buildStatus(st);
  buildCalibration(cal);
  buildSettings(set);
  if (!published || memcmp(st, lastStatus, sizeof st)) {
    memcpy(lastStatus, st, sizeof st);
    bleSetStatus(st, sizeof st);
  }
  if (force || !published || memcmp(cal, lastCalibration, sizeof cal)) {
    memcpy(lastCalibration, cal, sizeof cal);
    bleSetCalibration(cal, sizeof cal);
  }
  if (force || !published || memcmp(set, lastSettings, sizeof set)) {
    memcpy(lastSettings, set, sizeof set);
    bleSetSettings(set, sizeof set);
  }
  published = true;
}

static float getF32(const uint8_t* p) {
  uint32_t b = (uint32_t)p[0] | p[1] << 8 | p[2] << 16 | (uint32_t)p[3] << 24;
  float v;
  memcpy(&v, &b, 4);
  return v;
}

// What the app wrote. Each change prints on serial too, as the serial commands do.
static bool takeWrites() {
  bool changed = false;
  uint8_t buf[256];
  size_t len;
  while (bleTakeControl(buf, &len)) {
    changed = true;
    if (len == 2 && buf[0] == proto::CTL_CAL_MODE && buf[1] <= 1) {
      Serial.printf("BLE: calibration mode %s (from the app)\n", buf[1] ? "on" : "off");
      calibSetMode(buf[1]);
    } else if (len == 9 && buf[0] == proto::CTL_POINT && fabsf(getF32(buf + 1)) <= 1000 &&
               fabsf(getF32(buf + 5)) <= 1000) {
      calibSetPoint(true, getF32(buf + 1), getF32(buf + 5));
    } else if (len == 1 && buf[0] == proto::CTL_NO_POINT) {
      calibSetPoint(false, 0, 0);
    } else {
      Serial.printf("BLE: control command ignored (%u bytes, 0x%02X)\n", (unsigned)len, len ? buf[0] : 0);
    }
  }
  if (bleTakeCalibrationWrite(buf, &len)) {
    changed = true;
    proto::Calibration c;
    if (proto::decodeCalibration(buf, len, c) && proto::calibrationValid(c)) {
      Serial.println("BLE: a calibration record from the app");
      calibSetChannels(c.ch, c.date, c.points, c.sparks, c.resNs);
    } else {
      Serial.printf("BLE: calibration write rejected (%u bytes: wrong length, layout or a value out "
                    "of range); nothing changed\n", (unsigned)len);
    }
  }
  if (bleTakeSettingsWrite(buf, &len)) {
    changed = true;
    proto::Settings s;
    if (proto::decodeSettings(buf, len, s) && proto::settingsValid(s, captureSpanMaxMm()) &&
        captureSetSettings(s.mask, s.spanMm, s.deadtimeMs)) {
      measure::Cfg p = calibCfg();
      p.resLimitNs = s.resLimitNs;
      p.dropOne = s.dropOne;
      p.maxLost = s.maxLost;
      p.areaMm = s.areaMm;
      p.togetherNs = s.togetherNs;
      p.pulseMaxNs = s.pulseMaxNs;
      p.curtainLoNs = s.curtainLoNs;
      p.curtainHiNs = s.curtainHiNs;
      Serial.println("BLE: settings from the app");
      calibSetPolicy(p);
    } else {
      Serial.printf("BLE: settings write rejected (%u bytes: wrong length, layout, a value out of "
                    "range, or the mask during the self-test or simulator); nothing changed\n",
                    (unsigned)len);
    }
  }
  return changed;
}

// ---- Public ----

void uplinkBegin(uint32_t id) { bootId = id; }
uint32_t uplinkBoot() { return bootId; }

void uplinkService(uint32_t now) {
  takeShotRequests(now);
  takeEventRequests(now);
  send(now);
  publish(now, takeWrites());
}
