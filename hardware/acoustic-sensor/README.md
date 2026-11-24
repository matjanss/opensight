# Mic sensor board v0.3 (one acoustic channel)

GTA2718B-3 bottom-port MEMS mic → TLV9062 two-stage preamp (×6.3–17.7, ×12 at RV1 mid-travel, three 16 kHz high-pass poles) → TLV3202 dual comparator (VTH_L / VTH_H) → J1 to the host board.

## Files
- `mic_board.kicad_pro / .kicad_sch / .kicad_pcb`: KiCad project, KiCad 10 format (open it with KiCad 10 or newer).
- `mic_board.kicad_sym`, `mic_board.pretty/`: project-local symbols and the mic footprint (already in the project's sym-lib-table / fp-lib-table). All other footprints come from the standard KiCad libraries.
- `mic_board_schematic.pdf`: schematic.
- `mechanical/`: the board as STEP and STL for the holders, with the sizes that matter (see its README).
- `mic_board_parts_list.csv`: all parts incl. hand-fitted ones, with quantities for 5 boards.
- `production/`: the JLCPCB files from the Fabrication Toolkit plugin (options in `fabrication-toolkit-options.json`): `mic_board.zip` (gerbers + drill files), `bom.csv`, `positions.csv`, `designators.csv`, `netlist.ipc`. The BOM and positions also list the hand-fitted parts (J1, RV1–RV3, test points): leave those out of the JLC assembly order.

## Board
- 44 × 27 mm, 2 layers, **1.0 mm** FR-4 (set this when ordering), bottom layer = GND plane, top has a GND pour. No mounting holes: the board is clamped.
- No parts on top in the end strips (5.3 mm at the west end, 6.1 mm at the east end): room for the clamps.
- The board stands on its long edge. Bottom edge = paper side: mic port 5.5 mm from it. Flipped over: 21.5 mm. The mic height h (PLAN D3) is that plus however far the holder keeps the board's edge from the paper. The port is centred on the length (x = 22 mm), so it stays at the same place along the frame when the board is flipped.
- The 3D files and the sizes for the holders are in [mechanical/](mechanical/README.md).
- Sound enters through a 0.5 mm non-plated hole under the mic (bottom side, marked "MIC PORT"); the bottom plane and all signal tracks stay clear of it.
- J1: JST XH 5-pin vertical, **2.50 mm pitch**: 1 VA, 2 GND, 3 OUT_L, 4 GND, 5 OUT_H (labels on the bottom silkscreen).
- Test loops: SIG, VMID, GND (Keystone 5000-series, hand-fit). Test pads: VTH_L next to C5, VTH_H next to C6.
- Trimpots (Bourns 3296W, hand-fit): RV2 = low threshold, RV3 = high threshold, RV1 = gain.

## Checks done
- Schematic (wired, in blocks A–E like the project doc) = intended circuit (26 nets, 41 parts); PCB pads = schematic nets (96 pads), footprints linked to their symbols.
- DRC (KiCad 7 engine, JLC-friendly rules: 0.2 mm clearance, 0.25/0.4 mm tracks, 0.6/0.3 mm vias): 0 errors, 0 unconnected; the only warnings are "library not in this configuration" from the generating machine.

## Check before ordering
1. **Mic footprint: wrong, to fix before ordering** (checked 2026-11-23 against the
   GTA2718B-3 datasheet from LCSC, C50275772: Fig. 11 and "Recommended PCB and Stencil Design
   Pattern", p. 6 and 11). The GND ring (Ø0.95/1.45 mm) and the 0.5 mm hole are right
   (0.5–0.7 mm recommended). The signal pads are not:
   - **Pads 1 and 2 are mirrored, so VDD and OUTPUT are swapped.** The datasheet draws the
     pads in its bottom view (pin 1 left, pin 2 right, the port below); seen from the top, as
     a footprint is drawn, pin 1 is on the right, where the top view's polarity dot is. With
     the pads at +x of the port, as in this footprint, pin 1 belongs at **+y**; the footprint
     has pad 1 at −y. Check it in JLC's assembly preview too (item 2): MK1's pin-1 mark must
     sit on the VDD pad (the one to R1 and C1).
   - **Position and size:** the datasheet's pads are 0.65 × 0.545 mm (across × along the
     part), 0.95 mm apart (centres), their inner edge 1.205 mm from the port centre, so
     their centres at x = 1.4775, y = ±0.475 mm. The footprint has 0.65 × 0.65 mm pads at x
     = 1.6, y = ±0.6025 (1.205 mm apart): 0.12–0.13 mm out in each direction, about 70 % of
     each part pad on its land. The old note above read "1.6" (the pads' total span) and
     "1.205" (the port to the pads' edge) as other dimensions.
   - **Fixed 2026-11-23** in the footprint (`mic_board.pretty`) and its copy on the board:
     pad 1 at (1.4775, +0.475), pad 2 at (1.4775, −0.475), both 0.545 × 0.65 mm; the
     silkscreen dot at (2.25, +0.475), beside pad 1. The 3D model
     (`mic_board.3dshapes`) had the same pads and a GND ring of r 0.53–0.67 mm: now the
     datasheet's pads and r 0.475–0.725 mm. Its pin-1 dot (a recess in the lid) was at the
     old pad 1's corner (KiCad's 3D y points up, the footprint's down): moved to pad 1's,
     checked in KiCad's 3D render.
   - **VDD and OUT at MK1 re-routed** (the user, 2026-11-23) onto the moved pads. DRC
     (KiCad 10, `kicad-cli`, 2026-11-24): 0 errors, 0 unconnected pads, the 8 silkscreen
     warnings as before. The production files made again from it.
2. **JLC preview**: confirm the rotation of U1, U2 (pin 1 dot) and MK1 (pads towards U1, port towards J1). The CPL gives MK1 at its package centre (0.5 mm from the port).
3. **Routing**: a scripted first pass. Worth a look in KiCad, mainly around the mic/input (C2, R2, U1 pin 3) and the comparator outputs.
4. **ERC/DRC in KiCad 10** (2026-11-23, KiCad 10.0.8, `kicad-cli`): ERC 0 violations;
   schematic parity 0 issues. DRC: 2 clearance errors between the U1A + input track and the
   GND zone, which go away when the zones are refilled (a stale fill), and 8 silkscreen
   warnings. **Refill the zones (B) and save before making the Gerbers:** they carry the
   fill as saved.
