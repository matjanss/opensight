// OpenSights - the shot store: see shotstore.h.
#include "shotstore.h"

#include <string.h>

namespace store {

uint32_t crc32(const uint8_t* p, size_t n) {
  uint32_t c = 0xFFFFFFFFu;
  for (size_t i = 0; i < n; i++) {
    c ^= p[i];
    for (int k = 0; k < 8; k++) c = (c >> 1) ^ (0xEDB88320u & (0u - (c & 1)));
  }
  return ~c;
}

static void put32(uint8_t* p, uint32_t v) {
  p[0] = v;
  p[1] = v >> 8;
  p[2] = v >> 16;
  p[3] = v >> 24;
}

static uint32_t get32(const uint8_t* p) {
  return (uint32_t)p[0] | (uint32_t)p[1] << 8 | (uint32_t)p[2] << 16 | (uint32_t)p[3] << 24;
}

// A slot's bytes as a hit, if they are one: the magic, the CRC, and H belonging to this slot.
static bool decode(const uint8_t* r, uint32_t slot, uint32_t slots, Hit& out) {
  if (get32(r) != MAGIC || get32(r + 60) != crc32(r, 60)) return false;
  out.h = get32(r + 4);
  if (out.h == 0 || out.h % slots != slot) return false;
  out.boot = get32(r + 8);
  out.seq = get32(r + 12);
  memcpy(out.payload, r + 16, PAYLOAD);
  return true;
}

static bool allFF(const uint8_t* p, uint32_t n) {
  for (uint32_t i = 0; i < n; i++)
    if (p[i] != 0xFF) return false;
  return true;
}

bool ShotStore::blank(uint32_t slot) {
  uint8_t r[SLOT];
  return fl->read(slot * SLOT, r, SLOT) && allFF(r, SLOT);
}

void ShotStore::noteBoot(uint32_t boot, uint32_t h, uint32_t seq) {
  if (nBoots && boots[nBoots - 1].boot == boot) {
    boots[nBoots - 1].lastH = h;
    boots[nBoots - 1].lastSeq = seq;
    return;
  }
  if (nBoots == MAX_BOOTS) {   // the oldest boot can't be found by a request any more
    memmove(boots, boots + 1, sizeof(Boot) * (MAX_BOOTS - 1));
    nBoots--;
  }
  boots[nBoots++] = {boot, h, h, seq};
}

void ShotStore::updateOldest() {
  uint32_t m = 0;
  for (uint32_t s = 0; s < sectors; s++)
    if (secMin[s] && (!m || secMin[s] < m)) m = secMin[s];
  oldestH = m ? m : nextH;
  // Boots wholly gone from the ring.
  int drop = 0;
  while (drop < nBoots && boots[drop].lastH < oldestH) drop++;
  if (drop) {
    memmove(boots, boots + drop, sizeof(Boot) * (nBoots - drop));
    nBoots -= drop;
  }
  if (nBoots && boots[0].firstH < oldestH) boots[0].firstH = oldestH;
}

bool ShotStore::begin(Flash* flash, uint32_t clearedH) {
  fl = flash;
  slots = fl->size() / SLOT;
  sectors = fl->size() / SECTOR;
  if (sectors < 3 || sectors > 256) return false;
  slots = sectors * SLOTS_PER_SECTOR;
  memset(secMin, 0, sizeof secMin);
  nBoots = 0;
  info = {};
  pendingSector = -1;

  // Pass 1: every slot; the valid records' H range, per-sector minimum, torn slots.
  uint32_t maxH = 0, minH = 0, nValid = 0;
  uint8_t r[SLOT];
  for (uint32_t slot = 0; slot < slots; slot++) {
    if (!fl->read(slot * SLOT, r, SLOT)) return false;
    Hit h;
    if (decode(r, slot, slots, h)) {
      nValid++;
      if (h.h > maxH) maxH = h.h;
      if (!minH || h.h < minH) minH = h.h;
      const uint32_t s = sectorOf(slot);
      if (!secMin[s] || h.h < secMin[s]) secMin[s] = h.h;
    } else if (!allFF(r, SLOT)) {
      info.torn++;
    }
  }
  // Pass 2: the boots, in H order. The valid records span less than one lap of the ring
  // (a slot holds one H at a time, and a sector is erased before the ring reuses it), so
  // slot order from the oldest's slot on is H order.
  for (uint32_t k = 0; nValid && k < slots; k++) {
    const uint32_t slot = (minH + k) % slots;
    if (!fl->read(slot * SLOT, r, SLOT)) return false;
    Hit h;
    if (decode(r, slot, slots, h)) noteBoot(h.boot, h.h, h.seq);
  }
  info.valid = nValid;
  info.newestH = maxH;
  cleared = clearedH;
  // H never goes back: past the newest hit, and past what was cleared (a wiped ring must not
  // hand out numbers the clear already covers).
  nextH = (maxH > clearedH ? maxH : clearedH) + 1;
  updateOldest();
  // The sector ahead of the writing should be blank; if not, erase it at the next chance.
  const uint32_t ahead = (sectorOf(slotOf(nextH)) + 1) % sectors;
  for (uint32_t k = 0; k < SLOTS_PER_SECTOR; k++)
    if (!blank(ahead * SLOTS_PER_SECTOR + k)) {
      pendingSector = ahead;
      break;
    }
  info.boots = nBoots;
  return true;
}

bool ShotStore::eraseSector(uint32_t sec) {
  if (!fl->erase(sec * SECTOR, SECTOR)) return false;
  secMin[sec] = 0;
  if (pendingSector == (int32_t)sec) pendingSector = -1;
  updateOldest();
  return true;
}

bool ShotStore::eraseAhead() {
  if (pendingSector < 0) return true;
  return eraseSector(pendingSector);
}

uint32_t ShotStore::append(uint32_t boot, uint32_t seq, const uint8_t payload[PAYLOAD]) {
  if (!fl) return 0;
  // A slot that isn't blank (a torn record, or a sector not erased yet): a new sector is
  // erased as the ring enters it; otherwise H skips the slot.
  for (uint32_t tries = 0; tries <= slots; tries++) {
    const uint32_t slot = slotOf(nextH);
    const uint32_t sec = sectorOf(slot);
    if (slot % SLOTS_PER_SECTOR == 0 && (secMin[sec] || !blank(slot))) {
      if (!eraseSector(sec)) return 0;
    }
    if (!blank(slot)) {
      nextH++;
      continue;
    }
    uint8_t r[SLOT];
    memset(r, 0, sizeof r);
    put32(r, MAGIC);
    put32(r + 4, nextH);
    put32(r + 8, boot);
    put32(r + 12, seq);
    memcpy(r + 16, payload, PAYLOAD);
    put32(r + 60, crc32(r, 60));
    const uint32_t h = nextH++;
    if (!fl->write(slot * SLOT, r, SLOT)) return 0;
    Hit back;
    if (!read(h, back)) continue;   // didn't take (a bad cell): the next slot
    if (!secMin[sec]) {
      secMin[sec] = h;
      updateOldest();
    }
    noteBoot(boot, h, seq);
    // Entering a sector: the one after it becomes the sector to erase ahead.
    if (slot % SLOTS_PER_SECTOR == 0) {
      const uint32_t ahead = (sec + 1) % sectors;
      if (secMin[ahead] || !blank(ahead * SLOTS_PER_SECTOR)) pendingSector = ahead;
    }
    return h;
  }
  return 0;
}

bool ShotStore::read(uint32_t h, Hit& out) {
  if (!fl || h == 0 || h >= nextH || h < oldestH) return false;
  uint8_t r[SLOT];
  const uint32_t slot = slotOf(h);
  return fl->read(slot * SLOT, r, SLOT) && decode(r, slot, slots, out) && out.h == h;
}

uint32_t ShotStore::first() {
  uint32_t h = cleared + 1 > oldestH ? cleared + 1 : oldestH;
  Hit x;
  while (h < nextH && !read(h, x)) h++;
  return h;
}

uint32_t ShotStore::next(uint32_t h) {
  Hit x;
  for (h = h + 1 < oldestH ? oldestH : h + 1; h < nextH; h++)
    if (read(h, x)) return h;
  return nextH;
}

const ShotStore::Boot* ShotStore::findBoot(uint32_t boot) const {
  for (int i = 0; i < nBoots; i++)
    if (boots[i].boot == boot) return &boots[i];
  return nullptr;
}

uint32_t ShotStore::from(uint32_t boot, uint32_t nextSeq) {
  const Boot* b = findBoot(boot);
  uint32_t h;
  if (!b) {
    h = first();
  } else if (nextSeq > b->lastSeq) {
    h = next(b->lastH);
  } else {
    h = nextH;
    Hit x;
    for (uint32_t k = b->firstH; k <= b->lastH; k++)
      if (read(k, x) && x.seq >= nextSeq) {
        h = k;
        break;
      }
  }
  // Never a cleared hit.
  const uint32_t f = first();
  return h < f ? f : h;
}

uint32_t ShotStore::clearTo(uint32_t boot, uint32_t lastSeq) {
  const Boot* b = findBoot(boot);
  if (!b) return 0;
  uint32_t upTo = 0;
  if (lastSeq >= b->lastSeq) {
    upTo = b->lastH;
  } else {
    Hit x;
    for (uint32_t k = b->firstH; k <= b->lastH; k++)
      if (read(k, x)) {
        if (x.seq > lastSeq) break;
        upTo = k;
      }
  }
  if (upTo > cleared) cleared = upTo;
  return cleared;
}

// At least this many of the newest hits are held: one sector is kept erased ahead, and the
// one being written may hold only its first hit (up to slots - one sector are held at times).
uint32_t ShotStore::capacity() const { return slots - 2 * SLOTS_PER_SECTOR + 1; }

uint32_t ShotStore::held() {
  uint32_t n = 0;
  Hit x;
  for (uint32_t h = oldestH; h < nextH; h++)
    if (read(h, x)) n++;
  return n;
}

// ---- RamFlash ----

RamFlash::RamFlash(uint8_t* mem, uint32_t size) : m(mem), n(size) {}

bool RamFlash::read(uint32_t off, void* buf, uint32_t len) {
  if (off + len > n) return false;
  memcpy(buf, m + off, len);
  return true;
}

bool RamFlash::write(uint32_t off, const void* buf, uint32_t len) {
  if (off + len > n) return false;
  writes++;
  const uint8_t* b = (const uint8_t*)buf;
  uint32_t k = len;
  if (tearAfter >= 0 && (uint32_t)tearAfter < len) k = tearAfter;
  for (uint32_t i = 0; i < k; i++) m[off + i] &= b[i];   // NOR flash: bits only go 1 -> 0
  if (tearAfter >= 0) {
    tearAfter = -1;
    return k == len;
  }
  return true;
}

bool RamFlash::erase(uint32_t off, uint32_t len) {
  if (off % SECTOR || len % SECTOR || off + len > n) return false;
  erases++;
  memset(m + off, 0xFF, tornErase ? len / 2 : len);
  const bool ok = !tornErase;
  tornErase = false;
  return ok;
}

}  // namespace store
