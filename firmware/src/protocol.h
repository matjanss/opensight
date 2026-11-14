// ESP Sensor - BLE protocol constants. Mirrors protocol/PROTOCOL.md and app/src/protocol.rs:
// change all three together.
#pragma once
#include <stddef.h>
#include <stdint.h>

const uint8_t PROTOCOL_VERSION = 1;

const char* const SERVICE_UUID   = "5631f100-5314-4f02-b637-0654ff6d409d";
const char* const HEARTBEAT_UUID = "5631f101-5314-4f02-b637-0654ff6d409d";   // u32 LE millis()
const char* const LED_UUID       = "5631f102-5314-4f02-b637-0654ff6d409d";   // u8 0/1
const char* const NAME_UUID      = "5631f103-5314-4f02-b637-0654ff6d409d";   // 1..20 printable ASCII
const char* const VERSION_UUID   = "5631f104-5314-4f02-b637-0654ff6d409d";   // u8
const char* const PIN_UUID       = "5631f105-5314-4f02-b637-0654ff6d409d";   // u32 LE, write only
const char* const SHOT_UUID      = "5631f106-5314-4f02-b637-0654ff6d409d";   // indicate shots, write requests

const uint16_t MANUFACTURER_ID = 0xFFFF;     // reserved for testing
const uint8_t  ADV_FLAG_PAIRING = 0x01;      // manufacturer data byte 1, bit 0

const uint32_t DEFAULT_PASSKEY = 0;          // shown and typed as "000000"
const uint32_t PASSKEY_MAX = 999999;         // 6 digits
const int NAME_MAX_LEN = 20;

// Shot: seq u32, boot u32, time u32 (millis()), x i32, y i32 (0.01 mm), all LE.
const size_t SHOT_LEN = 20;
// Shot request (app → sensor): boot u32, next seq u32.
const size_t SHOT_REQUEST_LEN = 8;
const char* const NAME_PREFIX = "ESPSensor-";
