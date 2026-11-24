# Host board v0.3: 3D files for the enclosure

- `host_board.step`: the board with its parts and silkscreen, for CAD.
- `host_board.stl`: the same as a binary mesh, for a quick look or a slicer.

Both are flat (below).

Made by `hardware/export_mechanical.py`: kicad-cli (KiCad 10) exports the board, then
CadQuery cuts off everything below the board. Run it with a Python that has CadQuery 2.x
(`pip install cadquery`, in a venv) after any change to the board.

**Flat:** the soldered leads under the board are cut off, so the board lies flat on the
table, its underside at z = 0 in the x/y plane. The real leads reach up to 4.4 mm below the
underside until they are trimmed: allow for that under the board.

## Frame

- Origin: the board's south-west corner, at the board's underside.
- x points east, y north, z up. The units are mm.
- The board: 76 × 132 × 1.6 mm, its top at z = 1.6.
- North is the NodeMCU's antenna end.

## Sizes for the case

| What | Where |
|---|---|
| M3 holes (3.2 mm) | (4, 4), (72, 4), (4, 128), (72, 128) |
| Tallest part | the NodeMCU's ESP32 shield, 15.7 mm above the board's top (z 17.3) |
| Under the board | cut off in the files; the real leads reach up to 4.4 mm below the underside (the socket pins, before trimming) |
| Parts above the board | x 1.4–75.0, y 1.1–130.2 |
| NodeMCU USB socket | x 34.3–41.8, y 77.4–82.4, z 14.2–16.8; the plug goes in from the south (the area south of it is kept free) |
| PWR IN (J1), SWITCH (J2) | JST XH 2-pin at the east edge: x 71.6, y 51.0 and y 42.0 |
| IR LEDs (J3), photodiodes (J4) | JST XH at the south edge: (48.0, 4.5) and (15.5, 4.5) |

## The NodeMCU-32S model

KiCad has no model for the NodeMCU-32S, so `../host_board.3dshapes/NodeMCU-32S_on_sockets.step`
stands in for it. U2's footprint uses it. `make_nodemcu.py` (CadQuery 2.x) makes it again.

- **The stack:** KiCad's 1×19 sockets (8.5 mm), then the module's header plastic (2.5 mm),
  then the module's board (1.6 mm). So the module's underside is at 11.0 mm above the host
  board's top.
- **The parts on the module are boxes:**
  - the WROOM-32: 18 × 25.5 × 3.1 mm;
  - the micro-USB socket, 0.6 mm over the module's south edge;
  - the two buttons;
  - the regulator and the USB-serial chip.
- **The module's outline** (25.5 × 52.2 mm) is the footprint's.
- **Check these against the real module with calipers:** the heights and the position of
  the USB socket.

## Missing from the export

- The seven Keystone 5000 test loops (TP1–TP7). KiCad 10's library has no model for them.
  Allow for their height from Keystone's datasheet.
- The cables to J1–J18, and the parts on other boards (the curtain's LEDs and photodiodes).
