// OpenSights - the air for the speed of sound: see airtemp.h. Read once a minute by a
// housekeeping task on core 0, away from the capture (core 1). 0.1 degC moves a shot by
// about 0.01 mm, 1 % RH by about 0.003 mm, and the air changes slowly, so a reading a
// minute is plenty.
//
// J5 takes one of two sensors, and the task finds which: at boot and then once a minute,
// AIR_PROBE_TRIES times in all (so for two minutes), then it stops looking until `temp scan`
// (airScan). Three failed readings in a row and it looks again, the same way:
//
// - **BME280 / BMP280** on I2C (SDA AIR_SDA_PIN, SCL AIR_SCL_PIN, address 0x76 or 0x77).
//   Forced mode, oversampling x1 for each value, no filter (Bosch's "weather monitoring"
//   settings): one conversion of about 10 ms a minute, so no self-heating. Compensated with
//   the datasheet's integer formulas (lib/bme280). The BMP280 has no humidity.
// - **DS18B20** on 1-Wire (AIR_SDA_PIN), driven by UART2 with RX and TX on the same pin
//   (open drain; the Arduino core sets that up when both pins are the same). The UART
//   times every slot, so no interrupts are held off (a bit-banged bus would hold them off
//   ~70 us per bit):
//     reset:  0xF0 at 9600 baud; a sensor's presence pulse changes the byte that comes back
//     bit:    one byte at 115200 baud; 0xFF writes a 1 or reads a bit (0xFF back = 1),
//             0x00 writes a 0
//
// Probing one bus can't upset the other sensor: I2C pulls SDA low for a few bit times at
// most, far shorter than a 1-Wire reset (480 us); a 1-Wire reset on SDA while SCL is high
// looks to a BME280 like a START then a STOP, which it ignores.
//
// Readings are checked (DS18B20: CRC, the configuration byte's fixed bits, the 85 degC
// power-on value; BME280: the "no data" register values, an all-ones bus; both: a plausible
// range), so a missing sensor or a floating pin can't pass as a reading.
#include "airtemp.h"
#include <Wire.h>
#include "air.h"
#include "bme280_comp.h"
#include "config.h"
#include "driver/gpio.h"

static HardwareSerial bus(2);

// --- Shared state (the task writes, any core reads) ---
static portMUX_TYPE lock = portMUX_INITIALIZER_UNLOCKED;
static AirSensor sensor = AIR_NONE;
static AirSensor readBy = AIR_NONE;   // the sensor that gave the last good reading
static uint8_t   bmeAddr = 0;
static float     lastT = 0, lastRh = 0, lastHPa = 0;
static uint32_t  lastAtMs = 0;
static bool      haveReading = false;
static uint32_t  reads = 0, fails = 0, probes = 0;
static uint32_t  probesLeft = AIR_PROBE_TRIES;   // 0 with no sensor: stopped looking
static TaskHandle_t task = nullptr;
static uint8_t   lastFail = 0;

enum : uint8_t { FAIL_NONE, FAIL_NO_SENSOR, FAIL_BUS, FAIL_CRC, FAIL_VALUE };
static const char* const FAIL_TEXT[] = {
  "", "the sensor stopped answering", "the bus didn't answer", "bad CRC or data",
  "an impossible value (power-on value, or out of range)",
};
static const char* const SENSOR_TEXT[] = {"none", "BME280", "BMP280", "DS18B20"};

static const float RANGE_MIN_C = -20.0f, RANGE_MAX_C = 60.0f;
static const float RANGE_MIN_HPA = 300.0f, RANGE_MAX_HPA = 1100.0f;

// --- Which peripheral has the pins ---
enum BusMode { MODE_OFF, MODE_I2C, MODE_ONEWIRE };
static BusMode mode = MODE_OFF;

static void useI2C() {
  if (mode == MODE_I2C) return;
  if (mode == MODE_ONEWIRE) bus.end();
  Wire.begin(AIR_SDA_PIN, AIR_SCL_PIN, 100000);   // enables the internal pull-ups too
  Wire.setTimeOut(50);
  mode = MODE_I2C;
}

static void useOneWire() {
  if (mode == MODE_ONEWIRE) return;
  if (mode == MODE_I2C) Wire.end();
  bus.begin(9600, SERIAL_8N1, AIR_SDA_PIN, AIR_SDA_PIN);   // same pin for RX and TX: 1-Wire
  bus.setRxFIFOFull(1);                                      // hand each byte over at once
  bus.setTimeout(20);
  // One-wire mode turns the pin's pulls off. The internal pull-up keeps a pin with no
  // sensor (and no 4.7 k pull-up) from floating into a fake presence pulse.
  gpio_pullup_en((gpio_num_t)AIR_SDA_PIN);
  mode = MODE_ONEWIRE;
}

