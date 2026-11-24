# Host board v0.3: schematic and board

The host board for the acoustic target: a NodeMCU-32S, 8 sensor connectors, the IR curtain emitter and receiver, an air sensor (BME280 or DS18B20), and the panel LED and button. Every part is through-hole and hand-soldered. The board is 76 × 132 mm (the long side runs north–south), 2 layers, 1.6 mm thick, with an M3 mounting hole in each corner.

The files are in KiCad 10 format; open them with KiCad 10 or newer. "Update PCB from Schematic" reports no changes.

**Not ordered yet; ready to order** (2026-11-24). The two changes below are on the board now,
and the south half was laid out again with them (see [Layout](#layout)):

1. J5 takes a BME280 as well as the DS18B20. See [A BME280 on J5](#1-a-bme280-on-j5).
2. The curtain comparator U4 has a supply jumper: 3.3 V for a TLC372, 5 V for an LM393, with its own decoupling. See [The curtain comparator's supply](#2-the-curtain-comparators-supply-33-v-or-5-v-by-a-jumper).

## Changes before ordering (done)

### 1. A BME280 on J5

**Why:** the speed of sound depends on humidity as well as temperature:

- 0.011 m/s per % RH at 18 °C, and 0.017 at 25 °C;
- 30 → 60 % RH is about 0.08 mm rms of shot position (0.12 mm at 25 °C);
- pressure has almost no effect (100 hPa ≈ 0.01 mm).

The DS18B20 can't see humidity. A BME280 (temperature, humidity and pressure, over I2C) can. Its temperature is no more accurate than the DS18B20's (±0.5 °C); humidity is the gain. See `../../PLAN.md` (D8).

**The change:**

1. **J5:** a 4-pin JST XH (B4B-XH-A) instead of the 3-pin. Pins 1–3 stay as they are, so a DS18B20 cable still plugs into pins 1–3.

   | J5 pin | Now (v0.3) | New |
   |---|---|---|
   | 1 | GND | GND |
   | 2 | DQ: GPIO 14 (U2 pin 12) | SDA, or the DS18B20's DQ: GPIO 14 (U2 pin 12) |
   | 3 | +3V3 (the NodeMCU's 3V3, U2 pin 1) | +3V3 (unchanged) |
   | 4 | – | SCL: GPIO 15 (U2 pin 35) |

2. **R45, new:** 4.7 k from +3V3 to GPIO 15, the SCL pull-up, next to R43. R43 (4.7 k, +3V3 to GPIO 14) stays: it is the SDA pull-up, and the DS18B20's pull-up.
3. **GPIO 12 stays unconnected, with no pull-up.** The firmware moves the pulse generator's sync loopback from GPIO 15 to GPIO 12. GPIO 12 is the flash-voltage strapping pin: anything holding it high at boot stops the module from starting.
4. **Silkscreen:** J5 "BME280 / DS18B20", with pin labels GND, SDA, 3V3, SCL.
5. **Status (2026-11-23):** done in the schematic and the parts list. J5 has its own symbol,
   `CONN4_AIR` (pins GND, SDA/DQ, VDD, SCL), in `host_board.kicad_sym`; the net DQ is now
   `SDA_DQ`, and GPIO 15 is on the new net `SCL`. R45 has R43's footprint (horizontal,
   7.62 mm). ERC: 0 violations. **On the board (2026-11-24):** J5's 4-pin footprint beside J6
   and J7, turned so pin 1 (GND) is at its east end; R45 under the NodeMCU between R43 and
   R44; the silkscreen; the PDFs.

**The sensor and its cable:**

- A BME280 breakout (GY-BME280 or similar), run from 3.3 V.
- Buy carefully: many "BME280" modules are really BMP280s, which have no humidity. The firmware reports the chip ID: 0x60 is a BME280, 0x58 a BMP280.
- Breakouts usually have the pin order VCC, GND, SCL, SDA, which doesn't match J5 one to one: wire the cable by signal name.
- Keep the cable short, up to about 50 cm (I2C at 100 kHz). Many breakouts have their own 10 k pull-ups; with R43 and R45 in parallel that's about 3.2 k, which is fine.
- Mount the sensor in the air between the mics, away from the host board and the IR LEDs, which warm it.

**Firmware:** done in measurement step 3a. The sync loopback is on GPIO 12 (`SYNC_PIN` in
`config.h`); `airtemp.cpp` looks for a BME280 or BMP280 on I2C (SDA 14, SCL 15), else a
DS18B20 on GPIO 14; the speed of sound has the humidity term. Its bench tests with the real
sensors (MEASUREMENT 3a-3, 3a-4) wait for the parts.

### 2. The curtain comparator's supply: 3.3 V or 5 V, by a jumper

**Why:** U4 runs from VA_IR, 3.3 V. Its inputs have to work up to the threshold: RV1 sets
VTH_IR to 1.71–2.35 V (R41 1.5 k, RV1 1 k, R42 2.7 k), and VMID_IR is 1.65 V. When a shadow
lifts the + input past a threshold that is out of the input range, both inputs are out of
range at the crossing, and the comparator's output isn't guaranteed: it can switch late or
not at all.

- **The LM393** (as designed): inputs up to Vcc − 1.5 V, so 1.8 V at 3.3 V (25 °C; Vcc − 2.0
  V = 1.3 V over its temperature range). All but the bottom 90 mV of RV1's travel is out of
  range, and VMID_IR is only 0.15 V below the limit. **At 3.3 V the LM393 doesn't work.**
- **The TLC372CP** (TI CMOS, LM393 pinout, open drain, 200 ns, €1.95 at Okaphone): inputs up
  to Vdd − 1 V at 25 °C, so 2.3 V at 3.3 V; about 2.1–2.3 V at 15–25 °C indoors. TI
  guarantees only Vdd − 1.5 V (1.8 V) over its 0–70 °C range, and characterises it at 5 V
  (it is rated from 3 V). At 3.3 V it works with TP6 (VTH_IR) up to about 2.1 V: VMID_IR +
  450 mV, most of RV1's travel.
- **A TLV3202 on a SOIC-8 to DIP-8 adapter** (the sensor board's comparator: inputs beyond
  both rails, push-pull, 40 ns, 2.7–5.5 V, the LM393's pinout) works at 3.3 V without limits.
  It isn't sold by Dutch shops: it comes from LCSC.

So U4's supply gets a jumper: **3.3 V** for the TLC372 (or the TLV3202), **5 V** for the
LM393. Start with the TLC372 at 3.3 V; if RV1's sweep on the bench (see [TODO.md](TODO.md))
shows it running out of input range, plug in an LM393 and move the jumper to 5 V. See
[Buying U3 and U4](#buying-u3-and-u4-in-the-netherlands) for the parts.

**The change:**

1. **JP1, new:** a 1x3 pin header (2.54 mm) with a jumper shunt, next to U4. Its middle pin
   is U4 pin 8 (V+, a new net `VU4`):
   - pin 1: **VA_IR** (3.3 V): the TLC372, or the TLV3202;
   - pin 3: **VCMP** (about 4.7 V, below): the LM393.

   Silkscreen: "U4 SUPPLY", with "3V3 (TLC372)" and "5V (LM393)" at the ends.
2. **VCMP, a new net** fed from 5V_F (the LP2950's input, after the Schottky):
   - **R46, new:** 10 Ω from 5V_F to VCMP;
   - **C18, new:** 10 µF electrolytic from VCMP to AGND.

   With R46 that is a 1.6 kHz low-pass against the USB rail's noise.
3. **C17 moves with pin 8:** it is U4's 100 nF already (pin 8 to pin 4, AGND), so it now
   decouples VU4, whichever supply is chosen. No new capacitor.
4. **Everything else stays on VA_IR:** U3, VMID_IR, the threshold divider and RV1, and C16
   (U3's 100 nF). The threshold always comes from the quiet 3.3 V.
5. **The output stays as it is:** open collector (LM393) or open drain (TLC372), pulled up to
   +3V3 by R40, so GPIO 13 never sees more than 3.3 V. With a push-pull part (the TLV3202) R40
   only draws 0.33 mA while the output is low, but then the jumper must be at 3.3 V: a
   push-pull output at 5 V would drive GPIO 13 to 5 V. R39 (1 M, the hysteresis) and R38
   are unchanged.
6. **U4 unit B (unused)** stays as it is: + (pin 5) at AGND, − (pin 6) at VA_IR, output (pin
   7) open. Its inputs are inside the range of each part at either supply, except the LM393
   at 3.3 V, which isn't used.
7. **Status (2026-11-23):** done in the schematic and the parts list. JP1 has its own symbol,
   `JMP1x3` (a 1x3 header with the pins named 3V3, U4, 5V), in `host_board.kicad_sym`. ERC: 0
   violations. U4's value is now TLC372. **On the board (2026-11-24):** R46 beside NT1 where
   the supplies cross into AGND, JP1 north of U4 with C18, C17 at U4's pin 8; the
   silkscreen; the PDFs.

**The ranges, per setting:**

| U4 | Jumper | Inputs work up to | RV1's travel (TP6 1.71–2.35 V) |
|---|---|---|---|
| TLC372CP | 3.3 V | about 2.3 V (25 °C), 1.8 V guaranteed (0–70 °C) | up to about 2.1 V: most of it |
| TLV3202 on an adapter | 3.3 V (never 5 V) | 3.5 V | all |
| LM393N | 5 V | about 2.9–3.5 V (25 °C), 2.4–3.0 V over temperature (VCMP 4.4–5.0 V: USB 4.75–5.25 V less the Schottky) | all |
| LM393N | 3.3 V | 1.8 V | **don't use** |

The amplifier's output can reach 3.3 V, above some of these ranges. That's fine: the output
stays right as long as one input (here the threshold) is inside the range. No input goes above
U4's supply.

**Speed:** fast enough with each. The chain from a shadow to IR_IN rising is about 3–8 µs:
the TIA about 1 µs, the ×21 stage about 3.3 µs (τ; the MCP6002's 1 MHz is the slowest part),
the comparator 0.2 µs (TLC372, 10 mV overdrive) or 0.3–1.3 µs (LM393, faster with more
overdrive), R40's rise about 0.2 µs. That delay moves every dip later, so the curtain's window
(`curtain window`, 45–100 µs now) is set from the bench test of the real curtain; a slow air
pistol pellet (150 m/s) is already at 100 µs.

**Also note:** C13 (3.3 nF) into R35 (10 k) is a 33 µs high-pass. During a 30–40 µs shadow the
signal sinks back toward VMID_IR, so the comparator falls early: the dip lengths in the
firmware's `rec` lines (`ir=rise/len`) are shorter than the real shadow. The rise, which the
gate uses, is not affected.

## Changes from v0.2

- The BAT85 clamp diodes are gone; the ESP32's own input clamps and the 470 Ω series resistors protect the inputs.
- The 1x20 debug row (J8) is replaced by a 1x3 scope header per channel (J21–J28: H, GND, L), next to the channel's resistors.
- New GPIO map: the H line of each channel is on the northern pin of its pair, so no line crosses another between the module and the connector.
- IR_EN moved from GPIO16 to GPIO27, DQ (DS18B20) from GPIO17 to GPIO14.
- Rectangular board with a new layout (below).

## Files

- `host_board.kicad_pro`, `.kicad_sch`, `.kicad_pcb`: the project, the schematic (one A2 sheet) and the board.
- `host_board_schematic.pdf`: the schematic as a PDF.
- `host_board_assembly.pdf`: the assembly drawing, with every reference on its part (for soldering).
- Gerbers and drill files are not in the repo: plot them from the board in KiCad when ordering (they are made from it, and go stale with every change to it).
- `host_board_parts_list.csv`: every part, with how many to fit for a 6-channel first build.
- `host_board.pretty/`: the NodeMCU-32S footprint (two 1x19 sockets, rows 22.86 mm apart, from your 23.53 mm measurement over the outside of the pins).
- `host_board.3dshapes/`: the NodeMCU-32S on its sockets as a 3D model (boxes, made by `make_nodemcu.py`), used by U2's footprint.
- `mechanical/`: the board as STEP and STL for the enclosure, with the sizes that matter (see its README).

## Layout

North is the antenna end of the NodeMCU. The north half (the module and the eight channels)
is as it was; the south half was laid out again on 2026-11-24, 14 mm longer, for the J5 and
U4-supply changes and with room for 0.4 mm tracks.

| Where | What |
|---|---|
| Middle, north half | The NodeMCU-32S on sockets, antenna at the north edge, USB plug pointing south (the area south of it is kept free for the plug). No copper under the antenna (a keep-out on both layers) |
| West edge | Sensors CH1–CH4 (J11–J14), north to south. Beside each one: scope header J2n and the four channel resistors |
| East edge | Sensors CH5–CH8 (J15–J18), mirrored |
| Under the NodeMCU | R43 (SDA pull-up), R45 (SCL pull-up), R44 (panel LED), lying flat |
| South of the module, west | The sensor supply: R1 and C3/C4 (5 V filter), U1 LP2950, C5/C6, TP2 (VSENS). R40 (IR_IN pull-up). Then the row where the supplies cross into the receiver's AGND: R30 (VA_IR), NT1 (the ground link), R46 (VCMP) |
| South of the module, east | J6 panel LED and J7 panel button, J5 air sensor (BME280 / DS18B20) below them |
| South-west | IR receiver on its own AGND copper, outlined: U4 (comparator) with JP1, C17, C18 and the threshold (RV1, TP6) in the north; U3 (TIA and ×21 stage) in the south, with J4 photodiodes at the south edge. Test loops TP3 AGND, TP4 TIA, TP5 AMP, TP6 VTH |
| East, south of CH8 | Power: J1 5 V in and J2 panel switch on the east edge, D1, C1, C2, test loops TP1 (+5V), TP7 (GND) |
| South-east | IR emitter: four current limiters side by side (R2.2k, BC337, BC547, R22 each), R29, C7, J3 IR LEDs at the south edge |

Connector pinout (every sensor connector the same way round): VA at the south end, then G, L, G, H at the north end. The scope header pins H, G, L sit level with connector pins H, G, L.

- **Under the NodeMCU:** R43, R44 and R45 are mounted flat there. Solder them before you fit the sockets.
- **Sensor supply:** VSENS runs down each edge outside the sensor connectors, with a stub to
  each VA pin; the two buses meet at TP2 beside U1.
- **Ground:** GND is poured on both sides. The IR receiver has its own AGND pour, joined to GND only at NT1, a wire link or 0 Ω resistor on the north edge of the receiver area. Only the receiver's supplies (VA_IR through R30, VCMP through R46) and IR_IN cross that edge; no other net runs over the AGND copper.
- **Tracks:** 0.4 mm for signals; 0.6 mm for +3V3, VSENS, VA_IR, 5V_F, VCMP and VU4; 0.8 mm
  for VIN, VSW and +5V. Clearance 0.2 mm, vias 0.8/0.4 mm (the net classes in the project).

## Notes for assembly

- **Scope headers:** J21–J28 are optional; fit one when you want to probe a channel.
- **470 Ω resistors:** a wire link can replace any of them.
- **Standing parts:** resistors stand upright on a 2.54 mm pitch to fit 8 channels on the board.
- **Sockets:** use DIP-8 sockets for U3 (MCP6002) and U4 (TLC372, or LM393 with JP1 at 5 V), so either can be swapped (see [Buying U3 and U4](#buying-u3-and-u4-in-the-netherlands)).
- **NodeMCU sockets:** fit the two 1x19 female headers last.

## Buying U3 and U4 in the Netherlands

The curtain receiver's two ICs are both socketed, so a part can be swapped and compared on
the bench. These are the DIP-8 parts found at consumer shops on 2026-11-23. Prices include
21 % VAT. Prices and stock change: check the shop's page before ordering. Okaphone's site
blocks automated visits, so its prices come from search results.

### U3: the op-amp (TIA and ×21 stage)

**Needs:** a dual in DIP-8 with the standard pinout, running from 3.3 V (VA_IR), with a
rail-to-rail output. The TIA's output sits below VMID_IR in steady light, and the ×21
stage's output rises toward 3.3 V on a shadow.

| Part | Shop | Price | Stock | GBW, slew | At 3.3 V |
|---|---|---|---|---|---|
| MCP6002-I/P (as designed) | Okaphone; Conrad (651489) | €1.20; €1.29 | in stock at both | 1 MHz, 0.6 V/µs | yes, rail-to-rail in and out |
| **MCP602-I/P** | Okaphone | €1.95 (€1.75 from 10) | in stock | 2.8 MHz, 2.3 V/µs | yes, rail-to-rail out; inputs up to Vdd − 1.2 V (2.1 V) |
| TLV272IP | Okaphone | €2.25 | to order | 3 MHz, 2.4 V/µs | yes, rail-to-rail out |
| TS912IN | Okaphone | €4.50 | – | 0.8 MHz, 1 V/µs | yes, but no faster than the MCP6002 |
| MCP6022-I/P | Conrad (651498) | about €5 | – | 10 MHz, 7 V/µs | yes, rail-to-rail in and out |
| TLV2462 | Bits & Parts | €5.95 | in stock | 6.4 MHz, 1.6 V/µs | yes, rail-to-rail in and out |
| TLC2272 | Okaphone | €6.45–7.65 | in stock | – | no: 4.4 V minimum |
| LM358 | Tinytronics €0.90; Bits & Parts €0.40 | | in stock | – | no: its output stops about 1.5 V below the top rail, and its inputs at 1.8 V |
| NE5534, TL07x, LM741 | Tinytronics, Bits & Parts | | | – | no: they need about ±5 V or more |

The speeds are datasheet values, not from the shops. What the speed does to the curtain's
delay (shadow to IR_IN rising):

- **MCP6002:** the ×21 stage has 48 kHz of bandwidth (τ 3.3 µs). The chain is about 3–8 µs,
  and the delay varies with how deep the shadow is.
- **MCP602:** about 130 kHz (τ 1.2 µs), and about 4× the slew. The chain is about 2–4 µs.
- **MCP6022:** about 480 kHz (τ 0.33 µs). The chain is about 1.5–2.5 µs; the TIA and the
  LM393 are then the slowest parts.

A constant delay is taken up by the curtain's window (`curtain window`); the scatter is what
the faster parts reduce. Each option keeps the TIA stable with C12 (47 pF): the slowest
needs at least about 28 pF across R34 (MCP602), the fastest about 15 pF.

**Choice:** the MCP6002 works. The MCP602 is the cheap upgrade, at €0.75 more: buy both and
compare the dip timing on the bench. One thing to check with the MCP602: its inputs only work
up to 2.1 V. VMID_IR (1.65 V) is well inside, but in steady light the TIA's output sits
below VMID_IR by the photocurrent × 22 k. A full shadow then lifts U3B's + input by that much.
If TP4 sits more than about 0.45 V below VMID_IR with the LEDs on (about 20 µA), a deep
shadow takes that input past 2.1 V. The stage is already saturated high by then, so the dip
should still register; the MCP6002 has no such limit.

### U4: the comparator

**Needs:** a dual with the LM393's pinout, in DIP-8 or on an adapter. Its inputs must work
up to the threshold (TP6, up to 2.35 V) at U4's supply, which JP1 sets to 3.3 V or 5 V (see
[The curtain comparator's supply](#2-the-curtain-comparators-supply-33-v-or-5-v-by-a-jumper)).

| Part | Shop | Price | Stock | Output, speed | Inputs up to, at 3.3 V | Use |
|---|---|---|---|---|---|---|
| **TLC372CP** | Okaphone | €1.95 | in stock | open drain, 200 ns (10 mV overdrive) | 2.3 V at 25 °C (Vdd − 1 V); 1.8 V guaranteed over 0–70 °C | **first choice, at 3.3 V** |
| LM393N | Conrad (155603); Okaphone; Bits & Parts | €0.88 (€0.73 ex VAT, maybe a minimum of 3); €1.25; €0.60 | in stock at all three | open collector, 0.3–1.3 µs | 1.8 V (Vcc − 1.5 V): too low | **at 5 V only** |
| TLV3202 + SOIC-8 adapter | chip: LCSC (C129325, the sensor board's part); adapter: Okaphone 8PINSOIC €1.15–1.45, Tinytronics SOP8TODIP, Kiwi (Adafruit 1212) | about €1 + the adapter + shipping | – | push-pull, 40 ns | 3.5 V (beyond both rails) | the best part, at 3.3 V only |
| TLC3702CP | Okaphone | €2.95 | in stock | push-pull, 1.1 µs | same CMOS family as the TLC372 (not checked) | works at 3.3 V, slower and dearer |
| TLC393CP | Okaphone | €1.50 | in stock | open drain, 2.5 µs (5 mV overdrive) | same CMOS family (not checked) | too slow |
| LM2903N, LM293P | Okaphone | €2.95; €1.45 | in stock | the LM393 for wider temperatures | 1.8 V | like the LM393: 5 V only |
| MCP6542-E/P | Conrad (1084994) | €0.76 | out of stock | push-pull, up to 8 µs | rail to rail | too slow |
| TLC372CP | Conrad (marketplace) | €10.65 ex VAT | – | as above | as above | too dear there |
| LT1011CN8 | Conrad (150071) | – | – | a single comparator, 4 mA | – | doesn't fit |
| MCP6562, LMV393 | – | – | – | – | – | not available in DIP here |

The TLC372's figures are from TI's datasheet (SLCS114F, 2025). Tinytronics has no comparator.

## GPIO map

| Function | GPIO |
|---|---|
| CH1 L/H | 39 / 36 |
| CH2 L/H | 35 / 34 |
| CH3 L/H | 33 / 32 |
| CH4 L/H | 26 / 25 |
| CH5 L/H | 22 / 23 |
| CH6 L/H | 19 / 21 |
| CH7 L/H | 17 / 18 |
| CH8 L/H | 4 / 16 |
| IR_IN | 13 |
| IR_EN | 27 |
| SDA_DQ: the BME280's SDA, or the DS18B20's DQ (J5 pin 2) | 14 |
| SCL: the BME280's SCL (J5 pin 4) | 15 |
| STAT (blue LED + panel LED) | 2 |
| BOOT (button) | 0 |
| Spare, left open | 5, 12 |

The firmware's `config.h` has this map (`CAPTURE_LINE_PINS`, L then H per channel). It also uses two of the open pins inside the chip, with nothing wired to them on the board:

- GPIO 5: the capture clock loopback;
- GPIO 12: the pulse generator's sync loopback (it was GPIO 15 before step 3a; GPIO 15 is
  the BME280's SCL now).

## Checks done

- **KiCad 10 check (2026-11-24, KiCad 10.0.8, `kicad-cli`), after the new layout:** ERC 0
  errors, 0 warnings. DRC with schematic parity, all severities: 0 violations, 0 unconnected
  pads, 0 footprint errors (no silkscreen warnings either). Every track is 0.4, 0.6 or 0.8 mm
  by its net class; 4 vias.
- **Gerbers and drill files** plot from the board (`kicad-cli pcb export gerbers` and
  `export drill`); they stay out of the repo, as above.
- **Routing method (2026-11-24):** the supply runs (VSENS, +5V, +3V3, VIN, VSW), IR_EN (kept
  off the AGND copper) and the ground links of J5, J6 and J7 were drawn by script and locked;
  the rest was routed by Freerouting 2.5 (no neck-down) and checked in KiCad. GND and AGND
  come from the pours.
