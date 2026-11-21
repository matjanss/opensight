// OpenSights - hardware-timed pulse pairs (MCPWM), for the self-test and the simulator.
//
// Each channel is one MCPWM operator: its two comparators set when L and H rise, and both
// lines fall at the end of the run (the timer's peak), like a comparator pair on a real
// edge. The times are set by the timer hardware in ticks, so BLE, flash writes or the CPU's
// write buffer can't move them (a CPU-driven generator could: step 1, bench tests 1-4).
//
// Channels 0-2 share the timer of unit 0, so their pulses are exact against each other.
// Channels 3-5 run on unit 1, which has its own timer. Both timers take their count from one
// edge on SYNC_PIN (an output read back inside the chip by both units' sync inputs), so the
// CPU's timing doesn't matter. A run: a sync edge sets both stopped counts to 0 (a one-shot
// restart from the peak can stop again at once: step 1, extra test 11), both timers are
// started (one-shot: they stop at the next peak), and a second sync edge sets both counts to
// 0 again at the same moment. The rises must come later than that: rises from tick 1 on are
// safe only if the start took less than a tick, so callers leave a lead (pulsegenFire
// reports how long the start took).
#include "pulsegen.h"
#include "config.h"
#include "driver/gpio.h"
#include "driver/mcpwm_prelude.h"
#include "esp_cpu.h"
#include "esp_rom_gpio.h"
#include "esp_rom_sys.h"
#include "soc/gpio_reg.h"
#include "soc/gpio_sig_map.h"
#include "soc/mcpwm_struct.h"

static_assert(SYNC_PIN < 32, "pulsegenFire drives SYNC_PIN through GPIO_OUT_W1TS/W1TC");

static const int GROUPS = 2, OPS_PER_GROUP = 3;

static int      nChannels = 0, nGroups = 0;
static uint32_t genHz = 0, genPeriod = 0, maxPeriod = 0;
static mcpwm_timer_handle_t timer[GROUPS] = {};
static mcpwm_sync_handle_t  syncSrc[GROUPS] = {};
static bool                 phaseSet[GROUPS] = {};
static bool                 timerEnabled[GROUPS] = {};
static mcpwm_oper_handle_t  oper[PULSEGEN_MAX_CHANNELS] = {};
static mcpwm_cmpr_handle_t  cmpr[PULSEGEN_MAX_CHANNELS][2] = {};
static mcpwm_gen_handle_t   gen[PULSEGEN_MAX_CHANNELS][2] = {};
// The units' registers, for the start in pulsegenFire (the driver's start function runs
// from flash). The driver gives a unit's first timer id 0, which pulsegenCreate checks.
static mcpwm_dev_t* const DRAM_ATTR hw[GROUPS] = {&MCPWM0, &MCPWM1};
static volatile int DRAM_ATTR fireGroups = 0;

// The sync pin at rest: a plain input with a pull-down. gpio_reset_pin() turns the pull-up
// on, which a strapping pin (GPIO 12: flash voltage) must not have.
static void syncPinIdle() {
  gpio_reset_pin((gpio_num_t)SYNC_PIN);
  gpio_pullup_dis((gpio_num_t)SYNC_PIN);
  gpio_pulldown_en((gpio_num_t)SYNC_PIN);
}

static void IRAM_ATTR spinNs(uint32_t ns) {
  uint32_t cycles = ns * (uint32_t)esp_rom_get_cpu_ticks_per_us() / 1000;
  uint32_t c0 = esp_cpu_get_cycle_count();
  while (esp_cpu_get_cycle_count() - c0 < cycles) {}
}

