#!/usr/bin/env python3
"""OpenSights - the reference implementation of firmware/MODEL.md (numpy, double precision).

Writes the shared test vectors in firmware/test/vectors/, which the C++ core
(firmware/lib/measure, `pio test -e native`) and later the Rust implementation are checked
against. Run it again after any change to MODEL.md, and commit the vectors with it:

    python3 firmware/tools/model.py            # writes the vectors
    python3 firmware/tools/model.py --check    # only checks the model against itself

The vectors are made with a fixed random seed, so running it twice gives the same files.
"""
import argparse
import math
import os

import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
VECTORS = os.path.join(HERE, "..", "test", "vectors")

# --- The speed of sound (MODEL.md "Speed of sound": Cramer 1993) ---


def sound_speed(t_c, rh, p_pa, xc=0.0004):
    """m/s, from degC, % RH and Pa."""
    k = t_c + 273.15
    psv = math.exp(1.2378847e-5 * k * k - 1.9121316e-2 * k + 33.93711047 - 6.3431645e3 / k)
    f = 1.00062 + 3.14e-8 * p_pa + 5.6e-7 * t_c * t_c
    xw = rh / 100.0 * f * psv / p_pa
    t = t_c
    return (331.5024 + 0.603055 * t - 0.000528 * t * t
            + (51.471935 + 0.1495874 * t - 0.000782 * t * t) * xw
            + (-1.82e-7 + 3.73e-8 * t - 2.93e-10 * t * t) * p_pa
            + (-85.20931 - 0.228525 * t + 5.91e-5 * t * t) * xc
            - 2.835149 * xw * xw - 2.15e-13 * p_pa * p_pa + 29.179762 * xc * xc
            + 0.000486 * xw * p_pa * xc)


# --- Geometry and the forward model (MODEL.md "Propagation") ---

# PLAN D1/D2: CH1 bottom left, CH2 bottom right, CH3 top right moved out 25 mm along its
# diagonal (slid 40 mm along the top edge, to (65, 105), before 2026-11-24), CH4 top left,
# CH5 bottom middle, CH6 right middle.
LAYOUT_D2 = np.array([[-105, -105], [105, -105], [122.68, 122.68], [-105, 105], [0, -105], [105, 0]], float)
LAYOUT_SQUARE = np.array([[-105, -105], [105, -105], [105, 105], [-105, 105]], float)


def distances(mics, x, y):
    """mics: rows (x, y, h)."""
    return np.sqrt((x - mics[:, 0]) ** 2 + (y - mics[:, 1]) ** 2 + mics[:, 2] ** 2)


def arrivals(mics, x, y, tau, c):
    """t0 per mic, ns; c in mm/ns."""
    return tau + distances(mics, x, y) / c


# --- The solver (MODEL.md "Solver"), in double, to full precision ---


def fit(mics, t0, sigma, c, mask, start, iters=60):
    """Weighted least squares of (x, y, tau) by Gauss-Newton with damping, from start.
    Returns (cost, x, y, D, t_first, converged), D a range offset as in MODEL.md."""
    used = np.array([(mask >> i) & 1 for i in range(len(mics))], bool)
    first = np.argmin(np.where(used, t0, np.inf))
    s = c * (t0 - t0[first])
    sig_ref = sigma[used].min()
    w = np.where(used, (sig_ref / sigma) ** 2, 0.0)

    def state(x, y):
        d = distances(mics, x, y)
        D = (w * (s - d)).sum() / w.sum()
        r = d + D - s
        return (w * r * r).sum(), d, D

    x, y = start
    cost, d, D = state(x, y)
    lam = 1e-3
    converged = False
    for _ in range(iters):
        J = np.column_stack([(x - mics[:, 0]) / d, (y - mics[:, 1]) / d, np.ones(len(d))])
        r = d + D - s
        A = (J * w[:, None]).T @ J
        g = (J * w[:, None]).T @ r
        step = -np.linalg.solve(A + lam * np.diag(np.diag(A)), g)
        nx, ny = x + step[0], y + step[1]
        ncost, nd, nD = state(nx, ny)
        if ncost <= cost:
            x, y, cost, d, D = nx, ny, ncost, nd, nD
            lam = max(lam * 0.1, 1e-9)
            if math.hypot(step[0], step[1]) < 1e-9:
                converged = True
                break
        else:
            lam *= 10
            if math.hypot(step[0], step[1]) < 1e-9:
                converged = True
                break
    return cost, x, y, D, t0[first], converged