// --- BME280 / BMP280 ---
static Bme280Calib calib;

static bool i2cRead(uint8_t reg, uint8_t* buf, size_t n, uint8_t addr = 0) {
  if (!addr) addr = bmeAddr;
  Wire.beginTransmission(addr);
  Wire.write(reg);
  if (Wire.endTransmission(false) != 0) return false;
  if (Wire.requestFrom((uint16_t)addr, n, true) != n) return false;
  for (size_t i = 0; i < n; i++) buf[i] = Wire.read();
  return true;
}

static bool i2cWrite(uint8_t reg, uint8_t v) {
  Wire.beginTransmission(bmeAddr);
  Wire.write(reg);
  Wire.write(v);
  return Wire.endTransmission(true) == 0;
}

static AirSensor bmeProbe() {
  useI2C();
  for (uint8_t addr : {0x76, 0x77}) {
    uint8_t id;
    if (!i2cRead(0xD0, &id, 1, addr)) continue;
    AirSensor s = id == 0x60 ? AIR_BME280 : (id == 0x58 || id == 0x57 || id == 0x56) ? AIR_BMP280 : AIR_NONE;
    if (s == AIR_NONE) continue;
    bmeAddr = addr;
    if (!i2cWrite(0xE0, 0xB6)) continue;   // soft reset
    vTaskDelay(pdMS_TO_TICKS(5));
    uint8_t st = 1;
    for (int i = 0; i < 10 && (st & 1); i++) {   // im_update: calibration still being copied
      if (!i2cRead(0xF3, &st, 1)) break;
      if (st & 1) vTaskDelay(pdMS_TO_TICKS(2));
    }
    uint8_t a[26], b[7];
    if (!i2cRead(0x88, a, sizeof a)) continue;
    if (s == AIR_BME280 && !i2cRead(0xE1, b, sizeof b)) continue;
    bme280ParseCalib(a, s == AIR_BME280 ? b : nullptr, calib);
    if (calib.T1 == 0 || calib.P1 == 0) continue;   // calibration unreadable
    if (!i2cWrite(0xF5, 0x00)) continue;            // config: no filter
    return s;
  }
  bmeAddr = 0;
  return AIR_NONE;
}

static uint8_t bmeRead(AirSensor s, float* tC, float* rh, float* hPa) {
  useI2C();
  if (s == AIR_BME280 && !i2cWrite(0xF2, 0x01)) return FAIL_NO_SENSOR;   // humidity x1
  if (!i2cWrite(0xF4, 0x25)) return FAIL_NO_SENSOR;   // T x1, P x1, forced mode
  vTaskDelay(pdMS_TO_TICKS(12));
  uint8_t st = 0x08;
  for (int i = 0; i < 20 && (st & 0x08); i++) {   // measuring
    if (!i2cRead(0xF3, &st, 1)) return FAIL_BUS;
    if (st & 0x08) vTaskDelay(pdMS_TO_TICKS(2));
  }
  if (st & 0x08) return FAIL_BUS;
  uint8_t d[8];
  const size_t n = s == AIR_BME280 ? 8 : 6;
  if (!i2cRead(0xF7, d, n)) return FAIL_BUS;
  bool allOnes = true;
  for (size_t i = 0; i < n; i++) allOnes &= d[i] == 0xFF;
  if (allOnes) return FAIL_CRC;
  int32_t adcP = ((int32_t)d[0] << 12) | ((int32_t)d[1] << 4) | (d[2] >> 4);
  int32_t adcT = ((int32_t)d[3] << 12) | ((int32_t)d[4] << 4) | (d[5] >> 4);
  if (adcT == 0x80000 || adcP == 0x80000) return FAIL_VALUE;   // the reset value: no conversion
  int32_t tFine;
  float t = bme280CompT(calib, adcT, tFine) / 100.0f;
  float p = bme280CompP(calib, adcP, tFine) / 25600.0f;
  if (t < RANGE_MIN_C || t > RANGE_MAX_C || p < RANGE_MIN_HPA || p > RANGE_MAX_HPA) return FAIL_VALUE;
  float h = 0;
  if (s == AIR_BME280) {
    int32_t adcH = ((int32_t)d[6] << 8) | d[7];
    if (adcH == 0x8000) return FAIL_VALUE;
    h = bme280CompH(calib, adcH, tFine) / 1024.0f;
    if (h < 0 || h > 100) return FAIL_VALUE;
  }
  *tC = t;
  *rh = h;
  *hPa = p;
  return FAIL_NONE;
}

