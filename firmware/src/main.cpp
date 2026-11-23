// OpenSights - test firmware for the desktop app (NodeMCU-32S).
//
// - BLE peripheral, protocol in protocol/PROTOCOL.md
// - Heartbeat (millis()) every 2 s
// - BOOT short press makes a test shot (`testshot on|off`), BOOT held 2 s
//   opens a 60 s pairing window, BOOT held 10 s resets the PIN and deletes all bonds
// - Shots and events are numbered, buffered and indicated to the app until it confirms them
//   (uplink.cpp, protocol v3)
// - The onboard LED shows status only: pairing mode, a PIN reset, shots
// - Sensor name and pairing PIN stored in flash, changeable over BLE
// - Capture of the comparator lines (capture.cpp): events and commands on the serial port;
//   each event solved (position.cpp), a good fit sent to the app as a shot
#include <Arduino.h>
#include <Preferences.h>
#include <math.h>
#include "config.h"
#include "protocol.h"
#include "ble_link.h"
#include "airtemp.h"
#include "capture.h"
#include "position.h"
#include "uplink.h"

static Preferences prefs;
static char sensorName[NAME_MAX_LEN + 1];
static bool pairing = false;
static uint32_t pairingEnd;
static uint32_t resetSignalEnd = 0;   // LED stays on until then after a PIN reset

static bool isValidName(const char* s, size_t len) {
  if (len < 1 || len > (size_t)NAME_MAX_LEN) return false;
  for (size_t i = 0; i < len; i++)
    if (s[i] < 0x20 || s[i] > 0x7E) return false;
  return true;
}

// The stored name, or on first boot "OpenSights-xyz" with 3 random hex digits.
static void loadName() {
  String stored = prefs.getString("name", "");
  if (isValidName(stored.c_str(), stored.length())) {
    strlcpy(sensorName, stored.c_str(), sizeof sensorName);
  } else {
    snprintf(sensorName, sizeof sensorName, "%s%03x", NAME_PREFIX, (unsigned)(esp_random() & 0xFFF));
    prefs.putString("name", sensorName);
  }
}

static void setPairing(bool on) {
  pairing = on;
  if (on) pairingEnd = millis() + PAIRING_WINDOW_MS;
  bleSetPairingMode(on);
  Serial.printf("Pairing mode %s\n", on ? "on (60 s)" : "off");
}

// The stored PIN, or the default. The PIN itself is never printed.
static uint32_t loadPin() {
  uint32_t pin = prefs.getUInt("pin", DEFAULT_PASSKEY);
  return pin <= PASSKEY_MAX ? pin : DEFAULT_PASSKEY;
}

// Forgotten PIN: back to the default, every computer must pair again.
static void resetSecurity(uint32_t now) {
  prefs.remove("pin");
  bleSetPasskey(DEFAULT_PASSKEY);
  bleDeleteAllBonds();
  Serial.println("PIN reset to the default, all bonds deleted");
  resetSignalEnd = now + RESET_SIGNAL_MS;
  setPairing(true);   // restarts the 60 s window, so the user can pair right away
}

// A test shot, normally distributed around the centre (Box-Muller).
static void makeTestShot(uint32_t now) {
  float u1 = (esp_random() + 1.0f) / 4294967297.0f;   // (0, 1]: log() stays finite
  float u2 = esp_random() / 4294967296.0f;
  float r = sqrtf(-2.0f * logf(u1)) * TEST_SHOT_SIGMA_MM * 100.0f;
  const int32_t x = lroundf(r * cosf(2 * PI * u2)), y = lroundf(r * sinf(2 * PI * u2));
  const uint32_t seq = uplinkShot(proto::KIND_SHOT, x, y, proto::SIGMA_UNKNOWN, 0, 0, now);
  Serial.printf("Test shot %u: x %.2f mm, y %.2f mm\n", (unsigned)seq, x / 100.0, y / 100.0);
}

static uint32_t get32(const uint8_t* p) {
  return (uint32_t)p[0] | p[1] << 8 | p[2] << 16 | (uint32_t)p[3] << 24;
}

