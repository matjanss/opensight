#!/usr/bin/env python3
"""OpenSights calibration fit (MEASUREMENT.md step 3e, PLAN.md D5).

Reads a serial log's `cfg` and `rec` lines (MODEL.md "Log lines") and fits, per channel, the
mic position (x, y), the arrival-time factor k and the channel's own delay (offset) to
sparks made at known points: `rec` lines with a `pt` (calibration mode's template point,
or the simulator's truth). The mic height h stays as set (PLAN D3). Prints the fit, the
position error per point before and after, each value's uncertainty, and the lines to
send to the target: one `cal ch` per channel, then `cal id`.

Model, per spark e and channel i (MODEL.md "Arrival", "Propagation"):
    t1 - k_i (t2 - t1) - offset_i = tau_e + d(pt_e, mic_i, h_i) / c_e
with each spark's emission time tau_e unknown. A delay common to every channel only moves
every tau_e, so the offsets are fitted with mean 0. c_e is the speed of sound the target
logged with the event.

The fit: robust first (soft L1), then sparks with a residual beyond 5 MADs are left out,
then plain least squares. sigma per channel = its rms residual (the solver's weight).

It takes calibration mode's sparks (`mode=cal`) of the newest session in the log (the
settings the last one was made with); --any-pt adds the simulator's other shots, which
carry their true point too.

Usage: calibrate.py LOG... [--date YYYYMMDD] [--any-pt] [--json OUT]
       calibrate.py --selftest [--write-log LOG] [--json OUT]
           (a simulated calibration, fitted and checked; --write-log keeps its log)
--json writes the fit's results (per channel x, y, k, offset, sigma, their uncertainties, the
residual, the record's lines and id): the Rust fit (app/calib) is checked against them.
"""
import argparse
import datetime
import math
import re
import sys
import zlib

import numpy as np
from scipy.optimize import least_squares
from scipy.sparse import lil_matrix

MAX_MICS = 8
KV = re.compile(r"(\w+)=(\S+)")


# ---- The log lines (lib/measure/record.cpp) ----

def g9(v):
    """A float32 as the target prints it (%.9g)."""
    return "%.9g" % float(np.float32(v))


def fields_text(chans):
    """The cfg line's channel fields: chans[i] = None or (x, y, h, k, offset, sigma)."""
    out = []
    for i, c in enumerate(chans):
        out.append("CH%d=%s" % (i + 1, "-" if c is None else ",".join(g9(v) for v in c)))
    return " ".join(out)


def calibration_id(chans):
    return zlib.crc32(fields_text(chans).encode()) & 0xFFFFFFFF


def parse_cfg(line):
    i = line.find("cfg id=")
    if i < 0:
        return None
    rest = line[i + 7:].strip()
    cid, _, fields = rest.partition(" ")
    if zlib.crc32(fields.encode()) & 0xFFFFFFFF != int(cid, 16):
        return None  # corrupted
    chans = [None] * MAX_MICS
    lim, drop = 1500.0, 0
    for key, val in KV.findall(fields):
        if key.startswith("CH"):
            ch = int(key[2:]) - 1
            if val != "-":
                chans[ch] = tuple(float(v) for v in val.split(","))
        elif key == "lim":
            lim = float(val)
        elif key == "drop":
            drop = int(val)
    return {"id": int(cid, 16), "chans": chans, "lim": lim, "drop": drop}


def parse_rec(line):
    i = line.find("rec n=")
    if i < 0:
        return None
    r = {"edges": {}, "pt": None, "mode": None}
    for key, val in KV.findall(line[i:]):
        if key.startswith("CH"):
            t1, t2 = val.split(",")
            r["edges"][int(key[2:]) - 1] = None if "-" in (t1, t2) else (float(t1), float(t2))
        elif key == "n":
            r["n"] = int(val)
        elif key == "cfg":
            r["cfg"] = int(val, 16)
        elif key == "c":
            r["c"] = float(val)
        elif key == "T":
            r["T"] = float(val)
        elif key == "pt":
            r["pt"] = tuple(float(v) for v in val.split(","))
        elif key == "mode":
            r["mode"] = val
    return r if "cfg" in r and "c" in r else None


