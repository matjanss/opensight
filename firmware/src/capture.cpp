// OpenSights - capture of the comparator lines (measurement step 1, see ../MEASUREMENT.md).
//
// Data path:
//   16 lines --GPIO matrix--> I2S0 (camera mode, 16-bit, clocked by LEDC at 10 MHz)
//   --DMA--> ring of CAPTURE_RING_BLOCKS blocks --ISR--> activity scan per finished block
//   --task notify--> measureTask: first edges of the event window --queue--> loop(): serial
//
// The ring is frozen around each event: at the trigger the ISR links the window's last
// descriptor to two spare blocks that point to each other, so once the window is full the
// DMA keeps sampling into the spares and the window can't be overwritten, however long
// the task takes (a flash write stalls it: the ISR and everything it touches are in
// IRAM/DRAM and keep running, the task doesn't). The task links the ring back in when it
// is done. Blocks into the spares still count as time; activity in them after the dead
// time is reported as a possibly missed shot.
#include "capture.h"
#include <Preferences.h>
#include "config.h"
#include "driver/gpio.h"
#include "esp_cpu.h"
#include "esp_heap_caps.h"
#include "esp_intr_alloc.h"
#include "esp_private/periph_ctrl.h"
#include "esp_rom_gpio.h"
#include "esp_rom_lldesc.h"
#include "esp_timer.h"
#include "soc/gpio_periph.h"
#include "soc/gpio_pins.h"
#include "soc/gpio_reg.h"
#include "soc/gpio_sig_map.h"
#include "soc/i2s_struct.h"
#include "soc/io_mux_reg.h"

// ---- Sizes derived from config.h ----

static const int      LINES             = 16;
static const int      WORDS_PER_BLOCK   = CAPTURE_BLOCK_BYTES / 4;
static const int      SAMPLES_PER_BLOCK = CAPTURE_BLOCK_BYTES / 2;
static const uint32_t NS_PER_SAMPLE     = 1000000000UL / CAPTURE_SAMPLE_HZ;
static const uint32_t BLOCK_US          = (uint32_t)((uint64_t)SAMPLES_PER_BLOCK * 1000000 / CAPTURE_SAMPLE_HZ);
static const int      PRE_BLOCKS        = 1;   // the block before the first activity
// Blocks from the trigger block on. The window's last block must be at least 4 blocks after
// the trigger (time for the ISR to relink it) and the window at most the ring.
static const uint32_t POST_MIN          = 5;
static const uint32_t POST_MAX          = CAPTURE_RING_BLOCKS - PRE_BLOCKS - 1;
static const int      MAX_TRANSITIONS   = 16;  // per line and event, kept for verbose output

static_assert(CAPTURE_BLOCK_BYTES % 4 == 0 && CAPTURE_BLOCK_BYTES <= 4092, "DMA block size");
static_assert(POST_MAX >= POST_MIN, "ring too short");
static_assert((PRE_BLOCKS + POST_MAX) * SAMPLES_PER_BLOCK < 32768, "window indexes must fit 15 bits");

// Time the window covers after the first edge for a mic span. The first edge can be at the
// very end of the trigger block, so only the blocks after it count.
static uint32_t neededUs(uint32_t spanMm) {
  uint32_t us = spanMm * 1000 / SOUND_SPEED_MIN_M_S;   // mm / (m/s) = ms; * 1000 = us
  return us + us * CAPTURE_WINDOW_MARGIN_PCT / 100 + CAPTURE_WINDOW_EXTRA_US;
}
static uint32_t postBlocksFor(uint32_t spanMm) {
  uint32_t post = (neededUs(spanMm) + BLOCK_US - 1) / BLOCK_US + 1;
  return post < POST_MIN ? POST_MIN : post;
}
static uint32_t coveredUs(uint32_t post) { return (post - 1) * BLOCK_US; }

// ---- Event results (measureTask -> loop) ----

struct LineResult {
  int16_t rise;    // window sample of the first rising edge, -1: none
  int16_t fall;    // first falling edge after it, -1: none
  uint8_t edges;   // all transitions in the window (saturates at 255)
  uint8_t nTrans;  // transitions stored in trans[]
  uint16_t trans[MAX_TRANSITIONS];   // window sample | 0x8000 for a rising edge
};

struct EventResult {
  uint32_t number;
  uint32_t firstTime;    // window sample 0, in blocks since start (spare blocks included)
  uint16_t blocks;       // window length in blocks
  uint16_t lines;        // line mask used for this event
  uint16_t highAtStart;  // lines already high at window start
  LineResult line[LINES];
};

// ---- State shared with the ISR ----

enum : uint8_t { EV_IDLE, EV_COLLECTING, EV_PROCESSING };

