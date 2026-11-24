// OpenSights - the uplink: see uplink.h.
#include "uplink.h"
#include <Preferences.h>
#include <esp_partition.h>
#include <math.h>
#include "airtemp.h"
#include "ble_link.h"
#include "calib.h"
#include "capture.h"
#include "config.h"
#include "protocol.h"
#include "shotstore.h"

static uint32_t bootId;

// ---- Shots: the store (lib/store) ----

// The `shots` partition: reads through the memory map (plain cached reads, no flash lock),
// writes and erases through the partition API (which keeps the cache right). Timed for `shots`.
class PartFlash : public store::Flash {
 public:
  bool open() {
    part = esp_partition_find_first(ESP_PARTITION_TYPE_DATA,
                                    (esp_partition_subtype_t)STORE_PART_SUBTYPE, "shots");
    if (!part) return false;
    const void* p;
    if (esp_partition_mmap(part, 0, part->size, ESP_PARTITION_MMAP_DATA, &p, &map) != ESP_OK) {
      part = nullptr;
      return false;
    }
    base = (const uint8_t*)p;
    return true;
  }
  uint32_t size() const override { return part ? part->size : 0; }
  bool read(uint32_t off, void* buf, uint32_t n) override {
    if (off + n > size()) return false;
    memcpy(buf, base + off, n);
    return true;
  }
  bool write(uint32_t off, const void* buf, uint32_t n) override {
    const uint32_t t = micros();
    const bool ok = esp_partition_write(part, off, buf, n) == ESP_OK;
    note(micros() - t, writeMaxUs);
    return ok;
  }
  bool erase(uint32_t off, uint32_t n) override {
    const uint32_t t = micros();
    const bool ok = esp_partition_erase_range(part, off, n) == ESP_OK;
    eraseLastUs = micros() - t;
    note(eraseLastUs, eraseMaxUs);
    return ok;
  }
  uint32_t writeMaxUs = 0, eraseMaxUs = 0, eraseLastUs = 0;

 private:
  static void note(uint32_t us, uint32_t& max) {
    if (us > max) max = us;
  }
  const esp_partition_t* part = nullptr;
  esp_partition_mmap_handle_t map;
  const uint8_t* base = nullptr;
};

static PartFlash partFlash;
static store::RamFlash* ramFlash = nullptr;   // the fallback, no `shots` partition
static store::ShotStore shotStore;
static Preferences storePrefs;                // NVS "shots": "clr", the cleared H
static uint32_t quietErases = 0;              // erases done at a quiet moment (for `shots`)

static uint32_t lastSeq = 0;      // this boot's hits (the protocol's seq); 0: none yet
static uint32_t sendNext = 1;     // the next hit to indicate, as a store number H
static uint32_t inFlight = 0;     // the hit being indicated (H)
static uint32_t retryAt = 0;
static uint32_t lastShotMs = 0;
static uint32_t lastActivityMs = 0;   // the newest shot or event: the quiet erase waits

static void storeBegin() {
  storePrefs.begin("shots");
  const uint32_t clr = storePrefs.getUInt("clr", 0);
  store::Flash* f = &partFlash;
  if (!partFlash.open()) {
    uint8_t* ram = (uint8_t*)malloc(STORE_RAM_SECTORS * store::SECTOR);
    if (ram) memset(ram, 0xFF, STORE_RAM_SECTORS * store::SECTOR);
    ramFlash = ram ? new store::RamFlash(ram, STORE_RAM_SECTORS * store::SECTOR) : nullptr;
    f = ramFlash;
    Serial.println("shots: no `shots` partition (an old partition table? flash with `pio run -t "
                   "upload`): hits kept in RAM only, lost at a reboot");
  }
  const uint32_t t = millis();
  if (!f || !shotStore.begin(f, clr)) {
    Serial.println("shots: the store didn't start: hits are not kept");
    return;
  }
  const store::ScanInfo& i = shotStore.scan();
  Serial.printf("shots: %u hits held from %u boot(s), %u torn, capacity %u, read in %u ms%s\n",
                (unsigned)i.valid, (unsigned)i.boots, (unsigned)i.torn,
                (unsigned)shotStore.capacity(), (unsigned)(millis() - t),
                f == &partFlash ? "" : " (RAM)");
  sendNext = shotStore.end();
}

