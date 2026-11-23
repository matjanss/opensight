// OpenSights - the BLE GATT server (see protocol/PROTOCOL.md).
#include "ble_link.h"
#include "protocol.h"
#include <NimBLEDevice.h>
#include <Preferences.h>

static NimBLEServer*         server;
static NimBLECharacteristic* heartbeatChr;
static NimBLECharacteristic* nameChr;
static NimBLECharacteristic* statusChr;
static NimBLECharacteristic* calibrationChr;
static NimBLECharacteristic* settingsChr;
static std::string           advName;

static volatile bool pairingMode = false;
// Set when a central finishes a secured (passkey/MITM) link while bonded: a new pairing or a
// bonded central reconnecting. Ends pairing mode.
static volatile bool newBond = false;
static volatile uint32_t passkey = DEFAULT_PASSKEY;   // read by NimBLE's task during pairing

// Writes from the app, handed from the NimBLE task to the main loop.
static portMUX_TYPE writeMux = portMUX_INITIALIZER_UNLOCKED;
static char    nameBuf[32];                   // larger than NAME_MAX_LEN so too-long writes are seen
static size_t  nameLen;
static bool    namePending = false;
static uint8_t pinBuf[8];                     // larger than 4 so wrong lengths are seen
static size_t  pinLen;
static bool    pinPending = false;

// A write waiting for the main loop. N is larger than the longest valid value, so wrong
// lengths are seen as such.
template <size_t N> struct WriteSlot {
  uint8_t buf[N];
  size_t  len = 0;
  bool    pending = false;
  void put(const NimBLEAttValue& v) {   // NimBLE's task
    portENTER_CRITICAL(&writeMux);
    len = v.size() < N ? v.size() : N;
    memcpy(buf, v.data(), len);
    pending = true;
    portEXIT_CRITICAL(&writeMux);
  }
  bool take(uint8_t* out, size_t* outLen) {   // the main loop
    bool got = false;
    portENTER_CRITICAL(&writeMux);
    if (pending) {
      memcpy(out, buf, len);
      *outLen = len;
      pending = false;
      got = true;
    }
    portEXIT_CRITICAL(&writeMux);
    return got;
  }
};
static WriteSlot<256> calibrationWrite;
static WriteSlot<64>  settingsWrite;
// Control commands come in bursts (mode, then a point): a small queue, oldest first.
static const int CONTROL_QUEUE = 4;
static WriteSlot<16> controlQueue[CONTROL_QUEUE];
static volatile uint32_t controlIn = 0, controlOut = 0;   // in: NimBLE's task, out: the main loop

// The indicated streams (shots, events). A link has one indication in flight at a time: the
// state moves from PENDING to CONFIRMED/FAILED in NimBLE's task and back to IDLE in the main
// loop, so each side only ever changes it from its own states.
enum : uint8_t { TX_IDLE, TX_PENDING, TX_CONFIRMED, TX_FAILED };
static volatile uint8_t tx = TX_IDLE;
static volatile uint8_t txStream = BLE_SHOT;
struct Indicated {
  NimBLECharacteristic* chr = nullptr;
  volatile bool     subscribed = false;
  volatile bool     subscribeEvent = false;
  volatile uint16_t conn = BLE_HS_CONN_HANDLE_NONE;
  WriteSlot<16>     request;
};
static Indicated streams[2];

// "Services changed" per bonded computer (bleServicesChanged): NimBLE keeps a computer's
// subscription to it only if it was made on an encrypted link, and BlueZ subscribes before the
// link is encrypted, so an indication at boot reaches nobody and BlueZ keeps its cached GATT
// layout. Instead, after a firmware update every bonded computer is told on its next secured
// connection, on the live link. The list survives a reboot (NVS "blesc").
static const int SC_MAX = 8;
static const size_t SC_ADDR = 7;   // 6 address bytes + type
static uint8_t  scPending[SC_MAX][SC_ADDR];
static int      scCount = 0;
static uint8_t  scAddr[SC_ADDR];          // the computer that just secured its link
static volatile bool     scDue = false;   // set in NimBLE's task, sent by bleService()
static volatile uint32_t scAt = 0;
static Preferences scPrefs;

static void scKey(const NimBLEAddress& a, uint8_t* out) {
  memcpy(out, a.getVal(), 6);
  out[6] = a.getType();
}

static int scFind(const uint8_t* key) {
  for (int i = 0; i < scCount; i++)
    if (!memcmp(scPending[i], key, SC_ADDR)) return i;
  return -1;
}

static void scSave() {
  if (scCount) scPrefs.putBytes("pending", scPending, scCount * SC_ADDR);
  else if (scPrefs.isKey("pending")) scPrefs.remove("pending");
}

