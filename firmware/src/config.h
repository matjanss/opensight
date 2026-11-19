// OpenSights - pins and timings of this board (NodeMCU-32S).
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
const uint32_t SHOT_FLASH_MS       = 80;     // LED flashes this long for every shot

const int      SHOT_BUFFER          = 64;     // shots kept for resending after a link drop
const uint32_t SHOT_RETRY_MS        = 500;    // wait after a failed indication before retrying
// Test shots (BOOT short press) until there is measuring hardware: normally distributed
// around the centre. 15 mm puts most of them in the 7-10 rings of the air pistol card.
const float    TEST_SHOT_SIGMA_MM   = 15.0f;

// --- Measurement (see ../MEASUREMENT.md) ---
// Comparator lines of host board v0.2, in I2S data-bit order: bit 2n = CH(n+1) OUT_L,
// bit 2n+1 = CH(n+1) OUT_H. Lines idle low and go high while SIG is above the threshold.
const int CAPTURE_LINE_PINS[16] = {
  36, 39,   // CH1
  34, 35,   // CH2
  32, 33,   // CH3
  25, 26,   // CH4
  27, 14,   // CH5
   4, 23,   // CH6
  22, 21,   // CH7
  19, 18,   // CH8
};
const int IR_EN_PIN = 16;   // IR curtain LEDs, high = on; unused for now, held low

// The I2S sample clock is made by LEDC and looped back through this pin (spare on the
// host board; leave it unconnected).
const int      CAPTURE_CLK_PIN        = 5;
const uint32_t CAPTURE_SAMPLE_HZ      = 10000000;   // 100 ns per sample
const int      CAPTURE_BLOCK_BYTES    = 4000;       // per DMA descriptor: 2000 samples, 200 us
const int      CAPTURE_RING_BLOCKS    = 16;         // ring: 64 KB, 3.2 ms
const uint32_t CAPTURE_WINDOW_US      = 1200;       // event window after the first activity
// The I2S FIFO packs two 16-bit samples per 32-bit word. esp32-camera reads the earlier
// sample from the high half; the self-test shows extra transitions if this is wrong.
const bool     CAPTURE_HIGH_HALF_FIRST = true;

const uint16_t CAPTURE_MASK_DEFAULT     = 0x0F;   // fitted channels, bit n = CH(n+1)
const uint32_t CAPTURE_DEADTIME_DEFAULT = 30;     // ms after an event with no new event

// Self-test: pulses driven out on CH8's own pins (the I2S reads them back on the same
// pins, so no jumpers are needed). CH8 must have no sensor board plugged in.
const int      SELFTEST_PIN_L       = 19;   // CH8 OUT_L
const int      SELFTEST_PIN_H       = 18;   // CH8 OUT_H
const uint32_t SELFTEST_PULSE_US    = 10;   // OUT_L pulse length; OUT_H ends with it
