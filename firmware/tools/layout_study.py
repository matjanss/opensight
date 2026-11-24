#!/usr/bin/env python3
"""OpenSights mic layout study (see ../../PLAN.md, decisions D1-D3).

Compares mic layouts on the prototype frame (250 x 250 mm, mics around a 210 mm square,
shots anywhere in the 200 x 200 mm scoring area) on:
  - GDOP: position error per mm of range error (40 ns of timing ~ 0.014 mm), with the
    speed of sound known, and fitted per shot;
  - systematic errors: the speed of sound 1 % off, the mic height above the card plane
    (10.5 mm) left out of the model;
  - one channel 24 us late (a half-cycle-late trip): the residual it leaves;
  - blasts: point sources 10 or 25 m in front, 0-3 m off the axis (neighbouring lanes):
    how many fit a point inside the card with a residual under 1.5 us (would be scored);
  - with --drop: the same with a bad channel found and dropped (5+ mics).

Run: python3 layout_study.py [--drop] [--offsets]   (takes a few minutes)
"""
import argparse

import numpy as np

C = (331.3 + 0.606 * 18) * 1e-6     # mm/ns at 18 degC
R = 105 * np.sqrt(2)                # corner radius of the 210 mm square
H = 10.5                            # mic port above the card plane (sensor board v0.3)
CARD_MM = 100                       # scoring area: |x|, |y| <= this (200 x 200 mm)
RES_LIMIT_NS = 1500                 # step 4's planned residual limit


def ring(angles_deg, radii):
    a = np.radians(angles_deg)
    return np.column_stack([np.asarray(radii) * np.cos(a), np.asarray(radii) * np.sin(a)])


SQ = [225, 315, 45, 135]
LAYOUTS = {
    "A  square, corners (concyclic)": ring(SQ, [R] * 4),
    "B1 one corner rotated 20 deg (concyclic)": ring([225, 315, 65, 135], [R] * 4),
    "B2 one corner 25 mm further out": ring(SQ, [R, R, R + 25, R]),
    "B3 one corner 25 mm further in": ring(SQ, [R, R, R - 25, R]),
    "C  rectangle 210 x 170 (concyclic)": np.array([[-105, -85], [105, -85], [105, 85], [-105, 85]]),
    "D  isosceles trapezoid (concyclic)": np.array([[-105, -105], [105, -105], [85, 105], [-85, 105]]),
    "E  5: square + bottom middle": np.vstack([ring(SQ, [R] * 4), [[0, -105]]]),
    "F  5 on a circle (pentagon)": ring([270, 342, 54, 126, 198], [R] * 5),
    "G  6: square + left and right middles": np.vstack([ring(SQ, [R] * 4), [[105, 0], [-105, 0]]]),
    # Breaking the symmetry without entering the scoring area: a corner mic slid along the
    # top edge, towards the middle (all mics stay outside +-100 mm).
    "S25 top-right corner slid 25 mm along the top edge": np.array(
        [[-105, -105], [105, -105], [80, 105], [-105, 105]], float),
    "S40 top-right corner slid 40 mm along the top edge": np.array(
        [[-105, -105], [105, -105], [65, 105], [-105, 105]], float),
    "S60 top-right corner slid 60 mm along the top edge": np.array(
        [[-105, -105], [105, -105], [45, 105], [-105, 105]], float),
    "S40+B 5: S40 + bottom middle": np.array(
        [[-105, -105], [105, -105], [65, 105], [-105, 105], [0, -105]], float),
    "S40+R 5: S40 + right middle": np.array(
        [[-105, -105], [105, -105], [65, 105], [-105, 105], [105, 0]], float),
    "S40+BR 6: S40 + bottom and right middles": np.array(
        [[-105, -105], [105, -105], [65, 105], [-105, 105], [0, -105], [105, 0]], float),
    "S40+LR 6: S40 + left and right middles": np.array(
        [[-105, -105], [105, -105], [65, 105], [-105, 105], [105, 0], [-105, 0]], float),
    # The layout from 2026-11-24 (PLAN D1's update): the top-right corner moved out 25 mm
    # along its diagonal, to (122.68, 122.68), with and without the middles.
    "O25 top-right corner 25 mm out along the diagonal": np.array(
        [[-105, -105], [105, -105], [122.68, 122.68], [-105, 105]], float),
    "O25+B 5: O25 + bottom middle": np.array(
        [[-105, -105], [105, -105], [122.68, 122.68], [-105, 105], [0, -105]], float),
    "O25+BR 6: O25 + bottom and right middles (the D1/D2 layout)": np.array(
        [[-105, -105], [105, -105], [122.68, 122.68], [-105, 105], [0, -105], [105, 0]], float),
}
STARTS = [np.array([x, y], float) for x in (-60, 0, 60) for y in (-60, 0, 60)]


def dist(p, M, h=0.0):
    return np.sqrt(((p - M) ** 2).sum(axis=1) + h * h)


