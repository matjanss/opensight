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
#include "airtemp.h"
#include "arrival.h"
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
#include "event.h"
#include "pulsegen.h"
#include "simulate.h"
#include "solver.h"

// ---- Sizes derived from config.h ----

static const int      WORDS_PER_BLOCK   = CAPTURE_BLOCK_BYTES / 4;
static const int      PRE_BLOCKS        = 1;   // the block before the first activity
// Blocks from the trigger block on. The window's last block must be at least 4 blocks after
// the trigger (time for the ISR to relink it) and the window at most the ring.
static const uint32_t POST_MIN          = 5;
static const uint32_t POST_MAX          = CAPTURE_RING_BLOCKS - PRE_BLOCKS - 1;

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

// Event results (measureTask -> loop): EventResult, in event.h.

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
static volatile uint32_t evPre = PRE_BLOCKS; // blocks before the trigger block in this window
static volatile uint32_t evStartWord = 0;    // with evPre 0: the word just before the window
// The first ring block after a relink follows the last spare block in time, not the ring
// block before it: it is scanned against the spare's last word.
static volatile bool     afterFreeze = false;
static volatile uint32_t lastSpareWord = 0;
static volatile bool     spareWordFresh = false;
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
static uint32_t startHighCount = 0;   // events with a line already high at window start
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

// True if a fitted line changes level inside the block or from the block before into it.
// `before` is the last word of the block before (or this block's first word if there is
// none): folding it in catches an edge that falls exactly between two blocks, where each
// block on its own is constant (bench, span 600: an OUT_L edge on a block boundary was
// missed, and the window opened only at OUT_H, 1 ms later).
static inline bool IRAM_ATTR blockActive(const uint32_t* w, uint32_t mask, uint32_t before) {
  uint32_t orAll = before, andAll = before;
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
    // The first block after a relink: its ring neighbour is from before the freeze, so it is
    // scanned against the spare block the DMA filled just before it, and a window starting
    // there has no block before the trigger (bench 2c: second shots in this block were lost
    // when it wasn't scanned at all).
    bool firstAfter = afterFreeze && rb == contigFrom;
    uint32_t before = firstAfter ? (spareWordFresh ? lastSpareWord : ring[rb % CAPTURE_RING_BLOCKS][0])
                                 : ring[(rb - 1) % CAPTURE_RING_BLOCKS][WORDS_PER_BLOCK - 1];
    if (evState == EV_IDLE && (rb >= contigFrom + PRE_BLOCKS || firstAfter) &&
        (int32_t)(t - quietUntil) >= 0 &&
        blockActive(ring[rb % CAPTURE_RING_BLOCKS], mask, before)) {
      // Freeze: the window's last descriptor leads into the spares, which loop.
      uint32_t post = postBlocks;
      uint32_t end = (rb + post - 1) % CAPTURE_RING_BLOCKS;
      spare[0].qe.stqe_next = &spare[1];
      spare[1].qe.stqe_next = &spare[0];
      desc[end].qe.stqe_next = &spare[0];
      trigRing = rb;
      trigTime = t;
      evPost = post;
      evPre = firstAfter ? 0 : PRE_BLOCKS;
      evStartWord = before;
      endDesc = end;
      // Anything inside the window belongs to this event, so the dead time is never shorter
      // than the window (a second shot inside it would be merged in, unreported).
      quietUntil = t + (deadBlocks > post ? deadBlocks : post);
      missedThisFreeze = false;
      evState = EV_COLLECTING;
    }
    if (firstAfter) afterFreeze = false;
    spareWordFresh = false;
    ringDone = rb + 1;
    blocksDone = t + 1;
  }
  if (sp >= 0) {
    uint32_t t = blocksDone;
    blocksDone = t + 1;
    spareBlocks = spareBlocks + 1;
    if (!missedThisFreeze && (int32_t)(t - quietUntil) >= 0 && blockActive(spareBuf[sp], mask, spareBuf[sp][0])) {
      missedThisFreeze = true;
      missedCount = missedCount + 1;
      // In a spare finished after the relink: that shot gets its dead time from here, so the
      // rest of it can't open a window in its middle.
      if (evState == EV_IDLE) quietUntil = t + deadBlocks;
    }
    lastSpareWord = spareBuf[sp][WORDS_PER_BLOCK - 1];
    spareWordFresh = true;
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

// before: the word just before the window, if the ring block before it isn't (a window that
// starts at the first block after a relink); otherwise the window's own first sample.
static void extract(EventResult& r, uint32_t first, uint32_t blocks, const uint32_t* before) {
  const uint16_t mask = lineMask32 & 0xFFFF;
  r.lines = mask;
  for (int l = 0; l < LINES; l++) {
    r.line[l].rise = -1;
    r.line[l].fall = -1;
    r.line[l].edges = 0;
    r.line[l].nTrans = 0;
  }

  const uint32_t* w0 = ring[first % CAPTURE_RING_BLOCKS];
  uint16_t prev = before ? (CAPTURE_HIGH_HALF_FIRST ? (*before & 0xFFFF) : (*before >> 16)) & mask
                         : (CAPTURE_HIGH_HALF_FIRST ? (w0[0] >> 16) : (w0[0] & 0xFFFF)) & mask;
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
    const uint32_t pre = evPre;
    const uint32_t startWord = evStartWord;
    uint32_t blocks = pre + evPost;
    r.number = ++eventCount;
    r.firstTime = trigTime - pre;
    r.blocks = blocks;
    extract(r, trigRing - pre, blocks, pre ? nullptr : &startWord);
    if (r.highAtStart) startHighCount++;

    // Unfreeze: the DMA is in the spares; link the ring back in after the window's last
    // block. Ring first, so the descriptor is right before the DMA can come round to it.
    uint32_t frozen = blocksDone - (trigTime + evPost);
    if (frozen > frozenMax) frozenMax = frozen;
    lldesc_t* next = &desc[(endDesc + 1) % CAPTURE_RING_BLOCKS];
    contigFrom = trigRing + evPost;   // the next ring block follows the spares in time
    afterFreeze = true;
    desc[endDesc].qe.stqe_next = next;
    spare[0].qe.stqe_next = next;
    spare[1].qe.stqe_next = next;
    // A shot came while frozen (MISSED) and may still be going on: its remaining edges (lines
    // still high, ringing) would open a window in its middle, with no first edges in it
    // (bench, step 2c). It gets a dead time of its own from here, like any shot's ringing.
    if (missedThisFreeze) quietUntil = blocksDone + deadBlocks;
    evState = EV_IDLE;
    if (xQueueSend(results, &r, 0) != pdTRUE) droppedCount++;
  }
}

