#!/usr/bin/env python3
"""Exports both boards as STEP and STL into <board>/mechanical/, for enclosure design.

    python3 hardware/export_mechanical.py      (needs kicad-cli, KiCad 10)

The origin is each board's south-west corner, z = 0 at the board's underside, x east,
y north. STEP has the silkscreen (to see which way round a board is); the STL is binary
(KiCad writes ASCII, about 5 times larger).
"""

import os
import re
import struct
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
BOARDS = ["host-board/host_board", "acoustic-sensor/mic_board"]


def edge_bbox(pcb):
    """The board outline's bounding box (KiCad page coordinates, mm)."""
    xs, ys = [], []
    # The board's top-level graphic items each start on a line of their own.
    for item in re.split(r'\n\t(?=\()', open(pcb).read()):
        if item.startswith(("(gr_line", "(gr_rect", "(gr_arc")) and '"Edge.Cuts"' in item:
            for x, y in re.findall(r'\((?:start|mid|end) ([-\d.]+) ([-\d.]+)\)', item):
                xs.append(float(x))
                ys.append(float(y))
    return min(xs), max(xs), min(ys), max(ys)


def ascii_to_binary(src, dst):
    num = r'([-+\d.eE]+)'
    v3 = r'\s+' + num + r'\s+' + num + r'\s+' + num
    pat = re.compile(r'facet normal' + v3 + r'.*?vertex' + v3 + r'\s+vertex' + v3 +
                     r'\s+vertex' + v3, re.S)
    facets = pat.findall(open(src).read())
    lo, hi = [1e9] * 3, [-1e9] * 3
    with open(dst, "wb") as f:
        f.write(b"binary STL, from KiCad".ljust(80, b" "))
        f.write(struct.pack("<I", len(facets)))
        for t in facets:
            v = list(map(float, t))
            f.write(struct.pack("<12fH", *v, 0))
            for j in range(3, 12):
                lo[j % 3] = min(lo[j % 3], v[j])
                hi[j % 3] = max(hi[j % 3], v[j])
    return lo, hi


def run(*args):
    r = subprocess.run(["kicad-cli", *args], capture_output=True, text=True)
    for line in (r.stdout + r.stderr).splitlines():
        if "Could not add" in line or "not found" in line.lower():
            print("   ", line)
    if r.returncode:
        sys.exit(f"kicad-cli failed: {' '.join(args)}")


for b in BOARDS:
    pcb = os.path.join(HERE, b + ".kicad_pcb")
    out = os.path.join(HERE, os.path.dirname(b), "mechanical")
    name = os.path.basename(b)
    os.makedirs(out, exist_ok=True)
    x0, x1, y0, y1 = edge_bbox(pcb)
    origin = f"{x0}x{y1}mm"   # the south-west corner (KiCad's page y points down)
    print(f"{name}: {x1 - x0:g} x {y1 - y0:g} mm, origin {origin}")
    run("pcb", "export", "step", "-f", "--subst-models", "--include-silkscreen",
        "--user-origin", origin, "-o", os.path.join(out, name + ".step"), pcb)
    with tempfile.TemporaryDirectory() as tmp:
        ascii_stl = os.path.join(tmp, name + ".stl")
        run("pcb", "export", "stl", "-f", "--subst-models", "--user-origin", origin,
            "-o", ascii_stl, pcb)
        lo, hi = ascii_to_binary(ascii_stl, os.path.join(out, name + ".stl"))
    print("   extent: " + ", ".join(f"{'xyz'[k]} {lo[k]:.2f}..{hi[k]:.2f}" for k in range(3)))
