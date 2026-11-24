"""The NodeMCU-32S on its two 1x19 sockets, as a 3D model for the host board's U2.

Run with CadQuery 2.x:  python3 make_nodemcu.py  (writes NodeMCU-32S_on_sockets.step).

Frame: the footprint's origin (pad 1), x east, y north (KiCad's 3D y = minus the
footprint's y), z up from the host board's top. Boxes only, for enclosure design: the
heights are what count. The module's outline is the footprint's (25.5 x 52.2 mm); the
parts on it are approximate. Check with calipers on the real module.
"""

import os
import cadquery as cq

HERE = os.path.dirname(os.path.abspath(__file__))
KICAD_3D = os.environ.get("KICAD10_3DMODEL_DIR", "/usr/share/kicad/3dmodels")
SOCKET = f"{KICAD_3D}/Connector_PinSocket_2.54mm.3dshapes/PinSocket_1x19_P2.54mm_Vertical.step"

ROW = 22.86          # mm between the rows
PITCH = 2.54
SOCKET_H = 8.5       # female header body
SPACER_H = 2.5       # the module's male header plastic
PCB_Z = SOCKET_H + SPACER_H   # the module's underside: 11.0 mm
PCB_T = 1.6
TOP = PCB_Z + PCB_T           # the module's top: 12.6 mm


def box(fx0, fx1, fy0, fy1, z0, z1):
    """A box from footprint coordinates (y down) to the 3D frame (y up)."""
    return (cq.Workplane("XY")
            .box(fx1 - fx0, fy1 - fy0, z1 - z0, centered=False)
            .translate((fx0, -fy1, z0)))


asm = cq.Assembly(name="NodeMCU-32S_on_sockets")
black = cq.Color(0.08, 0.08, 0.08)
silver = cq.Color(0.75, 0.75, 0.78)
gold = cq.Color(0.85, 0.7, 0.3)

# The two sockets: KiCad's own model, pin 1 at its origin.
sock = cq.importers.importStep(SOCKET)
asm.add(sock, name="socket_west", color=black)
asm.add(sock, name="socket_east", loc=cq.Location((ROW, 0, 0)), color=black)

# The module's male headers: the plastic between the socket and the module.
for name, x in (("header_west", 0.0), ("header_east", ROW)):
    asm.add(box(x - 1.27, x + 1.27, -1.27, 18 * PITCH + 1.27, SOCKET_H, PCB_Z),
            name=name, color=black)

# The module's board: the footprint's outline, antenna end north (fy < 0), USB south.
asm.add(box(-1.32, 24.18, -3.2, 49.0, PCB_Z, TOP), name="pcb", color=cq.Color(0.1, 0.1, 0.12))

# ESP32-WROOM-32 (18 x 25.5 x 3.1 mm) at the antenna end, its shield over the pin end.
cx = ROW / 2
asm.add(box(cx - 9.0, cx + 9.0, -3.2, 22.3, TOP, TOP + 0.8), name="wroom_pcb",
        color=cq.Color(0.1, 0.25, 0.1))
asm.add(box(cx - 7.9, cx + 7.9, 4.2, 21.8, TOP + 0.8, TOP + 3.1), name="wroom_shield",
        color=silver)

# Micro-USB, 0.6 mm over the south edge; EN and BOOT buttons beside it.
asm.add(box(cx - 3.75, cx + 3.75, 44.6, 49.6, TOP, TOP + 2.6), name="usb", color=silver)
for name, x in (("button_en", 3.0), ("button_boot", 19.86)):
    asm.add(box(x - 2.0, x + 2.0, 43.5, 47.0, TOP, TOP + 2.0), name=name, color=silver)

# The regulator (SOT-223) and the USB-serial chip, roughly where they sit.
asm.add(box(cx - 3.3, cx + 3.3, 33.0, 39.5, TOP, TOP + 1.8), name="regulator", color=black)
asm.add(box(cx - 2.5, cx + 2.5, 26.0, 31.0, TOP, TOP + 0.9), name="usb_serial", color=black)

# The header pins' tails above the module.
for i, x in enumerate((0.0, ROW)):
    for n in range(19):
        y = n * PITCH
        asm.add(box(x - 0.32, x + 0.32, y - 0.32, y + 0.32, TOP, TOP + 2.0),
                name=f"pin_{i}_{n}", color=gold)

out = os.path.join(HERE, "NodeMCU-32S_on_sockets.step")
asm.export(out)
print("wrote", out)