// ---- For the self-test and the simulator ----

bool captureIdle() { return evState == EV_IDLE; }
uint16_t captureChannelMask() { return channelMask; }
uint32_t captureSpanMm() { return spanMm; }
uint32_t captureMissedCount() { return missedCount; }
uint32_t captureWindowUs() { return coveredUs(postBlocks); }
uint32_t captureDeadtimeMs() { return deadtimeMs; }
uint32_t captureDeadUs() {
  uint32_t windowUs = postBlocks * BLOCK_US;   // from the trigger block's start
  return deadtimeMs * 1000 > windowUs ? deadtimeMs * 1000 : windowUs;
}

void captureTestChannels(uint16_t channels) {
  savedMask = channelMask;
  applyMask(channelMask | channels);
}

void captureEndTestChannels() { applyMask(savedMask); }

// Back to plain inputs into the I2S, as captureBegin set them up.
void captureLinesToInputs(uint16_t lines) {
  for (int l = 0; l < LINES; l++) {
    if (!(lines & (1 << l))) continue;
    int pin = CAPTURE_LINE_PINS[l];
    gpio_config_t io = {};
    io.pin_bit_mask = 1ULL << pin;
    io.mode = GPIO_MODE_INPUT;
    io.pull_down_en = pin < 34 ? GPIO_PULLDOWN_ENABLE : GPIO_PULLDOWN_DISABLE;
    io.pull_up_en = GPIO_PULLUP_DISABLE;
    io.intr_type = GPIO_INTR_DISABLE;
    gpio_config(&io);
    esp_rom_gpio_connect_in_signal(pin, I2S0I_DATA_IN0_IDX + l, false);
  }
}

// ---- Self-test ----

// The pulse pair comes from MCPWM (pulsegen.cpp), not the CPU: the gap is set by the timer
// hardware in ticks, so BLE, flash writes or the CPU's write buffer can't change it (a
// CPU-driven generator could, bench tests 1-4). The CPU only starts each run.
// Tick rate: the finest of these whose 16-bit period still holds the pulse pair.
static const uint32_t GEN_HZ_CHOICES[] = {80000000, 40000000, 20000000, 10000000};
static const uint32_t SELFTEST_PULSE_MIN_NS = NS_PER_SAMPLE;   // shorter can fall between samples
static const uint32_t SELFTEST_LEAD_NS = 20000;                  // start of the run to L