uint32_t IRAM_ATTR pulsegenFire() {
  const uint32_t bit = 1u << SYNC_PIN;
  const int groups = fireGroups;
  uint32_t state = portSET_INTERRUPT_MASK_FROM_ISR();
  REG_WRITE(GPIO_OUT_W1TS_REG, bit);   // sync 1: both stopped counts to 0
  __asm__ __volatile__("memw");
  spinNs(200);
  REG_WRITE(GPIO_OUT_W1TC_REG, bit);
  __asm__ __volatile__("memw");
  spinNs(200);
  uint32_t c0 = esp_cpu_get_cycle_count();
  for (int g = 0; g < groups; g++) hw[g]->timer[0].timer_cfg1.timer_start = 4;   // start, stop at peak
  REG_WRITE(GPIO_OUT_W1TS_REG, bit);   // sync 2: both counts to 0 at the same moment
  __asm__ __volatile__("memw");
  uint32_t c1 = esp_cpu_get_cycle_count();
  spinNs(200);
  REG_WRITE(GPIO_OUT_W1TC_REG, bit);
  __asm__ __volatile__("memw");
  portCLEAR_INTERRUPT_MASK_FROM_ISR(state);
  return (c1 - c0) * 1000 / esp_rom_get_cpu_ticks_per_us();
}

void pulsegenDelete() {
  for (int c = 0; c < PULSEGEN_MAX_CHANNELS; c++) {
    for (int h = 0; h < 2; h++) {
      if (gen[c][h]) mcpwm_del_generator(gen[c][h]);
      if (cmpr[c][h]) mcpwm_del_comparator(cmpr[c][h]);
      gen[c][h] = nullptr;
      cmpr[c][h] = nullptr;
    }
    if (oper[c]) mcpwm_del_operator(oper[c]);
    oper[c] = nullptr;
  }
  for (int g = 0; g < GROUPS; g++) {
    if (timerEnabled[g]) mcpwm_timer_disable(timer[g]);
    timerEnabled[g] = false;
    if (phaseSet[g]) {
      mcpwm_timer_sync_phase_config_t ph = {};   // sync_src null: detach
      mcpwm_timer_set_phase_on_sync(timer[g], &ph);
    }
    phaseSet[g] = false;
    if (syncSrc[g]) mcpwm_del_sync_src(syncSrc[g]);
    syncSrc[g] = nullptr;
    if (timer[g]) mcpwm_del_timer(timer[g]);
    timer[g] = nullptr;
  }
  if (nGroups) syncPinIdle();
  nChannels = nGroups = 0;
  fireGroups = 0;
}

