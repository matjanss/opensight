// OpenSights - the BLE GATT server (see protocol/PROTOCOL.md).
//
// NimBLE calls its callbacks from its own task. Those only record requests; the main loop
// collects them with the bleTake*() functions and applies them, so all state changes,
// flash writes and pin changes happen in one place.
#pragma once
#include <Arduino.h>

void bleBegin(const char* name, uint32_t pin);

// Bonded computers cache the GATT layout. After a firmware update that may have changed it,
// call bleServicesChanged() once (after bleBegin): every bonded computer is then told to
// re-read it, on its next secured connection (kept in NVS until then). bleService() sends it.
extern const char* const GATT_BUILD_ID;
void bleServicesChanged();
void bleService(uint32_t nowMs);   // from loop()

// Publish a new value: stored for reads and notified to a subscribed app.
void bleSetHeartbeat(uint32_t ms);
void bleSetName(const char* name);      // also updates the advertised and GAP device name
void bleSetStatus(const uint8_t* value, size_t len);
// Stored for reads only (the app reads them after a write, or when it connects).
void bleSetCalibration(const uint8_t* value, size_t len);
void bleSetSettings(const uint8_t* value, size_t len);

// Pairing mode: while on, an unbonded central may connect and pair.
void bleSetPairingMode(bool on);
bool bleIsConnected();
uint16_t bleMtu();                      // the connected central's ATT MTU; 0 if none

// Pairing PIN (passkey) and bonds. Call from the main loop.
void bleSetPasskey(uint32_t pin);       // used for pairings from now on
int  bleDeleteOtherBonds();             // all but the connected central's; returns how many
void bleDeleteAllBonds();               // also disconnects the connected central

// The two indicated streams: shots and events. A link has one indication in flight at a
// time, so they take turns (the caller sends shots first). Start one with bleSend() when
// bleReady(), then bleTakeResult() reports whether the app confirmed it.
enum BleStream : uint8_t { BLE_SHOT = 0, BLE_EVENT = 1 };
bool bleReady(BleStream s);                                   // subscribed, nothing in flight
bool bleSend(BleStream s, const uint8_t* value, size_t len);  // false if it could not be sent
int  bleTakeResult(BleStream s);                              // 1 confirmed, -1 failed, 0 nothing new
bool bleTakeSubscribed(BleStream s);                          // the app (re)subscribed
bool bleTakeRequest(BleStream s, uint8_t* value, size_t* len);   // raw bytes (16-byte buffer)

// Requests from the app, or events, since the last call. Each returns true once per event.
bool bleTakeNameWrite(char* buf, size_t bufSize, size_t* len);
bool bleTakePinWrite(uint8_t* value, size_t* len);         // raw bytes, validated by the caller
bool bleTakeControl(uint8_t* value, size_t* len);          // one command (16-byte buffer); queued
bool bleTakeCalibrationWrite(uint8_t* value, size_t* len); // 256-byte buffer
bool bleTakeSettingsWrite(uint8_t* value, size_t* len);    // 64-byte buffer
bool bleTakeNewBond();                                     // a new central finished pairing