// Read/notify/write all need an encrypted link made with the passkey (MITM), see "Security".
static const uint32_t READ_SECURE  = NIMBLE_PROPERTY::READ | NIMBLE_PROPERTY::READ_ENC | NIMBLE_PROPERTY::READ_AUTHEN;
static const uint32_t WRITE_SECURE = NIMBLE_PROPERTY::WRITE | NIMBLE_PROPERTY::WRITE_ENC | NIMBLE_PROPERTY::WRITE_AUTHEN;

// Rebuilds the advertisement (pairing flag) and scan response (name), and restarts
// advertising if nobody is connected. While connected we don't advertise: one app at a time.
static void updateAdvertising() {
  NimBLEAdvertising* adv = NimBLEDevice::getAdvertising();

  NimBLEAdvertisementData data;
  data.setFlags(BLE_HS_ADV_F_DISC_GEN | BLE_HS_ADV_F_BREDR_UNSUP);
  data.setCompleteServices(NimBLEUUID(SERVICE_UUID));
  uint8_t mfr[4] = {(uint8_t)(MANUFACTURER_ID & 0xFF), (uint8_t)(MANUFACTURER_ID >> 8),
                    PROTOCOL_VERSION, (uint8_t)(pairingMode ? ADV_FLAG_PAIRING : 0)};
  data.setManufacturerData(mfr, sizeof mfr);

  // The name goes in the scan response: the advertisement has no room left for 20 characters.
  NimBLEAdvertisementData scan;
  scan.setName(advName);

  adv->setAdvertisementData(data);
  adv->setScanResponseData(scan);
  adv->enableScanResponse(true);

  if (server->getConnectedCount() == 0) {
    adv->stop();
    adv->start();
  }
}

class ServerCallbacks : public NimBLEServerCallbacks {
  void onConnect(NimBLEServer* s, NimBLEConnInfo& info) override {
    bool bonded = NimBLEDevice::isBonded(info.getIdAddress());
    Serial.printf("BLE: connect from %s (%s, %d bond(s) stored)\n", info.getIdAddress().toString().c_str(),
                  bonded ? "bonded" : "not bonded", NimBLEDevice::getNumBonds());
    if (!bonded && !pairingMode) {
      // Strangers only get in during pairing mode. Advertising restarts on disconnect.
      Serial.println("BLE: not bonded and not in pairing mode, disconnecting");
      s->disconnect(info);
      return;
    }
  }

  void onDisconnect(NimBLEServer*, NimBLEConnInfo&, int reason) override {
    Serial.printf("BLE: disconnected (reason 0x%x)\n", reason);
    for (Indicated& st : streams) st.subscribed = false;
    if (tx == TX_PENDING) tx = TX_FAILED;
  }

  void onAuthenticationComplete(NimBLEConnInfo& info) override {
    if (!info.isEncrypted()) {
      Serial.println("BLE: pairing failed, disconnecting");
      server->disconnect(info);
      return;
    }
    Serial.printf("BLE: link secured (bonded: %s, passkey/MITM: %s)\n", info.isBonded() ? "yes" : "no",
                  info.isAuthenticated() ? "yes" : "no");
    if (!info.isAuthenticated()) {
      // Paired without the passkey ("Just Works"). Every characteristic needs MITM, so such
      // a bond is useless, and the central's next attempt would make NimBLE replace it
      // halfway. Drop it now. (NimBLE's own store, so safe from its task.)
      Serial.println("BLE: paired without the passkey, rejecting");
      NimBLEDevice::deleteBond(info.getIdAddress());
      server->disconnect(info);
      return;
    }
    if (info.isBonded()) newBond = true;   // ends pairing mode
    // A computer that hasn't been told of a firmware update yet: tell it shortly, on this link
    // (BlueZ's subscription to "services changed" is in place by then).
    uint8_t key[SC_ADDR];
    scKey(info.getIdAddress(), key);
    if (info.isBonded() && scFind(key) >= 0) {
      memcpy(scAddr, key, SC_ADDR);
      scAt = millis() + 1000;
      scDue = true;
      Serial.println("BLE: this computer hasn't been told of the firmware update yet: in 1 s");
    } else if (info.isBonded() && scCount) {
      Serial.printf("BLE: %d computer(s) to tell of a firmware update; not this one (%02x:%02x:%02x:%02x:%02x:%02x type %u)\n",
                    scCount, key[5], key[4], key[3], key[2], key[1], key[0], key[6]);
    }
  }

