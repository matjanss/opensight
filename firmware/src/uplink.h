// OpenSights - the uplink (PLAN D6): what goes to the app, and what the app sets, in protocol
// v3 (protocol/PROTOCOL.md). BLE now (ble_link.cpp), MQTT later with the same messages.
//
// - Shots: the last SHOT_BUFFER hits (kind shot, hole or missed), numbered per boot,
//   indicated one at a time until the app confirms each; a shot request asks for them again,
//   a clear request forgets them.
// - Events: the last EVENT_BUFFER events, every verdict, with everything a `rec` line has;
//   delivered the same way, after any waiting shot.
// - Status (notified when it changes), Control (calibration mode, template point),
//   Calibration and Settings (read, and written whole by the app).
#pragma once
#include <Arduino.h>
#include "proto.h"

void uplinkBegin(uint32_t bootId);
uint32_t uplinkBoot();
void uplinkService(uint32_t nowMs);   // from loop()

// A hit for the app's series: kind shot (with x, y in 0.01 mm), hole or missed. `eventSeq`:
// the event message it came from (0: none, the BOOT button's test shot). Returns its seq.
uint32_t uplinkShot(uint8_t kind, int32_t x, int32_t y, uint16_t sigma, uint8_t flags,
                    uint32_t eventSeq, uint32_t nowMs);
uint32_t uplinkLastShotMs();          // millis() of the newest shot (the LED's flash)

// An event message: boot, seq and time are set here. Returns its seq.
uint32_t uplinkEvent(proto::Event& e, uint32_t nowMs);

// A curtain dip with no acoustic event (curtain.cpp): a hole. `tNs`: its rise, ns since the
// capture started; `lenNs`: proto::NO_VALUE if its fall was not seen. An event, and a hit of
// kind hole unless in calibration mode. Returns the hit's seq, 0 if none.
uint32_t uplinkHole(uint64_t tNs, int32_t lenNs, uint32_t nowMs);
// Activity in the dead time while event `eventN` was read (capture.cpp): a missed shot.
uint32_t uplinkMissed(uint32_t eventN, uint32_t nowMs);