static lldesc_t  desc[CAPTURE_RING_BLOCKS];
static uint32_t* ring[CAPTURE_RING_BLOCKS];
static lldesc_t  spare[2];                   // the DMA's target while the ring is frozen
static uint32_t* spareBuf[2];
static uint32_t  descAddr0, spareAddr[2];    // as in_eof_des_addr reports them
static volatile uint32_t blocksDone = 0;     // all blocks since start, spares included: time
static volatile uint32_t ringDone = 0;       // ring blocks done; ring block k is ring[k % RING]
static uint32_t          lastDesc = CAPTURE_RING_BLOCKS - 1;   // last ring descriptor counted
static volatile uint32_t contigFrom = 0;     // ring blocks are continuous in time from here
static volatile uint32_t lineMask32 = 0;     // fitted lines, in both 16-bit halves
static volatile uint32_t postBlocks = POST_MIN;   // from the mic span setting
static volatile uint8_t  evState = EV_IDLE;
static volatile uint32_t trigRing = 0;       // ring block with the first activity
static volatile uint32_t trigTime = 0;       // ... and its time in blocks
static volatile uint32_t evPost = 0;         // postBlocks of this event
static volatile uint32_t endDesc = 0;        // ring descriptor of the window's last block
static volatile uint32_t quietUntil = 0;     // no new event before this time (dead time)
static volatile uint32_t spareBlocks = 0;    // blocks into the spares (frozen ring)
static volatile uint32_t missedCount = 0;    // activity in the spares after the dead time
static volatile bool     missedThisFreeze = false;
static volatile uint32_t isrMaxCycles = 0;
static volatile uint64_t isrCycles = 0;
static TaskHandle_t      measureTaskHandle = nullptr;
static intr_handle_t     isrHandle = nullptr;

// ---- Settings and statistics (loop / task) ----

static Preferences prefs;
static uint16_t channelMask = CAPTURE_MASK_DEFAULT;      // bit n = CH(n+1)
static uint16_t savedMask = CAPTURE_MASK_DEFAULT;        // stored mask, during a self-test
static uint32_t deadtimeMs = CAPTURE_DEADTIME_DEFAULT;
static volatile uint32_t deadBlocks = 0;
static uint32_t spanMm = CAPTURE_SPAN_MM_DEFAULT;
static bool     verbose = false;
static bool     running = false;
static QueueHandle_t results = nullptr;
static uint32_t eventCount = 0, droppedCount = 0, missedReported = 0;
static uint32_t frozenMax = 0;   // longest freeze, in blocks
static int64_t  startUs = 0;

static uint16_t linesOf(uint16_t channels) {
  uint16_t m = 0;
  for (int c = 0; c < 8; c++)
    if (channels & (1 << c)) m |= 3 << (2 * c);
  return m;
}

static void applyMask(uint16_t channels) {
  channelMask = channels;
  uint32_t m = linesOf(channels);
  lineMask32 = m | (m << 16);
}

// ---- ISR ----

// True if a fitted line changes level inside the block. Pulses last 5-50 us and a block
// 200 us, so every pulse has at least one edge inside some block.
static inline bool IRAM_ATTR blockActive(const uint32_t* w, uint32_t mask) {
  uint32_t orAll = 0, andAll = 0xFFFFFFFF;
  for (int i = 0; i < WORDS_PER_BLOCK; i += 4) {
    uint32_t a = w[i], b = w[i + 1], c = w[i + 2], d = w[i + 3];
    orAll |= a | b | c | d;
    andAll &= a & b & c & d;
  }
  uint32_t o = (orAll >> 16) | (orAll & 0xFFFF);
  uint32_t n = (andAll >> 16) & (andAll & 0xFFFF);
  return ((o & ~n) & mask & 0xFFFF) != 0;
}

static void IRAM_ATTR captureIsr(void*) {
  uint32_t c0 = esp_cpu_get_cycle_count();
  uint32_t st = I2S0.int_st.val;
  I2S0.int_clr.val = st;

  // Which descriptor ended: from the DMA's own record, so a late interrupt that covers two
  // ring blocks still counts both. A spare block means the window is complete: count the
  // window blocks not counted yet, then the spare.
  uint32_t addr = I2S0.in_eof_des_addr & 0xFFFFF;
  int sp = addr == spareAddr[0] ? 0 : addr == spareAddr[1] ? 1 : -1;
  uint32_t idx;
  if (sp >= 0) {
    // Also after the task has linked the ring back in: the DMA finishes the spare it is in
    // (and maybe the next one, already loaded). Those blocks are time too.
    idx = evState == EV_IDLE ? lastDesc : endDesc;
  } else {
    idx = (addr - descAddr0) / sizeof(lldesc_t);
    if (addr < descAddr0 || idx >= (uint32_t)CAPTURE_RING_BLOCKS) return;
  }
  uint32_t n = (idx + CAPTURE_RING_BLOCKS - lastDesc) % CAPTURE_RING_BLOCKS;
  lastDesc = idx;

  uint32_t mask = lineMask32;
  for (uint32_t k = 0; k < n; k++) {
    uint32_t rb = ringDone, t = blocksDone;
    if (evState == EV_IDLE && rb >= contigFrom + PRE_BLOCKS && (int32_t)(t - quietUntil) >= 0 &&
        blockActive(ring[rb % CAPTURE_RING_BLOCKS], mask)) {
      // Freeze: the window's last descriptor leads into the spares, which loop.
      uint32_t post = postBlocks;
      uint32_t end = (rb + post - 1) % CAPTURE_RING_BLOCKS;
      spare[0].qe.stqe_next = &spare[1];
      spare[1].qe.stqe_next = &spare[0];
      desc[end].qe.stqe_next = &spare[0];
      trigRing = rb;
      trigTime = t;
      evPost = post;
      endDesc = end;
      quietUntil = t + deadBlocks;
      missedThisFreeze = false;
      evState = EV_COLLECTING;
    }
    ringDone = rb + 1;
    blocksDone = t + 1;
  }
  if (sp >= 0) {
    uint32_t t = blocksDone;
    blocksDone = t + 1;
    spareBlocks = spareBlocks + 1;
    if (!missedThisFreeze && (int32_t)(t - quietUntil) >= 0 && blockActive(spareBuf[sp], mask)) {
      missedThisFreeze = true;
      missedCount = missedCount + 1;
    }
  }

  BaseType_t woken = pdFALSE;
  if (evState == EV_COLLECTING && ringDone >= trigRing + evPost) {
    evState = EV_PROCESSING;
    vTaskNotifyGiveFromISR(measureTaskHandle, &woken);
  }

  uint32_t dt = esp_cpu_get_cycle_count() - c0;
  isrCycles += dt;
  if (dt > isrMaxCycles) isrMaxCycles = dt;
  if (woken) portYIELD_FROM_ISR();
}

