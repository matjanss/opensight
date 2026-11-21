#!/usr/bin/env python3
"""OpenSights calibration fit (measurement step 3, see ../MEASUREMENT.md).

Reads the firmware's calibration-mode lines from a serial log and fits the mic positions,
the per-channel offsets and k to sparks made at known points (a template). Prints the fit,
the position errors before and after, and the serial commands that store the result.

    cal config k=0.33333 CH1=-105.000,-105.000,0.0 CH2=...      (on `cal on`: the start)
    cal evt=12 pt=-50.000,30.000 air=18.06 CH1=1200,1500 CH2=0,300 CH3=-,- ...
        t1,t2 (OUT_L, OUT_H rise) per channel, ns from the event's first edge

Model, per spark j and channel c:
    t1 - k (t2 - t1) - offset_c = T_j + |p_j - m_c| / v(air_j)
with the spark's emission time T_j unknown. A delay common to every channel only moves
every T_j, so the offsets are fitted with mean 0. k is global, as in the firmware.

Usage: calibrate.py LOG... (or - for stdin) [--fix-k] [--fix-mics] [--fix-offsets]
Log: `pio device monitor -d firmware --echo --filter log2file` writes one to the current
directory, or copy the terminal into a file.
"""
import argparse
import re
import sys
from collections import Counter, defaultdict

import numpy as np
from scipy.optimize import least_squares


def sound_speed(temp_c):
    """mm/ns, as tdoa.h: 331.3 m/s at 0 degC + 0.606 per degC."""
    return (331.3 + 0.606 * np.asarray(temp_c, dtype=float)) * 1e-6


def default_mic(ch):  # ch 1..8, as config.h
    i = (ch - 1) % 4
    return (105.0 if i in (1, 2) else -105.0, 105.0 if i >= 2 else -105.0)


# ---- Parsing ----

KV = re.compile(r"(\w+)=(\S+)")


def parse(lines):
    config, events, skipped = None, [], Counter()
    for raw in lines:
        i = raw.find("cal config ")
        if i >= 0:
            cfg = {"k": None, "mic": {}, "off": {}}
            for key, val in KV.findall(raw[i:]):
                if key == "k":
                    cfg["k"] = float(val)
                elif key.startswith("CH"):
                    x, y, off = (float(v) for v in val.split(","))
                    ch = int(key[2:])
                    cfg["mic"][ch] = (x, y)
                    cfg["off"][ch] = off
            config = cfg
            continue
        i = raw.find("cal evt=")
        if i < 0:
            continue
        ev = {"edges": {}}
        for key, val in KV.findall(raw[i:]):
            if key == "evt":
                ev["n"] = int(val)
            elif key == "pt":
                ev["pt"] = None if val == "-" else tuple(float(v) for v in val.split(","))
            elif key == "air":
                ev["air"] = float(val)
            elif key.startswith("CH"):
                t = val.split(",")
                ev["edges"][int(key[2:])] = None if "-" in t else (float(t[0]), float(t[1]))
        if ev.get("pt") is None:
            skipped["no template point (`cal point`)"] += 1
        elif any(e is None for e in ev["edges"].values()):
            skipped["a channel without both edges"] += 1
        elif len(ev["edges"]) < 3:
            skipped["fewer than 3 channels"] += 1
        else:
            events.append(ev)
    return config, events, skipped


# ---- The solver, as tdoa.h ----

def solve(mics, t0, v):
    """mics (n, 2) mm, t0 (n,) ns, v mm/ns -> (x, y) or None."""
    cen = mics.mean(axis=0)
    u = mics - cen
    first = int(np.argmin(t0))
    s = v * (t0 - t0[first])
    p = 0.5 * u[first]
    for _ in range(10):
        d = np.maximum(np.hypot(*(p - u).T), 1e-3)
        D = np.mean(s - d)
        r = d + D - s
        J = np.column_stack([(p[0] - u[:, 0]) / d, (p[1] - u[:, 1]) / d, np.ones(len(d))])
        try:
            step = np.linalg.solve(J.T @ J, -J.T @ r)
        except np.linalg.LinAlgError:
            return None
        p = p + step[:2]
        if np.hypot(*step[:2]) < 1e-3:
            return p + cen
    return None


# ---- Fit ----

