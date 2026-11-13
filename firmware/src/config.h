// ESP Sensor - pins and timings of this board (NodeMCU-32S).
#pragma once
#include <Arduino.h>

const int LED_PIN  = 2;    // onboard blue LED, active high
const int BOOT_BTN_PIN = 0;    // onboard BOOT button to GND (has an external pull-up)

const uint32_t HEARTBEAT_PERIOD_MS = 2000;
const uint32_t DEBOUNCE_MS         = 30;
const uint32_t LONG_PRESS_MS       = 2000;   // hold BOOT this long to enter pairing mode
const uint32_t RESET_PRESS_MS      = 10000;  // ... this long to reset the PIN and delete all bonds
const uint32_t RESET_SIGNAL_MS     = 1500;   // LED stays on this long to confirm the reset
const uint32_t PAIRING_WINDOW_MS   = 60000;  // pairing mode ends after this, or after a pairing
const uint32_t PAIRING_BLINK_MS    = 150;    // LED half-period while in pairing mode
const uint32_t LOOP_PERIOD_MS      = 10;     // button sampling rate; nothing here needs to be faster