// ---- Edge extraction (measureTask) ----

static void extract(EventResult& r, uint32_t first, uint32_t blocks) {
  const uint16_t mask = lineMask32 & 0xFFFF;
  r.lines = mask;
  for (int l = 0; l < LINES; l++) {
    r.line[l].rise = -1;
    r.line[l].fall = -1;
    r.line[l].edges = 0;
    r.line[l].nTrans = 0;
  }

  const uint32_t* w0 = ring[first % CAPTURE_RING_BLOCKS];
  uint16_t prev = (CAPTURE_HIGH_HALF_FIRST ? (w0[0] >> 16) : (w0[0] & 0xFFFF)) & mask;
  r.highAtStart = prev;
  uint32_t idle = prev | ((uint32_t)prev << 16);
  int idx = 0;

  for (uint32_t j = 0; j < blocks; j++) {
    const uint32_t* w = ring[(first + j) % CAPTURE_RING_BLOCKS];
    for (int k = 0; k < WORDS_PER_BLOCK; k++, idx += 2) {
      uint32_t v = w[k] & (mask | ((uint32_t)mask << 16));
      if (v == idle) continue;   // nothing changed in either sample
      uint16_t s[2];
      s[0] = CAPTURE_HIGH_HALF_FIRST ? (v >> 16) : (v & 0xFFFF);
      s[1] = CAPTURE_HIGH_HALF_FIRST ? (v & 0xFFFF) : (v >> 16);
      for (int h = 0; h < 2; h++) {
        uint16_t ch = s[h] ^ prev;
        while (ch) {
          int l = __builtin_ctz(ch);
          ch &= ch - 1;
          bool rising = s[h] & (1 << l);
          LineResult& L = r.line[l];
          int at = idx + h;
          if (L.edges < 255) L.edges++;
          if (L.nTrans < MAX_TRANSITIONS) L.trans[L.nTrans++] = at | (rising ? 0x8000 : 0);
          if (rising && L.rise < 0) L.rise = at;
          else if (!rising && L.rise >= 0 && L.fall < 0) L.fall = at;
        }
        prev = s[h];
      }
      idle = prev | ((uint32_t)prev << 16);
    }
  }
}

static void measureTask(void*) {
  static EventResult r;   // static: too big for this task's stack
  for (;;) {
    ulTaskNotifyTake(pdTRUE, portMAX_DELAY);
    uint32_t blocks = PRE_BLOCKS + evPost;
    r.number = ++eventCount;
    r.firstTime = trigTime - PRE_BLOCKS;
    r.blocks = blocks;
    extract(r, trigRing - PRE_BLOCKS, blocks);

    // Unfreeze: the DMA is in the spares; link the ring back in after the window's last
    // block. Ring first, so the descriptor is right before the DMA can come round to it.
    uint32_t frozen = blocksDone - (trigTime + evPost);
    if (frozen > frozenMax) frozenMax = frozen;
    lldesc_t* next = &desc[(endDesc + 1) % CAPTURE_RING_BLOCKS];
    desc[endDesc].qe.stqe_next = next;
    spare[0].qe.stqe_next = next;
    spare[1].qe.stqe_next = next;
    contigFrom = trigRing + evPost;   // the next ring block follows a gap in time
    evState = EV_IDLE;
    if (xQueueSend(results, &r, 0) != pdTRUE) droppedCount++;
  }
}

// ---- Self-test ----

static struct {
  bool     active;
  uint32_t gapNs, count, sent, seen, glitches, missedAtStart;
  uint32_t spacingMs, nextAt;
  int32_t  gapMin, gapMax;
  int64_t  gapSum;
  // The generator reads its pins back, so each captured gap is checked against the gap
  // the pins really had: BLE traffic or a flash write can delay the CPU's pin writes.
  bool     pending;          // a pulse was sent and its event not seen yet
  bool     genStuck;         // the last pulse: a pin did not read high
  int32_t  genGapNs;         // the last pulse: H rise - L rise, as the pins read back
  uint32_t genOff;           // pulses whose generated gap was off the set gap by > 1 sample
  uint32_t genStuckCount;    // pulses where a pin did not read high
  uint32_t mismatch;         // events whose captured gap disagreed with the generated one
  int32_t  diffMin, diffMax; // captured - generated gap
  uint32_t otherLines[LINES];
} st;

