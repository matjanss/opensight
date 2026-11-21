// OpenSights - air temperature for the speed of sound (measurement step 2): a DS18B20 read
// once a minute by a housekeeping task on core 0. See airtemp.cpp.
#pragma once
#include <Arduino.h>

struct AirTemp {
  float    c;          // degrees C: the last good reading, or AIR_TEMP_DEFAULT_C
  bool     measured;   // false: assumed (no sensor, or no good reading for AIR_TEMP_STALE_MS)
  uint32_t ageS;       // seconds since the last good reading (if there was one)
  bool     everRead;   // a sensor has answered since boot
};

// Starts the task. Call once from setup().
void airTempBegin();

AirTemp airTemp();

// "air: ..." lines for `stat` and the `temp` command.
void airTempPrintStatus();
