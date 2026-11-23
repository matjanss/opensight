// OpenSights - BLE protocol constants. Mirrors protocol/PROTOCOL.md and app/src/protocol.rs:
// change all three together. The messages themselves are encoded in lib/proto (proto.h).
#pragma once
#include <stddef.h>
#include <stdint.h>

const uint8_t PROTOCOL_VERSION = 3;

const char* const SERVICE_UUID     = "5631f100-5314-4f02-b637-0654ff6d409d";
const char* const HEARTBEAT_UUID   = "5631f101-5314-4f02-b637-0654ff6d409d";   // u32 LE millis()
// 5631f102 was the LED (version 1); not reused.
const char* const NAME_UUID        = "5631f103-5314-4f02-b637-0654ff6d409d";   // 1..20 printable ASCII
const char* const VERSION_UUID     = "5631f104-5314-4f02-b637-0654ff6d409d";   // u8
const char* const PIN_UUID         = "5631f105-5314-4f02-b637-0654ff6d409d";   // u32 LE, write only
const char* const SHOT_UUID        = "5631f106-5314-4f02-b637-0654ff6d409d";   // indicate shots, write requests
const char* const EVENT_UUID       = "5631f107-5314-4f02-b637-0654ff6d409d";   // indicate events, write requests
const char* const STATUS_UUID      = "5631f108-5314-4f02-b637-0654ff6d409d";   // read, notify
const char* const CONTROL_UUID     = "5631f109-5314-4f02-b637-0654ff6d409d";   // write commands
const char* const CALIBRATION_UUID = "5631f10a-5314-4f02-b637-0654ff6d409d";   // read, write (long)
const char* const SETTINGS_UUID    = "5631f10b-5314-4f02-b637-0654ff6d409d";   // read, write

const uint16_t MANUFACTURER_ID = 0xFFFF;     // reserved for testing
const uint8_t  ADV_FLAG_PAIRING = 0x01;      // manufacturer data byte 1, bit 0

const uint32_t DEFAULT_PASSKEY = 0;          // shown and typed as "000000"
const uint32_t PASSKEY_MAX = 999999;         // 6 digits
const int NAME_MAX_LEN = 20;
const char* const NAME_PREFIX = "OpenSights-";

// The ATT MTU a link needs for v3: the event (180 bytes) plus 3. Shots need 35.
const uint16_t MTU_EVENTS = 183;
const uint16_t MTU_SHOTS = 35;