static struct {
  bool     active;
  uint32_t gapNs, count, sent, seen, glitches, missedAtStart;
  uint32_t spacingMs, nextAt;
  int32_t  effGapNs;         // the gap the hardware makes: gapNs rounded to whole ticks
  int64_t  gapSum;
  uint32_t genHz;            // MCPWM tick rate for this run
  uint32_t leadNs;           // start of a run to L's rise
  uint32_t pulseNs;          // OUT_H pulse length; OUT_L is gapNs longer
  uint32_t timed;            // events with both rising edges
  uint32_t straddle;         // events with L and H in different DMA blocks
  uint32_t badLength;        // events whose pulse lengths were off
  uint32_t late;             // runs whose start took longer than the lead
  uint32_t blockPos[4];      // L's position within its block, in quarters
  uint32_t atLow, atHigh;    // gaps captured as effGapNs rounded down / up to whole samples
  uint32_t oneOff, outside;  // ... one sample beyond that (input skew) / further off
  uint32_t otherLines[LINES];
} st;

static const int LINE_ST_L = 14, LINE_ST_H = 15;   // CH8 L, CH8 H
static const uint16_t ST_LINES = (1 << LINE_ST_L) | (1 << LINE_ST_H);

static uint32_t toTicks(uint32_t ns, uint32_t hz) {
  return (uint32_t)(((uint64_t)ns * hz + 500000000) / 1000000000);
}

bool captureSelftestActive() { return st.active; }

static void selftestStart(uint32_t gapNs, uint32_t count, uint32_t pulseNs) {
  if (st.active) { Serial.println("selftest: already running"); return; }
  if (simActive()) { Serial.println("selftest: wait for the simulator to finish (`sim stop`)"); return; }
  if (count < 1 || count > 1000 || pulseNs < SELFTEST_PULSE_MIN_NS || pulseNs > 100000) {
    Serial.printf("selftest: count 1..1000, pulse %u..100000 ns\n",
                  (unsigned)SELFTEST_PULSE_MIN_NS);
    return;
  }
  // The whole H pulse must be inside the window wherever L falls in the trigger block.
  const uint32_t maxGapNs = coveredUs(postBlocks) * 1000 - pulseNs - NS_PER_SAMPLE;
  if (gapNs > maxGapNs) {
    Serial.printf("selftest: gap 0..%u ns with this pulse length and mic span (the window)\n",
                  (unsigned)maxGapNs);
    return;
  }
  // L rises after the lead, H gap later, both fall at the end of the run (its peak).
  uint32_t hz = 0, leadTicks = 0, gapTicks = 0, pulseTicks = 0;
  for (uint32_t c : GEN_HZ_CHOICES) {
    leadTicks = toTicks(SELFTEST_LEAD_NS, c);
    gapTicks = toTicks(gapNs, c);
    pulseTicks = toTicks(pulseNs, c);
    if (leadTicks + gapTicks + pulseTicks + 1 <= 65535 && pulseTicks >= 1) { hz = c; break; }
  }
  const int pins[1][2] = {{SELFTEST_PIN_L, SELFTEST_PIN_H}};
  if (!hz || !pulsegenCreate(1, pins, hz, leadTicks + gapTicks + pulseTicks + 1) ||
      !pulsegenSet(0, leadTicks, leadTicks + gapTicks)) {
    pulsegenDelete();
    captureLinesToInputs(ST_LINES);
    Serial.println("selftest: could not set up the MCPWM pulse generator");
    return;
  }
  memset(&st, 0, sizeof st);
  st.active = true;
  st.gapNs = gapNs;
  st.genHz = hz;
  st.leadNs = (uint32_t)((uint64_t)leadTicks * 1000000000 / hz);
  st.effGapNs = (int32_t)((uint64_t)gapTicks * 1000000000 / hz);
  // The peak is one tick before the period's end; H falls there.
  st.pulseNs = (uint32_t)((uint64_t)pulseTicks * 1000000000 / hz);
  st.count = count;
  st.spacingMs = deadtimeMs + 20 > 50 ? deadtimeMs + 20 : 50;
  st.nextAt = millis() + 100;
  st.missedAtStart = missedCount;
  captureTestChannels(0x80);   // CH8 for the duration, not stored
  Serial.printf("selftest: %u pulse pairs from MCPWM on CH8 (GPIO %d L, %d H), H %d ns after "
                "L, H %u ns long, %.1f ns ticks, every %u ms, at random points in the DMA "
                "blocks; captured in whole samples (%u ns)\n", (unsigned)count, SELFTEST_PIN_L,
                SELFTEST_PIN_H, (int)st.effGapNs, (unsigned)st.pulseNs, 1e9 / hz,
                (unsigned)st.spacingMs, (unsigned)NS_PER_SAMPLE);
}

