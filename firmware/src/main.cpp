// ESP Sensor - test firmware for the desktop app (NodeMCU-32S).
//
// - BLE peripheral, protocol in protocol/PROTOCOL.md
// - Heartbeat (millis()) every 2 s
// - BOOT short press toggles the onboard LED, BOOT held 2 s opens a 60 s pairing window
// - Sensor name stored in flash, changeable over BLE
#include <Arduino.h>
#include <Preferences.h>
#include "config.h"
#include "protocol.h"
#include "ble_link.h"

static Preferences prefs;
static char sensorName[NAME_MAX_LEN + 1];
static bool ledOn = false;
static bool pairing = false;
static uint32_t pairingEnd;

static bool isValidName(const char* s, size_t len) {
  if (len < 1 || len > (size_t)NAME_MAX_LEN) return false;
  for (size_t i = 0; i < len; i++)
    if (s[i] < 0x20 || s[i] > 0x7E) return false;
  return true;
}

// The stored name, or on first boot "ESPSensor-xyz" with 3 random hex digits.
static void loadName() {
  String stored = prefs.getString("name", "");
  if (isValidName(stored.c_str(), stored.length())) {
    strlcpy(sensorName, stored.c_str(), sizeof sensorName);
  } else {
    snprintf(sensorName, sizeof sensorName, "%s%03x", NAME_PREFIX, (unsigned)(esp_random() & 0xFFF));
    prefs.putString("name", sensorName);
  }
}

static void setLed(bool on) {
  ledOn = on;
  bleSetLed(on);                       // notify even if unchanged, so the app sees the real state
  Serial.printf("LED %s\n", on ? "on" : "off");
}

static void setPairing(bool on) {
  pairing = on;
  if (on) pairingEnd = millis() + PAIRING_WINDOW_MS;
  bleSetPairingMode(on);
  Serial.printf("Pairing mode %s\n", on ? "on (60 s)" : "off");
}

// Debounced BOOT button: short press toggles the LED, a 2 s hold starts pairing mode
// (fires while still held, so the user sees the LED start blinking).
static void pollButton(uint32_t now) {
  static int lastRaw = HIGH, stable = HIGH;
  static uint32_t changedAt = 0, pressedAt = 0;
  static bool longFired = false;

  int raw = digitalRead(BOOT_BTN_PIN);
  if (raw != lastRaw) { lastRaw = raw; changedAt = now; }
  if (now - changedAt >= DEBOUNCE_MS && raw != stable) {
    stable = raw;
    if (stable == LOW) {
      pressedAt = now;
      longFired = false;
    } else if (!longFired) {
      setLed(!ledOn);
    }
  }
  if (stable == LOW && !longFired && now - pressedAt >= LONG_PRESS_MS) {
    longFired = true;
    setPairing(true);
  }
}

// Apply what the app wrote. Invalid writes are answered with the unchanged value.
static void handleBleWrites() {
  uint8_t v[4];
  size_t len;
  if (bleTakeLedWrite(v, &len)) {
    if (len == 1 && v[0] <= 1) setLed(v[0] == 1);
    else bleSetLed(ledOn);
  }

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
}

void setup() {
  Serial.begin(115200);
  pinMode(LED_PIN, OUTPUT);
  digitalWrite(LED_PIN, LOW);
  pinMode(BOOT_BTN_PIN, INPUT_PULLUP);

  prefs.begin("sensor");
  loadName();
  Serial.printf("\nESP Sensor \"%s\", protocol v%d\n", sensorName, PROTOCOL_VERSION);

  bleBegin(sensorName, ledOn);
}

void loop() {
  static uint32_t lastBeat = 0;
  uint32_t now = millis();

  pollButton(now);
  handleBleWrites();

  if (pairing && (bleTakeNewBond() || (int32_t)(now - pairingEnd) >= 0)) setPairing(false);

  if (now - lastBeat >= HEARTBEAT_PERIOD_MS) {
    lastBeat = now;
    bleSetHeartbeat(now);
  }

  // In pairing mode the LED blinks; otherwise it shows the state the user set.
  bool out = pairing ? ((now / PAIRING_BLINK_MS) & 1) : ledOn;
  digitalWrite(LED_PIN, out ? HIGH : LOW);

  delay(LOOP_PERIOD_MS);
}
