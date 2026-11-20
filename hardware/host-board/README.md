# Host board v0.3: schematic and board

The host board for the acoustic target: a NodeMCU-32S, 8 sensor connectors, the IR curtain emitter and receiver, a DS18B20, and the panel LED and button. Every part is through-hole and hand-soldered. The board is 76 × 118 mm (the long side runs north–south), 2 layers, 1.6 mm thick, with an M3 mounting hole in each corner.

The files are in KiCad 10 format; open them with KiCad 10 or newer. "Update PCB from Schematic" should report no changes.

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

## Layout

North is the antenna end of the NodeMCU.

| Where | What |
|---|---|
| Middle, north half | The NodeMCU-32S on sockets, antenna at the north edge, USB plug pointing south (the area south of it is kept free for the plug) |
| West edge | Sensors CH1–CH4 (J11–J14), north to south. Beside each one: scope header J2n and the four channel resistors |
| East edge | Sensors CH5–CH8 (J15–J18), mirrored |
| Middle, south of the module | West: LP2950 sensor regulator and J5 DS18B20. East: J6 panel LED and J7 panel button |
| South-west (outlined) | IR receiver on its own AGND copper. J4 photodiodes at the south edge, RV1 = IR threshold. Test loops TP3 AGND, TP4 TIA, TP5 AMP, TP6 VTH |
| South middle | IR emitter: 4 current-limiter cells, J3 IR LEDs at the south edge |
| South-east | Power: J1 5 V in and J2 panel switch on the east edge, D1, C1, C2, test loops TP1 (+5V), TP2 (VSENS), TP7 (GND) |

Connector pinout (every sensor connector the same way round): VA at the south end, then G, L, G, H at the north end. The scope header pins H, G, L sit level with connector pins H, G, L.

- **Under the NodeMCU:** R43 and R44 are mounted flat there. Solder them before you fit the sockets.
- **Ground:** GND is poured on both sides. The IR receiver has its own AGND pour, joined to GND only at NT1, a wire link or 0 Ω resistor on the north edge of the receiver area.

## Notes for assembly

- **Scope headers:** J21–J28 are optional; fit one when you want to probe a channel.
- **470 Ω resistors:** a wire link can replace any of them.
- **Standing parts:** resistors stand upright on a 2.54 mm pitch to fit 8 channels on the board.
- **Sockets:** use DIP-8 sockets for U3 (MCP6002) and U4 (LM393).
- **NodeMCU sockets:** fit the two 1x19 female headers last.

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
| DQ (DS18B20) | 14 |
| STAT (blue LED + panel LED) | 2 |
| BOOT (button) | 0 |
| Spare, left open | 5, 12, 15 |

The firmware's `config.h` still has the v0.2 map: `CAPTURE_LINE_PINS` becomes `39, 36, 35, 34, 33, 32, 26, 25, 22, 23, 19, 21, 17, 18, 4, 16` (L, H per channel) and `IR_EN_PIN` becomes 27. GPIO5 stays free for the capture clock loopback.

## Checks done

- **Netlist:** the schematic netlist matches the design list: 71 nets, 119 parts.
- **Design-rule check (DRC):** 0 copper errors and 0 unconnected pads. GND and AGND connectivity was checked separately. What remains are silkscreen overlap warnings and library-path notes from the build environment.
- **Routing rules:** signal tracks 0.3 mm, power tracks 0.4–0.6 mm, 0.25 mm clearance, vias 0.7/0.35 mm. All of these are well inside JLCPCB's standard limits.
- **Routing method:** the board was routed with my own autorouter, not by hand, so some tracks take long ways round. It's electrically fine, but tidy it in KiCad if you like.