uint32_t uplinkShot(uint8_t kind, int32_t x, int32_t y, uint16_t sigma, uint8_t flags,
                    uint32_t eventSeq, uint32_t nowMs) {
  uint32_t calId;
  bool stale;
  const proto::Shot s = {++lastSeq, bootId, nowMs, x, y, eventSeq,
                         calibRecord(&calId, &stale) ? calId : 0, sigma, kind, flags};
  uint8_t buf[proto::SHOT_LEN];
  proto::encodeShot(s, buf);
  if (!shotStore.append(bootId, lastSeq, buf))
    Serial.printf("shots: hit %u NOT stored (the flash write failed)\n", (unsigned)lastSeq);
  lastShotMs = lastActivityMs = nowMs;
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
  lastActivityMs = nowMs;
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
  if (bleTakeSubscribed(BLE_SHOT)) sendNext = shotStore.end();   // a fresh subscription: new hits only
  uint8_t req[16];
  size_t len;
  proto::Request q;
  if (bleTakeRequest(BLE_SHOT, req, &len)) {
    if (proto::decodeRequest(req, len, q)) {
      // From the app's next hit of that boot on, then the later boots'; a boot the store has
      // no hits of (0: the app has none): every hit not cleared.
      sendNext = shotStore.from(q.boot, q.next);
    } else if (proto::decodeClear(req, len, q)) {
      // The app cleared its series: forget the hits it has, up to (boot, seq), older boots too.
      const uint32_t c = shotStore.clearTo(q.boot, q.next);
      if (c) {
        storePrefs.putUInt("clr", c);
        Serial.printf("Shots up to %u of boot %08X cleared\n", (unsigned)q.next, (unsigned)q.boot);
      } else {
        Serial.printf("Shot clear for boot %08X ignored: no hits of it held\n", (unsigned)q.boot);
      }
    }
  }
  switch (bleTakeResult(BLE_SHOT)) {
    case 1:  if (sendNext == inFlight) sendNext = shotStore.next(inFlight); break;   // (unless a request moved it)
    case -1: retryAt = now + SHOT_RETRY_MS; break;
  }
  if (sendNext < shotStore.floor()) sendNext = shotStore.floor();   // cleared, or dropped out
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
  if (sendNext < shotStore.end() && bleReady(BLE_SHOT)) {
    if (mtu < MTU_SHOTS || (int32_t)(now - retryAt) < 0) return;
    store::Hit h;
    if (!shotStore.read(sendNext, h)) {   // a slot a power cut tore, or gone: the next one
      sendNext = shotStore.next(sendNext);
      return;
    }
    if (bleSend(BLE_SHOT, h.payload, proto::SHOT_LEN)) inFlight = sendNext;
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

void uplinkBegin(uint32_t id) {
  bootId = id;
  storeBegin();
}
uint32_t uplinkBoot() { return bootId; }

// The sector ahead of the store's writing, erased at a quiet moment: no shot or event for
// STORE_QUIET_MS. (Hits waiting to be sent don't hold it back: with no app connected they
// wait for good, and an erase only delays the next indication by about 45 ms.)
static void storeService(uint32_t now) {
  if (!shotStore.erasePending() || (int32_t)(now - lastActivityMs) < (int32_t)STORE_QUIET_MS) return;
  if (shotStore.eraseAhead()) quietErases++;
}

void uplinkService(uint32_t now) {
  storeService(now);
  takeShotRequests(now);
  takeEventRequests(now);
  send(now);
  publish(now, takeWrites());
}

// ---- Serial ----

void uplinkHelp() {
  Serial.println("  shots                the shot store: hits held, boots, cleared, timings\n"
                 "  shots clear          forget every hit held (as the app's clear, all boots)");
}

bool uplinkCommand(const char* cmd, const char* line) {
  if (strcmp(cmd, "shots")) return false;
  char arg[12] = "";
  sscanf(line, "%*s %11s", arg);
  if (!strcmp(arg, "clear")) {
    store::Hit h;
    const uint32_t last = shotStore.end() - 1;
    if (last && shotStore.read(last, h) && shotStore.clearTo(h.boot, h.seq)) {
      storePrefs.putUInt("clr", shotStore.clearedH());
      Serial.printf("shots: all cleared (up to H %u)\n", (unsigned)last);
    } else {
      Serial.println("shots: nothing to clear");
    }
    return true;
  }
  const store::ScanInfo& i = shotStore.scan();
  Serial.printf("shots: %u held (H %u to %u), cleared up to H %u, next to send H %u; this boot "
                "%08X: %u hits; capacity %u (%s)\n",
                (unsigned)shotStore.held(), (unsigned)shotStore.floor(),
                (unsigned)(shotStore.end() - 1), (unsigned)shotStore.clearedH(), (unsigned)sendNext,
                (unsigned)bootId, (unsigned)lastSeq, (unsigned)shotStore.capacity(),
                ramFlash ? "RAM" : "flash");
  Serial.printf("shots: at boot %u valid, %u torn, %u boots; erase ahead %s, %u done quietly; "
                "slowest write %u us, erase %u us (last %u us)\n",
                (unsigned)i.valid, (unsigned)i.torn, (unsigned)i.boots,
                shotStore.erasePending() ? "pending" : "none due", (unsigned)quietErases,
                (unsigned)partFlash.writeMaxUs, (unsigned)partFlash.eraseMaxUs,
                (unsigned)partFlash.eraseLastUs);
  return true;
}
