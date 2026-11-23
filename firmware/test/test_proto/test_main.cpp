// OpenSights - the BLE protocol's messages (lib/proto) on the PC: `pio test -e native`.
// The byte examples are protocol/PROTOCOL.md's; app/src/protocol.rs checks the same ones.
#include <unity.h>

#include <cmath>
#include <cstdio>
#include <cstring>
#include <string>
#include <vector>

#include "proto.h"
#include "record.h"

using namespace proto;

void setUp() {}
void tearDown() {}

static std::vector<uint8_t> bytes(const char* hex) {
  std::vector<uint8_t> v;
  unsigned b;
  int n;
  while (sscanf(hex, " %2x%n", &b, &n) == 1) {
    v.push_back((uint8_t)b);
    hex += n;
  }
  return v;
}

static void expectBytes(const char* hex, const uint8_t* got, size_t len) {
  const std::vector<uint8_t> want = bytes(hex);
  TEST_ASSERT_EQUAL_size_t(want.size(), len);
  TEST_ASSERT_EQUAL_HEX8_ARRAY(want.data(), got, len);
}

// ---- PROTOCOL.md's examples ----

static const char* SHOT_HEX =
    "03 00 00 00 78 56 34 12 39 30 00 00 7D 00 00 00 06 FF FF FF 12 00 00 00 4D 3C 2B 1A 08 00 00 08";
static const char* HOLE_HEX =
    "04 00 00 00 78 56 34 12 C8 32 00 00 00 00 00 00 00 00 00 00 13 00 00 00 4D 3C 2B 1A FF FF 01 00";

// The event: rec n=21 of the 4b bench log, solved with its cfg.
static const char* CFG_LINE =
    "cfg id=4A87309E CH1=- CH2=- CH3=-105,-105,10.5,0.333330005,0,50 CH4=105,-105,10.5,0.333330005,0,50 "
    "CH5=65,105,10.5,0.333330005,0,50 CH6=-105,105,10.5,0.333330005,0,50 CH7=0,-105,10.5,0.333330005,0,50 "
    "CH8=105,0,10.5,0.333330005,0,50 lim=1500 drop=1 lost=1 area=105 tog=20000 pmax=10000 ir=45000,100000";
static const char* REC_LINE =
    "rec n=21 t=30.9781094 cfg=4A87309E T=18.00 RH=50.0 p=1013.25 meas=--- c=342.739716 "
    "CH3=320400,321500 CH4=214100,215000 CH5=5800,6200 CH6=214100,215000 CH7=167100,167900 CH8=0,500 "
    "pt=30.0000,30.0000 cal=- ir=-160725/27075";
static const char* EVENT_HEX =
    "00 00 00 01 78 56 34 12 12 00 00 00 15 00 00 00 39 30 00 00 D8 6F 70 36 07 00 00 00 9E 30 87 4A "
    "00 00 00 00 3C FC FC FF B7 0B 00 00 B7 0B 00 00 E0 B7 98 41 FF F5 2F 3C 00 00 90 41 00 00 48 42 "
    "00 50 7D 44 AF 5E AB 43 00 01 00 00 00 00 00 80 00 00 00 80 90 E3 04 00 54 44 03 00 A8 16 00 00 "
    "54 44 03 00 BC 8C 02 00 00 00 00 00 00 00 00 80 00 00 00 80 DC E7 04 00 D8 47 03 00 38 18 00 00 "
    "D8 47 03 00 DC 8F 02 00 F4 01 00 00 2B 8C FD FF 00 00 00 00 00 00 00 00 00 00 00 00 C3 69 00 00 "
    "00 00 00 00 00 00 00 00 00 00 00 00 00 00 F0 41 00 00 F0 41";
static const char* STATUS_HEX =
    "78 56 34 12 9E 30 87 4A 00 00 00 00 08 00 FC 00 00 00 00 00 00 00 00 00 00 00 90 41 00 00 48 42 "
    "00 50 7D 44 AF 5E AB 43 15 00 00 00 00 00 00 00 15 00 00 00";
static const char* SETTINGS_HEX =
    "01 01 01 3F 00 80 BB 44 00 00 D2 42 00 40 9C 46 00 40 1C 46 00 C8 2F 47 00 50 C3 47 2C 01 1E 00 "
    "00 00 00 00";
