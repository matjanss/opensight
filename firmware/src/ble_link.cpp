// OpenSights - the BLE GATT server (see protocol/PROTOCOL.md).
#include "ble_link.h"
#include "protocol.h"
#include <NimBLEDevice.h>

static NimBLEServer*         server;
static NimBLECharacteristic* heartbeatChr;
static NimBLECharacteristic* nameChr;
static NimBLECharacteristic* shotChr;
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
static uint8_t shotReqBuf[12];                // larger than SHOT_REQUEST_LEN so wrong lengths are seen
static size_t  shotReqLen;
static bool    shotReqPending = false;

// Shot indications. The state moves from PENDING to CONFIRMED/FAILED in NimBLE's task and
// back to IDLE in the main loop, so each side only ever changes it from its own states.
enum : uint8_t { TX_IDLE, TX_PENDING, TX_CONFIRMED, TX_FAILED };
static volatile uint8_t  shotTx = TX_IDLE;
static volatile bool     shotSubscribed = false;
static volatile bool     shotSubscribeEvent = false;
static volatile uint16_t shotConn = BLE_HS_CONN_HANDLE_NONE;

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
    shotSubscribed = false;
    if (shotTx == TX_PENDING) shotTx = TX_FAILED;
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

class ShotCallbacks : public NimBLECharacteristicCallbacks {
  // A request to resend buffered shots.
  void onWrite(NimBLECharacteristic* c, NimBLEConnInfo&) override {
    const NimBLEAttValue& v = c->getValue();
    portENTER_CRITICAL(&writeMux);
    shotReqLen = v.size() < sizeof shotReqBuf ? v.size() : sizeof shotReqBuf;
    memcpy(shotReqBuf, v.data(), shotReqLen);
    shotReqPending = true;
    portEXIT_CRITICAL(&writeMux);
  }

  // Only a secured link gets shots: indications don't check the link's security themselves.
  void onSubscribe(NimBLECharacteristic*, NimBLEConnInfo& info, uint16_t subValue) override {
    bool on = (subValue & 0x0002) && info.isAuthenticated();
    shotConn = info.getConnHandle();
    shotSubscribed = on;
    if (on) shotSubscribeEvent = true;
  }

  // BLE_HS_EDONE: the app confirmed the indication. Anything else: it was not delivered.
  void onStatus(NimBLECharacteristic*, NimBLEConnInfo&, int code) override {
    if (shotTx == TX_PENDING) shotTx = code == BLE_HS_EDONE ? TX_CONFIRMED : TX_FAILED;
  }
};

// Changes whenever this file (which defines the GATT layout) is rebuilt.
const char* const GATT_BUILD_ID = __DATE__ " " __TIME__;

static ServerCallbacks serverCallbacks;
static NameCallbacks   nameCallbacks;
static PinCallbacks    pinCallbacks;
static ShotCallbacks   shotCallbacks;

void bleBegin(const char* name, uint32_t pin) {
  advName = name;
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

  // Shots are indicated (the app confirms each one); writes ask for buffered shots again.
  shotChr = svc->createCharacteristic(SHOT_UUID, WRITE_SECURE | NIMBLE_PROPERTY::INDICATE);
  shotChr->setCallbacks(&shotCallbacks);

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

void bleServicesChanged() { server->sendServiceChangedIndication(); }

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

bool bleShotReady() { return shotSubscribed && shotTx == TX_IDLE; }

bool bleSendShot(const uint8_t* value, size_t len) {
  if (!bleShotReady()) return false;
  shotTx = TX_PENDING;   // before sending: the confirmation can arrive at once
  if (shotChr->indicate(value, len, shotConn)) return true;
  shotTx = TX_IDLE;
  return false;
}

int bleTakeShotResult() {
  uint8_t s = shotTx;
  if (s != TX_CONFIRMED && s != TX_FAILED) return 0;
  shotTx = TX_IDLE;
  return s == TX_CONFIRMED ? 1 : -1;
}

bool bleTakeShotSubscribed() {
  if (!shotSubscribeEvent) return false;
  shotSubscribeEvent = false;
  return true;
}

bool bleTakeShotRequest(uint8_t* value, size_t* len) {
  bool got = false;
  portENTER_CRITICAL(&writeMux);
  if (shotReqPending) {
    memcpy(value, shotReqBuf, shotReqLen);
    *len = shotReqLen;
    shotReqPending = false;
    got = true;
  }
  portEXIT_CRITICAL(&writeMux);
  return got;
}

bool bleTakeNewBond() {
  if (!newBond) return false;
  newBond = false;
  return true;
}