static const int LINE_ST_L = 14, LINE_ST_H = 15;   // CH8 L, CH8 H
// Captured vs generated gap: each edge is quantised to one sample, so up to one sample
// apart, plus the pin read-back jitter.
static const int32_t SELFTEST_TOLERANCE_NS = NS_PER_SAMPLE + 50;

// Cycle count at which the pins in `bits` all read high, or 0 after `limit` cycles.
static inline uint32_t IRAM_ATTR waitHigh(uint32_t bits, uint32_t c0, uint32_t limit) {
  for (;;) {
    uint32_t c = esp_cpu_get_cycle_count();
    if ((REG_READ(GPIO_IN_REG) & bits) == bits) return c ? c : 1;
    if (c - c0 > limit) return 0;
  }
}

// Both lines rise (OUT_L first, OUT_H gapCyc after OUT_L really is high) and fall
// together, like a comparator pair on a real edge. In IRAM with interrupts off, so a
// cache miss can't stretch it. Returns the read-back gap in cycles, or -1 if a pin
// did not go high.
static int32_t IRAM_ATTR selftestPulse(uint32_t gapCyc, uint32_t pulseCyc, uint32_t limit) {
  static portMUX_TYPE mux = portMUX_INITIALIZER_UNLOCKED;
  const uint32_t bL = 1u << SELFTEST_PIN_L, bH = 1u << SELFTEST_PIN_H;
  int32_t gap = -1;
  portENTER_CRITICAL(&mux);
  uint32_t c0 = esp_cpu_get_cycle_count();
  uint32_t tL, tH;
  if (gapCyc == 0) {
    REG_WRITE(GPIO_OUT_W1TS_REG, bL | bH);
    tL = tH = waitHigh(bL | bH, c0, limit);
  } else {
    REG_WRITE(GPIO_OUT_W1TS_REG, bL);
    tL = waitHigh(bL, c0, limit);
    if (tL) {
      while (esp_cpu_get_cycle_count() - tL < gapCyc) {}
      REG_WRITE(GPIO_OUT_W1TS_REG, bH);
      tH = waitHigh(bH, tL, gapCyc + limit);
    } else {
      tH = 0;
    }
  }
  if (tL && tH) {
    gap = (int32_t)(tH - tL);
    while (esp_cpu_get_cycle_count() - tL < pulseCyc) {}
  }
  REG_WRITE(GPIO_OUT_W1TC_REG, bL | bH);
  portEXIT_CRITICAL(&mux);
  return gap;
}

static void selftestSend() {
  const uint32_t mhz = getCpuFrequencyMhz();
  const uint32_t gapCyc = (uint64_t)st.gapNs * mhz / 1000;
  int32_t gap = selftestPulse(gapCyc, gapCyc + SELFTEST_PULSE_US * mhz, 5 * mhz);   // 5 us
  st.pending = true;
  st.genStuck = gap < 0;
  if (st.genStuck) {
    st.genStuckCount++;
    return;
  }
  st.genGapNs = (int32_t)((int64_t)gap * 1000 / mhz);
  if (abs(st.genGapNs - (int32_t)st.gapNs) > (int32_t)NS_PER_SAMPLE) st.genOff++;
}

static void selftestStart(uint32_t gapNs, uint32_t count) {
  if (st.active) { Serial.println("selftest: already running"); return; }
  if (gapNs > 100000 || count < 1 || count > 1000) {
    Serial.println("selftest: gap 0..100000 ns, count 1..1000");
    return;
  }
  memset(&st, 0, sizeof st);
  st.active = true;
  st.gapNs = gapNs;
  st.count = count;
  st.gapMin = st.diffMin = INT32_MAX;
  st.gapMax = st.diffMax = INT32_MIN;
  st.spacingMs = deadtimeMs + 20 > 50 ? deadtimeMs + 20 : 50;
  st.nextAt = millis() + 100;
  st.missedAtStart = missedCount;
  savedMask = channelMask;
  applyMask(channelMask | 0x80);   // CH8 for the duration, not stored
  REG_WRITE(GPIO_OUT_W1TC_REG, (1u << SELFTEST_PIN_L) | (1u << SELFTEST_PIN_H));
  gpio_set_direction((gpio_num_t)SELFTEST_PIN_L, GPIO_MODE_INPUT_OUTPUT);
  gpio_set_direction((gpio_num_t)SELFTEST_PIN_H, GPIO_MODE_INPUT_OUTPUT);
  Serial.printf("selftest: %u pulses on CH8 (GPIO %d L, %d H), H %u ns after L, every %u ms\n",
                (unsigned)count, SELFTEST_PIN_L, SELFTEST_PIN_H, (unsigned)gapNs,
                (unsigned)st.spacingMs);
}

