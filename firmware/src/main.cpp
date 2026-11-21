// OpenSights - test firmware for the desktop app (NodeMCU-32S).
//
// - BLE peripheral, protocol in protocol/PROTOCOL.md
// - Heartbeat (millis()) every 2 s
// - Shots: the solved positions of the capture (capture.cpp, solver.cpp); a BOOT short press
//   makes a random test shot if that setting is on (serial `testshot on`). BOOT held 2 s
//   opens a 60 s pairing window, BOOT held 10 s resets the PIN and deletes all bonds
// - Shots are numbered, buffered and indicated to the app until it confirms them
// - The onboard LED shows status only: pairing mode, a PIN reset, shots
// - Sensor name and pairing PIN stored in flash, changeable over BLE
// - Capture of the comparator lines (capture.cpp): events and commands on the serial port
#include <Arduino.h>
#include <Preferences.h>
#include <math.h>
#include "config.h"
#include "protocol.h"
#include "ble_link.h"
#include "airtemp.h"
#include "capture.h"

static Preferences prefs;
static char sensorName[NAME_MAX_LEN + 1];
static bool pairing = false;
static uint32_t pairingEnd;
static uint32_t resetSignalEnd = 0;   // LED stays on until then after a PIN reset
static uint32_t shotFlashEnd = 0;     // LED flashes until then after a shot

// The last SHOT_BUFFER shots, kept so the app can ask for them again after a link drop.
// Numbered from 1 per boot; bootId tells the app which boot a number belongs to.
struct Shot {
  uint32_t seq, timeMs;
  int32_t  x, y;   // 0.01 mm from the centre, x right, y up (as seen by the shooter)
};
static Shot     shots[SHOT_BUFFER];
static uint32_t bootId;
static uint32_t lastSeq = 0;    // 0: no shot yet
static uint32_t sendNext = 1;   // next shot to indicate; lastSeq + 1 when the app has them all
static uint32_t inFlight = 0;   // the shot being indicated
static uint32_t retryAt = 0;
static uint32_t clearedSeq = 0; // shots up to this one were cleared by the app
static bool     testShots = false;   // BOOT short press makes a test shot (NVS "testshot")

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

static uint32_t oldestSeq() {
  uint32_t oldest = lastSeq >= (uint32_t)SHOT_BUFFER ? lastSeq - SHOT_BUFFER + 1 : 1;
  return oldest > clearedSeq ? oldest : clearedSeq + 1;
}

static void addShot(int32_t x, int32_t y, uint32_t now) {
  Shot& s = shots[(++lastSeq) % SHOT_BUFFER];
  s = {lastSeq, now, x, y};
  shotFlashEnd = now + SHOT_FLASH_MS;
  Serial.printf("Shot %u: x %.2f mm, y %.2f mm\n", (unsigned)lastSeq, x / 100.0, y / 100.0);
}

// A solved shot from the capture (called from captureService, in loop()).
static void onSolvedShot(float xMm, float yMm) {
  addShot(lroundf(xMm * 100.0f), lroundf(yMm * 100.0f), millis());
}

// A test shot, normally distributed around the centre (Box-Muller).
static void makeTestShot(uint32_t now) {
  float u1 = (esp_random() + 1.0f) / 4294967297.0f;   // (0, 1]: log() stays finite
  float u2 = esp_random() / 4294967296.0f;
  float r = sqrtf(-2.0f * logf(u1)) * TEST_SHOT_SIGMA_MM * 100.0f;
  addShot(lroundf(r * cosf(2 * PI * u2)), lroundf(r * sinf(2 * PI * u2)), now);
}

static void put32(uint8_t* p, uint32_t v) {
  for (int i = 0; i < 4; i++) p[i] = v >> (8 * i);
}

static uint32_t get32(const uint8_t* p) {
  return (uint32_t)p[0] | p[1] << 8 | p[2] << 16 | (uint32_t)p[3] << 24;
}

