// OpenSights - the IR light curtain: see curtain.h.
#include "curtain.h"
#include "calib.h"
#include "capture.h"
#include "config.h"
#include "driver/gpio.h"
#include "driver/mcpwm_cap.h"
#include "esp_rom_gpio.h"
#include "soc/gpio_sig_map.h"
#include "soc/mcpwm_struct.h"

static mcpwm_cap_timer_handle_t   capTimer = nullptr;
static mcpwm_cap_channel_handle_t capRise = nullptr, capFall = nullptr;
static uint32_t capHz = 0;
static uint32_t ticksPerSample = 0;
static uint32_t startTick = 0;     // the capture timer at sample 0
static bool     ready = false;     // the timer runs and is tied to the samples
static bool     enabled = false;   // the channels are on (the gate)
static bool     simDrive = false;

// The last dips, as the capture timer latched them. Written by the capture interrupt.
struct Dip {
  uint32_t rise, fall;
  bool     hasFall;
  bool     claimed;   // by an event, or printed as a `dip` line
};
static const int RING = 16;
static Dip ring[RING];
static volatile uint32_t head = 0;   // dips since boot; the newest is ring[(head - 1) % RING]
static portMUX_TYPE mux = portMUX_INITIALIZER_UNLOCKED;

// For `stat` (PLAN D9: each defence counted on its own).
static uint32_t nOrphans = 0, nOrphansQuiet = 0;
static uint32_t nJudged[2][2] = {};   // [acoustic ok][a dip in the window]
static uint32_t lastPrintMs = 0;

static const int32_t CLAIM_NS = 1500000;       // a dip this close to an event's first edge is its
static const int64_t ORPHAN_AGE_NS = 50000000; // a dip this old with no event: printed

// A dip's rise and fall come on two channels. If their interrupt runs late (a dip is only
// 30-40 us), both are waiting, and the driver may hand over the fall first: it is kept
// until its rise comes.
static uint32_t pendingFall = 0;
static bool     hasPendingFall = false;
static const int32_t FALL_MAX_TICKS = 2000 * 80;   // a fall up to 2 ms after its rise

static bool fallsAfter(uint32_t fall, uint32_t rise) {
  const int32_t d = (int32_t)(fall - rise);
  return d > 0 && d <= FALL_MAX_TICKS;
}

static bool onRise(mcpwm_cap_channel_handle_t, const mcpwm_capture_event_data_t* e, void*) {
  portENTER_CRITICAL_ISR(&mux);
  Dip& d = ring[head % RING];
  d.rise = e->cap_value;
  d.hasFall = hasPendingFall && fallsAfter(pendingFall, d.rise);
  if (d.hasFall) d.fall = pendingFall;
  hasPendingFall = false;
  d.claimed = false;
  head = head + 1;
  portEXIT_CRITICAL_ISR(&mux);
  return false;
}

static bool onFall(mcpwm_cap_channel_handle_t, const mcpwm_capture_event_data_t* e, void*) {
  portENTER_CRITICAL_ISR(&mux);
  Dip* d = head ? &ring[(head - 1) % RING] : nullptr;
  if (d && !d->hasFall && fallsAfter(e->cap_value, d->rise)) {
    d->fall = e->cap_value;
    d->hasFall = true;
  } else {
    pendingFall = e->cap_value;   // its rise is still to come
    hasPendingFall = true;
  }
  portEXIT_CRITICAL_ISR(&mux);
  return false;
}

// Capture ticks from sample `sample` (samples since the capture start) to `tick`; within
// +-26 s (the 32-bit timer wraps every 53.7 s).
static int32_t ticksAfter(uint32_t tick, uint64_t sample) {
  return (int32_t)(tick - (startTick + (uint32_t)(sample * ticksPerSample)));
}
static int64_t ticksToNs(int64_t ticks) { return ticks * 1000000000LL / (int64_t)capHz; }

static esp_err_t newChannel(bool rise, mcpwm_cap_channel_handle_t* ch) {
  mcpwm_capture_channel_config_t cc = {};
  cc.gpio_num = IR_IN_PIN;
  cc.prescale = 1;
  cc.flags.pos_edge = rise;
  cc.flags.neg_edge = !rise;
  esp_err_t err = mcpwm_new_capture_channel(capTimer, &cc, ch);
  if (err != ESP_OK) return err;
  mcpwm_capture_event_callbacks_t cbs = {};
  cbs.on_cap = rise ? onRise : onFall;
  return mcpwm_capture_channel_register_event_callbacks(*ch, &cbs, nullptr);
}