  // We have no display: the passkey is the stored PIN, and the user types it on the computer.
  uint32_t onPassKeyDisplay() override {
    return passkey;
  }
};

class NameCallbacks : public NimBLECharacteristicCallbacks {
  void onWrite(NimBLECharacteristic* c, NimBLEConnInfo&) override {
    const NimBLEAttValue& v = c->getValue();
    portENTER_CRITICAL(&writeMux);
    nameLen = v.size() < sizeof nameBuf ? v.size() : sizeof nameBuf;
    memcpy(nameBuf, v.data(), nameLen);
    namePending = true;
    portEXIT_CRITICAL(&writeMux);
  }
};

class PinCallbacks : public NimBLECharacteristicCallbacks {
  void onWrite(NimBLECharacteristic* c, NimBLEConnInfo&) override {
    const NimBLEAttValue& v = c->getValue();
    portENTER_CRITICAL(&writeMux);
    pinLen = v.size() < sizeof pinBuf ? v.size() : sizeof pinBuf;
    memcpy(pinBuf, v.data(), pinLen);
    pinPending = true;
    portEXIT_CRITICAL(&writeMux);
    c->setValue((uint32_t)0);   // don't keep the PIN in the attribute table
  }
};

// Shots and events: a write is a request (or a clear), the subscription and the indication's
// confirmation as for both.
class StreamCallbacks : public NimBLECharacteristicCallbacks {
 public:
  explicit StreamCallbacks(BleStream s) : which(s) {}
  void onWrite(NimBLECharacteristic* c, NimBLEConnInfo&) override { streams[which].request.put(c->getValue()); }

  // Only a secured link gets them: indications don't check the link's security themselves.
  void onSubscribe(NimBLECharacteristic*, NimBLEConnInfo& info, uint16_t subValue) override {
    Indicated& st = streams[which];
    bool on = (subValue & 0x0002) && info.isAuthenticated();
    st.conn = info.getConnHandle();
    st.subscribed = on;
    if (on) st.subscribeEvent = true;
  }

  // BLE_HS_EDONE: the app confirmed the indication. Anything else: it was not delivered.
  void onStatus(NimBLECharacteristic*, NimBLEConnInfo&, int code) override {
    if (tx == TX_PENDING && txStream == which) tx = code == BLE_HS_EDONE ? TX_CONFIRMED : TX_FAILED;
  }

 private:
  BleStream which;
};

class ControlCallbacks : public NimBLECharacteristicCallbacks {
  void onWrite(NimBLECharacteristic* c, NimBLEConnInfo&) override {
    if (controlIn - controlOut >= (uint32_t)CONTROL_QUEUE) return;   // full: dropped
    controlQueue[controlIn % CONTROL_QUEUE].put(c->getValue());
    controlIn++;
  }
};

// Calibration and settings: the whole value is checked and applied by the main loop, which
// then sets the stored value again (so a rejected write reads back as the unchanged one).
class CalibrationCallbacks : public NimBLECharacteristicCallbacks {
  void onWrite(NimBLECharacteristic* c, NimBLEConnInfo&) override { calibrationWrite.put(c->getValue()); }
};
class SettingsCallbacks : public NimBLECharacteristicCallbacks {
  void onWrite(NimBLECharacteristic* c, NimBLEConnInfo&) override { settingsWrite.put(c->getValue()); }
};

// Changes whenever this file (which defines the GATT layout) is rebuilt.
const char* const GATT_BUILD_ID = __DATE__ " " __TIME__;

static ServerCallbacks serverCallbacks;
static NameCallbacks   nameCallbacks;
static PinCallbacks    pinCallbacks;
static StreamCallbacks shotCallbacks(BLE_SHOT);
static StreamCallbacks eventCallbacks(BLE_EVENT);
static ControlCallbacks controlCallbacks;
static CalibrationCallbacks calibrationCallbacks;
static SettingsCallbacks settingsCallbacks;