def read_logs(paths):
    cfgs, recs = {}, []
    for path in paths:
        with open(path, errors="replace") as f:
            for line in f:
                c = parse_cfg(line)
                if c:
                    cfgs[c["id"]] = c
                    continue
                r = parse_rec(line)
                if r:
                    recs.append(r)
    return cfgs, recs


# ---- The fit ----

def distances(mx, my, h, px, py):
    """(events, channels) mm."""
    return np.sqrt((px[:, None] - mx[None, :]) ** 2 + (py[:, None] - my[None, :]) ** 2 + h[None, :] ** 2)


class Fit:
    """Channels chans (indices), events with t1, t2 (E, n), points (E, 2), c (E,) mm/ns."""

    def __init__(self, h, t1, t2, pts, c):
        self.h, self.t1, self.g, self.pts, self.c = h, t1, t2 - t1, pts, c
        self.E, self.n = t1.shape

    def unpack(self, q):
        n = self.n
        mx, my, k = q[:n], q[n:2 * n], q[2 * n:3 * n]
        off = np.append(q[3 * n:4 * n - 1], -q[3 * n:4 * n - 1].sum())
        tau = q[4 * n - 1:]
        return mx, my, k, off, tau

    def pack(self, mx, my, k, off, tau):
        off = off - off.mean()
        return np.concatenate([mx, my, k, off[:-1], tau])

    def raw(self, q):
        """The residual in ns, (events, channels)."""
        mx, my, k, off, tau = self.unpack(q)
        d = distances(mx, my, self.h, self.pts[:, 0], self.pts[:, 1])
        t0 = self.t1 - k[None, :] * self.g - off[None, :]
        return t0 - tau[:, None] - d / self.c[:, None]

    def plain(self, q):
        return self.raw(q).ravel()

    def scaled(self, q):
        # t1 and t2 both carry the sampling's noise, so t0 = (1 + k) t1 - k t2 carries
        # sqrt((1 + k)^2 + k^2) times one edge's. Dividing by that makes the fit the maximum
        # likelihood one (Deming regression); without it, the noise in t2 - t1 pulls every k
        # low (by 0.02-0.05 in the self-test) and the offsets with it. It also has a false
        # minimum at k -> infinity (the scaled residual stays bounded): so it starts from the
        # plain fit, with k bounded.
        k = self.unpack(q)[2]
        return (self.raw(q) / np.sqrt((1 + k) ** 2 + k ** 2)[None, :]).ravel()

    def tau_for(self, mx, my, k, off):
        d = distances(mx, my, self.h, self.pts[:, 0], self.pts[:, 1])
        t0 = self.t1 - k[None, :] * self.g - off[None, :]
        return (t0 - d / self.c[:, None]).mean(axis=1)

    def sparsity(self):
        n, E = self.n, self.E
        S = lil_matrix((E * n, 4 * n - 1 + E), dtype=int)
        for e in range(E):
            for i in range(n):
                r = e * n + i
                S[r, i] = S[r, n + i] = S[r, 2 * n + i] = 1
                if i < n - 1:
                    S[r, 3 * n + i] = 1
                else:
                    S[r, 3 * n:4 * n - 1] = 1
                S[r, 4 * n - 1 + e] = 1
        return S

    def solve(self, q0, robust):
        """robust: the plain residual with a soft L1 loss (the first fit, for the outliers);
        else the scaled residual, least squares, k within 0..2 (the final fit)."""
        if robust:
            return least_squares(self.plain, q0, jac_sparsity=self.sparsity(), method="trf",
                                 x_scale="jac", loss="soft_l1", f_scale=50.0)
        n = self.n
        lo = np.full(len(q0), -np.inf)
        hi = np.full(len(q0), np.inf)
        lo[2 * n:3 * n], hi[2 * n:3 * n] = 0.0, 2.0
        q0 = q0.copy()
        q0[2 * n:3 * n] = np.clip(q0[2 * n:3 * n], 0.01, 1.99)
        # Tight tolerances: k and the offsets trade off along a flat valley, and scipy's
        # defaults (1e-8) stop a few ns short of the minimum, enough to change the record's
        # last digits (the Rust fit, app/calib, goes to the minimum: bench 5d-1).
        return least_squares(self.scaled, q0, jac_sparsity=self.sparsity(), method="trf",
                             x_scale="jac", bounds=(lo, hi), jac="3-point", ftol=1e-14, xtol=1e-14, gtol=1e-14)