// --- DS18B20 ---
static void drain() {
  while (bus.available()) bus.read();
}

// One reset; true if a sensor answered with a presence pulse.
static bool owReset() {
  bus.updateBaudRate(9600);
  drain();
  bus.write(0xF0);
  bus.flush();
  uint8_t b = 0xF0;
  size_t got = bus.readBytes(&b, 1);
  bus.updateBaudRate(115200);
  return got == 1 && b != 0xF0;
}

// Eight slots: writes v (LSB first); returns the bits read back (meaningful for 0xFF).
static bool owByte(uint8_t v, uint8_t* readBack) {
  uint8_t slots[8], back[8];
  for (int i = 0; i < 8; i++) slots[i] = (v >> i) & 1 ? 0xFF : 0x00;
  drain();
  bus.write(slots, 8);
  bus.flush();
  if (bus.readBytes(back, 8) != 8) return false;
  uint8_t r = 0;
  for (int i = 0; i < 8; i++)
    if (back[i] == 0xFF) r |= 1 << i;
  if (readBack) *readBack = r;
  return true;
}

static uint8_t crc8(const uint8_t* p, int n) {   // Dallas/Maxim, x^8 + x^5 + x^4 + 1
  uint8_t crc = 0;
  for (int i = 0; i < n; i++) {
    uint8_t b = p[i];
    for (int j = 0; j < 8; j++) {
      uint8_t mix = (crc ^ b) & 1;
      crc >>= 1;
      if (mix) crc ^= 0x8C;
      b >>= 1;
    }
  }
  return crc;
}

static bool dsProbe() {
  useOneWire();
  return owReset();
}

static uint8_t dsRead(float* tC) {
  useOneWire();
  if (!owReset()) return FAIL_NO_SENSOR;
  if (!owByte(0xCC, nullptr) || !owByte(0x44, nullptr)) return FAIL_BUS;   // skip ROM, convert
  vTaskDelay(pdMS_TO_TICKS(800));   // 12-bit conversion: 750 ms at most
  if (!owReset()) return FAIL_NO_SENSOR;
  if (!owByte(0xCC, nullptr) || !owByte(0xBE, nullptr)) return FAIL_BUS;   // read scratchpad
  uint8_t s[9];
  for (int i = 0; i < 9; i++)
    if (!owByte(0xFF, &s[i])) return FAIL_BUS;
  bool allSame = true;
  for (int i = 1; i < 9; i++) allSame &= s[i] == s[0];
  if (allSame || crc8(s, 8) != s[8]) return FAIL_CRC;   // all 0x00 would pass the CRC
  if ((s[4] & 0x9F) != 0x1F) return FAIL_CRC;           // configuration: bit 7 = 0, bits 0-4 = 1
  int16_t raw = (int16_t)(s[0] | (s[1] << 8));          // 1/16 degC
  if (raw == 0x0550) return FAIL_VALUE;                  // 85 C: the power-on value, no conversion
  float t = raw / 16.0f;
  if (t < RANGE_MIN_C || t > RANGE_MAX_C) return FAIL_VALUE;
  *tC = t;
  return FAIL_NONE;
}

// --- The task ---
static void airTask(void*) {
  int failsInRow = 0;
  for (;;) {
    AirSensor s = sensor;
    if (s == AIR_NONE && probesLeft > 0) {
      s = bmeProbe();
      if (s == AIR_NONE && dsProbe()) s = AIR_DS18B20;
      if (s == AIR_NONE) useI2C();   // between probes, leave the pins as I2C (idle high)
      portENTER_CRITICAL(&lock);
      sensor = s;
      probes++;
      probesLeft = s == AIR_NONE ? probesLeft - 1 : AIR_PROBE_TRIES;
      portEXIT_CRITICAL(&lock);
      failsInRow = 0;
    }
    if (s != AIR_NONE) {
      float t = 0, h = 0, p = 0;
      uint8_t f = s == AIR_DS18B20 ? dsRead(&t) : bmeRead(s, &t, &h, &p);
      portENTER_CRITICAL(&lock);
      if (f == FAIL_NONE) {
        lastT = t;
        lastRh = h;
        lastHPa = p;
        lastAtMs = millis();
        readBy = s;
        haveReading = true;
        reads++;
      } else {
        lastFail = f;
        fails++;
      }
      portEXIT_CRITICAL(&lock);
      failsInRow = f == FAIL_NONE ? 0 : failsInRow + 1;
      if (failsInRow >= 3) {   // gone, or another sensor plugged in: look again next time
        portENTER_CRITICAL(&lock);
        sensor = AIR_NONE;
        probesLeft = AIR_PROBE_TRIES;
        portEXIT_CRITICAL(&lock);
      }
    }
    // Wait a period, or for good once it has stopped looking; `temp scan` wakes it early.
    const bool stopped = sensor == AIR_NONE && probesLeft == 0;
    if (ulTaskNotifyTake(pdTRUE, stopped ? portMAX_DELAY : pdMS_TO_TICKS(AIR_TEMP_PERIOD_MS))) {
      portENTER_CRITICAL(&lock);
      sensor = AIR_NONE;   // look again from scratch (also finds a sensor swapped for another)
      probesLeft = AIR_PROBE_TRIES;
      portEXIT_CRITICAL(&lock);
    }
  }
}