class Model:
    def __init__(self, chans, events, start, fix):
        self.chans, self.events, self.fix = chans, events, fix
        self.n = len(chans)
        self.t1 = np.array([[e["edges"][c][0] for c in chans] for e in events])
        self.t2 = np.array([[e["edges"][c][1] for c in chans] for e in events])
        self.pt = np.array([e["pt"] for e in events])
        self.v = sound_speed([e["air"] for e in events])
        self.start = start

    def unpack(self, q):
        i = 0
        st = self.start
        if self.fix["mics"]:
            mics = st["mics"].copy()
        else:
            mics = q[i:i + 2 * self.n].reshape(self.n, 2)
            i += 2 * self.n
        if self.fix["offsets"]:
            off = st["off"].copy()
        else:
            rest = q[i:i + self.n - 1]
            off = np.concatenate([[-rest.sum()], rest])
            i += self.n - 1
        if self.fix["k"]:
            k = st["k"]
        else:
            k = q[i]
            i += 1
        T = q[i:]
        return mics, off, k, T

    def pack0(self):
        st = self.start
        q = []
        if not self.fix["mics"]:
            q += list(st["mics"].ravel())
        if not self.fix["offsets"]:
            off = st["off"] - st["off"].mean()
            q += list(off[1:])
        if not self.fix["k"]:
            q.append(st["k"])
        t0 = self.t0(st["k"], st["off"])
        d = self.dist(st["mics"])
        q += list((t0 - d / self.v[:, None]).mean(axis=1))
        return np.array(q)

    def t0(self, k, off):
        return self.t1 - k * (self.t2 - self.t1) - off[None, :]

    def dist(self, mics):
        return np.hypot(self.pt[:, None, 0] - mics[None, :, 0], self.pt[:, None, 1] - mics[None, :, 1])

    def residuals(self, q):
        mics, off, k, T = self.unpack(q)
        pred = T[:, None] + self.dist(mics) / self.v[:, None]
        return (self.t0(k, off) - pred).ravel()


def fit(model, robust=False):
    if robust:   # residuals over ~200 ns count less: one late trip can't bend the fit
        res = least_squares(model.residuals, model.pack0(), method="trf", x_scale="jac",
                            loss="soft_l1", f_scale=200.0)
    else:
        res = least_squares(model.residuals, model.pack0(), method="lm", x_scale="jac")
    r = res.fun.reshape(len(model.events), model.n)
    dof = max(r.size - len(res.x), 1)
    s2 = (r ** 2).sum() / dof
    try:
        cov = np.linalg.inv(res.jac.T @ res.jac) * s2
        cond = np.linalg.cond(res.jac)
    except np.linalg.LinAlgError:
        cov, cond = np.full((len(res.x), len(res.x)), np.nan), np.inf
    return res, r, np.sqrt(s2), cov, cond


