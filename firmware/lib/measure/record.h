// OpenSights measurement core - the log lines (../../MODEL.md "Log lines"): `cfg` (the
// settings the solver used) and `rec` (one event, everything needed to solve it again).
// The firmware prints them; tools/replay.cpp (and later the app's calibration fit) reads
// them. Both go through solveInput(), so a replay solves exactly what the target solved.
// Portable: no Arduino, no IDF; also built on the PC.
#pragma once
#include <stddef.h>
#include "solver.h"

namespace measure {

// One channel's settings: its mic (if it has one) and its arrival-time calibration.
struct ChannelCfg {
  bool  mic;        // false: no mic on this channel, never in a fit
  float x, y, h;    // mm (h: the mic port behind the paper plane)
  float k;          // arrival-time factor (MODEL.md "Arrival")
  float offsetNs;   // the channel's own delay
  float sigmaNs;    // its timing error, 1 sigma (the fit's weight)
};

struct Cfg {
  ChannelCfg ch[MAX_MICS];   // ch[0] = CH1
  float resLimitNs;          // a shot is good if its fit converged with an rms residual up to this
  bool  dropOne;             // over the limit: try the fit with one channel left out
};

static const float RES_LIMIT_DEFAULT_NS = 1500;   // also for logs from before `lim=`

// "cfg id=1A2B3C4D CH1=x,y,h,k,offset,sigma ... CH7=- CH8=- lim=1500 drop=1". Floats with
// 9 significant digits, so they read back to the same bits. The id is the CRC-32 of the text after
// "id=XXXXXXXX ", so it changes with any setting. Returns the length (snprintf's).
int formatCfg(char* out, size_t size, const Cfg& cfg);
uint32_t cfgId(const Cfg& cfg);
// Reads a cfg line (it may have text before "cfg id="). False if it isn't one, or if its
// id doesn't match its fields (a corrupted line).
bool parseCfg(const char* line, Cfg& cfg, uint32_t* id);

static const int32_t NO_EDGE = -1;

struct Rec {
  uint32_t n;                  // event number
  double   timeS;              // the event's first edge, s since capture start
  uint32_t cfgId;              // the cfg it was solved with
  float    tC, rh, hPa;        // the air
  char     measured[4];        // "TRP" where measured, '-' where assumed ("T--": a DS18B20)
  float    cMs;                // the speed of sound used, m/s
  uint8_t  channels;           // bit c: CH(c+1) was fitted in this event
  int32_t  t1[MAX_MICS];       // OUT_L's first rise, ns from the event's first edge, or NO_EDGE
  int32_t  t2[MAX_MICS];       // OUT_H's
};

// "rec n=2161 t=876.1903407 cfg=1A2B3C4D T=18.00 RH=50.0 p=1013.25 meas=--- c=342.739777
//  CH5=0,1500 CH6=100000,101500 ..." (one line).
int formatRec(char* out, size_t size, const Rec& rec);
bool parseRec(const char* line, Rec& rec);

// The solver's input for an event: t0 per channel from its two edges with that channel's k
// and offset, its mic and sigma, c from the record. Returns the mask of the channels to fit:
// fitted in the event, both edges seen, and a mic on the channel.
uint8_t solveInput(const Cfg& cfg, const Rec& rec, SolveInput& in);

// An event as the target solves it (MODEL.md "Solver" and "Drop-one", with the cfg's
// policy): the fit over the channels solveInput gives, if there are 4 or more; with dropOne
// and 5 or more, solveDropOne. `good`: converged, rms residual within resLimitNs (the shot
// goes to the app).
struct EventFit {
  uint8_t  mask;      // the channels given to the fit
  int      used;      // how many
  bool     solved;    // 4 or more: the fit was run
  Solution s;         // s.dropped: the channel drop-one left out, -1 if none
  bool     good;
};
EventFit solveEvent(const Cfg& cfg, const Rec& rec);

uint32_t crc32(const char* s);   // CRC-32 (IEEE 802.3), as zlib's

}  // namespace measure