void airBegin() {
  xTaskCreatePinnedToCore(airTask, "air", 3072, nullptr, 1, &task, 0);
}

void airScan() {
  if (task) xTaskNotifyGive(task);
}

float airSoundSpeed(float tC, float rh, float hPa) {
  return (float)measure::soundSpeedMs(tC, rh, hPa * 100.0);
}

Air air() {
  Air a;
  portENTER_CRITICAL(&lock);
  const AirSensor s = sensor;
  const bool have = haveReading;
  const uint32_t at = lastAtMs;
  const float t = lastT, h = lastRh, p = lastHPa;
  const AirSensor from = readBy;
  portEXIT_CRITICAL(&lock);
  const uint32_t age = millis() - at;
  const bool fresh = have && age < AIR_TEMP_STALE_MS;
  a.sensor = s;
  a.ageS = have ? age / 1000 : 0;
  a.tMeasured = fresh;
  a.rhMeasured = fresh && from == AIR_BME280;
  a.pMeasured = fresh && (from == AIR_BME280 || from == AIR_BMP280);
  a.tC = a.tMeasured ? t : AIR_TEMP_DEFAULT_C;
  a.rh = a.rhMeasured ? h : AIR_RH_DEFAULT;
  a.hPa = a.pMeasured ? p : AIR_HPA_DEFAULT;
  a.c = airSoundSpeed(a.tC, a.rh, a.hPa);
  return a;
}

int airFormat(char* out, size_t size, const Air& a) {
  return snprintf(out, size, "air %.1f °C%s %.0f %%RH%s %.1f hPa%s, c %.2f m/s", a.tC,
                  a.tMeasured ? "" : " (assumed)", a.rh, a.rhMeasured ? "" : " (assumed)", a.hPa,
                  a.pMeasured ? "" : " (assumed)", a.c);
}

void airPrintStatus() {
  Air a = air();
  portENTER_CRITICAL(&lock);
  const uint32_t r = reads, f = fails, pr = probes, left = probesLeft;
  const uint8_t lf = lastFail;
  const bool have = haveReading;
  const uint8_t addr = bmeAddr;
  portEXIT_CRITICAL(&lock);
  if (a.sensor == AIR_NONE) {
    Serial.printf("air: no sensor on J5 (BME280/BMP280 on GPIO %d/%d, DS18B20 on GPIO %d), "
                  "looked %u times", AIR_SDA_PIN, AIR_SCL_PIN, AIR_SDA_PIN, (unsigned)pr);
    if (left == 0) Serial.print(", stopped looking (`temp scan` to look again)");
    if (have) Serial.printf("; the last reading was %u min ago (%s)", (unsigned)(a.ageS / 60), FAIL_TEXT[lf]);
    Serial.println();
  } else if (a.sensor == AIR_DS18B20) {
    Serial.printf("air: DS18B20 on GPIO %d", AIR_SDA_PIN);
  } else {
    Serial.printf("air: %s at 0x%02X (SDA GPIO %d, SCL GPIO %d)", SENSOR_TEXT[a.sensor], addr,
                  AIR_SDA_PIN, AIR_SCL_PIN);
  }
  if (a.sensor != AIR_NONE) {
    if (a.tMeasured)
      Serial.printf(", read %u s ago (%u readings, %u failed)\n", (unsigned)a.ageS, (unsigned)r, (unsigned)f);
    else
      Serial.printf(", no good reading yet (%u failed%s%s)\n", (unsigned)f, f ? ": " : "", f ? FAIL_TEXT[lf] : "");
  }
  char line[160];
  airFormat(line, sizeof line, a);
  Serial.printf("  %s\n", line);
}
