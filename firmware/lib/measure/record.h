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
  // Validity (MODEL.md "Validity"; off where 0, or -1 for maxLost: logs from before 4a).
  int8_t maxLost;            // mic channels of the event that may be left out of the fit
  float areaMm;              // the hole within +-areaMm of the centre, x and y
  float togetherNs;          // every first edge within this: not a shot (a spike, a blast head-on)
  float pulseMaxNs;          // t2 - t1 up to this, or the channel is left out (and t2 >= t1)
  // The curtain's gate (4b): a dip must rise this long after the emission time (ns). 0, 0:
  // off (no curtain, or a log from before 4b).
  float curtainLoNs, curtainHiNs;
};

static const float RES_LIMIT_DEFAULT_NS = 1500;   // also for logs from before `lim=`
// The firmware's defaults for validity (step 4a).
static const int8_t MAX_LOST_DEFAULT     = 1;
static const float  AREA_DEFAULT_MM      = 105;     // the mics' edge: a hole beyond would hit the frame
static const float  TOGETHER_DEFAULT_NS  = 20000;   // a real shot spreads over 126 us or more
static const float  PULSE_MAX_DEFAULT_NS = 10000;
// The curtain's window (4b, PLAN D9): the sheet 15 mm behind the paper at 150-350 m/s. The
// firmware's curtain is off until one is fitted (`curtain on`).
static const float  CURTAIN_LO_DEFAULT_NS = 45000;
static const float  CURTAIN_HI_DEFAULT_NS = 100000;

// "cfg id=1A2B3C4D CH1=x,y,h,k,offset,sigma ... CH7=- CH8=- lim=1500 drop=1 lost=1 area=105
// tog=20000 pmax=10000 ir=45000,100000" (`ir=-`: the curtain's gate off). Floats with
// 9 significant digits, so they read back to the same bits. The id is the CRC-32 of the text after
// "id=XXXXXXXX ", so it changes with any setting. Returns the length (snprintf's).
int formatCfg(char* out, size_t size, const Cfg& cfg);
uint32_t cfgId(const Cfg& cfg);
// The calibration's id: the CRC-32 of the channel fields alone ("CH1=... CH8=-"), so the
// policy (lim, drop) doesn't change it. tools/calibrate.py computes the same.
uint32_t calibrationId(const Cfg& cfg);
// Reads a cfg line (it may have text before "cfg id="). False if it isn't one, or if its
// id doesn't match its fields (a corrupted line).
bool parseCfg(const char* line, Cfg& cfg, uint32_t* id);

static const int32_t NO_EDGE = -1;
static const int     MAX_DIPS = 4;   // curtain dips kept per event

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
  bool     hasPt;              // the true point: calibration mode's template point, or the simulator's
  float    ptX, ptY;           //   shot (`pt=x,y`, mm)
  bool     calMode;            // made in calibration mode (`mode=cal`): not sent to the app
  bool     hasCal;             // a calibration record (`cal=XXXXXXXX`, its id) ...
  uint32_t calId;
  bool     calStale;           // ... changed since (`stale=1`)
  // The curtain (4b): its dips near the event (`ir=rise/length,...`, ns, the rise from the
  // event's first edge; `ir=-`: on, but none). hasIr false: the curtain was off.
  bool     hasIr;
  uint8_t  nDips;
  int32_t  dipNs[MAX_DIPS];
  int32_t  dipLenNs[MAX_DIPS];   // NO_EDGE: its fall was not seen
};

// "rec n=2161 t=876.1903407 cfg=1A2B3C4D T=18.00 RH=50.0 p=1013.25 meas=--- c=342.739777
//  CH5=0,1500 CH6=100000,101500 ... ir=-150300/34500" (one line).
int formatRec(char* out, size_t size, const Rec& rec);
bool parseRec(const char* line, Rec& rec);

// The solver's input for an event: t0 per channel from its two edges with that channel's k
// and offset, its mic and sigma, c from the record. Returns the mask of the channels to fit:
// fitted in the event, a mic on the channel, both edges seen, and (with pulseMaxNs) a pulse
// shape in range: 0 <= t2 - t1 <= pulseMaxNs. `shapeBad`, if given: the channels left out for
// their shape alone.
uint8_t solveInput(const Cfg& cfg, const Rec& rec, SolveInput& in, uint8_t* shapeBad = nullptr);

// An event's verdict (MODEL.md "Validity"): ok, or the first check it failed, in this order.
enum Verdict : uint8_t {
  V_OK,         // a shot
  V_FEW,        // fewer than 4 channels to fit
  V_TOGETHER,   // every first edge within togetherNs: an electrical spike, or a blast head-on
  V_NOFIT,      // the fit did not converge
  V_RESIDUAL,   // rms residual over resLimitNs (after drop-one, if on)
  V_LOST,       // more than maxLost mic channels of the event left out (no edge, shape, dropped)
  V_OUTSIDE,    // the hole beyond +-areaMm
  V_CURTAIN,    // the gate on, and no dip in its window (checked last: PLAN D9)
  V_COUNT
};
const char* verdictName(Verdict v);   // "ok", "few", "together", ..., "outside", "curtain"
bool parseVerdict(const char* name, Verdict* v);

// An event as the target solves and judges it (MODEL.md "Solver", "Drop-one", "Validity",
// with the cfg's policy): the fit over the channels solveInput gives, if there are 4 or more;
// with dropOne and 5 or more, solveDropOne; then the verdict, the curtain last. `good`:
// verdict ok (the shot goes to the app).
struct EventFit {
  uint8_t  mics;      // the event's channels with a mic
  uint8_t  mask;      // the channels given to the fit
  uint8_t  shapeBad;  // left out for their pulse shape
  int      used;      // how many given to the fit
  int      lost;      // mic channels of the event not in the final fit
  float    spreadNs;  // the first edges' spread (t1) over the channels given to the fit
  bool     solved;    // 4 or more: the fit was run
  Solution s;         // s.dropped: the channel drop-one left out, -1 if none
  Verdict  verdict;
  bool     good;
  // The curtain, judged apart from the acoustic checks (PLAN D9 counts each defence).
  Verdict  acoustic;    // the verdict without the curtain
  int8_t   curtain;     // 1: a dip in the window, 0: none, -1: not judged (gate off, or no fit)
  int      dip;         // the dip in the window, else the one nearest to it; -1: no dip
  float    dipAfterNs;  // that dip's rise after the emission time (s.tauNs), ns; NAN if none
};
EventFit solveEvent(const Cfg& cfg, const Rec& rec);

uint32_t crc32(const char* s);   // CRC-32 (IEEE 802.3), as zlib's

}  // namespace measure