bool curtainBegin() {
  pinMode(IR_EN_PIN, OUTPUT);
  digitalWrite(IR_EN_PIN, LOW);
  mcpwm_capture_timer_config_t tc = {};
  tc.group_id = 0;
  tc.clk_src = MCPWM_CAPTURE_CLK_SRC_DEFAULT;   // APB, 80 MHz
  tc.resolution_hz = 80000000;
  esp_err_t err = mcpwm_new_capture_timer(&tc, &capTimer);
  if (err == ESP_OK) err = mcpwm_capture_timer_get_resolution(capTimer, &capHz);
  if (err == ESP_OK && (capHz % CAPTURE_SAMPLE_HZ || capHz < CAPTURE_SAMPLE_HZ)) err = ESP_ERR_INVALID_SIZE;
  if (err == ESP_OK) err = newChannel(true, &capRise);
  if (err == ESP_OK) err = newChannel(false, &capFall);
  if (err == ESP_OK) err = mcpwm_capture_timer_enable(capTimer);
  if (err == ESP_OK) err = mcpwm_capture_timer_start(capTimer);
  if (err != ESP_OK) {
    Serial.printf("curtain: could not set up the MCPWM capture on GPIO %d (%s): no curtain\n",
                  IR_IN_PIN, esp_err_to_name(err));
    return false;
  }
  ticksPerSample = capHz / CAPTURE_SAMPLE_HZ;
  // Channel 2 of the unit has no pin: its software catch ties the timer to the samples.
  MCPWM0.cap_chn_cfg[2].capn_mode = 0;
  MCPWM0.cap_chn_cfg[2].capn_en = 1;
  return true;
}

void curtainMarkStart() {
  if (capTimer) MCPWM0.cap_chn_cfg[2].capn_sw = 1;
}

void curtainMarkRead() {
  if (!capTimer) return;
  startTick = MCPWM0.cap_chn[2].capn_value;
  ready = true;
  curtainApply();
}

void curtainApply() {
  const bool want = ready && calibCfg().curtainHiNs > 0;
  digitalWrite(IR_EN_PIN, want ? HIGH : LOW);
  if (want == enabled) return;
  if (want) {
    portENTER_CRITICAL(&mux);
    for (int i = 0; i < RING; i++) ring[i].claimed = true;   // nothing old to report
    portEXIT_CRITICAL(&mux);
    mcpwm_capture_channel_enable(capRise);
    mcpwm_capture_channel_enable(capFall);
  } else {
    mcpwm_capture_channel_disable(capRise);
    mcpwm_capture_channel_disable(capFall);
  }
  enabled = want;
}

void curtainCollect(uint64_t firstSample, measure::Rec& rec) {
  if (!enabled) return;
  rec.hasIr = true;
  rec.nDips = 0;
  portENTER_CRITICAL(&mux);
  const uint32_t h = head;
  for (uint32_t k = h > RING ? h - RING : 0; k < h && rec.nDips < measure::MAX_DIPS; k++) {
    Dip& d = ring[k % RING];
    const int64_t ns = ticksToNs(ticksAfter(d.rise, firstSample));
    if (ns < -CLAIM_NS || ns > CLAIM_NS) continue;
    rec.dipNs[rec.nDips] = (int32_t)ns;
    rec.dipLenNs[rec.nDips] = d.hasFall ? (int32_t)ticksToNs((int32_t)(d.fall - d.rise)) : measure::NO_EDGE;
    rec.nDips++;
    d.claimed = true;
  }
  portEXIT_CRITICAL(&mux);
}

void curtainCount(const measure::EventFit& f) {
  if (f.curtain < 0) return;
  nJudged[f.acoustic == measure::V_OK][f.curtain]++;
}

void curtainService(bool quiet) {
  if (!enabled || !quiet) return;
  const uint64_t now = captureSamplesNow();
  for (;;) {
    Dip d;
    bool found = false;
    portENTER_CRITICAL(&mux);
    const uint32_t h = head;
    uint32_t k = h > RING ? h - RING : 0;
    for (; k < h; k++) {
      Dip& r = ring[k % RING];
      if (r.claimed) continue;
      if (-ticksToNs(ticksAfter(r.rise, now)) < ORPHAN_AGE_NS) break;   // too new; so are the later ones
      r.claimed = true;
      d = r;
      found = true;
      break;
    }
    portEXIT_CRITICAL(&mux);
    if (!found) return;
    nOrphans++;
    // Noise on IR_IN (nothing connected, a lamp) would flood the serial port: at most 10 a second.
    const uint32_t ms = millis();
    if (ms - lastPrintMs < 100) {
      nOrphansQuiet++;
      continue;
    }
    lastPrintMs = ms;
    const int64_t at = (int64_t)now * (1000000000LL / CAPTURE_SAMPLE_HZ) + ticksToNs(ticksAfter(d.rise, now));
    const int64_t len = d.hasFall ? ticksToNs((int32_t)(d.fall - d.rise)) : -1;
    char lenText[16];
    if (len >= 0) snprintf(lenText, sizeof lenText, "%lld", (long long)len);
    else strcpy(lenText, "-");
    Serial.printf("dip n=%u t=%lld.%09lld len=%s\n", (unsigned)nOrphans, (long long)(at / 1000000000LL),
                  (long long)(at % 1000000000LL), lenText);
    Serial.println("  a curtain dip with no acoustic event within 1.5 ms: a shot through an existing "
                   "hole? (no position: not sent)");
  }
}

