// ESP Sensor - the BLE GATT server (see protocol/PROTOCOL.md).
#include "ble_link.h"
#include "protocol.h"
#include <NimBLEDevice.h>

static NimBLEServer*         server;
static NimBLECharacteristic* heartbeatChr;
static NimBLECharacteristic* ledChr;
static NimBLECharacteristic* nameChr;
static std::string           advName;

static volatile bool pairingMode = false;
static volatile bool peerWasBonded = false;   // the connected central was bonded when it connected
static volatile bool newBond = false;

// Writes from the app, handed from the NimBLE task to the main loop.
static portMUX_TYPE writeMux = portMUX_INITIALIZER_UNLOCKED;
static uint8_t ledBuf[4];
static size_t  ledLen;
static bool    ledPending = false;
static char    nameBuf[32];                   // larger than NAME_MAX_LEN so too-long writes are seen
static size_t  nameLen;
static bool    namePending = false;

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
    peerWasBonded = bonded;
  }

  void onDisconnect(NimBLEServer*, NimBLEConnInfo&, int reason) override {
    Serial.printf("BLE: disconnected (reason 0x%x)\n", reason);
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
    if (info.isBonded() && !peerWasBonded) newBond = true;
  }

  // We have no display: the passkey is fixed, and the user types it on the computer.
  uint32_t onPassKeyDisplay() override { return PASSKEY; }
};

class LedCallbacks : public NimBLECharacteristicCallbacks {
  void onWrite(NimBLECharacteristic* c, NimBLEConnInfo&) override {
    const NimBLEAttValue& v = c->getValue();
    portENTER_CRITICAL(&writeMux);
    ledLen = v.size() < sizeof ledBuf ? v.size() : sizeof ledBuf;
    memcpy(ledBuf, v.data(), ledLen);
    ledPending = true;
    portEXIT_CRITICAL(&writeMux);
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

static ServerCallbacks serverCallbacks;
static LedCallbacks    ledCallbacks;
static NameCallbacks   nameCallbacks;

void bleBegin(const char* name, bool ledOn) {
  advName = name;
  NimBLEDevice::init(name);

  // Bond, require MITM protection and use LE Secure Connections. "Display only" makes the
  // computer ask the user for the passkey that we "display" (return from onPassKeyDisplay).
  NimBLEDevice::setSecurityAuth(true, true, true);
  NimBLEDevice::setSecurityPasskey(PASSKEY);
  NimBLEDevice::setSecurityIOCap(BLE_HS_IO_DISPLAY_ONLY);

  server = NimBLEDevice::createServer();
  server->setCallbacks(&serverCallbacks);
  server->advertiseOnDisconnect(true);

  NimBLEService* svc = server->createService(SERVICE_UUID);

  heartbeatChr = svc->createCharacteristic(HEARTBEAT_UUID, READ_SECURE | NIMBLE_PROPERTY::NOTIFY);
  heartbeatChr->setValue((uint32_t)0);

  ledChr = svc->createCharacteristic(LED_UUID, READ_SECURE | WRITE_SECURE | NIMBLE_PROPERTY::NOTIFY);
  ledChr->setValue((uint8_t)(ledOn ? 1 : 0));
  ledChr->setCallbacks(&ledCallbacks);

  nameChr = svc->createCharacteristic(NAME_UUID, READ_SECURE | WRITE_SECURE | NIMBLE_PROPERTY::NOTIFY);
  nameChr->setValue(name);
  nameChr->setCallbacks(&nameCallbacks);

  NimBLECharacteristic* versionChr = svc->createCharacteristic(VERSION_UUID, READ_SECURE);
  versionChr->setValue(PROTOCOL_VERSION);

  updateAdvertising();
}

void bleSetHeartbeat(uint32_t ms) {
  heartbeatChr->setValue(ms);
  heartbeatChr->notify();
}

void bleSetLed(bool on) {
  ledChr->setValue((uint8_t)(on ? 1 : 0));
  ledChr->notify();
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

bool bleTakeLedWrite(uint8_t* value, size_t* len) {
  bool got = false;
  portENTER_CRITICAL(&writeMux);
  if (ledPending) {
    memcpy(value, ledBuf, ledLen);
    *len = ledLen;
    ledPending = false;
    got = true;
  }
  portEXIT_CRITICAL(&writeMux);
  return got;
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

bool bleTakeNewBond() {
  if (!newBond) return false;
  newBond = false;
  return true;
}