bool pulsegenCreate(int nCh, const int pins[][2], uint32_t hz, uint32_t periodTicks) {
  if (nGroups || nCh < 1 || nCh > PULSEGEN_MAX_CHANNELS || periodTicks < 4 || periodTicks > 65535)
    return false;
  nChannels = nCh;
  nGroups = (nCh + OPS_PER_GROUP - 1) / OPS_PER_GROUP;
  genHz = hz;
  genPeriod = maxPeriod = periodTicks;

  // The sync pin: low before anything listens to it.
  syncPinIdle();
  gpio_set_level((gpio_num_t)SYNC_PIN, 0);

  for (int g = 0; g < nGroups; g++) {
    mcpwm_timer_config_t tc = {};
    tc.group_id = g;
    tc.clk_src = MCPWM_TIMER_CLK_SRC_DEFAULT;
    tc.resolution_hz = hz;
    tc.count_mode = MCPWM_TIMER_COUNT_MODE_UP;
    tc.period_ticks = periodTicks;
    if (mcpwm_new_timer(&tc, &timer[g]) != ESP_OK) return false;
    mcpwm_gpio_sync_src_config_t sc = {};
    sc.group_id = g;
    sc.gpio_num = SYNC_PIN;
    sc.flags.pull_down = 1;
    if (mcpwm_new_gpio_sync_src(&sc, &syncSrc[g]) != ESP_OK) return false;
    mcpwm_timer_sync_phase_config_t ph = {};
    ph.sync_src = syncSrc[g];
    ph.count_value = 0;
    ph.direction = MCPWM_TIMER_DIRECTION_UP;
    if (mcpwm_timer_set_phase_on_sync(timer[g], &ph) != ESP_OK) return false;
    phaseSet[g] = true;
  }
  // Now drive the pin too; the sync inputs keep reading it through the GPIO matrix.
  esp_rom_gpio_connect_out_signal(SYNC_PIN, SIG_GPIO_OUT_IDX, false, false);
  gpio_set_direction((gpio_num_t)SYNC_PIN, GPIO_MODE_INPUT_OUTPUT);
  gpio_set_level((gpio_num_t)SYNC_PIN, 0);

  for (int c = 0; c < nCh; c++) {
    int g = c / OPS_PER_GROUP;
    mcpwm_operator_config_t oc = {};
    oc.group_id = g;
    if (mcpwm_new_operator(&oc, &oper[c]) != ESP_OK) return false;
    if (mcpwm_operator_connect_timer(oper[c], timer[g]) != ESP_OK) return false;
    for (int h = 0; h < 2; h++) {
      mcpwm_comparator_config_t cc = {};   // no update flags: new values apply at once
      if (mcpwm_new_comparator(oper[c], &cc, &cmpr[c][h]) != ESP_OK) return false;
      if (mcpwm_comparator_set_compare_value(cmpr[c][h], periodTicks - 2) != ESP_OK) return false;
      mcpwm_generator_config_t gc = {};
      gc.gen_gpio_num = pins[c][h];
      gc.flags.io_loop_back = 1;   // keep the pad's input on: the I2S reads it
      if (mcpwm_new_generator(oper[c], &gc, &gen[c][h]) != ESP_OK) return false;
      // Low until pulsegenSet says when it rises.
      if (mcpwm_generator_set_force_level(gen[c][h], 0, true) != ESP_OK) return false;
      if (mcpwm_generator_set_action_on_compare_event(gen[c][h],
            MCPWM_GEN_COMPARE_EVENT_ACTION(MCPWM_TIMER_DIRECTION_UP, cmpr[c][h],
                                           MCPWM_GEN_ACTION_HIGH)) != ESP_OK) return false;
      if (mcpwm_generator_set_actions_on_timer_event(gen[c][h],
            MCPWM_GEN_TIMER_EVENT_ACTION(MCPWM_TIMER_DIRECTION_UP, MCPWM_TIMER_EVENT_FULL,
                                         MCPWM_GEN_ACTION_LOW),
            MCPWM_GEN_TIMER_EVENT_ACTION(MCPWM_TIMER_DIRECTION_UP, MCPWM_TIMER_EVENT_EMPTY,
                                         MCPWM_GEN_ACTION_LOW),
            MCPWM_GEN_TIMER_EVENT_ACTION_END()) != ESP_OK) return false;
    }
  }

  for (int g = 0; g < nGroups; g++) {
    if (mcpwm_timer_enable(timer[g]) != ESP_OK) return false;
    timerEnabled[g] = true;
    // Check that this unit's timer is timer 0, which pulsegenFire starts: run it once
    // (every line is still forced low) and see its count move.
    hw[g]->timer[0].timer_cfg1.timer_start = 4;
    delayMicroseconds(2);
    uint32_t count = hw[g]->timer[0].timer_status.timer_value;
    delayMicroseconds((uint64_t)periodTicks * 1000000 / hz + 10);   // let it reach its peak
    if (count == 0) {
      Serial.printf("pulsegen: MCPWM unit %d's timer is not timer 0\n", g);
      return false;
    }
  }
  fireGroups = nGroups;
  return true;
}

bool pulsegenSetPeriod(uint32_t periodTicks) {
  if (!nGroups || periodTicks < 4 || periodTicks > maxPeriod) return false;
  for (int g = 0; g < nGroups; g++)
    if (mcpwm_timer_set_period(timer[g], periodTicks) != ESP_OK) return false;   // takes effect at once
  genPeriod = periodTicks;
  return true;
}

bool pulsegenSet(int ch, uint32_t riseL, uint32_t riseH, bool onL, bool onH) {
  if (ch < 0 || ch >= nChannels) return false;
  const uint32_t rise[2] = {riseL, riseH};
  const bool on[2] = {onL, onH};
  for (int h = 0; h < 2; h++) {
    if (on[h] && (rise[h] < 1 || rise[h] > genPeriod - 2)) return false;
    if (on[h] && mcpwm_comparator_set_compare_value(cmpr[ch][h], rise[h]) != ESP_OK) return false;
    // Between runs every line is low (the run ended at the peak), so releasing the force
    // doesn't change the pin.
    if (mcpwm_generator_set_force_level(gen[ch][h], on[h] ? -1 : 0, true) != ESP_OK) return false;
  }
  return true;
}