def position_errors(model, mics, off, k):
    """Solve every spark with a calibration; error to its template point (NaN if unsolved)."""
    t0 = model.t0(k, off)
    err = np.full((len(model.events), 2), np.nan)
    for j in range(len(model.events)):
        p = solve(mics, t0[j], model.v[j])
        if p is not None:
            err[j] = p - model.pt[j]
    return err


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("logs", nargs="+", help="serial log files, - for stdin")
    ap.add_argument("--fix-k", action="store_true", help="keep k as it was")
    ap.add_argument("--fix-mics", action="store_true", help="keep the mic positions")
    ap.add_argument("--fix-offsets", action="store_true", help="keep the offsets")
    ap.add_argument("--outlier", type=float, default=5.0,
                    help="drop sparks with a residual over this many rms (default 5), refit once")
    args = ap.parse_args()

    lines = []
    for name in args.logs:
        with (sys.stdin if name == "-" else open(name, errors="replace")) as f:
            lines += f.readlines()
    config, events, skipped = parse(lines)
    for why, n in skipped.items():
        print(f"skipped {n} sparks: {why}")
    if not events:
        sys.exit("no calibration sparks (`cal evt=` lines with a point) in the log")

    # One channel set: the most common one (a mask change mid-log leaves the others out).
    sets = Counter(tuple(sorted(e["edges"])) for e in events)
    chans, _ = sets.most_common(1)[0]
    if len(sets) > 1:
        print(f"using the sparks with channels {chans}; {len(events) - sets[chans]} with other "
              "channels left out")
    events = [e for e in events if tuple(sorted(e["edges"])) == chans]
    chans = list(chans)
    points = sorted({e["pt"] for e in events})
    print(f"{len(events)} sparks at {len(points)} points, channels "
          + ", ".join(f"CH{c}" for c in chans))
    if len(points) < 6:
        print("warning: fewer than 6 points; 9-16 spread over the card pin the mics down")

    if config is None:
        print("no `cal config` line (typed `cal on`?): starting from the default mics, k 1/3, "
              "offsets 0")
        config = {"k": 1 / 3, "mic": {}, "off": {}}
    start = {
        "mics": np.array([config["mic"].get(c, default_mic(c)) for c in chans], dtype=float),
        "off": np.array([config["off"].get(c, 0.0) for c in chans], dtype=float),
        "k": config["k"] if config["k"] is not None else 1 / 3,
    }
    fix = {"k": args.fix_k, "mics": args.fix_mics, "offsets": args.fix_offsets}

    # Outliers (a late trip, a bad spark): from a robust fit, against a robust scale; then
    # the plain fit without them.
    model = Model(chans, events, start, fix)
    _, r, _, _, _ = fit(model, robust=True)
    scale = 1.4826 * np.median(np.abs(r))
    bad = np.abs(r).max(axis=1) > args.outlier * scale
    if bad.any() and (~bad).sum() >= 3:
        print(f"dropping {bad.sum()} outlier sparks (a residual over {args.outlier:.0f} x "
              f"{scale:.0f} ns): evt " + ", ".join(str(e["n"]) for e, b in zip(events, bad) if b))
        events = [e for e, b in zip(events, bad) if not b]
        model = Model(chans, events, start, fix)
    res, r, rms, cov, cond = fit(model)
    mics, off, k, _ = model.unpack(res.x)

    # Uncertainties of the calibration parameters (the T_j are nuisance).
    nc = len(chans)
    i, sd_mic, sd_off, sd_k = 0, np.full((nc, 2), np.nan), np.full(nc, np.nan), np.nan
    if not fix["mics"]:
        sd_mic = np.sqrt(np.diag(cov)[:2 * nc]).reshape(-1, 2)
        i += 2 * nc
    if not fix["offsets"]:
        # offset of the first channel = -(sum of the others)
        A = np.zeros((nc, len(res.x)))
        A[0, i:i + nc - 1] = -1
        A[np.arange(1, nc), i + np.arange(nc - 1)] = 1
        sd_off = np.sqrt(np.maximum(np.diag(A @ cov @ A.T), 0))
        i += nc - 1
    if not fix["k"]:
        sd_k = np.sqrt(cov[i, i])

    print(f"\nfit: residual {rms:.1f} ns rms over {r.size} arrival times "
          f"({rms * np.mean(model.v):.4f} mm)"
          f"{', poorly determined (condition ' + f'{cond:.0e})' if cond > 1e8 else ''}")
    print(f"  k {k:.5f} +- {sd_k:.5f}" + ("  (fixed)" if fix["k"] else "")
          + f"   (was {start['k']:.5f})")
    print("  channel   mic x mm   mic y mm          moved mm   offset ns")
    for c, ch in enumerate(chans):
        moved = np.hypot(*(mics[c] - start["mics"][c]))
        print(f"  CH{ch}   {mics[c, 0]:9.3f} {mics[c, 1]:9.3f}  +-{np.nanmax(sd_mic[c]):6.3f}"
              f"  {moved:7.3f}   {off[c]:8.1f}" + (f" +-{sd_off[c]:.1f}" if np.isfinite(sd_off[c]) else ""))
    print("  (the offsets have mean 0: a delay common to every channel doesn't move a shot)")

    before = position_errors(model, start["mics"], start["off"], start["k"])
    after = position_errors(model, mics, off, k)
    print("\nposition error at the template points (mm), with the old and the fitted calibration:")
    by_point = defaultdict(list)
    for j, e in enumerate(events):
        by_point[e["pt"]].append(j)
    if len(by_point) <= 30:
        print("  point x   point y    n   old mean dx  dy     new mean dx  dy    new sd x   y")
        for pt in sorted(by_point):
            js = by_point[pt]
            b, a = np.nanmean(before[js], axis=0), np.nanmean(after[js], axis=0)
            s = np.nanstd(after[js], axis=0, ddof=1) if len(js) > 1 else (np.nan, np.nan)
            print(f"  {pt[0]:8.2f} {pt[1]:8.2f} {len(js):4d}   {b[0]:+8.3f} {b[1]:+7.3f}   "
                  f"{a[0]:+8.3f} {a[1]:+7.3f}   {s[0]:7.3f} {s[1]:6.3f}")
    else:
        ea = np.hypot(after[:, 0], after[:, 1])
        print("  (too many points for a table) the worst sparks after the fit:")
        for j in np.argsort(np.nan_to_num(ea, nan=np.inf))[::-1][:5]:
            pt = events[j]["pt"]
            print(f"  evt {events[j]['n']} at ({pt[0]:.2f}, {pt[1]:.2f}): {after[j, 0]:+.3f} "
                  f"{after[j, 1]:+.3f} mm")
    rb = np.sqrt(np.nanmean((before ** 2).sum(axis=1)))
    ra = np.sqrt(np.nanmean((after ** 2).sum(axis=1)))
    print(f"  all sparks: rms {rb:.3f} mm before, {ra:.3f} mm after"
          + (f"; {np.isnan(after[:, 0]).sum()} not solved" if np.isnan(after[:, 0]).any() else ""))
    print("  (errors at the points the fit used are optimistic: check at other known points)")

    span = max(np.hypot(*(mics[a] - mics[b])) for a in range(len(chans)) for b in range(a))
    print("\nserial commands to store it (paste one at a time):")
    if not fix["k"]:
        print(f"k {k:.5f}")
    if not fix["offsets"]:
        for c, ch in enumerate(chans):
            print(f"offset {ch} {off[c]:.1f}")
    if not fix["mics"]:
        for c, ch in enumerate(chans):
            print(f"mic {ch} {mics[c, 0]:.3f} {mics[c, 1]:.3f}")
    print(f"(largest distance between the mics {span:.1f} mm: `span` must be at least that)")


if __name__ == "__main__":
    main()