void bleBegin(const char* name, uint32_t pin) {
  advName = name;
  scPrefs.begin("blesc");
  const size_t n = scPrefs.isKey("pending") ? scPrefs.getBytesLength("pending") : 0;
  if (n && n <= sizeof scPending && n % SC_ADDR == 0) {
    scPrefs.getBytes("pending", scPending, n);
    scCount = n / SC_ADDR;
  }
  if (scCount) Serial.printf("BLE: %d bonded computer(s) still to tell of a firmware update\n", scCount);
  NimBLEDevice::init(name);

  // Bond, require MITM protection and use LE Secure Connections. "Display only" makes the
  // computer ask the user for the passkey that we "display" (return from onPassKeyDisplay).
  NimBLEDevice::setSecurityAuth(true, true, true);
  bleSetPasskey(pin);
  NimBLEDevice::setSecurityIOCap(BLE_HS_IO_DISPLAY_ONLY);

  server = NimBLEDevice::createServer();
  server->setCallbacks(&serverCallbacks);
  server->advertiseOnDisconnect(true);

  NimBLEService* svc = server->createService(SERVICE_UUID);

  heartbeatChr = svc->createCharacteristic(HEARTBEAT_UUID, READ_SECURE | NIMBLE_PROPERTY::NOTIFY);
  heartbeatChr->setValue((uint32_t)0);

  nameChr = svc->createCharacteristic(NAME_UUID, READ_SECURE | WRITE_SECURE | NIMBLE_PROPERTY::NOTIFY);
  nameChr->setValue(name);
  nameChr->setCallbacks(&nameCallbacks);

  NimBLECharacteristic* versionChr = svc->createCharacteristic(VERSION_UUID, READ_SECURE);
  versionChr->setValue(PROTOCOL_VERSION);

  // Write only: the PIN can be changed but never read back.
  NimBLECharacteristic* pinChr = svc->createCharacteristic(PIN_UUID, WRITE_SECURE);
  pinChr->setCallbacks(&pinCallbacks);

  // Shots and events are indicated (the app confirms each one); writes ask for them again.
  streams[BLE_SHOT].chr = svc->createCharacteristic(SHOT_UUID, WRITE_SECURE | NIMBLE_PROPERTY::INDICATE);
  streams[BLE_SHOT].chr->setCallbacks(&shotCallbacks);
  streams[BLE_EVENT].chr = svc->createCharacteristic(EVENT_UUID, WRITE_SECURE | NIMBLE_PROPERTY::INDICATE);
  streams[BLE_EVENT].chr->setCallbacks(&eventCallbacks);

  statusChr = svc->createCharacteristic(STATUS_UUID, READ_SECURE | NIMBLE_PROPERTY::NOTIFY);
  NimBLECharacteristic* controlChr = svc->createCharacteristic(CONTROL_UUID, WRITE_SECURE);
  controlChr->setCallbacks(&controlCallbacks);
  // Longer than an MTU: GATT's long read and write (up to 512 bytes, NimBLE's default).
  calibrationChr = svc->createCharacteristic(CALIBRATION_UUID, READ_SECURE | WRITE_SECURE);
  calibrationChr->setCallbacks(&calibrationCallbacks);
  settingsChr = svc->createCharacteristic(SETTINGS_UUID, READ_SECURE | WRITE_SECURE);
  settingsChr->setCallbacks(&settingsCallbacks);

  updateAdvertising();
}

void bleSetHeartbeat(uint32_t ms) {
  heartbeatChr->setValue(ms);
  heartbeatChr->notify();
}

void bleSetName(const char* name) {
  nameChr->setValue(name);
  nameChr->notify();
  if (advName != name) {
    advName = name;
    NimBLEDevice::setDeviceName(name);
    updateAdvertising();
  }
}

void bleSetPairingMode(bool on) {
  if (pairingMode == on) return;
  pairingMode = on;
  updateAdvertising();
}

bool bleIsConnected() { return server->getConnectedCount() > 0; }

uint16_t bleMtu() { return server->getConnectedCount() > 0 ? server->getPeerInfo(0).getMTU() : 0; }

void bleSetStatus(const uint8_t* value, size_t len) {
  statusChr->setValue(value, len);
  statusChr->notify();
}
void bleSetCalibration(const uint8_t* value, size_t len) { calibrationChr->setValue(value, len); }
void bleSetSettings(const uint8_t* value, size_t len) { settingsChr->setValue(value, len); }

void bleServicesChanged() {
  server->sendServiceChangedIndication();   // for any that do keep the subscription
  scCount = 0;
  for (int i = 0; i < NimBLEDevice::getNumBonds() && scCount < SC_MAX; i++)
    scKey(NimBLEDevice::getBondedAddress(i), scPending[scCount++]);
  scSave();
  for (int i = 0; i < scCount; i++)
    Serial.printf("BLE: to tell of the update: %02x:%02x:%02x:%02x:%02x:%02x type %u\n", scPending[i][5],
                  scPending[i][4], scPending[i][3], scPending[i][2], scPending[i][1], scPending[i][0],
                  scPending[i][6]);
}

