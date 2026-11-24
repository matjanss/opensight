#!/usr/bin/env python3
"""Exports both boards as STEP and STL into <board>/mechanical/, for enclosure design.

    <python with CadQuery 2.x> hardware/export_mechanical.py      (and kicad-cli, KiCad 10)

- Origin: each board's south-west corner, z = 0 at the board's underside; x east, y north.
- Flat: everything below the underside (the soldered leads) is cut off, so the board lies
  flat on the table at z = 0. How far the leads reach is in each mechanical/README.md.
- STEP keeps KiCad's colours, face by face; a part that was cut gets its faces' most
  common colour. STEP has the silkscreen too. The STL is binary.
"""

import os
import re
import struct
import subprocess
import sys
import tempfile
from collections import Counter

import cadquery as cq
from OCP.BRepAlgoAPI import BRepAlgoAPI_Cut
from OCP.BRepBuilderAPI import BRepBuilderAPI_Transform
from OCP.BRepPrimAPI import BRepPrimAPI_MakeBox
from OCP.gp import gp_Pnt
from OCP.IFSelect import IFSelect_RetDone
from OCP.Quantity import Quantity_Color, Quantity_TOC_RGB
from OCP.STEPCAFControl import STEPCAFControl_Reader, STEPCAFControl_Writer
from OCP.STEPControl import STEPControl_AsIs
from OCP.TCollection import TCollection_ExtendedString
from OCP.TDF import TDF_Label, TDF_LabelSequence
from OCP.TDocStd import TDocStd_Document
from OCP.TopAbs import TopAbs_FACE, TopAbs_SOLID
from OCP.TopExp import TopExp_Explorer
from OCP.TopLoc import TopLoc_Location
from OCP.XCAFDoc import XCAFDoc_ColorGen, XCAFDoc_ColorSurf, XCAFDoc_DocumentTool

HERE = os.path.dirname(os.path.abspath(__file__))
BOARDS = ["host-board/host_board", "acoustic-sensor/mic_board"]
COLOURS = (XCAFDoc_ColorSurf, XCAFDoc_ColorGen)


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


def kicad_step(pcb, origin, out):
    r = subprocess.run(["kicad-cli", "pcb", "export", "step", "-f", "--subst-models",
                        "--include-silkscreen", "--user-origin", origin, "-o", out, pcb],
                       capture_output=True, text=True)
    for line in (r.stdout + r.stderr).splitlines():
        if "Could not add" in line:
            print("   ", line)
    if r.returncode:
        sys.exit(f"kicad-cli failed on {pcb}")


def new_doc():
    return TDocStd_Document(TCollection_ExtendedString("MDTV-XCAF"))


def rgb(c):
    return (c.Red(), c.Green(), c.Blue())


def read_parts(path):
    """Every solid of a STEP file, placed, with its faces' colours (in explorer order)."""
    reader = STEPCAFControl_Reader()
    reader.SetColorMode(True)
    reader.SetNameMode(True)
    if reader.ReadFile(path) != IFSelect_RetDone:
        sys.exit(f"can't read {path}")
    doc = new_doc()
    reader.Transfer(doc)
    st = XCAFDoc_DocumentTool.ShapeTool_s(doc.Main())
    ct = XCAFDoc_DocumentTool.ColorTool_s(doc.Main())
    parts = []

    def label_colour(label):
        c = Quantity_Color()
        return next((rgb(c) for k in COLOURS if ct.GetColor_s(label, k, c)), None)

    def shape_colour(shape):
        c = Quantity_Color()
        return next((rgb(c) for k in COLOURS if ct.GetColor(shape, k, c)), None)

    def walk(label, loc, inherited):
        inherited = label_colour(label) or inherited
        if st.IsAssembly_s(label):
            comps = TDF_LabelSequence()
            st.GetComponents_s(label, comps, False)
            for i in range(1, comps.Length() + 1):
                comp, ref = comps.Value(i), TDF_Label()
                if st.GetReferredShape_s(comp, ref):
                    walk(ref, loc * st.GetLocation_s(comp), label_colour(comp) or inherited)
            return
        ex = TopExp_Explorer(st.GetShape_s(label), TopAbs_SOLID)
        while ex.More():
            solid = ex.Current()
            base = shape_colour(solid) or inherited
            faces = []
            fx = TopExp_Explorer(solid, TopAbs_FACE)
            while fx.More():
                faces.append(shape_colour(fx.Current()) or base)
                fx.Next()
            parts.append((solid.Moved(loc), base, faces))
            ex.Next()

    free = TDF_LabelSequence()
    st.GetFreeShapes(free)
    for i in range(1, free.Length() + 1):
        walk(free.Value(i), TopLoc_Location(), None)
    return parts


