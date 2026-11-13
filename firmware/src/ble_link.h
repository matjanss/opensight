// ESP Sensor - the BLE GATT server (see protocol/PROTOCOL.md).
//
// NimBLE calls its callbacks from its own task. Those only record requests; the main loop
// collects them with the bleTake*() functions and applies them, so all state changes,
// flash writes and pin changes happen in one place.
#pragma once
#include <Arduino.h>

void bleBegin(const char* name, bool ledOn, uint32_t pin);

// Bonded computers cache the GATT layout. After a firmware update that may have changed it,
// call bleServicesChanged() once (after bleBegin): NimBLE tells every bonded computer to
// re-read it, on their next connection if they aren't connected.
extern const char* const GATT_BUILD_ID;
void bleServicesChanged();

// Publish a new value: stored for reads and notified to a subscribed app.
void bleSetHeartbeat(uint32_t ms);
void bleSetLed(bool on);
void bleSetName(const char* name);      // also updates the advertised and GAP device name

// Pairing mode: while on, an unbonded central may connect and pair.
void bleSetPairingMode(bool on);
bool bleIsConnected();

// Pairing PIN (passkey) and bonds. Call from the main loop.
void bleSetPasskey(uint32_t pin);       // used for pairings from now on
int  bleDeleteOtherBonds();             // all but the connected central's; returns how many
void bleDeleteAllBonds();               // also disconnects the connected central

// Requests from the app, or events, since the last call. Each returns true once per event.
bool bleTakeLedWrite(uint8_t* value, size_t* len);         // raw bytes, validated by the caller
bool bleTakeNameWrite(char* buf, size_t bufSize, size_t* len);
bool bleTakePinWrite(uint8_t* value, size_t* len);         // raw bytes, validated by the caller
bool bleTakeNewBond();                                     // a new central finished pairing
