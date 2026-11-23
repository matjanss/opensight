# Host board: to do

Changes to make in the design before the v0.3 board is ordered. The details are in
[README.md](README.md#changes-before-ordering-done). All done (2026-11-24): the board is
ready to order.

## Before ordering

- [x] **A BME280 on J5, in the schematic** (2026-11-23): J5 a 4-pin JST XH (GND, SDA/DQ on
  GPIO 14, VDD, SCL on GPIO 15), plus R45 (4.7 k SCL pull-up). See
  [A BME280 on J5](README.md#1-a-bme280-on-j5).
- [x] **J5 and R45 on the board** (2026-11-24): J5's 4-pin footprint beside J6 and J7 (pin 1,
  GND, at its east end), R45 under the NodeMCU between R43 and R44, the SCL track to GPIO 15;
  silkscreen "BME280 / DS18B20" with G, SDA, 3V3, SCL.
- [x] **The curtain comparator's supply jumper, in the schematic** (2026-11-23): JP1 (1x3
  header and shunt) feeds U4 pin 8 (net VU4, with C17) from VA_IR (3.3 V, for a TLC372CP)
  or a new net `VCMP` (5V_F through R46 10 Ω, with C18 10 µF; for an LM393). The LM393's
  inputs only work up to 1.8 V at 3.3 V, below most of RV1's threshold range; the TLC372's
  reach about 2.3 V. See
  [The curtain comparator's supply](README.md#2-the-curtain-comparators-supply-33-v-or-5-v-by-a-jumper).
- [x] **The supply jumper on the board** (2026-11-24): JP1 north of U4 with C18, R46 beside
  NT1, VU4 and VCMP routed; silkscreen "U4 SUPPLY", "3V3" and "5V" at JP1's ends (the parts
  for each are in the README).
- [x] **Afterwards** (2026-11-24): the south half laid out again (76 × 132 mm), the board
  routed again with 0.4/0.6/0.8 mm tracks; the schematic PDF and the assembly PDF made again
  (the parts list had the new parts already). DRC and ERC clean.

## On the bench, once it's built

- [ ] The curtain's delay and window: time real dips against shots and set `curtain
  window` from them (the receiver adds about 3–8 µs; slow air pistol pellets are at the
  window's 100 µs edge already). MEASUREMENT step 4b, PLAN phase 1.6.
- [ ] RV1's threshold, with the TLC372 at 3.3 V: turn it up slowly while dropping pellets
  through the curtain; note the TP6 voltage at which dips stop being seen, and set it well
  below that, above the noise. If they stop near 2.1–2.3 V rather than at the signal's peak,
  that's the TLC372's input range: fit an LM393 and set JP1 to 5 V. Never a push-pull part
  (TLV3202) at 5 V.
- [ ] U3: the MCP6002 against the MCP602 (both socketed): the dip timing's scatter with
  each, and TP4's level with the LEDs on (the MCP602's inputs stop at 2.1 V). See
  [Buying U3 and U4](README.md#buying-u3-and-u4-in-the-netherlands).
- [ ] MEASUREMENT step 1 tests 3 and 14 (the v0.3 pin map).
