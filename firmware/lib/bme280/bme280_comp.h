// OpenSights - BME280 / BMP280 compensation: the integer formulas of the Bosch datasheets
// (BME280 rev. 1.6 section 4.2.3, BMP280 rev. 1.14 section 3.11.3), unchanged. Portable (no
// Arduino), so it is tested on the PC against the BMP280 datasheet's worked example.
#pragma once
#include <stdint.h>

struct Bme280Calib {
  uint16_t T1; int16_t T2, T3;
  uint16_t P1; int16_t P2, P3, P4, P5, P6, P7, P8, P9;
  uint8_t  H1; int16_t H2; uint8_t H3; int16_t H4, H5; int8_t H6;
};

// From the two calibration blocks: 0x88..0xA1 (26 bytes) and 0xE1..0xE7 (7 bytes; the
// BMP280 has no second block: pass nullptr).
inline void bme280ParseCalib(const uint8_t* a, const uint8_t* b, Bme280Calib& c) {
  auto u16 = [](const uint8_t* p) { return (uint16_t)(p[0] | (p[1] << 8)); };
  c.T1 = u16(a + 0);  c.T2 = (int16_t)u16(a + 2);  c.T3 = (int16_t)u16(a + 4);
  c.P1 = u16(a + 6);  c.P2 = (int16_t)u16(a + 8);  c.P3 = (int16_t)u16(a + 10);
  c.P4 = (int16_t)u16(a + 12); c.P5 = (int16_t)u16(a + 14); c.P6 = (int16_t)u16(a + 16);
  c.P7 = (int16_t)u16(a + 18); c.P8 = (int16_t)u16(a + 20); c.P9 = (int16_t)u16(a + 22);
  c.H1 = a[25];
  if (b) {
    c.H2 = (int16_t)u16(b + 0);
    c.H3 = b[2];
    c.H4 = (int16_t)(((int16_t)(int8_t)b[3] * 16) | (b[4] & 0x0F));
    c.H5 = (int16_t)(((int16_t)(int8_t)b[5] * 16) | (b[4] >> 4));
    c.H6 = (int8_t)b[6];
  } else {
    c.H2 = 0; c.H3 = 0; c.H4 = 0; c.H5 = 0; c.H6 = 0;
  }
}

// Temperature in 0.01 degC; sets tFine for the other two.
inline int32_t bme280CompT(const Bme280Calib& c, int32_t adcT, int32_t& tFine) {
  int32_t var1 = ((((adcT >> 3) - ((int32_t)c.T1 << 1))) * ((int32_t)c.T2)) >> 11;
  int32_t var2 = (((((adcT >> 4) - ((int32_t)c.T1)) * ((adcT >> 4) - ((int32_t)c.T1))) >> 12) *
                  ((int32_t)c.T3)) >> 14;
  tFine = var1 + var2;
  return (tFine * 5 + 128) >> 8;
}

// Pressure in Pa as Q24.8 (divide by 256); 0 if the calibration is broken.
inline uint32_t bme280CompP(const Bme280Calib& c, int32_t adcP, int32_t tFine) {
  int64_t var1 = ((int64_t)tFine) - 128000;
  int64_t var2 = var1 * var1 * (int64_t)c.P6;
  var2 = var2 + ((var1 * (int64_t)c.P5) * 131072);
  var2 = var2 + (((int64_t)c.P4) * 34359738368LL);
  var1 = ((var1 * var1 * (int64_t)c.P3) >> 8) + ((var1 * (int64_t)c.P2) * 4096);
  var1 = ((((int64_t)1) << 47) + var1) * ((int64_t)c.P1) >> 33;
  if (var1 == 0) return 0;
  int64_t p = 1048576 - adcP;
  p = (((p * 2147483648LL) - var2) * 3125) / var1;
  var1 = (((int64_t)c.P9) * (p >> 13) * (p >> 13)) >> 25;
  var2 = (((int64_t)c.P8) * p) >> 19;
  p = ((p + var1 + var2) >> 8) + (((int64_t)c.P7) << 4);
  return (uint32_t)p;
}

// Relative humidity in % as Q22.10 (divide by 1024). BME280 only.
inline uint32_t bme280CompH(const Bme280Calib& c, int32_t adcH, int32_t tFine) {
  int32_t v = tFine - ((int32_t)76800);
  v = (((((adcH << 14) - (((int32_t)c.H4) << 20) - (((int32_t)c.H5) * v)) + ((int32_t)16384)) >> 15) *
       (((((((v * ((int32_t)c.H6)) >> 10) * (((v * ((int32_t)c.H3)) >> 11) + ((int32_t)32768))) >> 10) +
          ((int32_t)2097152)) * ((int32_t)c.H2) + 8192) >> 14));
  v = (v - (((((v >> 15) * (v >> 15)) >> 7) * ((int32_t)c.H1)) >> 4));
  v = (v < 0 ? 0 : v);
  v = (v > 419430400 ? 419430400 : v);
  return (uint32_t)(v >> 12);
}
