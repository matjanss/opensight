// OpenSights - hardware-timed pulse pairs on the comparator pins (MCPWM), for the self-test
// and the simulator. See pulsegen.cpp.
#pragma once
#include <Arduino.h>

const int PULSEGEN_MAX_CHANNELS = 6;   // 2 MCPWM units x 3 operators

// Sets up nCh channels, pins[i] = {L pin, H pin}. Channels 0-2 run on MCPWM unit 0, 3-5 on
// unit 1. One run lasts periodTicks ticks at hz (at most 65535); every line falls at its
// end. False if it could not (then call pulsegenDelete).
bool pulsegenCreate(int nCh, const int pins[][2], uint32_t hz, uint32_t periodTicks);

// When channel ch's lines rise in the next runs, in ticks from the start of the run
// (1 .. periodTicks - 2). A line that is off stays low.
bool pulsegenSet(int ch, uint32_t riseL, uint32_t riseH, bool onL = true, bool onH = true);

// A new run length (at most the one pulsegenCreate was given), for the next runs. Call it
// before pulsegenSet when the rises move earlier.
bool pulsegenSetPeriod(uint32_t periodTicks);

// One run. Returns how long the start took in ns (start of the first timer to the sync
// edge that lines both units up); a rise earlier than that would have come too soon.
// `syncCycle`, if given: the CPU cycle count just after that edge, the run's tick 0 (the
// simulator times the curtain's dip against it).
uint32_t pulsegenFire(uint32_t* syncCycle = nullptr);

// Frees the MCPWM units and the sync pin. The line pins are left to the caller.
void pulsegenDelete();
