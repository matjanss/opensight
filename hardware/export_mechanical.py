#!/usr/bin/env python3
"""Exports both boards as STEP and STL into <board>/mechanical/, for enclosure design.

    <python with CadQuery 2.x> hardware/export_mechanical.py      (and kicad-cli, KiCad 10)

- Origin: each board's south-west corner, z = 0 at the board's underside; x east, y north.
- Flat: everything below the underside (the soldered leads) is cut off, so the board lies
  flat on the table at z = 0. How far the leads reach is in each mechanical/README.md.
- STEP keeps KiCad's colours face by face, through the cuts too. STEP has the silkscreen
  too. The STL is binary.
- The host board also gets a simplified model (host_board_simplified.*): a bare plate
  (no holes or vias), the NodeMCU's two sockets (not the module), the DIP sockets, the
  trimpot, the connectors without their pins, and the electrolytic capacitors.
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
from OCP.BRepGProp import BRepGProp
from OCP.GProp import GProp_GProps
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
from OCP.TopTools import TopTools_ListOfShape
from OCP.XCAFDoc import XCAFDoc_ColorGen, XCAFDoc_ColorSurf, XCAFDoc_DocumentTool

HERE = os.path.dirname(os.path.abspath(__file__))
BOARDS = ["host-board/host_board", "acoustic-sensor/mic_board"]
COLOURS = (XCAFDoc_ColorSurf, XCAFDoc_ColorGen)
KICAD_3D = os.environ.get("KICAD10_3DMODEL_DIR", "/usr/share/kicad/3dmodels")
BOARD_COLOUR = (0.08, 0.2, 0.14)
# The simplified host board: footprints kept, by library. Connectors lose their pins.
SIMPLE_KEEP = ("Package_DIP:DIP-8_W7.62mm_Socket", "Potentiometer_THT:",
               "Capacitor_THT:CP_Radial")
SIMPLE_NO_PINS = ("Connector_JST:", "Connector_PinHeader_2.54mm:")
PIN_HOLE = 0.75     # mm square cut at each pin (the pins are 0.64 mm square)


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


def kicad_step(pcb, origin, out, *extra):
    r = subprocess.run(["kicad-cli", "pcb", "export", "step", "-f", "--subst-models",
                        "--user-origin", origin, "-o", out, *extra, pcb],
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


def faces_of(shape):
    out, fx = [], TopExp_Explorer(shape, TopAbs_FACE)
    while fx.More():
        out.append(fx.Current())
        fx.Next()
    return out


def solids_of(shape):
    out, ex = [], TopExp_Explorer(shape, TopAbs_SOLID)
    while ex.More():
        out.append(ex.Current())
        ex.Next()
    return out


def area(face):
    props = GProp_GProps()
    BRepGProp.SurfaceProperties_s(face, props)
    return props.Mass()


def box(x0, y0, z0, x1, y1, z1):
    return BRepPrimAPI_MakeBox(gp_Pnt(x0, y0, z0), x1 - x0, y1 - y0, z1 - z0).Shape()


BELOW = box(-1000, -1000, -1000, 1000, 1000, 0)


def cut(part, tools):
    """Cuts the tools out of a part. Every face keeps its colour through the cut; the new
    faces (the cut's walls) take the part's main colour, by area."""
    shape, base, colours = part
    args, tl = TopTools_ListOfShape(), TopTools_ListOfShape()
    args.Append(shape)
    for t in tools:
        tl.Append(t)
    op = BRepAlgoAPI_Cut()
    op.SetArguments(args)
    op.SetTools(tl)
    op.Build()
    if not op.IsDone():
        sys.exit("a cut failed")
    faces = faces_of(shape)
    colours = colours or [base] * len(faces)
    by_area = Counter()
    for f, c in zip(faces, colours):
        by_area[c] += area(f)
    main = max(by_area, key=by_area.get) if by_area else base
    known = []                            # (face in the result, its colour)
    for f, c in zip(faces, colours):
        if op.IsDeleted(f):
            continue
        mods = op.Modified(f)
        if mods.Extent() == 0:
            known.append((f, c))
        known += [(m, c) for m in mods]
    out = []
    for solid in solids_of(op.Shape()):
        fc = [next((c for k, c in known if k.IsSame(f)), main) for f in faces_of(solid)]
        out.append((solid, main, fc))
    return out


def flatten(parts, extra_tools=lambda bb: [], floor=0.0):
    """Cuts off everything below z = floor, and whatever extra_tools gives for a part."""
    below = BELOW if floor == 0 else box(-1000, -1000, -1000, 1000, 1000, floor)
    out = []
    for part in parts:
        bb = cq.Shape.cast(part[0]).BoundingBox()
        if bb.zmax <= floor + 1e-4:
            continue                      # all of it under the floor
        tools = ([below] if bb.zmin < floor - 1e-4 else []) + extra_tools(bb)
        out += cut(part, tools) if tools else [part]
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



def footprints(pcb):
    """(reference, footprint name) of every footprint."""
    out = []
    for p in re.split(r'\n\t\(footprint ', open(pcb).read())[1:]:
        ref = re.search(r'\(property "Reference" "([^"]+)"', p).group(1)
        out.append((ref, p.split('"')[1]))
    return out


def pads(pcb, ox, oy):
    """{reference: [(x, y) of each pad]}, in the south-west frame (KiCad's IPC-D-356)."""
    with tempfile.TemporaryDirectory() as tmp:
        ipc = os.path.join(tmp, "board.ipc")
        subprocess.run(["kicad-cli", "pcb", "export", "ipcd356", "-o", ipc, pcb],
                       capture_output=True, check=True)
        text = open(ipc).read()
    assert "UNITS CUST 0" in text     # inches, 0.0001 in a step
    out = {}
    for m in re.finditer(r'^3[123]7\S*\s+(\S+)\s+-\S+.*?X([+-]\d{6})Y([+-]\d{6})', text, re.M):
        x, y = int(m.group(2)) * 0.00254, -int(m.group(3)) * 0.00254
        out.setdefault(m.group(1), []).append((x - ox, oy - y))
    return out


def report(name, parts, n, lo, hi):
    print(f"   {name}: {len(parts)} solids, {n} triangles, "
          + ", ".join(f"{'xyz'[k]} {lo[k]:.3f}..{hi[k]:.3f}" for k in range(3)))
    if lo[2] < -1e-4:
        sys.exit(f"{name}: something is still below z = 0")


def simplified(pcb, origin, ox, oy, out):
    """The host board, simplified (see the top of this file)."""
    fps = footprints(pcb)
    keep = [r for r, f in fps if f.startswith(SIMPLE_KEEP)]
    nopins = [r for r, f in fps if f.startswith(SIMPLE_NO_PINS)]
    pins = pads(pcb, ox, oy)
    thick = float(re.search(r'\(general\s*\(thickness ([\d.]+)\)', open(pcb).read()).group(1))
    h = PIN_HOLE / 2
    columns = [box(x - h, y - h, -1, x + h, y + h, 50) for r in nopins for x, y in pins[r]]

    def pin_columns(bb):
        return [c for c in columns if bb.xmin - 1 < cq.Shape.cast(c).BoundingBox().center.x
                < bb.xmax + 1 and bb.ymin - 1 < cq.Shape.cast(c).BoundingBox().center.y
                < bb.ymax + 1]

    parts = []
    with tempfile.TemporaryDirectory() as tmp:
        for refs, tools in ((keep, lambda bb: []), (nopins, pin_columns)):
            raw = os.path.join(tmp, "part.step")
            kicad_step(pcb, origin, raw, "--no-board-body", "--component-filter", ",".join(refs))
            # Cut at the plate's top: the plate fills the board (the pins in it go).
            parts += flatten(read_parts(raw), tools, thick)
    # The NodeMCU's sockets, KiCad's model at U2's rows (pin 1 and pin 20), without the module.
    sock = read_parts(f"{KICAD_3D}/Connector_PinSocket_2.54mm.3dshapes/"
                      "PinSocket_1x19_P2.54mm_Vertical.step")
    p1 = min(pins["U2"], key=lambda p: (-p[1], p[0]))       # the north-west pad: pin 1
    for dx in (0.0, 22.86):
        for shape, base, colours in sock:
            trsf = cq.Location(cq.Vector(p1[0] + dx, p1[1], thick)).wrapped
            parts += flatten([(shape.Moved(trsf), base, colours)], floor=thick)
    # The bare plate: the outline (a rounded rectangle) with no holes.
    s = open(pcb).read()
    r = re.search(r'\(gr_arc \(start ([\d.]+) ([\d.]+)\) \(mid [^)]*\) \(end ([\d.]+) ([\d.]+)\)'
                  r'[^\n]*?"Edge.Cuts"', " ".join(s.split()))
    radius = abs(float(r.group(1)) - float(r.group(3))) if r else 0
    x0, x1, y0, y1 = edge_bbox(pcb)
    plate = cq.Workplane("XY").box(x1 - x0, y1 - y0, thick, centered=False)
    if radius:
        plate = plate.edges("|Z").fillet(radius)
    parts.append((plate.val().wrapped, BOARD_COLOUR, None))
    print(f"   simplified: {len(keep)} kept, {len(nopins)} connectors without pins "
          f"({len(columns)} pins), 2 sockets, a {thick} mm plate with {radius:g} mm corners")
    write_step(parts, os.path.join(out, "host_board_simplified.step"))
    report("host_board_simplified", parts,
           *write_stl(parts, os.path.join(out, "host_board_simplified.stl")))


for b in BOARDS:
    pcb = os.path.join(HERE, b + ".kicad_pcb")
    out = os.path.join(HERE, os.path.dirname(b), "mechanical")
    name = os.path.basename(b)
    os.makedirs(out, exist_ok=True)
    x0, x1, y0, y1 = edge_bbox(pcb)
    origin = f"{x0}x{y1}mm"   # the south-west corner (KiCad's page y points down)
    print(f"{name}: {x1 - x0:g} x {y1 - y0:g} mm, origin {origin}")
    if "--simplified-only" not in sys.argv:
        with tempfile.TemporaryDirectory() as tmp:
            raw = os.path.join(tmp, name + ".step")
            kicad_step(pcb, origin, raw, "--include-silkscreen")
            parts = read_parts(raw)
        lowest = min(cq.Shape.cast(s).BoundingBox().zmin for s, _, _ in parts)
        flat = flatten(parts)
        write_step(flat, os.path.join(out, name + ".step"))
        print(f"   leads to {lowest:.2f} mm cut off")
        report(name, flat, *write_stl(flat, os.path.join(out, name + ".stl")))
    if name == "host_board":
        simplified(pcb, origin, x0, y1, out)
