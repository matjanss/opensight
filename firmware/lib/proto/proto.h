// OpenSights - the BLE protocol's messages, version 3 (../../../protocol/PROTOCOL.md), encoded
// and decoded. Portable: no Arduino, no IDF; built and tested on the PC (`pio test -e native`)
// against the byte examples in PROTOCOL.md, which app/src/protocol.rs checks too. The same
// bytes go over MQTT later (PLAN D7).
#pragma once
#include <stddef.h>
#include <stdint.h>
#include "record.h"

namespace proto {

const size_t SHOT_LEN = 32;
const size_t EVENT_LEN = 180;
const size_t STATUS_LEN = 52;
const size_t SETTINGS_LEN = 36;
const size_t CALIBRATION_LEN = 213;
const size_t REQUEST_LEN = 8;      // shot and event requests: boot u32, next u32
const size_t CLEAR_LEN = 9;        // shot clear: 0x01, boot u32, last seq u32
const uint8_t CLEAR = 0x01;
const uint8_t LAYOUT = 1;          // Settings and Calibration, byte 0
const int32_t NO_VALUE = (int32_t)0x80000000;   // an edge, dip or length not seen
const uint16_t SIGMA_UNKNOWN = 0xFFFF;

// Shot and event kinds.
enum Kind : uint8_t { KIND_SHOT = 0, KIND_HOLE = 1, KIND_MISSED = 2 };

// Shot flags.
const uint8_t SHOT_DROPPED = 0x01, SHOT_AIR_ASSUMED = 0x02, SHOT_CAL_STALE = 0x04, SHOT_CURTAIN = 0x08;
// Event flags.
const uint8_t EV_CAL_MODE = 0x01, EV_CAL_STALE = 0x02, EV_HAS_PT = 0x04, EV_CURTAIN_ON = 0x08,
              EV_SOLVED = 0x10, EV_SENT = 0x20;
const uint8_t VERDICT_NONE = 0xFF;   // kinds hole and missed
// Status flags.
const uint8_t ST_CAL_MODE = 0x01, ST_CAL_STALE = 0x02, ST_POINT = 0x04, ST_CURTAIN_ON = 0x08,
              ST_MTU_SMALL = 0x10, ST_AIR_SENSOR = 0x20;
// Measured bits (events and Status).
const uint8_t MEAS_T = 0x01, MEAS_RH = 0x02, MEAS_P = 0x04;
// Control commands.
const uint8_t CTL_CAL_MODE = 0x01, CTL_POINT = 0x02, CTL_NO_POINT = 0x03;

struct Shot {
  uint32_t seq, boot, time;
  int32_t  x, y;       // 0.01 mm
  uint32_t event;      // the event number (0: none)
  uint32_t cal;        // the calibration id (0: none)
  uint16_t sigma;      // 0.01 mm, SIGMA_UNKNOWN
  uint8_t  kind, flags;
};

struct Event {
  uint8_t  kind, verdict, acoustic;
  int8_t   curtain;
  uint32_t boot, seq, n, time;
  uint64_t tNs;        // the first edge, ns since the capture started
  uint32_t cfg, cal;
  uint8_t  flags, channels, fitted;
  int8_t   dropped;
  int32_t  x, y;       // 0.01 mm
  float    rmsNs, sigmaMm, tC, rh, hPa, c;
  uint8_t  measured, dips;
  int32_t  t1[8], t2[8], dip[4], dipLen[4];
  float    ptX, ptY;
};

struct Status {
  uint32_t boot, cfg, cal;
  uint8_t  flags, measured, mics;
  float    ptX, ptY, tC, rh, hPa, c;
  uint32_t events, missed, shots;
};

// The policy and capture settings (the cfg line apart from its channels).
struct Settings {
  bool     dropOne;
  int8_t   maxLost;
  uint8_t  mask;
  float    resLimitNs, areaMm, togetherNs, pulseMaxNs, curtainLoNs, curtainHiNs;
  uint16_t spanMm, deadtimeMs;
};

// The calibration record: the channels' values and where they came from (PLAN D5).
struct Calibration {
  uint32_t date;       // yyyymmdd (the target has no clock)
  float    resNs;      // the fit's rms residual
  uint16_t points, sparks;
  measure::ChannelCfg ch[8];
};

struct Request { uint32_t boot, next; };

void encodeShot(const Shot& s, uint8_t out[SHOT_LEN]);
bool decodeShot(const uint8_t* in, size_t len, Shot& s);
void encodeEvent(const Event& e, uint8_t out[EVENT_LEN]);
bool decodeEvent(const uint8_t* in, size_t len, Event& e);
void encodeStatus(const Status& s, uint8_t out[STATUS_LEN]);
bool decodeStatus(const uint8_t* in, size_t len, Status& s);
void encodeSettings(const Settings& s, uint8_t out[SETTINGS_LEN]);
bool decodeSettings(const uint8_t* in, size_t len, Settings& s);   // false: length or layout
void encodeCalibration(const Calibration& c, uint8_t out[CALIBRATION_LEN]);
bool decodeCalibration(const uint8_t* in, size_t len, Calibration& c);
bool decodeRequest(const uint8_t* in, size_t len, Request& r);       // exactly 8 bytes
bool decodeClear(const uint8_t* in, size_t len, Request& r);         // 0x01 + 9: next = last seq

// Range checks for a write from the app (the same limits as the serial commands).
bool settingsValid(const Settings& s, uint16_t spanMaxMm);
bool calibrationValid(const Calibration& c);

// An acoustic event's message from its record and the target's solve of it. Boot, seq, time,
// and the sent flag are the caller's.
void eventFromRec(const measure::Rec& rec, const measure::EventFit& f, Event& e);
// The record back from a message (the app's log, the tests): the fields a `rec` line has.
bool recFromEvent(const Event& e, measure::Rec& rec);

}  // namespace proto