static void selftestRecord(const EventResult& r) {
  st.seen++;
  const LineResult& L = r.line[LINE_ST_L];
  const LineResult& H = r.line[LINE_ST_H];
  if (L.edges != 2 || H.edges != 2) st.glitches++;
  // Both fall together: H lasts pulseNs, L gap + pulseNs, to within one sample.
  if (L.rise >= 0 && L.fall >= 0 && H.rise >= 0 && H.fall >= 0) {
    int32_t lenH = (int32_t)(H.fall - H.rise) * (int32_t)NS_PER_SAMPLE;
    int32_t fallD = (int32_t)(H.fall - L.fall) * (int32_t)NS_PER_SAMPLE;
    if (abs(lenH - (int32_t)st.pulseNs) > (int32_t)NS_PER_SAMPLE || abs(fallD) > (int32_t)NS_PER_SAMPLE) {
      st.badLength++;
      Serial.printf("selftest: evt %u: H %d ns long (set %u), falls %d ns from L\n",
                    (unsigned)r.number, (int)lenH, (unsigned)st.pulseNs, (int)fallD);
    }
  }
  if (L.rise >= 0 && H.rise >= 0) {
    int32_t gap = (int32_t)(H.rise - L.rise) * (int32_t)NS_PER_SAMPLE;
    st.timed++;
    if (L.rise / SAMPLES_PER_BLOCK != H.rise / SAMPLES_PER_BLOCK) st.straddle++;
    st.blockPos[(L.rise % SAMPLES_PER_BLOCK) * 4 / SAMPLES_PER_BLOCK]++;
    st.gapSum += gap;
    // Each edge is sampled every 100 ns, so the captured gap is the set one rounded down
    // or up to whole samples (the same value both ways when it is a whole number). A few
    // ns of skew between the two inputs can add one sample when an edge sits right on a
    // sample; more than one sample away is an error.
    const int32_t S = (int32_t)NS_PER_SAMPLE;
    int32_t low = st.effGapNs / S * S, high = (st.effGapNs + S - 1) / S * S;
    if (gap == low) st.atLow++;
    else if (gap == high) st.atHigh++;
    else if (gap == low - S || gap == high + S) st.oneOff++;
    else {
      st.outside++;
      Serial.printf("selftest: evt %u: captured %d ns, set %d ns\n", (unsigned)r.number,
                    (int)gap, (int)st.effGapNs);
    }
  }
  for (int l = 0; l < LINES; l++)
    if (l != LINE_ST_L && l != LINE_ST_H && r.line[l].edges) st.otherLines[l]++;
}

