// The shot store (lib/store) on the PC: reboots, requests across boots, wrap-around, torn
// writes and erases (a power cut), clears. MEASUREMENT 5e.
#include <unity.h>

#include <stdio.h>
#include <string.h>

#include <random>
#include <vector>

#include "shotstore.h"

using namespace store;

void setUp() {}
void tearDown() {}

static const uint32_t SECTORS = 8;
static uint8_t mem[SECTORS * SECTOR];

static void wipe() { memset(mem, 0xFF, sizeof mem); }

// A payload that says which hit it is, so any mix-up shows.
static void payloadFor(uint32_t boot, uint32_t seq, uint8_t* p) {
  for (uint32_t i = 0; i < PAYLOAD; i++) p[i] = (uint8_t)(boot * 31 + seq * 7 + i * 13);
}

static uint32_t put(ShotStore& s, uint32_t boot, uint32_t seq) {
  uint8_t p[PAYLOAD];
  payloadFor(boot, seq, p);
  return s.append(boot, seq, p);
}

// Every hit the store hands out, in order: whole, in H order, and its payload its own.
struct Seen {
  uint32_t h, boot, seq;
};
static std::vector<Seen> walk(ShotStore& s, uint32_t from) {
  std::vector<Seen> out;
  uint32_t last = 0;
  for (uint32_t h = from; h < s.end(); h = s.next(h)) {
    Hit x;
    TEST_ASSERT_TRUE_MESSAGE(s.read(h, x), "next() gave a hit read() refuses");
    TEST_ASSERT_TRUE(h > last);
    last = h;
    uint8_t p[PAYLOAD];
    payloadFor(x.boot, x.seq, p);
    TEST_ASSERT_EQUAL_MEMORY_MESSAGE(p, x.payload, PAYLOAD, "a hit's payload is not its own");
    out.push_back({h, x.boot, x.seq});
  }
  return out;
}

static void test_empty() {
  wipe();
  RamFlash f(mem, sizeof mem);
  ShotStore s;
  TEST_ASSERT_TRUE(s.begin(&f, 0));
  TEST_ASSERT_EQUAL_UINT32(0, s.scan().valid);
  TEST_ASSERT_EQUAL_UINT32(1, s.first());
  TEST_ASSERT_EQUAL_UINT32(1, s.end());
  TEST_ASSERT_EQUAL_UINT32(s.end(), s.from(0, 0));
  TEST_ASSERT_EQUAL_UINT32((SECTORS - 2) * SLOTS_PER_SECTOR + 1, s.capacity());
}

static void test_reboot_keeps_hits() {
  wipe();
  RamFlash f(mem, sizeof mem);
  {
    ShotStore s;
    s.begin(&f, 0);
    for (uint32_t q = 1; q <= 100; q++) TEST_ASSERT_EQUAL_UINT32(q, put(s, 0xA, q));
  }
  ShotStore s;   // a reboot: a new store on the same flash
  TEST_ASSERT_TRUE(s.begin(&f, 0));
  TEST_ASSERT_EQUAL_UINT32(100, s.scan().valid);
  TEST_ASSERT_EQUAL_UINT32(0, s.scan().torn);
  std::vector<Seen> all = walk(s, s.first());
  TEST_ASSERT_EQUAL_UINT32(100, all.size());
  for (uint32_t i = 0; i < 100; i++) TEST_ASSERT_EQUAL_UINT32(i + 1, all[i].seq);
  // Requests: from a seq, past the last, an unknown boot.
  TEST_ASSERT_EQUAL_UINT32(51, s.from(0xA, 51));
  TEST_ASSERT_EQUAL_UINT32(s.end(), s.from(0xA, 101));
  TEST_ASSERT_EQUAL_UINT32(1, s.from(0x1234, 5));
  TEST_ASSERT_EQUAL_UINT32(1, s.from(0, 0));
  TEST_ASSERT_EQUAL_UINT32(101, put(s, 0xB, 1));   // H goes on after a reboot
}

static void test_across_boots() {
  wipe();
  RamFlash f(mem, sizeof mem);
  {
    ShotStore s;
    s.begin(&f, 0);
    for (uint32_t q = 1; q <= 10; q++) put(s, 0xA, q);
  }
  {
    ShotStore s;
    s.begin(&f, 0);
    for (uint32_t q = 1; q <= 5; q++) put(s, 0xB, q);
  }
  ShotStore s;
  s.begin(&f, 0);
  TEST_ASSERT_EQUAL_UINT32(2, s.scan().boots);
  // The app had A 7: it gets A 8, 9, 10, then B 1-5.
  std::vector<Seen> got = walk(s, s.from(0xA, 8));
  TEST_ASSERT_EQUAL_UINT32(8, got.size());
  TEST_ASSERT_EQUAL_UINT32(0xA, got[0].boot);
  TEST_ASSERT_EQUAL_UINT32(8, got[0].seq);
  TEST_ASSERT_EQUAL_UINT32(0xB, got[3].boot);
  TEST_ASSERT_EQUAL_UINT32(1, got[3].seq);
  // The app had all of A: B from 1. All of B: nothing.
  TEST_ASSERT_EQUAL_UINT32(11, s.from(0xA, 11));
  TEST_ASSERT_EQUAL_UINT32(s.end(), s.from(0xB, 6));
  // An app with nothing: everything, both boots.
  TEST_ASSERT_EQUAL_UINT32(15, walk(s, s.from(0, 0)).size());
}

