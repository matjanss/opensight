from fontTools.ttLib import TTFont
from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.recordingPen import RecordingPen
from fontTools.pens.boundsPen import BoundsPen
from fontTools.pens.transformPen import TransformPen
import cairosvg, io
from PIL import Image

F = TTFont("/usr/share/fonts/truetype/ibm-plex/IBMPlexSans-Bold.ttf")
gs = F.getGlyphSet(); cmap = F.getBestCmap()
TILE, INK, DOT = "#1A1A1A", "#F2F2F2", "#E86034"   # app: dark panel greys, light text, LAST_SHOT_COLOR

def glyph(ch):
    g = gs[cmap[ord(ch)]]
    bp = BoundsPen(gs); g.draw(bp)
    return g, bp.bounds, g.width

def contours(g):
    rp = RecordingPen(); g.draw(rp)
    cs, cur = [], []
    for op, args in rp.value:
        cur.append((op, args))
        if op in ("closePath", "endPath"): cs.append(cur); cur = []
    return cs

def bounds_of(ops):
    xs, ys = [], []
    for op, args in ops:
        for p in args: xs.append(p[0]); ys.append(p[1])
    return min(xs), min(ys), max(xs), max(ys)

def build(size=1024, inset=0):
    """Tile of `size` px, optionally inset (macOS grid: 100 px margin on 1024).
    O = a round ring with the stroke weight of Plex Sans Bold's O; S = the Plex Sans Bold glyph."""
    t = size - 2 * inset
    gO, bO, wO = glyph("O"); gS, bS, wS = glyph("S")
    inner = min(contours(gO), key=lambda c: (bounds_of(c)[2] - bounds_of(c)[0]))
    ix0, iy0, ix1, iy1 = bounds_of(inner)
    stroke_u = ((bO[2] - bO[0]) - (ix1 - ix0)) / 2          # side stroke of the O
    cap = bS[3] - bS[1]
    scale = 0.40 * t / cap                                   # letters 40 % of the tile height
    D = cap * scale                                          # ring outer diameter = cap height
    sw = stroke_u * scale * 1.05
    gap = 0.07 * D
    sW = (bS[2] - bS[0]) * scale
    total = D + gap + sW
    x0 = inset + (t - total) / 2
    cy = inset + t / 2
    rO = D / 2 - sw / 2
    cxO = x0 + D / 2
    # S: place its bbox left edge at x0 + D + gap, vertically centred
    sx = x0 + D + gap - bS[0] * scale
    sy = cy + (bS[1] + bS[3]) / 2 * scale
    pen = SVGPathPen(gs); gS.draw(TransformPen(pen, (scale, 0, 0, -scale, sx, sy)))
    rdot = 0.50 * (D / 2 - sw)
    rx = 0.225 * t
    bw = max(1.0, 0.012 * t)
    return (f'<svg xmlns="http://www.w3.org/2000/svg" width="{size}" height="{size}" viewBox="0 0 {size} {size}">'
            f'<rect x="{inset + bw / 2:.1f}" y="{inset + bw / 2:.1f}" width="{t - bw:.1f}" height="{t - bw:.1f}" rx="{rx:.1f}" fill="{TILE}" stroke="#3C3C3C" stroke-width="{bw:.1f}"/>'
            f'<circle cx="{cxO:.1f}" cy="{cy:.1f}" r="{rO:.1f}" fill="none" stroke="{INK}" stroke-width="{sw:.1f}"/>'
            f'<path d="{pen.getCommands()}" fill="{INK}"/>'
            f'<circle cx="{cxO:.1f}" cy="{cy:.1f}" r="{rdot:.1f}" fill="{DOT}"/></svg>')

svg = build(); open("opensights-icon.svg", "w").write(svg)
svg_mac = build(inset=100); open("opensights-icon-macos.svg", "w").write(svg_mac)
def png(s, n): return Image.open(io.BytesIO(cairosvg.svg2png(bytestring=s.encode(), output_width=n, output_height=n))).convert("RGBA")
import os
os.makedirs("png", exist_ok=True)
for n in (16, 24, 32, 48, 64, 128, 256, 512, 1024):
    png(svg, n).save(f"png/opensights-icon-{n}.png")
big = png(svg, 256)
big.save("opensights.ico", sizes=[(16,16),(24,24),(32,32),(48,48),(64,64),(128,128),(256,256)])
png(svg_mac, 1024).save("opensights.icns")
print("done")
