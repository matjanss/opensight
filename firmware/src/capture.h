// OpenSights - capture of the comparator lines (measurement steps 1-2, see ../MEASUREMENT.md).
//
// I2S0 in camera (parallel) mode samples all 16 lines into a DMA ring. An interrupt scans
// each finished block for activity; a task on core 1 then extracts the first edges of the
// event window. Results go out over serial from loop(), so the measurement task never
// waits on the UART.
#pragma once
#include <Arduino.h>

// Sets up the pins, the DMA ring, I2S0 and the measurement task. Call once from setup(),
// after BLE (the ring takes 64 KB of DMA-capable RAM). False if it could not start.
bool captureBegin();

// From loop(): prints finished events and runs the self-test.
void captureService(uint32_t nowMs);

// One line typed on the serial port (without the line end).
void captureCommand(const char* line);

// For the self-test and the simulator (simulate.cpp).
bool     captureIdle();               // no event being collected or read
uint64_t captureSamplesNow();         // samples since the start, to the last finished block
bool     captureSelftestActive();
uint32_t captureMissedCount();        // activity in the frozen ring after the dead time
uint32_t captureWindowUs();           // the event window covers this long after the first edge
uint32_t captureDeadtimeMs();         // the setting
uint32_t captureDeadUs();             // in effect: at least the event window
void     captureTestChannels(uint16_t channels);   // add channels (bit n = CH(n+1)), not stored
void     captureEndTestChannels();                 // back to the stored mask
void     captureLinesToInputs(uint16_t lines);     // pins back to plain inputs into the I2S