// Many laps; boots change every 37 hits; a reboot now and then; the quiet erase sometimes.
static void test_wrap_around() {
  wipe();
  RamFlash f(mem, sizeof mem);
  ShotStore s;
  s.begin(&f, 0);
  const uint32_t cap = s.capacity();
  uint32_t boot = 1, seq = 0, total = 0;
  for (uint32_t i = 0; i < 5 * SECTORS * SLOTS_PER_SECTOR; i++) {
    if (i % 37 == 0) {
      boot++;
      seq = 0;
      if (i % 111 == 0) {   // a reboot
        s = ShotStore();
        TEST_ASSERT_TRUE(s.begin(&f, 0));
      }
    }
    TEST_ASSERT_NOT_EQUAL(0, put(s, boot, ++seq));
    total++;
    if (i % 5 == 0) s.eraseAhead();
  }
  std::vector<Seen> all = walk(s, s.first());
  TEST_ASSERT_TRUE(all.size() >= cap);
  TEST_ASSERT_TRUE(all.size() <= SECTORS * SLOTS_PER_SECTOR);
  // The newest hits, without a gap: H consecutive, seq consecutive within a boot.
  for (size_t i = 1; i < all.size(); i++) {
    TEST_ASSERT_EQUAL_UINT32(all[i - 1].h + 1, all[i].h);
    if (all[i].boot == all[i - 1].boot) TEST_ASSERT_EQUAL_UINT32(all[i - 1].seq + 1, all[i].seq);
  }
  TEST_ASSERT_EQUAL_UINT32(total, all.back().h);
  // A reboot finds the same.
  ShotStore t;
  t.begin(&f, 0);
  TEST_ASSERT_EQUAL_UINT32(all.size(), walk(t, t.first()).size());
  TEST_ASSERT_EQUAL_UINT32(0, t.scan().torn);
  // A request for a boot that has dropped out: everything held.
  TEST_ASSERT_EQUAL_UINT32(t.first(), t.from(2, 1));
  // A boot in the middle: from its seq on.
  const Seen& mid = all[all.size() / 2];
  TEST_ASSERT_EQUAL_UINT32(mid.h, t.from(mid.boot, mid.seq));
  printf("wrap: %u hits written, %u held (capacity %u), erases %u\n", (unsigned)total,
         (unsigned)all.size(), (unsigned)cap, (unsigned)f.erases);
}

// A power cut in the middle of a write, at every byte, over and over: no hit is ever handed
// out damaged, none written before is lost, the torn one is absent, and the store goes on.
static void test_torn_writes() {
  std::mt19937 rng(20261124);
  uint32_t trials = 0;
  for (int cut = 0; cut < (int)SLOT; cut++) {
    wipe();
    RamFlash f(mem, sizeof mem);
    ShotStore s;
    s.begin(&f, 0);
    uint32_t boot = 100 + cut, seq = 0;
    const uint32_t before = 50 + rng() % (2 * SECTORS * SLOTS_PER_SECTOR);
    for (uint32_t i = 0; i < before; i++) put(s, boot, ++seq);
    f.tearAfter = cut;
    TEST_ASSERT_EQUAL_UINT32(0, put(s, boot, ++seq));   // torn: the write fails
    // What the store held just before the cut (entering a new lap, this append may have
    // erased the oldest sector first: those hits dropped out of the ring, not to the cut).
    std::vector<Seen> kept = walk(s, s.first());
    // Reboot.
    ShotStore t;
    TEST_ASSERT_TRUE(t.begin(&f, 0));
    std::vector<Seen> after = walk(t, t.first());
    // Everything kept before is still there (the torn hit, if it decoded at all, is extra;
    // a write cut at byte 64 doesn't exist, so it is always absent here).
    TEST_ASSERT_EQUAL_UINT32(kept.size(), after.size());
    for (size_t i = 0; i < kept.size(); i++) {
      TEST_ASSERT_EQUAL_UINT32(kept[i].h, after[i].h);
      TEST_ASSERT_EQUAL_UINT32(kept[i].seq, after[i].seq);
    }
    if (cut > 0) TEST_ASSERT_EQUAL_UINT32(1, t.scan().torn);
    // It goes on: 30 more, a new boot, all found after another reboot.
    for (uint32_t q = 1; q <= 30; q++) TEST_ASSERT_NOT_EQUAL(0, put(t, boot + 1000, q));
    ShotStore u;
    u.begin(&f, 0);
    std::vector<Seen> last = walk(u, u.from(boot + 1000, 1));
    TEST_ASSERT_EQUAL_UINT32(30, last.size());
    TEST_ASSERT_EQUAL_UINT32(30, last.back().seq);
    trials++;
  }
  printf("torn writes: cut at each of %u bytes, all whole\n", (unsigned)trials);
}