static void selftestRecord(const EventResult& r) {
  st.seen++;
  bool pending = st.pending;
  st.pending = false;
  const LineResult& L = r.line[LINE_ST_L];
  const LineResult& H = r.line[LINE_ST_H];
  if (L.edges != 2 || H.edges != 2) st.glitches++;
  if (L.rise >= 0 && H.rise >= 0) {
    int32_t gap = (int32_t)(H.rise - L.rise) * (int32_t)NS_PER_SAMPLE;
    st.gapSum += gap;
    if (gap < st.gapMin) st.gapMin = gap;
    if (gap > st.gapMax) st.gapMax = gap;
    if (pending && !st.genStuck) {
      int32_t d = gap - st.genGapNs;
      if (d < st.diffMin) st.diffMin = d;
      if (d > st.diffMax) st.diffMax = d;
      if (abs(d) > SELFTEST_TOLERANCE_NS) {
        st.mismatch++;
        Serial.printf("selftest: evt %u: pins had %d ns, captured %d ns\n",
                      (unsigned)r.number, (int)st.genGapNs, (int)gap);
      } else if (abs(st.genGapNs - (int32_t)st.gapNs) > (int32_t)NS_PER_SAMPLE) {
        Serial.printf("selftest: evt %u: generator was off (pins had %d ns), captured "
                      "%d ns: agrees\n", (unsigned)r.number, (int)st.genGapNs, (int)gap);
      }
    }
  }
  for (int l = 0; l < LINES; l++)
    if (l != LINE_ST_L && l != LINE_ST_H && r.line[l].edges) st.otherLines[l]++;
}

static void selftestFinish() {
  gpio_set_direction((gpio_num_t)SELFTEST_PIN_L, GPIO_MODE_INPUT);
  gpio_set_direction((gpio_num_t)SELFTEST_PIN_H, GPIO_MODE_INPUT);
  applyMask(savedMask);
  st.active = false;

  uint32_t missed = missedCount - st.missedAtStart;
  Serial.printf("selftest: %u of %u pulses seen, %u possibly missed\n",
                (unsigned)st.seen, (unsigned)st.count, (unsigned)missed);
  if (st.seen > 0 && st.gapMin != INT32_MAX)
    Serial.printf("selftest: gap set %u ns, captured mean %d, min %d, max %d ns\n",
                  (unsigned)st.gapNs, (int)(st.gapSum / (int64_t)st.seen), (int)st.gapMin,
                  (int)st.gapMax);
  if (st.diffMin != INT32_MAX)
    Serial.printf("selftest: captured - generated gap: min %d, max %d ns (allowed +-%d), "
                  "%u events outside\n", (int)st.diffMin, (int)st.diffMax,
                  (int)SELFTEST_TOLERANCE_NS, (unsigned)st.mismatch);
  if (st.genOff)
    Serial.printf("selftest: the generator itself was off in %u pulses (CPU held up by BLE or "
                  "flash); those are checked against what the pins really did\n",
                  (unsigned)st.genOff);
  if (st.genStuckCount)
    Serial.printf("selftest: a CH8 pin did not go high in %u pulses: is something plugged "
                  "into CH8?\n", (unsigned)st.genStuckCount);
  if (st.glitches)
    Serial.printf("selftest: %u events with extra or missing transitions on CH8\n",
                  (unsigned)st.glitches);
  bool others = false;
  for (int l = 0; l < LINES; l++) {
    if (!st.otherLines[l]) continue;
    Serial.printf("selftest: CH%d %c also had edges in %u events\n", l / 2 + 1,
                  (l & 1) ? 'H' : 'L', (unsigned)st.otherLines[l]);
    others = true;
  }
  bool pass = st.seen == st.count && missed == 0 && st.glitches == 0 && st.mismatch == 0 &&
              st.genStuckCount == 0 && st.diffMin != INT32_MAX;
  Serial.printf("selftest: %s%s\n", pass ? "PASS" : "FAIL",
                others ? " (other lines had edges: a jumper, or floating inputs)" : "");
}

static void selftestService(uint32_t now) {
  if (!st.active) return;
  if ((int32_t)(now - st.nextAt) < 0) return;
  if (st.sent < st.count) {
    if (evState != EV_IDLE) return;   // wait for the previous event to finish
    selftestSend();
    st.sent++;
    st.nextAt = now + st.spacingMs;
  } else {
    selftestFinish();   // spacingMs after the last pulse, so its event has been printed
  }
}

// ---- Output ----