def position(mics_xyh, t0, c):
    """The shot position from one spark, as the target's solver (model.py's reference)."""
    sys.path.insert(0, __file__.rsplit("/", 1)[0])
    import model
    n = len(t0)
    out = model.solve_best(np.asarray(mics_xyh, float), np.asarray(t0, float), np.full(n, 50.0),
                           c, (1 << n) - 1)
    return None if out is None else (out["x"], out["y"])


def calibrate(cfgs, recs, date, out=print, any_pt=False):
    # Calibration mode's sparks (mode=cal); with any_pt, also the simulator's other shots,
    # which carry their true point too (only right if its truth didn't change in between).
    with_pt = [r for r in recs if r["pt"] is not None]
    sparks = with_pt if any_pt else [r for r in with_pt if r["mode"] == "cal"]
    if with_pt and not sparks:
        out("calibrate: %d events have a point, but none was made in calibration mode "
            "(`cal on`); --any-pt uses them anyway" % len(with_pt))
        return None
    if not sparks:
        out("calibrate: no events with a point (pt): run the sparks in calibration mode "
            "(`cal on`, `cal pt <x> <y>`), or `sim cal <n>`")
        return None
    # The newest session's: the settings the last spark was made with (an earlier
    # calibration in the same log had other values).
    ids = {}
    for r in sparks:
        ids[r["cfg"]] = ids.get(r["cfg"], 0) + 1
    cid = sparks[-1]["cfg"]
    if cid not in cfgs:
        out("calibrate: the log has no cfg line %08X (type `mic` while logging)" % cid)
        return None
    cfg = cfgs[cid]
    chans = [i for i in range(MAX_MICS) if cfg["chans"][i] is not None]
    skipped = {"made with other settings (an earlier session)": sum(v for k, v in ids.items() if k != cid)}
    use = []
    for r in sparks:
        if r["cfg"] != cid:
            continue
        if all(r["edges"].get(i) is not None for i in chans):
            use.append(r)
        else:
            skipped["a channel without both edges"] = skipped.get("a channel without both edges", 0) + 1
    if len(use) < 4 * len(chans):
        out("calibrate: only %d usable sparks for %d channels: too few" % (len(use), len(chans)))
        return None
    n = len(chans)
    nom = np.array([cfg["chans"][i] for i in chans])  # x, y, h, k, offset, sigma
    h = nom[:, 2]
    t1 = np.array([[r["edges"][i][0] for i in chans] for r in use])
    t2 = np.array([[r["edges"][i][1] for i in chans] for r in use])
    pts = np.array([r["pt"] for r in use])
    c = np.array([r["c"] for r in use]) * 1e-6
    points = sorted({(round(p[0], 3), round(p[1], 3)) for p in pts})

    fit = Fit(h, t1, t2, pts, c)
    mx0, my0, k0, off0 = nom[:, 0], nom[:, 1], nom[:, 3], nom[:, 4]
    q0 = fit.pack(mx0, my0, k0, off0, fit.tau_for(mx0, my0, k0, off0 - off0.mean()))
    res0 = fit.raw(q0)

    first = fit.solve(q0, robust=True)
    r1 = fit.raw(first.x)
    mad = 1.4826 * np.median(np.abs(r1 - np.median(r1)))
    limit = max(5 * mad, 20.0)
    keep = np.all(np.abs(r1) <= limit, axis=1)
    outliers = int((~keep).sum())
    fit2 = Fit(h, t1[keep], t2[keep], pts[keep], c[keep])
    mx, my, k, off, tau = fit.unpack(first.x)
    second = fit2.solve(fit2.pack(mx, my, k, off, tau[keep]), robust=False)
    mx, my, k, off, tau = fit2.unpack(second.x)
    r2 = fit2.raw(second.x)
    dof = r2.size - len(second.x)
    s2 = (second.fun ** 2).sum() / max(dof, 1)
    J = second.jac.toarray() if hasattr(second.jac, "toarray") else second.jac
    cov = np.linalg.pinv(J.T @ J) * s2
    sd = np.sqrt(np.maximum(np.diag(cov), 0))
    sd_x, sd_y, sd_k = sd[:n], sd[n:2 * n], sd[2 * n:3 * n]
    # The last offset is minus the sum of the others: its variance from the covariance.
    co = cov[3 * n:4 * n - 1, 3 * n:4 * n - 1]
    sd_off = np.append(np.sqrt(np.maximum(np.diag(co), 0)), math.sqrt(max(co.sum(), 0)))
    rms = math.sqrt((r2 ** 2).mean())
    sigma = np.maximum(np.sqrt((r2 ** 2).mean(axis=0)), 5.0)

    out("calibrate: %d sparks at %d points (cfg %08X, %d channels: %s); %d left out as outliers "
        "(beyond %.0f ns); skipped: %s" % (len(use), len(points), cid, n,
                                             " ".join("CH%d" % (i + 1) for i in chans), outliers,
                                             limit, ", ".join("%d %s" % (v, k) for k, v in skipped.items() if v) or "none"))
    out("calibrate: rms residual %.1f ns with the logged values, %.1f ns after the fit" %
        (math.sqrt((res0 - res0.mean(axis=1, keepdims=True)) .__pow__(2).mean()), rms))
    out("")
    out("  ch      x mm (±1 sd)       y mm (±1 sd)        k (±1 sd)       offset ns (±1 sd)   sigma ns   change from the logged values")
    for j, i in enumerate(chans):
        out("  CH%d  %9.4f (%.4f)  %9.4f (%.4f)  %7.4f (%.4f)  %8.1f (%.1f)  %7.1f    "
            "x %+.3f y %+.3f mm, k %+.4f, offset %+.1f ns" %
            (i + 1, mx[j], sd_x[j], my[j], sd_y[j], k[j], sd_k[j], off[j], sd_off[j], sigma[j],
             mx[j] - mx0[j], my[j] - my0[j], k[j] - k0[j], off[j] - off0[j]))

    # The position error per point, before and after, through the target's solver.
    out("")
    out("  point (mm)        before: mean error, worst     after: mean error, worst")
    tot_before, tot_after = [], []
    for p in points:
        idx = [e for e in range(len(use)) if (round(pts[e][0], 3), round(pts[e][1], 3)) == p]
        errs = {}
        for name, (X, Y, K, O) in (("before", (mx0, my0, k0, off0)), ("after", (mx, my, k, off))):
            es = []
            for e in idx:
                t0 = t1[e] - K * (t2[e] - t1[e]) - O
                pos = position(np.column_stack([X, Y, h]), t0, c[e])
                if pos is not None:
                    es.append(math.hypot(pos[0] - p[0], pos[1] - p[1]))
            errs[name] = es
        tot_before += errs["before"]
        tot_after += errs["after"]
        fmt = lambda es: "%.3f, %.3f" % (np.mean(es), max(es)) if es else "-"
        out("  (%7.2f, %7.2f)   %-26s   %s" % (p[0], p[1], fmt(errs["before"]), fmt(errs["after"])))
    out("  all points:        rms %.3f mm                  rms %.3f mm" %
        (math.sqrt(np.mean(np.square(tot_before))), math.sqrt(np.mean(np.square(tot_after)))))

    # What the target will hold: the printed decimals, read as float32.
    lines, new = [], list(cfg["chans"])
    for j, i in enumerate(chans):
        vals = ("%.4f" % mx[j], "%.4f" % my[j], g9(h[j]), "%.5f" % k[j], "%.2f" % off[j], "%.1f" % sigma[j])
        lines.append("cal ch %d %s" % (i + 1, " ".join(vals)))
        new[i] = tuple(float(v) for v in vals)
    cal_id = calibration_id(new)
    lines.append("cal id %08X %s %d %d %.0f" % (cal_id, date, len(points), int(keep.sum()), rms))
    out("")
    out("Send these to the target (paste them into the serial console):")
    for l in lines:
        out(l)
    return {"chans": chans, "x": mx, "y": my, "k": k, "off": off, "sigma": sigma, "rms": rms,
            "sd_x": sd_x, "sd_y": sd_y, "sd_k": sd_k, "sd_off": sd_off,
            "sparks": int(keep.sum()), "points": len(points), "outliers": outliers,
            "lines": lines, "id": cal_id, "err_before": tot_before, "err_after": tot_after}


