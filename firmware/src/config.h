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

// The shot store (lib/store, MEASUREMENT 5e): every hit in flash, in the `shots` partition
// (partitions.csv: data, subtype 0x40, 128 KB = 1920 hits at least), across reboots. With no
// such partition (an old partition table), a ring in RAM of STORE_RAM_SECTORS sectors.
const uint8_t  STORE_PART_SUBTYPE   = 0x40;
const int      STORE_RAM_SECTORS    = 4;      // the fallback: 4 KB each, 129 hits at least
const uint32_t STORE_QUIET_MS       = 2000;   // erase the sector ahead only after this long
                                              // without a shot or an event (about 45 ms)
const uint32_t SHOT_RETRY_MS        = 500;    // wait after a failed indication before retrying
const int      EVENT_BUFFER         = 32;     // events kept for resending (protocol v3, 180 bytes each)
const uint32_t STATUS_PERIOD_MS     = 500;    // Status is rebuilt this often, notified if it changed
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
const int IR_EN_PIN = 27;   // IR curtain LEDs, high = on (with `curtain on`, step 4b)
const int IR_IN_PIN = 13;   // IR curtain comparator: high while the beam is blocked (step 4b)

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

// The pulse generator (self-test and simulator) starts its MCPWM units from an edge on this
// pin, driven and read back inside the chip: leave it unconnected (spare on the host
// board). A strapping pin (flash voltage: nothing may pull it up at reset), so it is only
// driven after boot and held by a pull-down otherwise. Was GPIO 15 until step 3a; 15 is
// the BME280's SCL now.
const int      SYNC_PIN             = 12;

// --- Step 2 (see ../MEASUREMENT.md) ---
// Arrival time t0 = t1 - k (t2 - t1) - offset; k and the offsets are settings (NVS).
const float    ARRIVAL_K_DEFAULT    = 1.0f / 3;

// --- Step 3c: the solver on the board ---
// The calibration (NVS "cal", calib.cpp): per channel its mic (x, y, h), k, offset and
// sigma. Defaults: the D1/D2 layout on CH1-CH6 (`mic default`).
const float    MIC_H_DEFAULT_MM     = 5.5f;    // mic port behind the paper: the board's edge on it (21.5 flipped)
// The D1/D2 layout (PLAN.md), mm: bottom left, bottom right, top right moved out 25 mm along
// its diagonal (105 + 25/sqrt 2; it was slid 40 mm along the top edge, to (65, 105), before
// 2026-11-24), top left, bottom middle, right middle.
const float    MIC_LAYOUT_D2[6][2]  = {{-105, -105}, {105, -105}, {122.68f, 122.68f}, {-105, 105}, {0, -105}, {105, 0}};
const float    SIGMA_DEFAULT_NS     = 50.0f;   // timing error per channel until calibrated (3e)
// A solved shot goes to the app if the fit converged with an rms residual up to this (ns;
// `reslimit`, stored: measure::RES_LIMIT_DEFAULT_NS). Validity proper is step 4.
const float    CARD_HALF_MM         = 100.0f;  // the scoring area: |x|, |y| up to this

// Simulator (3d): 6 mics on CH3-CH8 (their pins can be outputs; CH1/CH2 are input-only),
// the pulse generator's 6 MCPWM operators: CH3-CH5 on unit 0, CH6-CH8 on unit 1. Its mics
// default to the D1/D2 layout in order: CH3 bottom left, CH4 bottom right, CH5 top right
// (slid), CH6 top left, CH7 bottom middle, CH8 right middle. Those connectors must have no
// sensor board plugged in while it runs.
const int      SIM_FIRST_CHANNEL    = 2;      // CH3, 0-based
const int      SIM_CHANNELS         = 6;
const float    SIM_CARD_MM          = 100.0f; // random shots land within +-this of the centre
const float    SIM_POS_LIMIT_MM     = 0.1f;   // position check: worst error vs the truth (3c)
const float    SIM_POS_LIMIT_DROP_MM = 0.15f; // the same with a faulty channel dropped (5 mics, 3d)

// The air (step 3a): J5 takes a BME280/BMP280 on I2C (SDA, SCL) or a DS18B20 on 1-Wire
// (SDA's pin, read through UART2: the UART times the slots, so no interrupts are held off),
// read by a task on core 0, once a minute. 4.7 k pull-ups to 3V3 on the host board.
const int      AIR_SDA_PIN          = 14;      // also the DS18B20's DQ
const int      AIR_SCL_PIN          = 15;
const uint32_t AIR_TEMP_PERIOD_MS   = 60000;
const uint32_t AIR_PROBE_TRIES      = 3;       // look for a sensor this often, a period apart, then stop
const uint32_t AIR_TEMP_STALE_MS    = 600000;  // no good reading for this long: use the defaults
const float    AIR_TEMP_DEFAULT_C   = 18.0f;   // without a reading
const float    AIR_RH_DEFAULT       = 50.0f;   // % RH, without a BME280
const float    AIR_HPA_DEFAULT      = 1013.25f;   // without a BME280 or BMP280
