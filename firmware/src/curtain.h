// OpenSights - the IR light curtain (measurement step 4b, see ../MEASUREMENT.md, PLAN D9).
//
// IR_IN (GPIO 13) is the curtain comparator: low while the beam is clear, high while
// something crosses the sheet behind the paper (a pellet: 30-40 us). MCPWM group 0's
// capture unit latches each rise and fall in hardware, on two channels, at the 80 MHz APB
// clock: 8 ticks per 10 MHz sample, from the same crystal. The capture timer is tied to the
// samples at boot by one edge both see (capture.cpp: a pulse on GPIO 12, into capture
// channel 2 and, for that moment, into the I2S in place of CH1 L), so a dip's time is
// known on the event's own clock, to about half a sample, however late its interrupt runs.
// A software catch at the I2S start gives a first tie; register writes to the MCPWM unit
// can stall for µs, so it can be off by several (bench 4b), and the edge replaces it.
//
// The gate (`curtain on`, MODEL.md "Validity"): a shot needs a dip rising 45-100 us after its
// emission time. A dip with no acoustic event within 1.5 ms is printed as a `dip` line (a
// shot through an existing hole: no position, so not sent with protocol v2).
#pragma once
#include <Arduino.h>
#include "record.h"

// The capture timer and its channels (enabled with the gate). From captureBegin, after
// calibBegin and before the I2S starts.
bool curtainBegin();
// Around the I2S start, with interrupts off: the catch, then (after rx_start) its value: the
// first tie.
void curtainMarkStart();
void curtainMarkRead();
// The tie by an edge: channel 2 latches GPIO 12's rises (curtainSimSync(true)), then
// curtainTie(the latched tick, the sample the I2S saw the edge rise in). Returns how far the
// first tie was off, ns.
double curtainTie(uint32_t tick, uint64_t sample);

// After a change of the window (calib.cpp): the capture channels and IR_EN follow the gate.
void curtainApply();

// An event's dips: those rising within 1.5 ms of its first edge (`firstSample`, samples since
// the capture start), in ns from it, into rec (hasIr, the dips). Marks them as the event's.
void curtainCollect(uint64_t firstSample, measure::Rec& rec);
// Counts an event's curtain result for `stat` (the gate on): PLAN D9 counts each defence.
void curtainCount(const measure::EventFit& f);

// From loop() (captureService), with `quiet` when no event is being collected or waiting:
// prints the dips no event has claimed as `dip` lines.
void curtainService(bool quiet);

void curtainPrintStatus();   // `curtain`, `stat`
uint32_t curtainOrphans();   // dips with no acoustic event since boot (the simulator)

// For the simulator: GPIO 13 driven by it (true; low at rest) or a plain input again.
// Only without the IR receiver: on the host board, take U4 (LM393) out of its socket first.
void curtainSimDrive(bool on);
bool curtainSimDriving();
// For the simulator's truth, all on the capture timer's clock: channel 2 (free after the
// start's catch) latches the sync pin's rises during a run, so the run's tick 0 is known
// in hardware, like the dip's edges.
void curtainSimSync(bool on);
uint32_t curtainSimSyncTick();                       // the sync pin's last rise
uint32_t curtainDipCount();                          // dips latched since boot
bool curtainNewestDip(uint32_t* rise, uint32_t* fall);   // false: none, or its fall not yet
double curtainTickNs();                              // one capture tick, ns (12.5)
uint32_t curtainSampleTick(uint64_t sample);         // the capture tick of a sample (the tie)
