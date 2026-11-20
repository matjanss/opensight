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
// Comparator lines of host board v0.3 (hardware/host-board/README.md), in I2S data-bit
// order: bit 2n = CH(n+1) OUT_L, bit 2n+1 = CH(n+1) OUT_H. Lines idle low and go high
// while SIG is above the threshold.
constexpr int CAPTURE_LINE_PINS[16] = {
  39, 36,   // CH1
  35, 34,   // CH2
  33, 32,   // CH3
  26, 25,   // CH4
  22, 23,   // CH5
  19, 21,   // CH6
  17, 18,   // CH7
   4, 16,   // CH8
};
const int IR_EN_PIN = 27;   // IR curtain LEDs, high = on; unused for now, held low
// Also on the board, unused so far: IR_IN 13 (IR receiver), DQ 14 (DS18B20).

// The I2S sample clock is made by LEDC and looped back through this pin (spare on the
// host board; leave it unconnected).
const int      CAPTURE_CLK_PIN        = 5;
const uint32_t CAPTURE_SAMPLE_HZ      = 10000000;   // 100 ns per sample
const int      CAPTURE_BLOCK_BYTES    = 4000;       // per DMA descriptor: 2000 samples, 200 us
const int      CAPTURE_RING_BLOCKS    = 16;         // ring: 64 KB, 3.2 ms
// The event window must hold every mic's first edge. Sound reaches two mics at most
// (distance between them) / (speed of sound) apart, wherever the shot lands, so the window
// follows from the largest distance between two mics (the diagonal for corner mics). It is
// a setting (serial `span`, stored in NVS; later an app setting, see MEASUREMENT.md step 5).
// Default: corner mics just outside a 200 x 200 mm card, about 210 mm apart: 300 mm.
const uint32_t CAPTURE_SPAN_MM_DEFAULT = 300;
const uint32_t SOUND_SPEED_MIN_M_S     = 331;   // at 0 degC: the slowest the window allows for
const uint32_t CAPTURE_WINDOW_MARGIN_PCT = 20;  // on top of span / speed of sound
const uint32_t CAPTURE_WINDOW_EXTRA_US = 100;   // comparator delay on a quiet far mic, pulse end
// The I2S FIFO packs two 16-bit samples per 32-bit word. esp32-camera reads the earlier
// sample from the high half; confirmed on the bench (step 1: pulse lengths exact).
const bool     CAPTURE_HIGH_HALF_FIRST = true;

const uint16_t CAPTURE_MASK_DEFAULT     = 0x0F;   // fitted channels, bit n = CH(n+1)
const uint32_t CAPTURE_DEADTIME_DEFAULT = 30;     // ms after an event with no new event

// Self-test: pulses driven out on CH8's own pins (the I2S reads them back on the same
// pins, so no jumpers are needed). CH8 must have no sensor board plugged in.
constexpr int  SELFTEST_PIN_L       = CAPTURE_LINE_PINS[14];   // CH8 OUT_L, GPIO 4
constexpr int  SELFTEST_PIN_H       = CAPTURE_LINE_PINS[15];   // CH8 OUT_H, GPIO 16
const uint32_t SELFTEST_PULSE_US    = 10;   // OUT_L pulse length; OUT_H ends with it