def outputs(mics, t0, sigma, c, mask, x, y, D, t_first):
    used = np.array([(mask >> i) & 1 for i in range(len(mics))], bool)
    s = c * (t0 - t_first)
    d = distances(mics, x, y)
    res_ns = (s - d - D) / c
    rms = math.sqrt((res_ns[used] ** 2).mean())
    sig_ref = sigma[used].min()
    w = np.where(used, (sig_ref / sigma) ** 2, 0.0)
    J = np.column_stack([(x - mics[:, 0]) / d, (y - mics[:, 1]) / d, np.ones(len(d))])
    A = (J * w[:, None]).T @ J
    cov = (sig_ref * c) ** 2 * np.linalg.inv(A)
    sx, sy = math.sqrt(cov[0, 0]), math.sqrt(cov[1, 1])
    rho = cov[0, 1] / (sx * sy)
    major = math.sqrt(np.linalg.eigvalsh(cov[:2, :2]).max())
    tau = t_first + D / c
    return dict(x=x, y=y, tau=tau, rms=rms, res=res_ns, sx=sx, sy=sy, rho=rho, major=major)


def solve_best(mics, t0, sigma, c, mask, truth=None):
    """The global optimum: the best of several starts (and the truth, if known)."""
    starts = [(0.0, 0.0)] + [(a, b) for a in (-80, 0, 80) for b in (-80, 0, 80)]
    if truth is not None:
        starts.insert(0, truth)
    best = None
    for st in starts:
        r = fit(mics, t0, sigma, c, mask, st)
        if r[5] and (best is None or r[0] < best[0]):
            best = r
    if best is None:
        return None
    _, x, y, D, tf, _ = best
    return outputs(mics, t0, sigma, c, mask, x, y, D, tf)


def solve_drop_one(mics, t0, sigma, c, mask, limit_ns, truth=None):
    """MODEL.md "Drop-one": the full fit, or the best fit with one mic left out."""
    full = solve_best(mics, t0, sigma, c, mask, truth)
    n_used = bin(mask).count("1")
    if (full is not None and full["rms"] <= limit_ns) or n_used < 5:
        return full, -1
    best, drop = None, -1
    for i in range(len(mics)):
        if not (mask >> i) & 1:
            continue
        r = solve_best(mics, t0, sigma, c, mask & ~(1 << i), truth)
        if r is not None and (best is None or r["rms"] < best["rms"]):
            best, drop = r, i
    return best, drop


# --- Vectors (MODEL.md "Test vectors") ---


def fmt(v, digits):
    return f"{v:.{digits}f}"


def solve_line(name, kind, limit, mics, t0, sigma, c, mask, out, dropped):
    """One case: inputs | expected. See MODEL.md for the fields."""
    n = len(mics)
    f = [name, kind, fmt(limit, 1), str(n), f"{mask:02X}", f"{c:.12e}"]
    for m in mics:
        f += [fmt(m[0], 3), fmt(m[1], 3), fmt(m[2], 3)]
    f += [fmt(v, 4) for v in t0]
    f += [fmt(v, 3) for v in sigma]
    f.append("|")
    if out is None:
        f += ["0"]
    else:
        f += ["1", str(dropped), fmt(out["x"], 6), fmt(out["y"], 6), fmt(out["tau"], 4),
              fmt(out["rms"], 4)]
        f += [fmt(v, 4) for v in out["res"]]
        f += [fmt(out["sx"], 6), fmt(out["sy"], 6), fmt(out["rho"], 6), fmt(out["major"], 6)]
    return " ".join(f)


