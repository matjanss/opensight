// OpenSights - air temperature: a DS18B20 on DQ_PIN, read once a minute by a housekeeping
// task on core 0, away from the capture (core 1). A temperature error of 0.1 degC moves a
// shot by about 0.01 mm and the air changes slowly, so a reading a minute is plenty.
//
// The 1-Wire bus is driven by UART2, with RX and TX on the same pin (open drain; the
// Arduino core sets that up when both pins are the same). The UART times every slot, so no
// interrupts are held off (a bit-banged bus would hold them off ~70 us per bit):
//   reset:  0xF0 at 9600 baud; a sensor's presence pulse changes the byte that comes back
//   bit:    one byte at 115200 baud; 0xFF writes a 1 or reads a bit (0xFF back = 1),
//           0x00 writes a 0
// Readings are checked (CRC, the configuration byte's fixed bits, the power-on value 85 C,
// a plausible range), so a missing sensor or a floating pin can't pass as a temperature.
#include "airtemp.h"
#include "config.h"
#include "driver/gpio.h"

static HardwareSerial bus(2);

static volatile int32_t  lastMilliC = 0;      // last good reading, milli-degC
static volatile uint32_t lastAtMs = 0;
static volatile bool     haveReading = false;
static volatile uint32_t reads = 0, fails = 0;
static volatile uint8_t  lastFail = 0;

enum : uint8_t { FAIL_NONE, FAIL_NO_SENSOR, FAIL_BUS, FAIL_CRC, FAIL_VALUE };
static const char* const FAIL_TEXT[] = {
  "", "no DS18B20 answered", "the bus didn't answer every slot", "bad CRC",
  "an impossible value (85 C power-on, or out of range)",
};

static const float RANGE_MIN_C = -20.0f, RANGE_MAX_C = 60.0f;

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

static uint8_t readOnce(int32_t* milliC) {
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
  int32_t m = (int32_t)raw * 1000 / 16;
  if (m < RANGE_MIN_C * 1000 || m > RANGE_MAX_C * 1000) return FAIL_VALUE;
  *milliC = m;
  return FAIL_NONE;
}

static void airTempTask(void*) {
  bus.begin(9600, SERIAL_8N1, DQ_PIN, DQ_PIN);   // same pin for RX and TX: 1-Wire mode
  bus.setRxFIFOFull(1);                          // hand each byte over at once
  bus.setTimeout(20);
  // One-wire mode turns the pin's pulls off. The internal pull-up keeps a pin with no
  // sensor (and no 4.7 k pull-up) from floating into a fake presence pulse.
  gpio_pullup_en((gpio_num_t)DQ_PIN);
  for (;;) {
    int32_t m;
    uint8_t f = readOnce(&m);
    if (f == FAIL_NONE) {
      lastMilliC = m;
      lastAtMs = millis();
      haveReading = true;
      reads = reads + 1;
    } else {
      lastFail = f;
      fails = fails + 1;
    }
    vTaskDelay(pdMS_TO_TICKS(AIR_TEMP_PERIOD_MS));
  }
}

void airTempBegin() {
  xTaskCreatePinnedToCore(airTempTask, "airtemp", 3072, nullptr, 1, nullptr, 0);
}

AirTemp airTemp() {
  AirTemp t;
  t.everRead = haveReading;
  uint32_t age = millis() - lastAtMs;
  t.ageS = t.everRead ? age / 1000 : 0;
  t.measured = t.everRead && age < AIR_TEMP_STALE_MS;
  t.c = t.measured ? lastMilliC / 1000.0f : AIR_TEMP_DEFAULT_C;
  return t;
}

void airTempPrintStatus() {
  AirTemp t = airTemp();
  uint8_t f = lastFail;
  if (t.measured) {
    Serial.printf("air: %.2f °C from the DS18B20 on GPIO %d, read %u s ago (%u readings, %u "
                  "failed)\n", t.c, DQ_PIN, (unsigned)t.ageS, (unsigned)reads, (unsigned)fails);
  } else if (t.everRead) {
    Serial.printf("air: %.1f °C assumed: no good DS18B20 reading for %u min (last failure: "
                  "%s)\n", t.c, (unsigned)(t.ageS / 60), FAIL_TEXT[f]);
  } else {
    Serial.printf("air: %.1f °C assumed: %s on GPIO %d (%u tries)\n", t.c,
                  fails ? FAIL_TEXT[f] : "no reading yet", DQ_PIN, (unsigned)fails);
  }
}