def fit(M, t, starts=STARTS):
    """Point source in the card plane, speed C, mics at h = 0: best (point, rms residual ns)."""
    best = None
    for p0 in starts:
        p = np.array(p0, float)
        for _ in range(30):
            d = np.maximum(dist(p, M), 1e-6)
            T = np.mean(t - d / C)
            r = d - C * (t - T)
            J = np.column_stack([(p[0] - M[:, 0]) / d, (p[1] - M[:, 1]) / d, np.full(len(d), C)])
            s = np.linalg.lstsq(J, -r, rcond=None)[0]
            p = p + s[:2]
            if np.hypot(*s[:2]) < 1e-6:
                break
        d = dist(p, M)
        r = d - C * (t - np.mean(t - d / C))
        rms = np.sqrt((r ** 2).mean()) / C
        if best is None or rms < best[1]:
            best = (p, rms)
    return best


def fit_drop(M, t):
    """As fit, but if the residual is over the limit, the best fit with one channel dropped."""
    full = fit(M, t)
    if full[1] < RES_LIMIT_NS or len(M) < 5:
        return full, None
    best = None
    for drop in range(len(M)):
        keep = [i for i in range(len(M)) if i != drop]
        q = fit(M[keep], t[keep])
        if best is None or q[1] < best[0][1]:
            best = (q, drop)
    return best


def gdop(M, p, fit_c=False):
    d = dist(p, M)
    J = np.column_stack([(p[0] - M[:, 0]) / d, (p[1] - M[:, 1]) / d, np.ones(len(d))])
    if fit_c:
        J = np.column_stack([J, d])
    cov = np.linalg.pinv(J.T @ J)
    return np.sqrt(cov[0, 0] + cov[1, 1])


def blasts(M, solver):
    out = []
    for Z in (10000, 25000):
        for lat in np.linspace(0, 3000, 13):
            for phi in np.radians(np.arange(0, 360, 15)):
                S = np.array([lat * np.cos(phi), lat * np.sin(phi)])
                t = np.sqrt(((M - S) ** 2).sum(axis=1) + Z * Z) / C
                q, res = solver(M, t)[:2] if solver is fit else solver(M, t)[0]
                out.append(res if max(abs(q[0]), abs(q[1])) <= CARD_MM else np.inf)
    return np.array(out)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--drop", action="store_true", help="also: drop-one-channel search (5+ mics)")
    ap.add_argument("--offsets", action="store_true", help="also: how far off the circle is enough")
    ap.add_argument("--only", help="only the layouts whose name starts with this")
    args = ap.parse_args()

    area = [np.array([x, y]) for x in np.linspace(-CARD_MM, CARD_MM, 21)
            for y in np.linspace(-CARD_MM, CARD_MM, 21)]
    centre = [p for p in area if np.hypot(*p) <= 30]
    for name, M in LAYOUTS.items():
        if args.only and not name.startswith(args.only):
            continue
        g = [gdop(M, p) for p in area]
        gc = [gdop(M, p, True) for p in area]
        speed, height = [], []
        for p in area:
            speed.append(np.hypot(*(fit(M, dist(p, M) / (C * 1.01), [p])[0] - p)))
            height.append(np.hypot(*(fit(M, dist(p, M, H) / C, [p])[0] - p)))
        late = []
        for p in centre:
            for i in range(len(M)):
                t = dist(p, M) / C
                t[i] += 24000
                late.append(fit(M, t)[1])
        b = blasts(M, fit)
        print(f"\n{name}")
        print(f"  GDOP mean {np.mean(g):.2f}, max {np.max(g):.2f}; speed fitted too: mean "
              f"{np.mean(gc):.2f}, max {np.max(gc):.2f}")
        print(f"  speed 1 % off: shift mean {np.mean(speed):.3f}, max {np.max(speed):.3f} mm, at the "
              f"centre {np.hypot(*(fit(M, dist(np.zeros(2), M) / (C * 1.01))[0])):.3f}")
        print(f"  mic height {H} mm not modelled: shift mean {np.mean(height):.3f}, max "
              f"{np.max(height):.3f} mm")
        print(f"  one channel 24 us late (within 30 mm of the centre): residual at least "
              f"{min(late) / 1000:.2f} us")
        print(f"  blasts scored as hits: {(b < RES_LIMIT_NS).sum()} of {len(b)} (smallest residual "
              f"inside the card {b.min() / 1000:.2f} us)")
        if args.drop and len(M) >= 5:
            found = n = 0
            for p in area[::4]:
                for i in range(len(M)):
                    t = dist(p, M) / C
                    t[i] += 24000
                    (q, res), drop = fit_drop(M, t)
                    n += 1
                    found += res < RES_LIMIT_NS and drop == i
            bd = blasts(M, fit_drop)
            print(f"  dropping a bad channel: the late one found {found} of {n}; blasts scored "
                  f"{(bd < RES_LIMIT_NS).sum()} of {len(bd)}")

    if args.offsets:
        print()
        for off in (5, 10, 15, 25, 40):
            b = blasts(ring(SQ, [R, R, R + off, R]), fit)
            print(f"one corner {off} mm off the circle: blasts scored {(b < RES_LIMIT_NS).sum()} of "
                  f"{len(b)}, smallest residual inside the card {b.min() / 1000:.2f} us")


if __name__ == "__main__":
    main()
