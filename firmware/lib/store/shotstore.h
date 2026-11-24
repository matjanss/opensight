// OpenSights - the shot store (PLAN D6, MEASUREMENT 5e): every hit kept in flash, across
// reboots, until it drops out of the ring or the app clears it. Portable: the flash comes in
// through `Flash` (the ESP32's partition on the target, RAM in the PC tests).
//
// The flash is a ring of 64-byte slots in 4 KB sectors. Each hit gets a store number H
// (from 1, never reused, also across reboots) and goes into slot H mod (slots). A record:
//
//   0  u32 magic "HIT1"   4  u32 H   8  u32 boot   12  u32 seq
//   16 32 bytes: the Shot message as sent (PROTOCOL.md)   48 12 bytes 0   60 u32 CRC-32 of 0..59
//
// - A record is written once and never changed: NOR flash only clears bits. A sector is
//   erased whole before the ring reuses it, which drops its 64 oldest hits.
// - One sector ahead of the writing is kept erased, at a quiet moment (eraseAhead), so a
//   4 KB erase (about 45 ms with the flash cache off) seldom falls on a shot.
// - Power cut while writing: the record fails its CRC and is skipped (begin counts it as
//   torn); H moves on past a slot that isn't blank. A hit is either whole or absent.
// - Hits of one boot are consecutive in H; begin rebuilds the list of boots from the slots.
// - The app's clear (PROTOCOL.md) is kept as the H up to which hits are cleared; the caller
//   stores it (NVS on the target) and hands it to begin.
#pragma once
#include <stddef.h>
#include <stdint.h>

namespace store {

constexpr uint32_t SLOT = 64;
constexpr uint32_t SECTOR = 4096;
constexpr uint32_t SLOTS_PER_SECTOR = SECTOR / SLOT;
constexpr uint32_t PAYLOAD = 32;
constexpr uint32_t MAGIC = 0x31544948;   // "HIT1" in the flash's byte order
constexpr int MAX_BOOTS = 64;             // boots findable by a request (older: "unknown")

// The flash, as offsets into the store's own area. Writes never cross a slot; erases are
// whole sectors.
class Flash {
 public:
  virtual ~Flash() {}
  virtual uint32_t size() const = 0;
  virtual bool read(uint32_t off, void* buf, uint32_t n) = 0;
  virtual bool write(uint32_t off, const void* buf, uint32_t n) = 0;
  virtual bool erase(uint32_t off, uint32_t n) = 0;
};

struct Hit {
  uint32_t h, boot, seq;
  uint8_t payload[PAYLOAD];
};

// What begin found: valid hits, torn slots (not blank, not valid), boots, and the newest H.
struct ScanInfo {
  uint32_t valid, torn, boots, newestH;
};

class ShotStore {
 public:
  // Reads the whole ring. `clearedH`: hits up to this H were cleared (0: none).
  bool begin(Flash* flash, uint32_t clearedH);

  // Stores a hit; returns its H, or 0 if the flash failed.
  uint32_t append(uint32_t boot, uint32_t seq, const uint8_t payload[PAYLOAD]);

  // The hit with store number h, if the ring still has it (whole).
  bool read(uint32_t h, Hit& out);

  uint32_t first();                      // the oldest hit not cleared (end() if none)
  // Below this nothing is sent (cleared or dropped out); no flash read (first() may be higher).
  uint32_t floor() const { return cleared + 1 > oldestH ? cleared + 1 : oldestH; }
  uint32_t end() const { return nextH; } // one past the newest
  uint32_t next(uint32_t h);             // the next hit held after h, or end()

  // Where a shot request (boot, next seq) starts: that boot's hit with seq >= next, then the
  // later boots'; a boot the ring has no hits of: every hit not cleared (first()).
  uint32_t from(uint32_t boot, uint32_t nextSeq);
  // A clear request (boot, last seq): hits up to it are cleared. Returns the new cleared H,
  // or 0 if the ring has no hits of that boot (ignored).
  uint32_t clearTo(uint32_t boot, uint32_t lastSeq);
  uint32_t clearedH() const { return cleared; }

  bool erasePending() const { return pendingSector >= 0; }
  bool eraseAhead();                     // erases the sector ahead now

  uint32_t capacity() const;             // the newest hits held at least (up to a sector more)
  uint32_t held();                       // hits it holds, cleared or not
  const ScanInfo& scan() const { return info; }

 private:
  struct Boot {
    uint32_t boot, firstH, lastH, lastSeq;
  };
  Flash* fl = nullptr;
  uint32_t slots = 0, sectors = 0;
  uint32_t nextH = 1, cleared = 0, oldestH = 1;
  int32_t pendingSector = -1;
  uint32_t secMin[256];                  // per sector: smallest H in it (0: none)
  Boot boots[MAX_BOOTS];
  int nBoots = 0;
  ScanInfo info = {};

  uint32_t slotOf(uint32_t h) const { return h % slots; }
  uint32_t sectorOf(uint32_t slot) const { return slot / SLOTS_PER_SECTOR; }
  bool blank(uint32_t slot);
  bool eraseSector(uint32_t sec);
  void noteBoot(uint32_t boot, uint32_t h, uint32_t seq);
  void updateOldest();
  const Boot* findBoot(uint32_t boot) const;
};

uint32_t crc32(const uint8_t* p, size_t n);

// A flash in RAM: the PC tests, and the target's fallback when the partition is missing.
// `tearAfter`: the next write stops after that many bytes, as a power cut would (the test
// then begins a new store on the same bytes); -1 = off.
class RamFlash : public Flash {
 public:
  RamFlash(uint8_t* mem, uint32_t size);
  uint32_t size() const override { return n; }
  bool read(uint32_t off, void* buf, uint32_t len) override;
  bool write(uint32_t off, const void* buf, uint32_t len) override;
  bool erase(uint32_t off, uint32_t len) override;
  int tearAfter = -1;
  bool tornErase = false;   // the next erase only blanks the first half (a cut mid-erase)
  uint32_t writes = 0, erases = 0;

 private:
  uint8_t* m;
  uint32_t n;
};

}  // namespace store
