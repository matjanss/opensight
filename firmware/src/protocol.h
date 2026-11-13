// ESP Sensor - BLE protocol constants. Mirrors protocol/PROTOCOL.md and app/src/protocol.rs:
// change all three together.
#pragma once
#include <stdint.h>

const uint8_t PROTOCOL_VERSION = 1;

const char* const SERVICE_UUID   = "5631f100-5314-4f02-b637-0654ff6d409d";
const char* const HEARTBEAT_UUID = "5631f101-5314-4f02-b637-0654ff6d409d";   // u32 LE millis()
const char* const LED_UUID       = "5631f102-5314-4f02-b637-0654ff6d409d";   // u8 0/1
const char* const NAME_UUID      = "5631f103-5314-4f02-b637-0654ff6d409d";   // 1..20 printable ASCII
const char* const VERSION_UUID   = "5631f104-5314-4f02-b637-0654ff6d409d";   // u8

const uint16_t MANUFACTURER_ID = 0xFFFF;     // reserved for testing
const uint8_t  ADV_FLAG_PAIRING = 0x01;      // manufacturer data byte 1, bit 0

const uint32_t PASSKEY = 1234;               // typed by the user as "1234"
const int NAME_MAX_LEN = 20;
const char* const NAME_PREFIX = "ESPSensor-";