// A power cut in the middle of an erase: half the sector blank, half old records.
static void test_torn_erase() {
  wipe();
  RamFlash f(mem, sizeof mem);
  ShotStore s;
  s.begin(&f, 0);
  uint32_t seq = 0;
  while (!s.erasePending() || seq < SECTORS * SLOTS_PER_SECTOR + 10) put(s, 7, ++seq);
  f.tornErase = true;
  TEST_ASSERT_FALSE(s.eraseAhead());
  ShotStore t;
  TEST_ASSERT_TRUE(t.begin(&f, 0));
  walk(t, t.first());   // every hit whole
  TEST_ASSERT_TRUE(t.erasePending());
  for (uint32_t q = 1; q <= 3 * SLOTS_PER_SECTOR; q++) TEST_ASSERT_NOT_EQUAL(0, put(t, 8, q));
  ShotStore u;
  u.begin(&f, 0);
  std::vector<Seen> got = walk(u, u.from(8, 1));
  TEST_ASSERT_EQUAL_UINT32(3 * SLOTS_PER_SECTOR, got.size());
}

static void test_clear() {
  wipe();
  RamFlash f(mem, sizeof mem);
  ShotStore s;
  s.begin(&f, 0);
  for (uint32_t q = 1; q <= 20; q++) put(s, 0xA, q);
  TEST_ASSERT_EQUAL_UINT32(0, s.clearTo(0xDEAD, 5));   // not a boot it has: ignored
  const uint32_t c = s.clearTo(0xA, 12);
  TEST_ASSERT_EQUAL_UINT32(12, c);
  TEST_ASSERT_EQUAL_UINT32(13, s.first());
  TEST_ASSERT_EQUAL_UINT32(13, s.from(0xA, 5));        // a cleared hit is never sent again
  TEST_ASSERT_EQUAL_UINT32(13, s.from(0, 0));
  TEST_ASSERT_EQUAL_UINT32(12, s.clearTo(0xA, 3));     // a clear never goes back
  TEST_ASSERT_EQUAL_UINT32(20, s.clearTo(0xA, 999));   // beyond the newest: all
  TEST_ASSERT_EQUAL_UINT32(s.end(), s.first());
  // Kept across a reboot (the caller stores the cleared H).
  ShotStore t;
  t.begin(&f, c);
  TEST_ASSERT_EQUAL_UINT32(13, t.first());
  // A wiped ring with a cleared H left in NVS: new hits are numbered past it, never cleared.
  wipe();
  ShotStore u;
  u.begin(&f, 500);
  const uint32_t h = put(u, 0xC, 1);
  TEST_ASSERT_EQUAL_UINT32(501, h);
  TEST_ASSERT_EQUAL_UINT32(h, u.first());
  TEST_ASSERT_EQUAL_UINT32(h, u.from(0, 0));
}

static void test_erase_ahead() {
  wipe();
  RamFlash f(mem, sizeof mem);
  ShotStore s;
  s.begin(&f, 0);
  uint32_t seq = 0;
  // First lap: the sectors ahead are blank, nothing to erase.
  for (; seq < SLOTS_PER_SECTOR * SECTORS - 1; seq++) put(s, 1, seq + 1);
  TEST_ASSERT_EQUAL_UINT32(0, f.erases);
  // Into the second lap: the ring enters sector 0 again (erased then), sector 1 is pending.
  put(s, 1, ++seq);
  put(s, 1, ++seq);
  TEST_ASSERT_TRUE(s.erasePending());
  const uint32_t e = f.erases;
  TEST_ASSERT_TRUE(s.eraseAhead());
  TEST_ASSERT_FALSE(s.erasePending());
  TEST_ASSERT_EQUAL_UINT32(e + 1, f.erases);
  // Through sector 1 without a quiet moment: no erase needed (done), then sector 2 is due
  // as the ring enters sector 1 ... and gets erased on entry if nobody erased it ahead.
  for (uint32_t i = 0; i < 2 * SLOTS_PER_SECTOR; i++) TEST_ASSERT_NOT_EQUAL(0, put(s, 1, ++seq));
  walk(s, s.first());
}

int main() {
  UNITY_BEGIN();
  RUN_TEST(test_empty);
  RUN_TEST(test_reboot_keeps_hits);
  RUN_TEST(test_across_boots);
  RUN_TEST(test_wrap_around);
  RUN_TEST(test_torn_writes);
  RUN_TEST(test_torn_erase);
  RUN_TEST(test_clear);
  RUN_TEST(test_erase_ahead);
  return UNITY_END();
}