uint32_t curtainOrphans() { return nOrphans; }

void curtainPrintStatus() {
  const measure::Cfg& cfg = calibCfg();
  if (!capTimer) {
    Serial.println("curtain: no MCPWM capture (see the start-up messages)");
    return;
  }
  Serial.print("curtain: ");
  if (cfg.curtainHiNs > 0)
    Serial.printf("on, a dip %.0f-%.0f us after the shot; IR_EN high", cfg.curtainLoNs / 1000,
                  cfg.curtainHiNs / 1000);
  else
    Serial.print("off (no gate); IR_EN low");
  Serial.printf("; IR_IN (GPIO %d) %s now%s\n", IR_IN_PIN,
                gpio_get_level((gpio_num_t)IR_IN_PIN) ? "high (blocked, or no receiver)" : "low (clear)",
                simDrive ? ", driven by the simulator" : "");
  Serial.printf("curtain: since boot %u dips; acoustic ok with a dip in the window %u, without %u; "
                "acoustic reject with %u, without %u; %u dips with no event",
                (unsigned)head, (unsigned)nJudged[1][1], (unsigned)nJudged[1][0],
                (unsigned)nJudged[0][1], (unsigned)nJudged[0][0], (unsigned)nOrphans);
  if (nOrphansQuiet) Serial.printf(" (%u of them not printed: more than 10 a second)", (unsigned)nOrphansQuiet);
  Serial.printf("; capture %.0f MHz, %u ticks per sample\n", capHz / 1e6, (unsigned)ticksPerSample);
}

void curtainSimDrive(bool on) {
  if (on == simDrive) return;
  if (on) {
    gpio_set_level((gpio_num_t)IR_IN_PIN, 0);
    esp_rom_gpio_connect_out_signal(IR_IN_PIN, SIG_GPIO_OUT_IDX, false, false);
    gpio_set_direction((gpio_num_t)IR_IN_PIN, GPIO_MODE_INPUT_OUTPUT);   // the capture keeps reading it
    gpio_set_level((gpio_num_t)IR_IN_PIN, 0);
  } else {
    gpio_set_direction((gpio_num_t)IR_IN_PIN, GPIO_MODE_INPUT);
  }
  simDrive = on;
}

bool curtainSimDriving() { return simDrive; }

void curtainSimSync(bool on) {
  if (!capTimer) return;
  if (on) {
    esp_rom_gpio_connect_in_signal(SYNC_PIN, PWM0_CAP2_IN_IDX, false);
    MCPWM0.cap_chn_cfg[2].capn_mode = 2;   // rising edges
  } else {
    MCPWM0.cap_chn_cfg[2].capn_mode = 0;
    esp_rom_gpio_connect_in_signal(GPIO_MATRIX_CONST_ZERO_INPUT, PWM0_CAP2_IN_IDX, false);
  }
}

uint32_t curtainSimSyncTick() { return MCPWM0.cap_chn[2].capn_value; }

uint32_t curtainDipCount() { return head; }

bool curtainNewestDip(uint32_t* rise, uint32_t* fall) {
  portENTER_CRITICAL(&mux);
  const bool ok = head && ring[(head - 1) % RING].hasFall;
  if (ok) {
    *rise = ring[(head - 1) % RING].rise;
    *fall = ring[(head - 1) % RING].fall;
  }
  portEXIT_CRITICAL(&mux);
  return ok;
}

double curtainTickNs() { return capHz ? 1e9 / capHz : 0; }

uint32_t curtainSampleTick(uint64_t sample) { return startTick + (uint32_t)(sample * ticksPerSample); }

double curtainTie(uint32_t tick, uint64_t sample) {
  // The I2S saw the line high first at `sample`: the edge came in the sample before it, on
  // average half a sample earlier.
  const uint32_t tied = tick - (uint32_t)(sample * ticksPerSample) + ticksPerSample / 2;
  const double off = (int32_t)(startTick - tied) * curtainTickNs();
  startTick = tied;
  return off;
}