# ---- The self-test: a simulated calibration, as the target's simulator makes one ----

def write_json(path, res):
    import json
    chans = res["chans"]
    out = {
        "channels": [i + 1 for i in chans],
        "x": list(map(float, res["x"])), "y": list(map(float, res["y"])),
        "k": list(map(float, res["k"])), "offset": list(map(float, res["off"])),
        "sigma": list(map(float, res["sigma"])),
        "sd_x": list(map(float, res["sd_x"])), "sd_y": list(map(float, res["sd_y"])),
        "sd_k": list(map(float, res["sd_k"])), "sd_offset": list(map(float, res["sd_off"])),
        "rms": float(res["rms"]), "sparks": int(res["sparks"]), "points": int(res["points"]),
        "outliers": int(res["outliers"]), "id": "%08X" % res["id"], "lines": res["lines"],
        "err_before_rms": float(math.sqrt(np.mean(np.square(res["err_before"])))),
        "err_after_rms": float(math.sqrt(np.mean(np.square(res["err_after"])))),
    }
    if "truth" in res:
        out["truth"] = res["truth"]
    with open(path, "w") as f:
        json.dump(out, f, indent=1)


def selftest(per_point=10, seed=20261122, write_log=None, json_out=None):
    sys.path.insert(0, __file__.rsplit("/", 1)[0])
    import model
    rng = np.random.default_rng(seed)
    layout = model.LAYOUT_D2
    n = len(layout)
    chans = list(range(2, 2 + n))  # CH3-CH8, as the simulator
    nominal = [None] * MAX_MICS
    for j, i in enumerate(chans):
        nominal[i] = (layout[j][0], layout[j][1], 10.5, 0.33333, 0.0, 50.0)
    fields = fields_text(nominal) + " lim=1500 drop=1"
    cfg_id = zlib.crc32(fields.encode()) & 0xFFFFFFFF
    true_xy = layout + rng.uniform(-1.5, 1.5, layout.shape)
    true_off = rng.uniform(-500, 500, n)
    true_k = rng.uniform(0.25, 0.6, n)
    c_ms = model.sound_speed(18, 50, 101325)
    c = c_ms * 1e-6
    scale = 2 * 12000 / math.pi
    lines = ["cfg id=%08X %s" % (cfg_id, fields)]
    grid = [-75, -25, 25, 75]
    ev = 0
    for py in grid:
        for px in grid:
            for _ in range(per_point):
                ev += 1
                d = np.sqrt((px - true_xy[:, 0]) ** 2 + (py - true_xy[:, 1]) ** 2 + 10.5 ** 2)
                a = 40 * 100 / d * math.exp(rng.uniform(math.log(0.5), math.log(2)))
                L, H = 1.0, 1 + 1 / true_k
                T1 = d / c + true_off + scale * L / a
                T2 = d / c + true_off + scale * H / a
                first = T1.min()
                r1 = np.round((T1 - first) / 25) * 25
                r2 = np.round((T2 - first) / 25) * 25
                ph = rng.uniform(0, 100)
                s1 = np.ceil((r1 + ph) / 100) * 100
                s2 = np.ceil((r2 + ph) / 100) * 100
                ref = s1.min()
                edges = " ".join("CH%d=%d,%d" % (i + 1, s1[j] - ref, s2[j] - ref) for j, i in enumerate(chans))
                lines.append("rec n=%d t=%.7f cfg=%08X T=18.00 RH=50.0 p=1013.25 meas=--- c=%.9g %s "
                             "pt=%.4f,%.4f mode=cal cal=-" % (ev, ev * 0.15, cfg_id, c_ms, edges, px, py))
    if write_log:
        with open(write_log, "w") as f:
            f.write("\n".join(lines) + "\n")
    cfgs, recs = {}, []
    for l in lines:
        cc = parse_cfg(l)
        if cc:
            cfgs[cc["id"]] = cc
        r = parse_rec(l)
        if r:
            recs.append(r)
    res = calibrate(cfgs, recs, "20261122")
    if res is None:
        print("selftest: FAIL (no fit)")
        return 1
    if json_out:
        res["truth"] = {"x": true_xy[:, 0].tolist(), "y": true_xy[:, 1].tolist(), "k": true_k.tolist(),
                        "offset": (true_off - true_off.mean()).tolist()}
        write_json(json_out, res)
    ex = np.abs(res["x"] - true_xy[:, 0]).max()
    ey = np.abs(res["y"] - true_xy[:, 1]).max()
    zk = np.abs(res["k"] - true_k) / res["sd_k"]
    zo = np.abs(res["off"] - (true_off - true_off.mean())) / res["sd_off"]
    after = math.sqrt(np.mean(np.square(res["err_after"])))
    # The values are pinned down as well as the fit says (within 3.5 sd), the mics to a few
    # hundredths of a mm, and shots land as with the exact values (3d-2: 0.013 mm rms).
    ok = ex < 0.05 and ey < 0.05 and zk.max() < 3.5 and zo.max() < 3.5 and after < 0.02
    print("")
    print("selftest (%d sparks per point): against the truth: mics within %.4f / %.4f mm; k within "
          "%.4f (%.1f sd), offsets within %.1f ns (%.1f sd; mean removed); residual %.1f ns; "
          "position error after %.4f mm rms: %s" % (per_point, ex, ey,
          np.abs(res["k"] - true_k).max(), zk.max(), np.abs(res["off"] - (true_off - true_off.mean())).max(),
          zo.max(), res["rms"], after, "PASS" if ok else "FAIL"))
    return 0 if ok else 1


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("logs", nargs="*")
    ap.add_argument("--date", default=datetime.date.today().strftime("%Y%m%d"))
    ap.add_argument("--any-pt", action="store_true",
                    help="also the simulator's shots outside calibration mode (they carry their point)")
    ap.add_argument("--selftest", action="store_true")
    ap.add_argument("--sparks", type=int, default=10, help="--selftest: sparks per point")
    ap.add_argument("--seed", type=int, default=20261122, help="--selftest: random seed")
    ap.add_argument("--write-log", help="--selftest: write its simulated log here")
    ap.add_argument("--json", help="write the fit's results as JSON here")
    a = ap.parse_args()
    if a.selftest:
        return selftest(a.sparks, a.seed, a.write_log, a.json)
    if not a.logs:
        ap.error("give a log, or --selftest")
    cfgs, recs = read_logs(a.logs)
    res = calibrate(cfgs, recs, a.date, any_pt=a.any_pt)
    if res and a.json:
        write_json(a.json, res)
    return 0 if res else 1


if __name__ == "__main__":
    sys.exit(main())
