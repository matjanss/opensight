# Sensor board v0.3: 3D files for the holders

- `mic_board.step`: the board with its parts and silkscreen, for CAD.
- `mic_board.stl`: the same as a binary mesh.

Made by `python3 hardware/export_mechanical.py` (kicad-cli, KiCad 10). Run it again after
any change to the board.

## Frame

- Origin: the board's south-west corner, at the board's underside (the bottom layer).
- x points east, y north, z up. The units are mm.
- The board: 44 × 27 × 1.0 mm, its top at z = 1.0.
- South is the paper side: J1 and the mic are near it.

## Sizes for the holder

| What | Where |
|---|---|
| The mic's sound port | (22.0, 5.5): the 0.5 mm hole through the board, opening on the underside. Keep the underside open around it |
| Port to the edges | 5.5 mm from the south edge, 21.5 mm from the north edge (the board flipped) |
| Tallest parts | the trimpots and J1, 11.0 mm above the board's top (z 12.0) |
| Under the board | soldered leads up to 5.0 mm below the underside (before trimming), over x 7.4–35.9, y 4.4–24.3 |
| Free of parts on top | x 0–5.3 (west end) and x 37.9–44 (east end): room for a clamp |
| Trimpots RV2, RV3, RV1 | (13.0, 24.0), (24.3, 24.0), (35.6, 24.0), at the north edge, adjusted from the top: keep them reachable |
| J1 (JST XH 5-pin) | (7.75, 4.75), the cable leaves upwards |
| Mounting holes | none: the board is clamped |

## Missing from the export

- The two Keystone 5000 test loops (TP5, TP6, GND). KiCad 10's library has no model for
  them.
- The cable on J1.

## Notes

- The outline's north edge is 0.1 mm out of square: it runs from y = 27.0 at the west end
  to y = 26.9 at the east end (page coordinates (103, 100) to (147, 100.1)).
- The board README still gives 50 × 32 mm, 7.5 mm clamp strips and the port 10.5 mm from
  the paper. The board itself is 44 × 27 mm, with the port 5.5 mm from its south edge.
  The holder sets the real mic height (PLAN D3).