static void printEvent(const EventResult& r) {
  int ref = INT16_MAX;
  for (int l = 0; l < LINES; l++)
    if ((r.lines & (1 << l)) && r.line[l].rise >= 0 && r.line[l].rise < ref) ref = r.line[l].rise;
  if (ref == INT16_MAX) ref = 0;   // only falling edges

  uint64_t t = (uint64_t)r.firstTime * SAMPLES_PER_BLOCK + ref;   // samples since start
  uint64_t perSec = CAPTURE_SAMPLE_HZ;
  char line[256];
  int n = snprintf(line, sizeof line, "evt %u at %llu.%07llu s, ns from first edge (L/H):",
                   (unsigned)r.number, t / perSec, (t % perSec) * 10000000ULL / perSec);
  for (int c = 0; c < 8; c++) {
    if (!(r.lines & (3 << (2 * c)))) continue;
    n += snprintf(line + n, sizeof line - n, "  CH%d ", c + 1);
    for (int h = 0; h < 2; h++) {
      const LineResult& L = r.line[2 * c + h];
      if (L.rise >= 0) n += snprintf(line + n, sizeof line - n, "%d", (L.rise - ref) * (int)NS_PER_SAMPLE);
      else n += snprintf(line + n, sizeof line - n, "-");
      if (h == 0) n += snprintf(line + n, sizeof line - n, "/");
    }
  }
  Serial.println(line);

  if (r.highAtStart) {
    for (int l = 0; l < LINES; l++)
      if (r.highAtStart & (1 << l))
        Serial.printf("  CH%d %c was already high at window start\n", l / 2 + 1, (l & 1) ? 'H' : 'L');
  }
  if (!verbose) return;
  for (int l = 0; l < LINES; l++) {
    const LineResult& L = r.line[l];
    if (!(r.lines & (1 << l)) || !L.edges) continue;
    n = snprintf(line, sizeof line, "  CH%d %c:", l / 2 + 1, (l & 1) ? 'H' : 'L');
    for (int i = 0; i < L.nTrans && n < (int)sizeof line - 16; i++) {
      int at = L.trans[i] & 0x7FFF;
      n += snprintf(line + n, sizeof line - n, " %c%d", (L.trans[i] & 0x8000) ? '+' : '-',
                    (at - ref) * (int)NS_PER_SAMPLE);
    }
    if (L.rise >= 0 && L.fall >= 0)
      n += snprintf(line + n, sizeof line - n, "  (pulse %d ns)", (L.fall - L.rise) * (int)NS_PER_SAMPLE);
    if (L.edges > L.nTrans)
      n += snprintf(line + n, sizeof line - n, "  (%u edges, first %d shown)", L.edges, MAX_TRANSITIONS);
    Serial.println(line);
  }
}

static void printStatus() {
  uint32_t done = blocksDone;
  int64_t us = esp_timer_get_time() - startUs;
  double rate = us > 0 ? (double)done * SAMPLES_PER_BLOCK / (us / 1e6) : 0;
  double load = us > 0 ? (double)isrCycles / ((double)us * getCpuFrequencyMhz()) * 100.0 : 0;
  Serial.printf("capture: %s, %u blocks in %.1f s, sample rate %.3f MHz (set %.3f)\n",
                running ? "running" : "NOT running", (unsigned)done, us / 1e6, rate / 1e6,
                CAPTURE_SAMPLE_HZ / 1e6);
  Serial.printf("capture: interrupt %.1f %% of core 1, longest %.1f us\n", load,
                (double)isrMaxCycles / getCpuFrequencyMhz());
  Serial.printf("capture: %u events, %u possibly missed (while frozen), %u not printed (queue full)\n",
                (unsigned)eventCount, (unsigned)missedCount, (unsigned)droppedCount);
  Serial.printf("capture: ring frozen %.1f ms in all, longest %.1f ms\n",
                spareBlocks * BLOCK_US / 1000.0, frozenMax * BLOCK_US / 1000.0);
  Serial.printf("capture: channel mask %02X, dead time %u ms, mic span %u mm (window covers "
                "%u us after the first edge), verbose %s\n", channelMask, (unsigned)deadtimeMs,
                (unsigned)spanMm, (unsigned)coveredUs(postBlocks), verbose ? "on" : "off");
  Serial.printf("memory: %u bytes free, %u DMA-capable, largest DMA block %u\n",
                (unsigned)ESP.getFreeHeap(), (unsigned)heap_caps_get_free_size(MALLOC_CAP_DMA),
                (unsigned)heap_caps_get_largest_free_block(MALLOC_CAP_DMA));
}

// ---- Public ----