static void selftestFinish() {
  pulsegenDelete();
  captureLinesToInputs(ST_LINES);
  captureEndTestChannels();
  st.active = false;

  uint32_t missed = missedCount - st.missedAtStart;
  Serial.printf("selftest: %u of %u pulse pairs seen, %u possibly missed\n",
                (unsigned)st.seen, (unsigned)st.count, (unsigned)missed);
  if (st.timed < st.seen)
    Serial.printf("selftest: %u events without both rising edges\n",
                  (unsigned)(st.seen - st.timed));
  if (st.timed) {
    // With the pulse pair at a random position against the samples, a gap of n samples
    // plus a fraction p is captured as n + 1 samples in a share p of the pulses: the mean
    // tends to the set gap, within a standard error of S * sqrt(p (1 - p) / count).
    const int32_t S = (int32_t)NS_PER_SAMPLE;
    int32_t low = st.effGapNs / S * S, high = (st.effGapNs + S - 1) / S * S;
    double p = (double)(st.effGapNs - low) / S;
    double mean = (double)st.gapSum / st.timed;
    double se = S * sqrt(p * (1 - p) / st.timed);
    if (low == high) {
      Serial.printf("selftest: gap set %d ns (a whole number of samples): captured as %d ns "
                    "%u times, one sample off %u times, further off %u times\n",
                    (int)st.effGapNs, (int)low, (unsigned)st.atLow, (unsigned)st.oneOff,
                    (unsigned)st.outside);
    } else {
      Serial.printf("selftest: gap set %d ns, between two samples (%d ns each): captured as "
                    "%d ns %u times, %d ns %u times (%.0f %% at %d; expected about %.0f %%), "
                    "one sample beyond %u, further off %u\n",
                    (int)st.effGapNs, (int)S, (int)low, (unsigned)st.atLow, (int)high,
                    (unsigned)st.atHigh, 100.0 * st.atHigh / st.timed, (int)high, 100.0 * p,
                    (unsigned)st.oneOff, (unsigned)st.outside);
      bool off = fabs(mean - st.effGapNs) > 3 * se + 1;
      Serial.printf("selftest: captured mean %.1f ns, set %d ns: %.1f ns off; the rounding "
                    "alone gives about +-%.1f ns over %u pairs%s\n",
                    mean, (int)st.effGapNs, mean - st.effGapNs, se, (unsigned)st.timed,
                    off ? " (more than 3x that: the pulses are not at random points in a "
                          "sample)" : "");
    }
    // L and H fall in different DMA blocks in a share gap / block of the pulses.
    double straddleExp = st.timed * fmin(1.0, st.effGapNs / (BLOCK_US * 1000.0));
    Serial.printf("selftest: L in the 1st/2nd/3rd/4th quarter of its DMA block: %u/%u/%u/%u "
                  "(expected about equal); L and H in different blocks: %u (expected about "
                  "%.1f)\n", (unsigned)st.blockPos[0], (unsigned)st.blockPos[1],
                  (unsigned)st.blockPos[2], (unsigned)st.blockPos[3], (unsigned)st.straddle,
                  straddleExp);
  }
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
  if (st.badLength)
    Serial.printf("selftest: %u events with wrong pulse lengths\n", (unsigned)st.badLength);
  if (st.late)
    Serial.printf("selftest: %u pulse pairs started late (the start took longer than the lead)\n",
                  (unsigned)st.late);
  bool pass = st.seen == st.count && st.timed == st.seen && missed == 0 && st.glitches == 0 &&
              st.outside == 0 && st.badLength == 0 && st.late == 0;
  Serial.printf("selftest: %s%s\n", pass ? "PASS" : "FAIL",
                others ? " (other lines had edges: a jumper, or floating inputs)" : "");
}

static void selftestService(uint32_t now) {
  if (!st.active) return;
  if ((int32_t)(now - st.nextAt) < 0) return;
  if (st.sent < st.count) {
    if (evState != EV_IDLE) return;   // wait for the previous event to finish
    // CPU, MCPWM and sample clock all run off one crystal, so without this every pulse
    // would start at the same point within a sample (bench test 5); real sound doesn't.
    // ... and so that it lands anywhere in a DMA block, not at the same point every time
    // (loop() runs on a 10 ms tick, a whole number of blocks).
    delayMicroseconds(esp_random() % (2 * BLOCK_US));
    uint32_t wait = esp_random() % (2 * NS_PER_SAMPLE * getCpuFrequencyMhz() / 1000);
    uint32_t c0 = esp_cpu_get_cycle_count();
    while (esp_cpu_get_cycle_count() - c0 < wait) {}
    if (pulsegenFire() >= st.leadNs) st.late++;   // one pulse pair
    st.sent++;
    st.nextAt = now + st.spacingMs;
  } else {
    selftestFinish();   // spacingMs after the last pulse, so its event has been printed
  }
}

// ---- Output ----

// Free serial buffer needed before printing an event (its line, the already-high lines, the
// two arrival lines, the position, the calibration line and the simulator's), and before its
// transition lists (verbose: up to 16 lines).
static const int PRINT_ROOM = 1200;
static const int PRINT_ROOM_VERBOSE = 1536;
static uint32_t verboseSkipped = 0;

