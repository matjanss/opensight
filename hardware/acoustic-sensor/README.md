# Mic sensor board v0.2 (one acoustic channel)

GTA2718B-3 bottom-port MEMS mic → TLV9062 two-stage preamp (×6.3–17.7, ×12 at RV1 mid-travel, three 16 kHz high-pass poles) → TLV3202 dual comparator (VTH_L / VTH_H) → J1 to the host board.

## Files
- `mic_board.kicad_pro / .kicad_sch / .kicad_pcb`: KiCad project, KiCad 10 format (open it with KiCad 10 or newer).
- `mic_board.kicad_sym`, `mic_board.pretty/`: project-local symbols and the mic footprint (already in the project's sym-lib-table / fp-lib-table). All other footprints come from the standard KiCad libraries.
- `mic_board_schematic.pdf`: schematic.
- `mic_board_parts_list.csv`: all parts incl. hand-fitted ones, with quantities for 5 boards.
- `fab/mic_board_gerbers_JLCPCB.zip`: gerbers + drill files (PTH and NPTH).
- `fab/mic_board_BOM_JLCPCB.csv`, `fab/mic_board_CPL_JLCPCB.csv`: JLCPCB Economic PCBA BOM and placement (SMD parts only).

## Board
- 50 × 32 mm, 2 layers, **1.0 mm** FR-4 (set this when ordering), bottom layer = GND plane, top has a GND pour.
- 7.5 mm clamp strips at both short ends: no parts, no tracks.
- The board stands on its long edge. Bottom edge = paper side: mic port 10.5 mm from it. Flipped over: 21.5 mm. The port is centred on the length (x = 25 mm), so it stays at the same place along the frame when the board is flipped.
- Sound enters through a 0.5 mm non-plated hole under the mic (bottom side, marked "MIC PORT"); the bottom plane and all signal tracks stay clear of it.
- J1: JST XH 5-pin vertical, **2.50 mm pitch**: 1 VA, 2 GND, 3 OUT_L, 4 GND, 5 OUT_H (labels on the bottom silkscreen).
- Test loops: SIG, VMID, GND (Keystone 5000-series, hand-fit). Test pads: VTH_L next to C5, VTH_H next to C6.
- Trimpots (Bourns 3296W, hand-fit): RV2 = low threshold, RV3 = high threshold, RV1 = gain.

## Checks done
- Schematic (wired, in blocks A–E like the project doc) = intended circuit (26 nets, 41 parts); PCB pads = schematic nets (96 pads), footprints linked to their symbols.
- DRC (KiCad 7 engine, JLC-friendly rules: 0.2 mm clearance, 0.25/0.4 mm tracks, 0.6/0.3 mm vias): 0 errors, 0 unconnected; the only warnings are "library not in this configuration" from the generating machine.

## Check before ordering
1. **Mic footprint**: the land pattern is interpreted from the GTA2718B-3HA datasheet text (GND ring Ø1.45/0.95 mm around the port, pads 1 = VDD and 2 = OUTPUT 1.6 mm from the port centre, 1.205 mm apart). Compare pad 1/2 positions with the -3 drawing.
2. **JLC preview**: confirm the rotation of U1, U2 (pin 1 dot) and MK1 (pads towards U1, port towards J1). The CPL gives MK1 at its package centre (0.5 mm from the port).
3. **Routing**: a scripted first pass. Worth a look in KiCad, mainly around the mic/input (C2, R2, U1 pin 3) and the comparator outputs.
4. After opening in KiCad 10: run ERC/DRC and "Update PCB from schematic" once to confirm the link.