void bleService(uint32_t now) {
  if (!scDue || (int32_t)(now - scAt) < 0) return;
  scDue = false;
  if (server->getConnectedCount() == 0) return;   // gone again: next time
  // NimBLE indicates "services changed" only where the subscription is in its own state, and
  // BlueZ doesn't write it again on a bonded link (it counts on the server keeping it). So the
  // indication goes to this link directly, on the characteristic's handle: BlueZ handles it.
  server->sendServiceChangedIndication();   // sets the changed range (all handles)
  uint16_t sc = 0;
  static const ble_uuid16_t gattSvc = BLE_UUID16_INIT(0x1801), changedChr = BLE_UUID16_INIT(0x2A05);
  const int found = ble_gatts_find_chr(&gattSvc.u, &changedChr.u, nullptr, &sc);
  const int sent = found == 0 ? ble_gatts_indicate(server->getPeerInfo(0).getConnHandle(), sc) : found;
  Serial.printf("BLE: services changed indicated directly on handle 0x%04x (rc %d)\n", sc, sent);
  const int i = scFind(scAddr);
  if (i >= 0) {
    memmove(scPending[i], scPending[i + 1], (scCount - i - 1) * SC_ADDR);
    scCount--;
    scSave();
  }
  Serial.printf("BLE: told %02x:%02x:%02x:%02x:%02x:%02x to re-read the services (new firmware); "
                "%d bonded computer(s) still to tell\n", scAddr[5], scAddr[4], scAddr[3], scAddr[2],
                scAddr[1], scAddr[0], scCount);
}

// NimBLE's own static passkey stays at its default (123456): only then does NimBLE ask
// onPassKeyDisplay(), which hands out this PIN and notes that a pairing happened.
void bleSetPasskey(uint32_t pin) { passkey = pin; }

int bleDeleteOtherBonds() {
  bool keep = server->getConnectedCount() > 0;
  NimBLEAddress peer = keep ? server->getPeerInfo(0).getIdAddress() : NimBLEAddress();
  int deleted = 0;
  for (int i = NimBLEDevice::getNumBonds() - 1; i >= 0; i--) {   // backwards: deleting shifts
    NimBLEAddress a = NimBLEDevice::getBondedAddress(i);
    if (keep && a == peer) continue;
    if (NimBLEDevice::deleteBond(a)) deleted++;
  }
  return deleted;
}

void bleDeleteAllBonds() {
  if (server->getConnectedCount() > 0) server->disconnect(server->getPeerInfo(0));
  NimBLEDevice::deleteAllBonds();
}

bool bleTakeNameWrite(char* buf, size_t bufSize, size_t* len) {
  bool got = false;
  portENTER_CRITICAL(&writeMux);
  if (namePending) {
    *len = nameLen < bufSize ? nameLen : bufSize;
    memcpy(buf, nameBuf, *len);
    namePending = false;
    got = true;
  }
  portEXIT_CRITICAL(&writeMux);
  return got;
}

bool bleTakePinWrite(uint8_t* value, size_t* len) {
  bool got = false;
  portENTER_CRITICAL(&writeMux);
  if (pinPending) {
    memcpy(value, pinBuf, pinLen);
    *len = pinLen;
    pinPending = false;
    got = true;
  }
  portEXIT_CRITICAL(&writeMux);
  return got;
}

bool bleReady(BleStream s) { return streams[s].subscribed && tx == TX_IDLE; }

bool bleSend(BleStream s, const uint8_t* value, size_t len) {
  if (!bleReady(s)) return false;
  txStream = s;
  tx = TX_PENDING;   // before sending: the confirmation can arrive at once
  if (streams[s].chr->indicate(value, len, streams[s].conn)) return true;
  tx = TX_IDLE;
  return false;
}

int bleTakeResult(BleStream s) {
  uint8_t t = tx;
  if (txStream != s || (t != TX_CONFIRMED && t != TX_FAILED)) return 0;
  tx = TX_IDLE;
  return t == TX_CONFIRMED ? 1 : -1;
}

bool bleTakeSubscribed(BleStream s) {
  if (!streams[s].subscribeEvent) return false;
  streams[s].subscribeEvent = false;
  return true;
}

bool bleTakeRequest(BleStream s, uint8_t* value, size_t* len) { return streams[s].request.take(value, len); }

bool bleTakeControl(uint8_t* value, size_t* len) {
  if (controlOut == controlIn) return false;
  bool got = controlQueue[controlOut % CONTROL_QUEUE].take(value, len);
  controlOut++;
  return got;
}

bool bleTakeCalibrationWrite(uint8_t* value, size_t* len) { return calibrationWrite.take(value, len); }
bool bleTakeSettingsWrite(uint8_t* value, size_t* len) { return settingsWrite.take(value, len); }

bool bleTakeNewBond() {
  if (!newBond) return false;
  newBond = false;
  return true;
}