// Debounced BOOT button: short press makes a test shot, a 2 s hold starts pairing mode,
// a 10 s hold resets the PIN. Both fire while still held, so the user sees the LED react.
static void pollButton(uint32_t now) {
  static int lastRaw = HIGH, stable = HIGH;
  static uint32_t changedAt = 0, pressedAt = 0;
  static bool longFired = false, resetFired = false;

  int raw = digitalRead(BOOT_BTN_PIN);
  if (raw != lastRaw) { lastRaw = raw; changedAt = now; }
  if (now - changedAt >= DEBOUNCE_MS && raw != stable) {
    stable = raw;
    if (stable == LOW) {
      pressedAt = now;
      longFired = false;
      resetFired = false;
    } else if (!longFired) {
      if (positionTestShot()) makeTestShot(now);
      else Serial.println("BOOT: test shots are off (`testshot on`)");
    }
  }
  if (stable == LOW && !longFired && now - pressedAt >= LONG_PRESS_MS) {
    longFired = true;
    setPairing(true);
  }
  if (stable == LOW && !resetFired && now - pressedAt >= RESET_PRESS_MS) {
    resetFired = true;
    resetSecurity(now);
  }
}

// Apply what the app wrote. Invalid writes are answered with the unchanged value.
static void handleBleWrites() {
  size_t len;
  char buf[32];
  if (bleTakeNameWrite(buf, sizeof buf, &len)) {
    if (isValidName(buf, len)) {
      memcpy(sensorName, buf, len);
      sensorName[len] = '\0';
      prefs.putString("name", sensorName);
      Serial.printf("Renamed to \"%s\"\n", sensorName);
    }
    bleSetName(sensorName);
  }

  uint8_t p[8];
  if (bleTakePinWrite(p, &len)) {
    uint32_t pin = len == 4 ? get32(p) : UINT32_MAX;
    if (pin <= PASSKEY_MAX) {
      prefs.putUInt("pin", pin);
      bleSetPasskey(pin);
      int deleted = bleDeleteOtherBonds();
      Serial.printf("PIN changed, %d other bond(s) deleted\n", deleted);
    } else {
      Serial.println("Invalid PIN write ignored");
    }
  }
}

// Serial commands, one per line (see captureCommand).
static void pollSerial() {
  static char line[128];
  static size_t len = 0;
  while (Serial.available()) {
    int c = Serial.read();
    if (c == '\r' || c == '\n') {
      if (len) {
        line[len] = 0;
        // `pairing`: as BOOT held 2 s (the serial port means the board is in hand anyway).
        if (!strcmp(line, "pairing")) setPairing(true);
        else captureCommand(line);
        len = 0;
      }
    } else if (c == '\b' || c == 0x7F) {   // backspace: the terminal shows the corrected line
      if (len) len--;
    } else if (len < sizeof line - 1) {
      line[len++] = (char)c;
    }
  }
}

void setup() {
  Serial.setTxBufferSize(4096);   // event lines go out without blocking loop()
  Serial.setRxBufferSize(1024);   // a pasted block of commands (tools/calibrate.py's)
  Serial.begin(115200);
  pinMode(LED_PIN, OUTPUT);
  digitalWrite(LED_PIN, LOW);
  pinMode(BOOT_BTN_PIN, INPUT_PULLUP);

  uplinkBegin(esp_random() | 1);   // the boot id: never 0, which the app sends when it has no shots yet
  prefs.begin("sensor");
  loadName();
  Serial.printf("\nOpenSights \"%s\", protocol v%d\n", sensorName, PROTOCOL_VERSION);

  bleBegin(sensorName, loadPin());
  if (prefs.getString("gatt", "") != GATT_BUILD_ID) {
    bleServicesChanged();
    prefs.putString("gatt", GATT_BUILD_ID);
    Serial.println("New firmware: told bonded computers to re-read the services");
  }
  captureBegin();   // after BLE: the DMA ring takes 64 KB
  airBegin();   // the air sensor on J5 (BME280 or DS18B20), once a minute, on core 0
}

void loop() {
  static uint32_t lastBeat = 0;
  uint32_t now = millis();

  pollButton(now);
  handleBleWrites();
  uplinkService(now);
  bleService(now);
  captureService(now);
  pollSerial();

  if (pairing && (bleTakeNewBond() || (int32_t)(now - pairingEnd) >= 0)) setPairing(false);

  if (now - lastBeat >= HEARTBEAT_PERIOD_MS) {
    lastBeat = now;
    bleSetHeartbeat(now);
  }

  // Solid after a PIN reset, blinking in pairing mode, a short flash after a shot,
  // otherwise off.
  bool out = (int32_t)(now - resetSignalEnd) < 0 ? true
           : pairing                              ? ((now / PAIRING_BLINK_MS) & 1)
                                                  : uplinkLastShotMs() && now - uplinkLastShotMs() < SHOT_FLASH_MS;
  digitalWrite(LED_PIN, out ? HIGH : LOW);

  delay(LOOP_PERIOD_MS);
}