bool captureBegin() {
  prefs.begin("measure");
  applyMask(prefs.getUShort("mask", CAPTURE_MASK_DEFAULT));
  savedMask = channelMask;
  deadtimeMs = prefs.getUInt("deadtime", CAPTURE_DEADTIME_DEFAULT);
  deadBlocks = deadtimeMs * 1000 / BLOCK_US;
  spanMm = prefs.getUInt("span", CAPTURE_SPAN_MM_DEFAULT);
  if (postBlocksFor(spanMm) > POST_MAX) spanMm = CAPTURE_SPAN_MM_DEFAULT;
  postBlocks = postBlocksFor(spanMm);

  pinMode(IR_EN_PIN, OUTPUT);
  digitalWrite(IR_EN_PIN, LOW);

  // Comparator lines into I2S0 data bits 0-15. GPIO 34-39 have no internal pulls; the host
  // board has 100 k pull-downs on every line.
  for (int i = 0; i < LINES; i++) {
    int pin = CAPTURE_LINE_PINS[i];
    gpio_config_t io = {};
    io.pin_bit_mask = 1ULL << pin;
    io.mode = GPIO_MODE_INPUT;
    io.pull_down_en = pin < 34 ? GPIO_PULLDOWN_ENABLE : GPIO_PULLDOWN_DISABLE;
    io.pull_up_en = GPIO_PULLUP_DISABLE;
    io.intr_type = GPIO_INTR_DISABLE;
    gpio_config(&io);
    esp_rom_gpio_connect_in_signal(pin, I2S0I_DATA_IN0_IDX + i, false);
  }

  // DMA ring: one buffer per descriptor, linked in a circle; plus the two spares, which
  // are only linked in while the ring is frozen.
  for (int i = 0; i < CAPTURE_RING_BLOCKS + 2; i++) {
    uint32_t* b = (uint32_t*)heap_caps_calloc(1, CAPTURE_BLOCK_BYTES, MALLOC_CAP_DMA | MALLOC_CAP_INTERNAL);
    if (!b) {
      Serial.printf("capture: out of DMA memory at block %d\n", i);
      return false;
    }
    if (i < CAPTURE_RING_BLOCKS) ring[i] = b;
    else spareBuf[i - CAPTURE_RING_BLOCKS] = b;
  }
  auto setup = [](lldesc_t& d, uint32_t* buf, lldesc_t* next) {
    d = {};
    d.size = CAPTURE_BLOCK_BYTES;
    d.length = CAPTURE_BLOCK_BYTES;
    d.owner = 1;
    d.buf = (const uint8_t*)buf;
    d.qe.stqe_next = next;
  };
  for (int i = 0; i < CAPTURE_RING_BLOCKS; i++)
    setup(desc[i], ring[i], &desc[(i + 1) % CAPTURE_RING_BLOCKS]);
  setup(spare[0], spareBuf[0], &spare[1]);
  setup(spare[1], spareBuf[1], &spare[0]);
  descAddr0 = (uint32_t)&desc[0] & 0xFFFFF;
  spareAddr[0] = (uint32_t)&spare[0] & 0xFFFFF;
  spareAddr[1] = (uint32_t)&spare[1] & 0xFFFFF;

  // Sample clock: LEDC at the sample rate, 50 % duty, read back from the same pad as the
  // I2S word-select (pixel clock) input.
  if (!ledcAttach(CAPTURE_CLK_PIN, CAPTURE_SAMPLE_HZ, 1)) {
    Serial.println("capture: LEDC could not make the sample clock");
    return false;
  }
  ledcWrite(CAPTURE_CLK_PIN, 1);
  PIN_INPUT_ENABLE(GPIO_PIN_MUX_REG[CAPTURE_CLK_PIN]);
  esp_rom_gpio_connect_in_signal(CAPTURE_CLK_PIN, I2S0I_WS_IN_IDX, false);
  // Camera sync inputs held active: sample continuously.
  esp_rom_gpio_connect_in_signal(GPIO_MATRIX_CONST_ONE_INPUT, I2S0I_V_SYNC_IDX, false);
  esp_rom_gpio_connect_in_signal(GPIO_MATRIX_CONST_ONE_INPUT, I2S0I_H_SYNC_IDX, false);
  esp_rom_gpio_connect_in_signal(GPIO_MATRIX_CONST_ONE_INPUT, I2S0I_H_ENABLE_IDX, false);

  // I2S0 in camera mode, as esp32-camera configures it on the ESP32.
  periph_module_enable(PERIPH_I2S0_MODULE);
  I2S0.conf.rx_reset = 1;       I2S0.conf.rx_reset = 0;
  I2S0.conf.rx_fifo_reset = 1;  I2S0.conf.rx_fifo_reset = 0;
  I2S0.lc_conf.in_rst = 1;      I2S0.lc_conf.in_rst = 0;
  I2S0.lc_conf.ahbm_fifo_rst = 1; I2S0.lc_conf.ahbm_fifo_rst = 0;
  I2S0.lc_conf.ahbm_rst = 1;    I2S0.lc_conf.ahbm_rst = 0;
  I2S0.lc_conf.check_owner = 0;
  I2S0.conf.rx_slave_mod = 1;   // clocked by the WS input
  I2S0.conf.rx_right_first = 0;
  I2S0.conf.rx_msb_right = 0;
  I2S0.conf.rx_msb_shift = 0;
  I2S0.conf.rx_mono = 0;
  I2S0.conf.rx_short_sync = 0;
  I2S0.conf2.lcd_en = 1;
  I2S0.conf2.camera_en = 1;
  I2S0.clkm_conf.clka_en = 0;   // PLL_D2, 160 MHz
  I2S0.clkm_conf.clkm_div_a = 1;
  I2S0.clkm_conf.clkm_div_b = 0;
  I2S0.clkm_conf.clkm_div_num = 2;   // 80 MHz internal clock: 8x the sample clock
  I2S0.clkm_conf.clk_en = 1;
  I2S0.fifo_conf.dscr_en = 1;
  I2S0.fifo_conf.rx_fifo_mod = 1;    // 16-bit samples, two per word
  I2S0.fifo_conf.rx_fifo_mod_force_en = 1;
  I2S0.conf_chan.rx_chan_mod = 1;
  I2S0.sample_rate_conf.rx_bits_mod = 0;
  I2S0.timing.val = 0;
  I2S0.timing.rx_dsync_sw = 1;

  results = xQueueCreate(4, sizeof(EventResult));
  if (!results ||
      xTaskCreatePinnedToCore(measureTask, "measure", 4096, nullptr, 10, &measureTaskHandle, 1) != pdPASS) {
    Serial.println("capture: could not start the measurement task");
    return false;
  }
  // Allocated from here (core 1), so the interrupt runs on core 1, away from BLE.
  if (esp_intr_alloc(ETS_I2S0_INTR_SOURCE, ESP_INTR_FLAG_IRAM | ESP_INTR_FLAG_LEVEL2,
                     captureIsr, nullptr, &isrHandle) != ESP_OK) {
    Serial.println("capture: could not allocate the I2S interrupt");
    return false;
  }

  I2S0.rx_eof_num = WORDS_PER_BLOCK;   // one EOF per descriptor
  I2S0.in_link.addr = (uint32_t)&desc[0] & 0xFFFFF;
  I2S0.in_link.start = 1;
  I2S0.int_clr.val = 0xFFFFFFFF;
  I2S0.int_ena.val = 0;
  I2S0.int_ena.in_suc_eof = 1;
  startUs = esp_timer_get_time();
  I2S0.conf.rx_start = 1;
  running = true;

  Serial.printf("capture: %.1f MHz, 16 lines, ring %d KB (%u ms), mic span %u mm: window "
                "covers %u us after the first edge, channel mask %02X, dead time %u ms. Type "
                "'help' for commands.\n",
                CAPTURE_SAMPLE_HZ / 1e6, CAPTURE_RING_BLOCKS * CAPTURE_BLOCK_BYTES / 1000,
                (unsigned)(CAPTURE_RING_BLOCKS * BLOCK_US / 1000), (unsigned)spanMm,
                (unsigned)coveredUs(postBlocks), channelMask, (unsigned)deadtimeMs);
  return true;
}