static const char* CALIBRATION_HEX =
    "01 03 29 35 01 00 00 2A 42 14 00 A0 00 00 00 00 00 00 00 00 00 00 00 00 28 41 3B AA AA 3E 00 00 "
    "00 00 00 00 48 42 00 00 00 00 00 00 00 00 00 00 00 28 41 3B AA AA 3E 00 00 00 00 00 00 48 42 01 "
    "00 00 D2 C2 00 00 D2 C2 00 00 28 41 3B AA AA 3E 00 00 00 00 00 00 48 42 01 00 00 D2 42 00 00 D2 "
    "C2 00 00 28 41 3B AA AA 3E 00 00 00 00 00 00 48 42 01 00 00 82 42 00 00 D2 42 00 00 28 41 3B AA "
    "AA 3E 00 00 00 00 00 00 48 42 01 00 00 D2 C2 00 00 D2 42 00 00 28 41 3B AA AA 3E 00 00 00 00 00 "
    "00 48 42 01 00 00 00 00 00 00 D2 C2 00 00 28 41 3B AA AA 3E 00 00 00 00 00 00 48 42 01 00 00 D2 "
    "42 00 00 00 00 00 00 28 41 3B AA AA 3E 00 00 00 00 00 00 48 42";

static const Shot SHOT = {3, 0x12345678, 12345, 125, -250, 18, 0x1A2B3C4D, 8, KIND_SHOT, SHOT_CURTAIN};
static const Shot HOLE = {4, 0x12345678, 13000, 0, 0, 19, 0x1A2B3C4D, SIGMA_UNKNOWN, KIND_HOLE, 0};

static Settings defaults() {
  return {true, 1, 0x3F, 1500, 105, 20000, 10000, 45000, 100000, 300, 30};
}

static Calibration exampleCalibration() {
  measure::Cfg cfg;
  uint32_t id;
  TEST_ASSERT_TRUE(measure::parseCfg(CFG_LINE, cfg, &id));
  Calibration c = {};
  c.date = 20261123;
  c.resNs = 42.5f;
  c.points = 20;
  c.sparks = 160;
  for (int i = 0; i < 8; i++) {
    c.ch[i] = cfg.ch[i];
    if (!c.ch[i].mic) c.ch[i] = {false, 0, 0, 10.5f, 0.333330005f, 0, 50};
  }
  return c;
}

static void test_shot() {
  uint8_t b[SHOT_LEN];
  encodeShot(SHOT, b);
  expectBytes(SHOT_HEX, b, sizeof b);
  encodeShot(HOLE, b);
  expectBytes(HOLE_HEX, b, sizeof b);
  Shot s;
  TEST_ASSERT_TRUE(decodeShot(b, sizeof b, s));
  TEST_ASSERT_EQUAL_UINT32(4, s.seq);
  TEST_ASSERT_EQUAL_UINT8(KIND_HOLE, s.kind);
  TEST_ASSERT_EQUAL_UINT16(SIGMA_UNKNOWN, s.sigma);
  TEST_ASSERT_FALSE(decodeShot(b, 20, s));   // a v2 shot
}

static void test_event_from_rec() {
  measure::Cfg cfg;
  uint32_t id;
  measure::Rec rec;
  TEST_ASSERT_TRUE(measure::parseCfg(CFG_LINE, cfg, &id));
  TEST_ASSERT_TRUE(measure::parseRec(REC_LINE, rec));
  const measure::EventFit f = measure::solveEvent(cfg, rec);
  TEST_ASSERT_EQUAL(measure::V_OK, f.verdict);
  Event e;
  eventFromRec(rec, f, e);
  e.boot = 0x12345678;
  e.seq = 18;
  e.time = 12345;
  e.flags |= EV_SENT;
  // The fit's floats depend on the FPU (FMA on the ESP32 and in the example, maybe not here):
  // close to the example's, then the example's own, so the bytes are compared exactly.
  Event want;
  const std::vector<uint8_t> wb = bytes(EVENT_HEX);
  TEST_ASSERT_TRUE(decodeEvent(wb.data(), wb.size(), want));
  TEST_ASSERT_FLOAT_WITHIN(0.01f, want.rmsNs, e.rmsNs);
  TEST_ASSERT_FLOAT_WITHIN(0.0001f, want.sigmaMm, e.sigmaMm);
  TEST_ASSERT_INT32_WITHIN(1, want.x, e.x);
  TEST_ASSERT_INT32_WITHIN(1, want.y, e.y);
  e.rmsNs = want.rmsNs;
  e.sigmaMm = want.sigmaMm;
  e.x = want.x;
  e.y = want.y;
  uint8_t b[EVENT_LEN];
  encodeEvent(e, b);
  expectBytes(EVENT_HEX, b, sizeof b);

  // Back: the message gives the same rec line, so an app's log replays like a serial one.
  Event d;
  TEST_ASSERT_TRUE(decodeEvent(b, sizeof b, d));
  TEST_ASSERT_EQUAL_INT8(1, d.curtain);
  TEST_ASSERT_EQUAL_HEX8(0xFC, d.fitted);
  TEST_ASSERT_EQUAL_INT8(-1, d.dropped);
  TEST_ASSERT_EQUAL_INT32(2999, d.x);
  measure::Rec back;
  TEST_ASSERT_TRUE(recFromEvent(d, back));
  char line[400];
  measure::formatRec(line, sizeof line, back);
  TEST_ASSERT_EQUAL_STRING(REC_LINE, line);
  const measure::EventFit g = measure::solveEvent(cfg, back);
  TEST_ASSERT_EQUAL(f.verdict, g.verdict);
  TEST_ASSERT_EQUAL_FLOAT(f.s.x, g.s.x);   // the same record, the same FPU: the same fit
  TEST_ASSERT_EQUAL_FLOAT(f.s.y, g.s.y);
}