static void printEvent(const EventResult& r) {
  const int ref = eventFirstEdge(r);

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
      else if (r.highAtStart & (1 << (2 * c + h)))   // rose before the window: earlier than its start
        n += snprintf(line + n, sizeof line - n, "<%d", -ref * (int)NS_PER_SAMPLE);
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
  if (Serial.availableForWrite() < PRINT_ROOM_VERBOSE) {
    verboseSkipped++;
    Serial.println("  (transitions not shown: serial output busy)");
    return;
  }
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
  if (verboseSkipped)
    Serial.printf("capture: %u events printed without their transitions (serial output busy)\n",
                  (unsigned)verboseSkipped);
  Serial.printf("capture: %u events with a line already high at window start (should be 0 "
                "with nothing floating)\n", (unsigned)startHighCount);
  Serial.printf("capture: ring frozen %.1f ms in all, longest %.1f ms\n",
                spareBlocks * BLOCK_US / 1000.0, frozenMax * BLOCK_US / 1000.0);
  Serial.printf("capture: channel mask %02X, dead time %u ms (in effect %.1f), mic span %u mm (window covers "
                "%u us after the first edge), verbose %s\n", channelMask, (unsigned)deadtimeMs,
                captureDeadUs() / 1000.0,
                (unsigned)spanMm, (unsigned)coveredUs(postBlocks), verbose ? "on" : "off");
  airTempPrintStatus();
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
  arrivalBegin();
  solverBegin();

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

static void (*shotFn)(float, float) = nullptr;
void captureOnShot(void (*fn)(float, float)) { shotFn = fn; }

void captureService(uint32_t nowMs) {
  if (!running) return;
  static EventResult r;
  // Only as much as fits in the serial buffer: loop() must never wait on the UART, or it
  // misses button presses (bench: a finger on a pin with verbose on, test shots lost).
  // What doesn't fit waits in the queue; if that fills, `stat` counts it as not printed.
  while (Serial.availableForWrite() >= PRINT_ROOM && xQueueReceive(results, &r, 0) == pdTRUE) {
    if (st.active) selftestRecord(r);
    printEvent(r);
    if (st.active) continue;   // the self-test's events are CH8 pulse pairs only
    Arrival a;
    arrivalCompute(r, a);
    arrivalPrint(r, a);
    Solution s;
    solverSolve(r, a, s);
    solverPrint(r, a, s);
    if (simActive()) simRecord(r, a, s);
    if (s.solved && s.complete && shotFn) shotFn(s.x, s.y);
  }
  uint32_t missed = missedCount;
  if (missed != missedReported) {
    Serial.printf("MISSED: activity after the dead time while the previous event was still "
                  "being read (%u in all): a shot may not have been measured\n", (unsigned)missed);
    missedReported = missed;
  }
  selftestService(nowMs);
  simService(nowMs);
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
                   "  selftest [ns] [n] [pulse]  n pulse pairs on CH8 (unplugged!), OUT_H ns\n"
                   "                       after OUT_L, OUT_H pulse ns long (default 1000 ns,\n"
                   "                       20 pairs, 10000 ns)\n"
                   "  temp                 the air temperature (DS18B20)");
    arrivalHelp();
    solverHelp();
    simHelp();
  } else if (!running) {
    Serial.println("capture is not running (see the start-up messages)");
  } else if (!strcmp(cmd, "stat")) {
    printStatus();
  } else if (!strcmp(cmd, "mask")) {
    if (n < 2 || sscanf(arg, "%lx", &a) != 1 || a > 0xFF) { Serial.println("mask <hex 00..FF>"); return; }
    if (st.active || simActive()) { Serial.println("mask: wait for the self-test or simulator to finish"); return; }
    applyMask((uint16_t)a);
    savedMask = channelMask;
    prefs.putUShort("mask", channelMask);
    Serial.printf("channel mask %02X\n", channelMask);
  } else if (!strcmp(cmd, "deadtime")) {
    if (n < 2 || sscanf(arg, "%lu", &a) != 1 || a < 1 || a > 1000) { Serial.println("deadtime <1..1000 ms>"); return; }
    deadtimeMs = a;
    deadBlocks = deadtimeMs * 1000 / BLOCK_US;
    prefs.putUInt("deadtime", deadtimeMs);
    Serial.printf("dead time %u ms", (unsigned)deadtimeMs);
    if (captureDeadUs() > deadtimeMs * 1000)
      Serial.printf(" (the event window makes it %.1f ms: a dead time is never shorter than the "
                    "window)", captureDeadUs() / 1000.0);
    Serial.println();
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
    unsigned long c = SELFTEST_PULSE_US * 1000;
    sscanf(line, "%*s %lu %lu %lu", &a, &b, &c);
    selftestStart(a, b, c);
  } else if (!strcmp(cmd, "temp")) {
    airTempPrintStatus();
  } else if (!arrivalCommand(cmd, line) && !solverCommand(cmd, line) && !simCommand(cmd, line)) {
    Serial.printf("unknown command '%s', type 'help'\n", cmd);
  }
}