def rounded(mics, t0, sigma, c):
    """The inputs as the vector file holds them, so the expected values are computed from
    exactly what the other implementations read."""
    return (np.round(mics, 3), np.round(t0, 4), np.round(sigma, 3), float(f"{c:.12e}"))


def mics_with_heights(layout, rng, heights=(10.5, 21.5), mixed=False):
    h = rng.choice(heights, len(layout)) if mixed else np.full(len(layout), rng.choice(heights))
    return np.column_stack([layout, h])


def make_vectors(rng):
    lines = {"air": [], "solve": []}
    # Speed of sound over the range the firmware sees.
    for t in (-10, 0, 10, 18, 20, 25, 30, 40):
        for rh in (0, 30, 50, 80, 100):
            for p in (90000, 95000, 101325, 105000):
                lines["air"].append(f"{t} {rh} {p} {sound_speed(t, rh, p):.6f}")

    out = lines["solve"]
    full6 = 0x3F
    limit = 1500.0

    def case(name, kind, mics, x, y, noise_ns, mask, sigma_ns=50.0, fault=None, lim=limit):
        n = len(mics)
        t_c = rng.uniform(10, 30)
        rh = rng.uniform(20, 80)
        c = sound_speed(t_c, rh, 101325) * 1e-6
        tau = rng.uniform(0, 5000)
        t0 = arrivals(mics, x, y, tau, c) + rng.normal(0, noise_ns, n)
        if fault is not None:
            ch, dt = fault
            t0[ch] += dt
        sigma = np.full(n, sigma_ns)
        mics, t0, sigma, c = rounded(mics, t0, sigma, c)
        if kind == "drop":
            res, dropped = solve_drop_one(mics, t0, sigma, c, mask, lim, (x, y))
            want = fault[0] if fault is not None else -1
            assert dropped == want, (name, dropped, want)
        else:
            res, dropped = solve_best(mics, t0, sigma, c, mask, (x, y)), -1
        out.append(solve_line(name, kind, lim, mics, t0, sigma, c, mask, res, dropped))

    # 1. Random points in the scoring area, 50 ns noise, the D1/D2 layout, one height or
    #    mixed heights, a few with unequal sigmas.
    for i in range(200):
        mics = mics_with_heights(LAYOUT_D2, rng, mixed=i % 2 == 1)
        x, y = rng.uniform(-100, 100, 2)
        case(f"d2-{i}", "fit", mics, x, y, 50.0, full6)
    for i in range(30):
        mics = mics_with_heights(LAYOUT_D2, rng)
        x, y = rng.uniform(-100, 100, 2)
        n = len(mics)
        t_c = 20.0
        c = sound_speed(t_c, 50, 101325) * 1e-6
        sigma = rng.uniform(30, 200, n)
        t0 = arrivals(mics, x, y, 1000.0, c) + rng.normal(0, 1, n) * sigma
        mics, t0, sigma, c = rounded(mics, t0, sigma, c)
        res = solve_best(mics, t0, sigma, c, full6, (x, y))
        out.append(solve_line(f"d2w-{i}", "fit", limit, mics, t0, sigma, c, full6, res, -1))
    # 2. Fewer mics: the D1 corners only, and the square (4 mics).
    for i in range(40):
        mics = mics_with_heights(LAYOUT_D2, rng)
        x, y = rng.uniform(-100, 100, 2)
        case(f"d1-{i}", "fit", mics, x, y, 50.0, 0x0F)
    for i in range(40):
        mics = mics_with_heights(LAYOUT_SQUARE, rng)
        x, y = rng.uniform(-100, 100, 2)
        case(f"sq-{i}", "fit", mics, x, y, 50.0, 0x0F)
    # 3. Outside the frame, up to 100 mm beyond the mics, no noise.
    for i in range(60):
        mics = mics_with_heights(LAYOUT_D2, rng)
        while True:
            x, y = rng.uniform(-205, 205, 2)
            if max(abs(x), abs(y)) > 110:
                break
        case(f"out-{i}", "outside", mics, x, y, 0.0, full6)
    # 4. One channel 24 us late or early (half a cycle; noise before the wavefront).
    for i in range(80):
        mics = mics_with_heights(LAYOUT_D2, rng)
        x, y = rng.uniform(-100, 100, 2)
        ch = int(rng.integers(0, 6))
        dt = 24000.0 if i % 2 == 0 else -24000.0
        case(f"bad-{i}", "drop", mics, x, y, 50.0, full6, fault=(ch, dt))
    # 5. Clean shots through the drop-one search: it must keep all six.
    for i in range(20):
        mics = mics_with_heights(LAYOUT_D2, rng)
        x, y = rng.uniform(-100, 100, 2)
        case(f"keep-{i}", "drop", mics, x, y, 50.0, full6)
    # 6. Blasts: a point source 10 or 25 m in front, 0-3 m off the axis (no expected
    #    solution: the check is that none fits as a hit).
    for i in range(60):
        mics = mics_with_heights(LAYOUT_D2, rng)
        z = 10000.0 if i % 2 == 0 else 25000.0
        lat = rng.uniform(0, 3000)
        phi = rng.uniform(0, 2 * math.pi)
        sx, sy = lat * math.cos(phi), lat * math.sin(phi)
        c = sound_speed(20, 50, 101325) * 1e-6
        d = np.sqrt((sx - mics[:, 0]) ** 2 + (sy - mics[:, 1]) ** 2 + (z + mics[:, 2]) ** 2)
        t0 = d / c + rng.normal(0, 50, len(mics))
        t0 -= t0.min()
        sigma = np.full(len(mics), 50.0)
        mics, t0, sigma, c = rounded(mics, t0, sigma, c)
        out.append(solve_line(f"blast-{i}", "blast", limit, mics, t0, sigma, c, full6, None, -1))
    return lines


