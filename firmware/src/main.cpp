// ESP Sensor - test firmware for the desktop app (NodeMCU-32S).
//
// - BLE peripheral, protocol in protocol/PROTOCOL.md
// - Heartbeat (millis()) every 2 s
// - BOOT short press toggles the onboard LED, BOOT held 2 s opens a 60 s pairing window,
//   BOOT held 10 s resets the PIN and deletes all bonds
// - Sensor name and pairing PIN stored in flash, changeable over BLE
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
static uint32_t resetSignalEnd = 0;   // LED stays on until then after a PIN reset

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

// Debounced BOOT button: short press toggles the LED, a 2 s hold starts pairing mode,
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
      setLed(!ledOn);
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

  uint8_t p[8];
  if (bleTakePinWrite(p, &len)) {
    uint32_t pin = len == 4 ? (uint32_t)p[0] | p[1] << 8 | p[2] << 16 | (uint32_t)p[3] << 24 : UINT32_MAX;
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

void setup() {
  Serial.begin(115200);
  pinMode(LED_PIN, OUTPUT);
  digitalWrite(LED_PIN, LOW);
  pinMode(BOOT_BTN_PIN, INPUT_PULLUP);

  prefs.begin("sensor");
  loadName();
  Serial.printf("\nESP Sensor \"%s\", protocol v%d\n", sensorName, PROTOCOL_VERSION);

  bleBegin(sensorName, ledOn, loadPin());
  if (prefs.getString("gatt", "") != GATT_BUILD_ID) {
    bleServicesChanged();
    prefs.putString("gatt", GATT_BUILD_ID);
    Serial.println("New firmware: told bonded computers to re-read the services");
  }
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

  // Solid after a PIN reset, blinking in pairing mode, otherwise the state the user set.
  bool out = (int32_t)(now - resetSignalEnd) < 0 ? true : pairing ? ((now / PAIRING_BLINK_MS) & 1) : ledOn;
  digitalWrite(LED_PIN, out ? HIGH : LOW);

  delay(LOOP_PERIOD_MS);
}
