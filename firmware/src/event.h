// OpenSights - one captured event: the first edges of every line in the event window.
// Made by the measurement task (capture.cpp), read by the output, the arrival times
// (arrival.cpp), the self-test and the simulator (simulate.cpp).
#pragma once
#include <Arduino.h>
#include "config.h"

static const int      LINES             = 16;   // 8 channels x (OUT_L, OUT_H)
static const int      CHANNELS          = 8;
static const int      SAMPLES_PER_BLOCK = CAPTURE_BLOCK_BYTES / 2;
static const uint32_t NS_PER_SAMPLE     = 1000000000UL / CAPTURE_SAMPLE_HZ;
static const uint32_t BLOCK_US          = (uint32_t)((uint64_t)SAMPLES_PER_BLOCK * 1000000 / CAPTURE_SAMPLE_HZ);
static const int      MAX_TRANSITIONS   = 16;   // per line and event, kept for verbose output

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
  uint16_t lines;        // line mask used for this event (bit 2c = CH(c+1) L, 2c+1 = H)
  uint16_t highAtStart;  // lines already high at window start
  LineResult line[LINES];
};

// Window sample of the event's first rising edge on any fitted line (0 if there is none):
// the reference all printed times count from.
inline int eventFirstEdge(const EventResult& r) {
  int ref = INT16_MAX;
  for (int l = 0; l < LINES; l++)
    if ((r.lines & (1 << l)) && r.line[l].rise >= 0 && r.line[l].rise < ref) ref = r.line[l].rise;
  return ref == INT16_MAX ? 0 : ref;
}