// Every rec line shape round-trips: no edge, no dips, the curtain off, a calibration, stale.
static void test_event_rec_variants() {
  measure::Cfg cfg;
  uint32_t id;
  TEST_ASSERT_TRUE(measure::parseCfg(CFG_LINE, cfg, &id));
  const char* lines[] = {
      "rec n=7 t=1.0000001 cfg=4A87309E T=21.37 RH=43.2 p=998.40 meas=TRP c=344.012329 CH3=-,321500 "
      "CH4=214100,- CH5=5800,6200 pt=-12.5000,40.2500 mode=cal cal=746AAA08 stale=0",
      "rec n=8 t=2.5000000 cfg=4A87309E T=18.00 RH=50.0 p=1013.25 meas=T-- c=342.739716 CH3=0,100 "
      "cal=746AAA08 stale=1 ir=-",
      "rec n=9 t=3.0000000 cfg=4A87309E T=18.00 RH=50.0 p=1013.25 meas=--- c=342.739716 CH8=0,500 "
      "cal=- ir=-160725/27075,90000/-,120000/1000,130000/2000",
  };
  for (const char* l : lines) {
    measure::Rec rec, back;
    TEST_ASSERT_TRUE_MESSAGE(measure::parseRec(l, rec), l);
    const measure::EventFit f = measure::solveEvent(cfg, rec);
    Event e, d;
    eventFromRec(rec, f, e);
    uint8_t b[EVENT_LEN];
    encodeEvent(e, b);
    TEST_ASSERT_TRUE(decodeEvent(b, sizeof b, d));
    TEST_ASSERT_TRUE(recFromEvent(d, back));
    char out[400];
    measure::formatRec(out, sizeof out, back);
    TEST_ASSERT_EQUAL_STRING(l, out);
  }
}

static void test_status() {
  const Status s = {0x12345678, 0x4A87309E, 0, ST_CURTAIN_ON, 0, 0xFC, 0, 0, 18.0f, 50.0f, 1013.25f,
                    342.739716f, 21, 0, 21};
  uint8_t b[STATUS_LEN];
  encodeStatus(s, b);
  expectBytes(STATUS_HEX, b, sizeof b);
  Status d;
  TEST_ASSERT_TRUE(decodeStatus(b, sizeof b, d));
  TEST_ASSERT_EQUAL_UINT32(21, d.shots);
  TEST_ASSERT_EQUAL_FLOAT(342.739716f, d.c);
}

