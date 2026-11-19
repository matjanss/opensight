// OpenSights - capture of the comparator lines (measurement step 1, see ../MEASUREMENT.md).
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
