// OpenSights - the air for the speed of sound (measurement steps 2-3): a BME280 (or BMP280)
// on I2C, or a DS18B20 on 1-Wire, on the host board's J5, read once a minute by a
// housekeeping task on core 0. See airtemp.cpp.
#pragma once
#include <Arduino.h>

enum AirSensor : uint8_t { AIR_NONE, AIR_BME280, AIR_BMP280, AIR_DS18B20 };

struct Air {
  float    tC;          // degC
  float    rh;          // % relative humidity
  float    hPa;         // pressure
  bool     tMeasured;   // false: that value is assumed (AIR_*_DEFAULT in config.h)
  bool     rhMeasured;
  bool     pMeasured;
  float    c;           // speed of sound from the three, m/s
  AirSensor sensor;     // the sensor found (AIR_NONE: none answers)
  uint32_t ageS;        // seconds since the last good reading (if there was one)
};

// Starts the task. Call once from setup().
void airBegin();

// The current values; a reading older than AIR_TEMP_STALE_MS counts as missing.
Air air();

// The speed of sound for these values (measure::soundSpeedMs), m/s.
float airSoundSpeed(float tC, float rh, float hPa);

// "air: ..." lines for `stat` and the `temp` command.
void airPrintStatus();

// "air 21.3 °C 45 %RH 1009.8 hPa, c 344.62 m/s", each value marked if assumed.
int airFormat(char* out, size_t size, const Air& a);
