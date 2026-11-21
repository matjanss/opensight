// OpenSights - the shot buffer (main.cpp): the last SHOT_BUFFER shots, indicated to the app
// one at a time until it confirms each (protocol/PROTOCOL.md, the v2 shot message).
#pragma once
#include <stdint.h>

// Adds a shot at (x, y), 0.01 mm from the centre, and flashes the LED. Returns its number.
uint32_t shotAdd(int32_t x, int32_t y, uint32_t nowMs);