def self_check():
    """The model against itself: noise-free points come back exactly, the reference
    speed of sound values."""
    assert abs(sound_speed(0, 0, 101325) - 331.448) < 0.001
    assert abs(sound_speed(20, 50, 101325) - 343.987) < 0.001
    rng = np.random.default_rng(1)
    c = sound_speed(20, 50, 101325) * 1e-6
    worst = 0
    for _ in range(200):
        mics = mics_with_heights(LAYOUT_D2, rng, mixed=True)
        x, y = rng.uniform(-100, 100, 2)
        t0 = arrivals(mics, x, y, 1234.5, c)
        r = solve_best(mics, t0, np.full(6, 50.0), c, 0x3F)
        worst = max(worst, math.hypot(r["x"] - x, r["y"] - y), abs(r["tau"] - 1234.5) * c)
    assert worst < 1e-6, worst
    print(f"model.py: self-check passed (noise-free points within {worst:.1e} mm)")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--check", action="store_true", help="only the self-check")
    args = ap.parse_args()
    self_check()
    if args.check:
        return
    lines = make_vectors(np.random.default_rng(20261121))
    os.makedirs(VECTORS, exist_ok=True)
    head = {
        "air": "# T_degC RH_percent p_Pa c_m_per_s  (MODEL.md \"Test vectors\"; made by tools/model.py)",
        "solve": "# name kind limit_ns n mask c_mm_per_ns (x y h)*n t0_ns*n sigma_ns*n | ok "
                 "[dropped x y tau_ns rms_ns res_ns*n sx sy rho major]  (MODEL.md \"Test "
                 "vectors\"; made by tools/model.py)",
    }
    for name, rows in lines.items():
        path = os.path.join(VECTORS, f"{name}.txt")
        with open(path, "w") as f:
            f.write(head[name] + "\n")
            f.write("\n".join(rows) + "\n")
        print(f"model.py: {len(rows)} cases in {os.path.relpath(path)}")


if __name__ == "__main__":
    main()