void captureService(uint32_t nowMs) {
  if (!running) return;
  static EventResult r;
  while (xQueueReceive(results, &r, 0) == pdTRUE) {
    if (st.active) selftestRecord(r);
    printEvent(r);
  }
  uint32_t missed = missedCount;
  if (missed != missedReported) {
    Serial.printf("MISSED: activity after the dead time while the previous event was still "
                  "being read (%u in all): a shot may not have been measured\n", (unsigned)missed);
    missedReported = missed;
  }
  selftestService(nowMs);
}

void captureCommand(const char* line) {
  char cmd[16] = "";
  char arg[24] = "";
  unsigned long a = 0, b = 0;
  if ((unsigned char)line[0] < 0x20) return;   // arrow keys and other escape sequences
  int n = sscanf(line, "%15s %23s", cmd, arg);
  if (n < 1) return;

  if (!strcmp(cmd, "help")) {
    Serial.println("commands:\n"
                   "  stat                 capture status, sample rate, interrupt load, memory\n"
                   "  mask <hex>           fitted channels, bit 0 = CH1 ... bit 7 = CH8 (stored)\n"
                   "  deadtime <ms>        no new event for this long after one (stored)\n"
                   "  span <mm>            largest distance between two mics; sets the event\n"
                   "                       window (stored)\n"
                   "  verbose on|off       list every transition of every line\n"
                   "  selftest [ns] [n]    n pulses on CH8 (unplugged!), OUT_H ns after OUT_L\n"
                   "                       (default 1000 ns, 20 pulses)");
  } else if (!running) {
    Serial.println("capture is not running (see the start-up messages)");
  } else if (!strcmp(cmd, "stat")) {
    printStatus();
  } else if (!strcmp(cmd, "mask")) {
    if (n < 2 || sscanf(arg, "%lx", &a) != 1 || a > 0xFF) { Serial.println("mask <hex 00..FF>"); return; }
    if (st.active) { Serial.println("mask: wait for the self-test to finish"); return; }
    applyMask((uint16_t)a);
    savedMask = channelMask;
    prefs.putUShort("mask", channelMask);
    Serial.printf("channel mask %02X\n", channelMask);
  } else if (!strcmp(cmd, "deadtime")) {
    if (n < 2 || sscanf(arg, "%lu", &a) != 1 || a < 1 || a > 1000) { Serial.println("deadtime <1..1000 ms>"); return; }
    deadtimeMs = a;
    deadBlocks = deadtimeMs * 1000 / BLOCK_US;
    prefs.putUInt("deadtime", deadtimeMs);
    Serial.printf("dead time %u ms\n", (unsigned)deadtimeMs);
  } else if (!strcmp(cmd, "span")) {
    if (n < 2 || sscanf(arg, "%lu", &a) != 1 || a < 50 || postBlocksFor(a) > POST_MAX) {
      uint32_t max = 50;
      while (postBlocksFor(max + 10) <= POST_MAX) max += 10;
      Serial.printf("span <50..%u mm>\n", (unsigned)max);
      return;
    }
    spanMm = a;
    postBlocks = postBlocksFor(spanMm);
    prefs.putUInt("span", spanMm);
    Serial.printf("mic span %u mm: the window covers %u us after the first edge (needs %u)\n",
                  (unsigned)spanMm, (unsigned)coveredUs(postBlocks), (unsigned)neededUs(spanMm));
  } else if (!strcmp(cmd, "verbose")) {
    verbose = !strcmp(arg, "on");
    Serial.printf("verbose %s\n", verbose ? "on" : "off");
  } else if (!strcmp(cmd, "selftest")) {
    a = 1000; b = 20;
    sscanf(line, "%*s %lu %lu", &a, &b);
    selftestStart(a, b);
  } else {
    Serial.printf("unknown command '%s', type 'help'\n", cmd);
  }
}