// Indicates the shots the app doesn't have yet, one at a time, oldest first. A shot counts
// as delivered only once the app confirmed it; a failed one is sent again.
static void serviceShots(uint32_t now) {
  // A fresh subscription gets new shots only, unless the app asks for older ones.
  if (bleTakeShotSubscribed()) sendNext = lastSeq + 1;

  uint8_t req[12];
  size_t len;
  if (bleTakeShotRequest(req, &len)) {
    if (len == SHOT_REQUEST_LEN) {
      // From the shot after the app's last one; a different boot means it has none of ours.
      uint32_t from = get32(req) == bootId ? get32(req + 4) : 1;
      sendNext = constrain(from, oldestSeq(), lastSeq + 1);
    } else if (len == SHOT_CLEAR_LEN && req[0] == SHOT_CLEAR && get32(req + 1) == bootId) {
      // The app cleared its series: forget the shots it has (never ones it hasn't seen).
      uint32_t upTo = get32(req + 5);
      if (upTo > lastSeq) upTo = lastSeq;
      if (upTo > clearedSeq) clearedSeq = upTo;
      Serial.printf("Shots up to %u cleared\n", (unsigned)clearedSeq);
    }
  }

  switch (bleTakeShotResult()) {
    case 1:  if (sendNext == inFlight) sendNext++; break;   // (unless a request moved it)
    case -1: retryAt = now + SHOT_RETRY_MS; break;
  }

  if (sendNext < oldestSeq()) sendNext = oldestSeq();   // overwritten meanwhile: lost
  if (sendNext > lastSeq || (int32_t)(now - retryAt) < 0 || !bleShotReady()) return;
  const Shot& s = shots[sendNext % SHOT_BUFFER];
  uint8_t buf[SHOT_LEN];
  put32(buf, s.seq);
  put32(buf + 4, bootId);
  put32(buf + 8, s.timeMs);
  put32(buf + 12, (uint32_t)s.x);
  put32(buf + 16, (uint32_t)s.y);
  if (bleSendShot(buf, sizeof buf)) inFlight = sendNext;
  else retryAt = now + SHOT_RETRY_MS;
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
    } else if (!longFired && testShots) {
      makeTestShot(now);
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

// The serial commands of this file; the rest go to captureCommand.
static void command(const char* line) {
  char cmd[16] = "", arg[8] = "";
  sscanf(line, "%15s %7s", cmd, arg);
  if (!strcmp(cmd, "testshot")) {
    if (!strcmp(arg, "on") || !strcmp(arg, "off")) {
      testShots = !strcmp(arg, "on");
      prefs.putBool("testshot", testShots);
    } else if (arg[0]) {
      Serial.println("testshot on|off");
      return;
    }
    Serial.printf("test shots (BOOT short press) %s\n", testShots ? "on" : "off");
    return;
  }
  captureCommand(line);
  if (!strcmp(cmd, "help"))
    Serial.printf("  testshot on|off      BOOT short press makes a random test shot (stored; now "
                  "%s)\n", testShots ? "on" : "off");
}

// Serial commands, one per line.
static void pollSerial() {
  static char line[64];
  static size_t len = 0;
  while (Serial.available()) {
    int c = Serial.read();
    if (c == '\r' || c == '\n') {
      if (len) { line[len] = 0; command(line); len = 0; }
    } else if (c == '\b' || c == 0x7F) {   // backspace: the terminal shows the corrected line
      if (len) len--;
    } else if (len < sizeof line - 1) {
      line[len++] = (char)c;
    }
  }
}

void setup() {
  Serial.setTxBufferSize(2048);   // event lines go out without blocking loop()
  Serial.begin(115200);
  pinMode(LED_PIN, OUTPUT);
  digitalWrite(LED_PIN, LOW);
  pinMode(BOOT_BTN_PIN, INPUT_PULLUP);

  bootId = esp_random() | 1;   // never 0, which the app sends when it has no shots yet
  prefs.begin("sensor");
  loadName();
  testShots = prefs.getBool("testshot", false);
  Serial.printf("\nOpenSights \"%s\", protocol v%d\n", sensorName, PROTOCOL_VERSION);

  bleBegin(sensorName, loadPin());
  if (prefs.getString("gatt", "") != GATT_BUILD_ID) {
    bleServicesChanged();
    prefs.putString("gatt", GATT_BUILD_ID);
    Serial.println("New firmware: told bonded computers to re-read the services");
  }
  captureOnShot(onSolvedShot);
  captureBegin();   // after BLE: the DMA ring takes 64 KB
  airTempBegin();   // DS18B20, once a minute, on core 0
}

void loop() {
  static uint32_t lastBeat = 0;
  uint32_t now = millis();

  pollButton(now);
  handleBleWrites();
  serviceShots(now);
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
                                                  : (int32_t)(now - shotFlashEnd) < 0;
  digitalWrite(LED_PIN, out ? HIGH : LOW);

  delay(LOOP_PERIOD_MS);
}