static void test_settings() {
  uint8_t b[SETTINGS_LEN];
  encodeSettings(defaults(), b);
  expectBytes(SETTINGS_HEX, b, sizeof b);
  Settings d;
  TEST_ASSERT_TRUE(decodeSettings(b, sizeof b, d));
  TEST_ASSERT_TRUE(settingsValid(d, 400));
  TEST_ASSERT_EQUAL_UINT16(300, d.spanMm);
  TEST_ASSERT_EQUAL_FLOAT(100000, d.curtainHiNs);
  TEST_ASSERT_FALSE(decodeSettings(b, sizeof b - 1, d));
  b[0] = 2;   // an unknown layout
  TEST_ASSERT_FALSE(decodeSettings(b, sizeof b, d));
  // The limits: each one out of range on its own.
  Settings s = defaults();
  TEST_ASSERT_FALSE(settingsValid(s, 250));   // span over this board's maximum
  s = defaults(); s.resLimitNs = 0;            TEST_ASSERT_FALSE(settingsValid(s, 400));
  s = defaults(); s.maxLost = 5;               TEST_ASSERT_FALSE(settingsValid(s, 400));
  s = defaults(); s.maxLost = -1;              TEST_ASSERT_TRUE(settingsValid(s, 400));
  s = defaults(); s.areaMm = 5;                TEST_ASSERT_FALSE(settingsValid(s, 400));
  s = defaults(); s.areaMm = 0;                TEST_ASSERT_TRUE(settingsValid(s, 400));
  s = defaults(); s.curtainLoNs = s.curtainHiNs = 0; TEST_ASSERT_TRUE(settingsValid(s, 400));
  s = defaults(); s.curtainLoNs = 100000;      TEST_ASSERT_FALSE(settingsValid(s, 400));
  s = defaults(); s.deadtimeMs = 0;            TEST_ASSERT_FALSE(settingsValid(s, 400));
  s = defaults(); s.pulseMaxNs = NAN;          TEST_ASSERT_FALSE(settingsValid(s, 400));
}

static void test_calibration() {
  const Calibration c = exampleCalibration();
  TEST_ASSERT_TRUE(calibrationValid(c));
  uint8_t b[CALIBRATION_LEN];
  encodeCalibration(c, b);
  expectBytes(CALIBRATION_HEX, b, sizeof b);
  Calibration d;
  TEST_ASSERT_TRUE(decodeCalibration(b, sizeof b, d));
  TEST_ASSERT_EQUAL_UINT32(20261123, d.date);
  TEST_ASSERT_TRUE(d.ch[2].mic);
  TEST_ASSERT_EQUAL_FLOAT(-105, d.ch[2].x);
  TEST_ASSERT_EQUAL_FLOAT(0.333330005f, d.ch[2].k);
  // Its id is the sensor's: the cfg line's channel fields.
  measure::Cfg cfg = {};
  for (int i = 0; i < 8; i++) cfg.ch[i] = d.ch[i];
  TEST_ASSERT_EQUAL_HEX32(0x746AAA08, measure::calibrationId(cfg));
  // Rejected whole: too few mics, a value out of range, a wrong length or layout.
  Calibration e = c;
  e.ch[2].mic = e.ch[3].mic = e.ch[4].mic = false;
  TEST_ASSERT_FALSE(calibrationValid(e));
  e = c; e.ch[5].k = 2.5f;          TEST_ASSERT_FALSE(calibrationValid(e));
  e = c; e.ch[0].sigmaNs = 0;       TEST_ASSERT_FALSE(calibrationValid(e));
  e = c; e.ch[6].h = NAN;           TEST_ASSERT_FALSE(calibrationValid(e));
  TEST_ASSERT_FALSE(decodeCalibration(b, sizeof b - 1, d));
  b[13] = 2;   // CH1's mic byte
  TEST_ASSERT_FALSE(decodeCalibration(b, sizeof b, d));
}

static void test_requests() {
  Request q;
  const std::vector<uint8_t> req = bytes("78 56 34 12 04 00 00 00");
  TEST_ASSERT_TRUE(decodeRequest(req.data(), req.size(), q));
  TEST_ASSERT_EQUAL_HEX32(0x12345678, q.boot);
  TEST_ASSERT_EQUAL_UINT32(4, q.next);
  TEST_ASSERT_FALSE(decodeRequest(req.data(), 7, q));
  const std::vector<uint8_t> clr = bytes("01 78 56 34 12 03 00 00 00");
  TEST_ASSERT_TRUE(decodeClear(clr.data(), clr.size(), q));
  TEST_ASSERT_EQUAL_UINT32(3, q.next);
  TEST_ASSERT_FALSE(decodeClear(req.data(), req.size(), q));
}

int main() {
  UNITY_BEGIN();
  RUN_TEST(test_shot);
  RUN_TEST(test_event_from_rec);
  RUN_TEST(test_event_rec_variants);
  RUN_TEST(test_status);
  RUN_TEST(test_settings);
  RUN_TEST(test_calibration);
  RUN_TEST(test_requests);
  return UNITY_END();
}