def flatten(parts):
    """Cuts off everything below z = 0. A part that was cut keeps one colour."""
    below = BRepPrimAPI_MakeBox(gp_Pnt(-1000, -1000, -1000), 2000, 2000, 1000).Shape()
    out = []
    for shape, base, faces in parts:
        bb = cq.Shape.cast(shape).BoundingBox()
        if bb.zmin >= -1e-4:
            out.append((shape, base, faces))
            continue
        if bb.zmax <= 1e-4:
            continue                      # all of it under the board
        cut = BRepAlgoAPI_Cut(shape, below)
        cut.Build()
        common = Counter(f for f in faces if f).most_common(1)
        colour = common[0][0] if common else base
        ex = TopExp_Explorer(cut.Shape(), TopAbs_SOLID)
        while ex.More():
            out.append((ex.Current(), colour, None))
            ex.Next()
    return out


def write_step(parts, path):
    doc = new_doc()
    st = XCAFDoc_DocumentTool.ShapeTool_s(doc.Main())
    ct = XCAFDoc_DocumentTool.ColorTool_s(doc.Main())

    def colour(c):
        return Quantity_Color(c[0], c[1], c[2], Quantity_TOC_RGB)

    for shape, base, faces in parts:
        # Bake the placement into the geometry: XCAF stores a placed shape as a reference,
        # and the face colours wouldn't find their faces. The copy keeps the face order.
        trsf = shape.Location().Transformation()
        shape = BRepBuilderAPI_Transform(shape.Located(TopLoc_Location()), trsf, True).Shape()
        label = st.AddShape(shape, False)
        if base:
            ct.SetColor(label, colour(base), XCAFDoc_ColorSurf)
        if faces:
            fx, i = TopExp_Explorer(shape, TopAbs_FACE), 0
            while fx.More():
                if faces[i] and faces[i] != base:
                    sub = st.AddSubShape(label, fx.Current())
                    if not sub.IsNull():
                        ct.SetColor(sub, colour(faces[i]), XCAFDoc_ColorSurf)
                fx.Next()
                i += 1
    writer = STEPCAFControl_Writer()
    writer.SetColorMode(True)
    writer.Transfer(doc, STEPControl_AsIs)
    if writer.Write(path) != IFSelect_RetDone:
        sys.exit(f"can't write {path}")


def write_stl(parts, path):
    whole = cq.Compound.makeCompound([cq.Shape.cast(s) for s, _, _ in parts])
    cq.exporters.export(whole, path, tolerance=0.05, angularTolerance=0.3)
    # The check on the mesh itself (a B-rep bounding box carries some slack).
    d = open(path, "rb").read()
    n = struct.unpack("<I", d[80:84])[0]
    lo, hi = [1e9] * 3, [-1e9] * 3
    for i in range(n):
        v = struct.unpack("<9f", d[96 + i * 50:132 + i * 50])
        for j in range(9):
            lo[j % 3] = min(lo[j % 3], v[j])
            hi[j % 3] = max(hi[j % 3], v[j])
    return n, lo, hi


for b in BOARDS:
    pcb = os.path.join(HERE, b + ".kicad_pcb")
    out = os.path.join(HERE, os.path.dirname(b), "mechanical")
    name = os.path.basename(b)
    os.makedirs(out, exist_ok=True)
    x0, x1, y0, y1 = edge_bbox(pcb)
    origin = f"{x0}x{y1}mm"   # the south-west corner (KiCad's page y points down)
    print(f"{name}: {x1 - x0:g} x {y1 - y0:g} mm, origin {origin}")
    with tempfile.TemporaryDirectory() as tmp:
        raw = os.path.join(tmp, name + ".step")
        kicad_step(pcb, origin, raw)
        parts = read_parts(raw)
    lowest = min(cq.Shape.cast(s).BoundingBox().zmin for s, _, _ in parts)
    flat = flatten(parts)
    write_step(flat, os.path.join(out, name + ".step"))
    n, lo, hi = write_stl(flat, os.path.join(out, name + ".stl"))
    print(f"   {len(parts)} solids, leads to {lowest:.2f} mm cut off; {n} triangles, "
          + ", ".join(f"{'xyz'[k]} {lo[k]:.3f}..{hi[k]:.3f}" for k in range(3)))
    if lo[2] < -1e-4:
        sys.exit(f"{name}: something is still below z = 0")
