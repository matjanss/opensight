#!/usr/bin/env python3
"""OpenSights phase 1 bench analysis (MEASUREMENT.md step 7, PLAN.md phase 1).

Reads serial or app logs (`cfg` and `rec` lines, MODEL.md "Log lines"), with each event's
t0 made as the target makes it: t0 = t1 - k (t2 - t1) - offset, the settings from the
event's `cfg`. Events can be narrowed by number (--n 120-170) and to calibration mode's
(--cal).

  repeat LOG    Sparks at one point (7-3, PLAN 1.2): each channel's timing noise (1 sigma,
                from the spread of every pair's t0 difference), the scatter of the solved
                positions, and the scatter that noise predicts. Flags outliers and
                half-cycle-late trips (a channel 10-40 us late). The noise floor.
  kcheck LOG    Sparks at known points (`pt`, 7-2, PLAN 1.1): is a straight-line k right?
                Per channel, each spark's t0 against the geometry, as a function of
                t2 - t1: the k and offset that fit, and whether a curve fits clearly
                better (a curved rise). Uses the cfg's mic positions: after a calibration,
                or with the mics measured.
  verify LOG... Sparks at known points (7-4, 7-5, PLAN 1.3-1.4): the position error against
                the truth, per point and over all (bias, rms, 95 %, worst), per log (a log
                per temperature). --drop: the same with each channel left out in turn
                (what the sixth mic adds, D2).
  swap LOG --ch N
                After replacing channel N's sensor board (7-6, PLAN 1.5): sparks at known
                points with the old calibration. Refits channel N's k and offset only (the
                other channels as they are) and shows the error before and after, and the
                serial lines for the new values.
  --selftest    Synthetic logs with known noise, k, curvature and a swapped board: every
                command must find them.

Goal (PLAN): 0.2-0.5 mm, 1 sigma, after calibration.
"""
import argparse
import math
import os
import sys
import zlib

import numpy as np

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import calibrate  # noqa: E402  (the log lines' parser)
import model  # noqa: E402      (the reference solver)

GOAL_MM = (0.2, 0.5)
HALF_CYCLE_NS = (10000, 40000)   # a trip a half cycle late: about 24 us (MEASUREMENT 2)


# ---- Events ----

class Event:
    """One rec line with its cfg: per channel (index 0-7) t1, t2, t0 and the mic."""

    def __init__(self, rec, cfg):
        self.n = rec.get("n", 0)
        self.c = rec["c"] * 1e-6          # m/s -> mm/ns
        self.T = rec.get("T")
        self.pt = rec["pt"]
        self.cfg = cfg
        self.t1, self.t2, self.t0, self.mic = {}, {}, {}, {}
        for ch, e in rec["edges"].items():
            p = cfg["chans"][ch]
            if e is None or p is None:
                continue
            x, y, h, k, off, sig = p
            self.t1[ch], self.t2[ch] = e
            self.t0[ch] = e[0] - k * (e[1] - e[0]) - off
            self.mic[ch] = (x, y, h, sig)


def load(paths, nrange=None, cal_only=False, need_pt=False):
    cfgs, recs = calibrate.read_logs(paths)
    out = []
    for r in recs:
        if nrange and not (nrange[0] <= r.get("n", 0) <= nrange[1]):
            continue
        if cal_only and r.get("mode") != "cal":
            continue
        if need_pt and r["pt"] is None:
            continue
        cfg = cfgs.get(r["cfg"])
        if cfg is None:
            continue
        out.append(Event(r, cfg))
    return out


def channels(events, at_least=0.9):
    """The channels with a t0 in at least that share of the events."""
    count = {}
    for e in events:
        for ch in e.t0:
            count[ch] = count.get(ch, 0) + 1
    return sorted(ch for ch, n in count.items() if n >= at_least * len(events))


def solve(e, chans, sigma=None):
    """The position from one event's channels (the reference solver, all channels used)."""
    chans = [ch for ch in chans if ch in e.t0]
    if len(chans) < 4:
        return None
    mics = np.array([e.mic[ch][:3] for ch in chans], float)
    sig = np.array([sigma[ch] if sigma else e.mic[ch][3] for ch in chans], float)
    t0 = np.array([e.t0[ch] for ch in chans], float)
    return model.solve_best(mics, t0, sig, e.c, (1 << len(chans)) - 1, truth=e.pt)


def name(ch):
    return "CH%d" % (ch + 1)


def robust_sd(v):
    v = np.asarray(v, float)
    return 1.4826 * np.median(np.abs(v - np.median(v)))


# ---- repeat ----

def repeat(events, out=print):
    chans = channels(events)
    if len(events) < 10 or len(chans) < 4:
        out("repeat: needs 10 or more events with 4 or more channels (have %d, %d)"
            % (len(events), len(chans)))
        return None
    ref = chans[0]
    full = [e for e in events if all(ch in e.t0 for ch in chans)]
    # Outliers: a channel far from its median difference to the reference.
    d = {ch: np.array([e.t0[ch] - e.t0[ref] for e in full]) for ch in chans}
    bad = set()
    for ch in chans[1:]:
        med, sd = np.median(d[ch]), max(robust_sd(d[ch]), 20.0)
        for i, v in enumerate(d[ch]):
            if abs(v - med) > max(8 * sd, 1000):
                bad.add(i)
                late = HALF_CYCLE_NS[0] <= abs(v - med) <= HALF_CYCLE_NS[1]
                out("  outlier: event n=%d, %s %+.0f ns against its median%s" % (
                    full[i].n, name(ch), v - med,
                    " (a half-cycle-late trip? MEASUREMENT 2: polarity, thresholds)" if late else ""))
    good = [e for i, e in enumerate(full) if i not in bad]
    out("repeat: %d events, %d used (%d with an outlier, %d with a channel missing); "
        "channels %s" % (len(events), len(good), len(bad), len(events) - len(full),
                         " ".join(name(c) for c in chans)))
    if len(good) < 10:
        out("repeat: too few clean events")
        return None
    T0 = np.array([[e.t0[ch] for ch in chans] for e in good])
    U = np.array([[e.t2[ch] - e.t1[ch] for ch in chans] for e in good])
    # Each channel's own noise: var(t0_i - t0_j) = s_i^2 + s_j^2 over every pair.
    rows, rhs = [], []
    n = len(chans)
    for i in range(n):
        for j in range(i + 1, n):
            r = np.zeros(n)
            r[i] = r[j] = 1
            rows.append(r)
            rhs.append(np.var(T0[:, i] - T0[:, j], ddof=1))
    s2, *_ = np.linalg.lstsq(np.array(rows), np.array(rhs), rcond=None)
    sig = np.sqrt(np.clip(s2, 0, None))
    out("")
    out("  channel  t0 - %s: mean     sd   | own noise | t2 - t1: mean     sd" % name(ref))
    for i, ch in enumerate(chans):
        dd = T0[:, i] - T0[:, 0]
        out("  %-7s %10.1f %7.1f ns  | %6.1f ns | %10.1f %7.1f ns" % (
            name(ch), dd.mean(), dd.std(ddof=1), sig[i], U[:, i].mean(), U[:, i].std(ddof=1)))
    # Positions: measured scatter, and what the channels' noise predicts.
    pos = [solve(e, chans) for e in good]
    xy = np.array([(p["x"], p["y"]) for p in pos if p is not None])
    sx, sy = xy[:, 0].std(ddof=1), xy[:, 1].std(ddof=1)
    cov = np.cov(xy.T)
    major = math.sqrt(np.linalg.eigvalsh(cov).max())
    mx, my = xy.mean(axis=0)
    e0 = good[0]
    mics = np.array([e0.mic[ch][:3] for ch in chans], float)
    c = np.mean([e.c for e in good])
    t0 = model.arrivals(mics, mx, my, 0.0, c)
    pred = model.outputs(mics, t0, np.maximum(sig, 1.0), c, (1 << n) - 1, mx, my, 0.0, t0.min())
    out("")
    out("  position: mean (%.2f, %.2f) mm, sd x %.3f, y %.3f mm, major axis %.3f mm (1 sigma)"
        % (mx, my, sx, sy, major))
    out("  from the channels' noise: sd x %.3f, y %.3f mm, major axis %.3f mm" % (
        pred["sx"], pred["sy"], pred["major"]))
    if e0.pt is not None:
        out("  against the point (%.2f, %.2f): bias (%+.3f, %+.3f) mm (before calibration: "
            "expected)" % (e0.pt[0], e0.pt[1], mx - e0.pt[0], my - e0.pt[1]))
    verdict = ("within the goal's best (%.1f mm)" % GOAL_MM[0] if major <= GOAL_MM[0] else
               "within the goal (%.1f mm)" % GOAL_MM[1] if major <= GOAL_MM[1] else
               "OVER the goal (%.1f mm)" % GOAL_MM[1])
    out("  noise floor: %s" % verdict)
    return {"chans": chans, "sigma": dict(zip(chans, sig)), "sx": sx, "sy": sy, "major": major,
            "pred_major": pred["major"], "outliers": len(bad), "n": len(good)}


# ---- kcheck ----

def residuals(events, chans, k=None, off=None):
    """Per event and channel: t0 (with k, off per channel if given) minus the geometry,
    with each event's emission time taken out (its weighted mean over the channels)."""
    R = np.full((len(events), len(chans)), np.nan)
    U = np.full_like(R, np.nan)
    for a, e in enumerate(events):
        r = []
        for b, ch in enumerate(chans):
            if ch not in e.t0:
                continue
            u = e.t2[ch] - e.t1[ch]
            t0 = e.t0[ch] if k is None else e.t1[ch] - k[b] * u - off[b]
            x, y, h, _ = e.mic[ch]
            dist = math.sqrt((e.pt[0] - x) ** 2 + (e.pt[1] - y) ** 2 + h * h)
            R[a, b] = t0 - dist / e.c
            U[a, b] = u
        R[a] -= np.nanmean(R[a])
    return R, U


SAMPLE_NS = 100.0       # the capture's sampling step (10 MHz)


def kcheck(events, out=print, jitter=0.0):
    """jitter: each edge's own timing noise, ns (1 sigma, beyond the sampling)."""
    chans = channels(events, 0.5)
    if len(events) < 20 or len(chans) < 4:
        out("kcheck: needs 20 or more sparks at known points (`pt`) on 4 or more channels")
        return None
    k0 = np.array([events[0].cfg["chans"][ch][3] for ch in chans])
    off0 = np.array([events[0].cfg["chans"][ch][4] for ch in chans])
    k, off = k0.copy(), off0.copy()
    # Fit k and the offset per channel against the geometry, a few rounds (the emission
    # time of each spark comes from all channels).
    for _ in range(8):
        R, U = residuals(events, chans, k, off)
        for b in range(len(chans)):
            ok = ~np.isnan(R[:, b])
            A = np.column_stack([U[ok, b], np.ones(ok.sum())])
            (dk, doff), *_ = np.linalg.lstsq(A, R[ok, b], rcond=None)
            k[b] += dk
            off[b] += doff
        off -= off.mean()     # a delay common to all channels is the emission time's
    # Each edge's error (sampling, jitter) is in t1 and in the measured t2 - t1 alike, so
    # the line's slope comes out low by e2 (2k + 1) / var(t2 - t1), e2 one edge's error
    # variance. Take that out: the slope s = k - e2 (2k + 1) / V gives
    # k = (s V + e2) / (V - 2 e2). The offsets follow from the mean.
    e2 = SAMPLE_NS ** 2 / 12 + jitter ** 2
    R, U = residuals(events, chans, k, off)
    for b in range(len(chans)):
        ok = ~np.isnan(R[:, b])
        V = U[ok, b].var()
        if V > 4 * e2:
            kc = (k[b] * V + e2) / (V - 2 * e2)
            off[b] -= (kc - k[b]) * U[ok, b].mean()
            k[b] = kc
    off -= off.mean()
    R, U = residuals(events, chans, k, off)
    out("kcheck: %d sparks at %d points; channels %s" % (
        len(events), len({e.pt for e in events}), " ".join(name(c) for c in chans)))
    out("")
    out("  channel  k (cfg)   k (fit) +- sd     offset (fit) | t2 - t1 range | rms: line   curve | curve")
    res = {}
    for b, ch in enumerate(chans):
        ok = ~np.isnan(R[:, b])
        u, r = U[ok, b], R[ok, b]
        lin = math.sqrt(np.mean(r ** 2))
        # k's own uncertainty: the slope's, from the spread of t2 - t1 and the residuals.
        rr = r - r.mean() - np.polyfit(u, r, 1)[0] * (u - u.mean())
        sd_k = math.sqrt((rr @ rr) / max(len(r) - 2, 1) / max(((u - u.mean()) ** 2).sum(), 1e-9))
        uc = (u - u.mean()) / max(u.std(), 1e-9)
        A = np.column_stack([np.ones_like(uc), uc, uc ** 2])
        coef, *_ = np.linalg.lstsq(A, r, rcond=None)
        q = r - A @ coef
        quad = math.sqrt(np.mean(q ** 2))
        # The curve's term against its own uncertainty.
        dof = max(len(r) - 3, 1)
        s2 = (q @ q) / dof
        cov = s2 * np.linalg.inv(A.T @ A)
        z = abs(coef[2]) / math.sqrt(cov[2, 2])
        curved = z > 5 and lin - quad > 5
        res[ch] = {"k": k[b], "sd_k": sd_k, "off": off[b], "lin": lin, "quad": quad, "z": z,
                   "curved": curved}
        out("  %-7s %7.4f   %7.4f +- %.4f %+9.1f ns   | %5.0f-%5.0f ns | %6.1f %7.1f ns | %s" % (
            name(ch), k0[b], k[b], sd_k, off[b], u.min(), u.max(), lin, quad,
            "CURVED (z %.0f): a straight-line k leaves %.0f ns" % (z, lin) if curved else "straight"))
    spread = k.max() - k.min()
    sds = np.array([res[ch]["sd_k"] for ch in chans])
    # Do the boards differ? Their k against a common k, in units of their uncertainty.
    kw = np.sum(k / sds ** 2) / np.sum(1 / sds ** 2)
    chi2 = float(np.sum(((k - kw) / sds) ** 2))
    differ = chi2 > len(chans) - 1 + 4 * math.sqrt(2 * (len(chans) - 1))
    # With t2 - t1 varying by less than about 1 us the comparison misfires (the self-test's
    # short-range runs: chi2 up to 31 for equal boards): no verdict then.
    usd = float(np.median([np.nanstd(U[:, b]) for b in range(len(chans))]))
    if usd < 10 * SAMPLE_NS:
        differ = None
    out("")
    out("  the boards' k: %.4f to %.4f (spread %.4f; a k off by 0.01 moves t0 by 1 %% of t2 - t1)"
        % (k.min(), k.max(), spread))
    if differ is None:
        out("  no verdict on the boards' k: t2 - t1 varies by only %.0f ns (1 sigma), under 1 us. "
            "Sparks nearer to and farther from the mics (or a gain change) widen it" % usd)
    else:
        out("  %s (chi2 %.1f for %d channels)" % (
            "the boards' k DIFFER: keep k per channel (PLAN D3)" if differ else
            "the boards' k agree within their uncertainty: the spread is noise, not the boards",
            chi2, len(chans)))
    if np.median(sds) > 0.01:
        out("  k is only known to about %.3f here: sparks over a wider range of t2 - t1 (nearer "
            "and farther from the mics) pin it down" % np.median(sds))
    for ch in res:
        res[ch]["differ"] = differ
    return res


# ---- verify ----

def errors(events, chans, sigma=None):
    out = []
    for e in events:
        p = solve(e, chans, sigma)
        if p is not None:
            out.append((e, p["x"] - e.pt[0], p["y"] - e.pt[1]))
    return out


def summary(errs):
    ex = np.array([a for _, a, _ in errs])
    ey = np.array([b for _, _, b in errs])
    r = np.hypot(ex, ey)
    return {"n": len(r), "bx": ex.mean(), "by": ey.mean(), "rms": math.sqrt(np.mean(r ** 2)),
            "p95": np.percentile(r, 95), "worst": r.max(),
            "sd1": math.sqrt((ex.var() + ey.var()) / 2)}


def verify(paths, nrange=None, cal_only=False, drop=False, out=print, logs=None):
    results = []
    for path in paths:
        events = logs[path] if logs else load([path], nrange, cal_only, need_pt=True)
        chans = channels(events, 0.5)
        if len(events) < 5 or len(chans) < 4:
            out("verify %s: needs sparks at known points (`pt`) on 4 or more channels" % path)
            continue
        errs = errors(events, chans)
        s = summary(errs)
        T = [e.T for e in events if e.T is not None]
        out("verify %s: %d sparks, %d points, %s, air %.1f-%.1f C" % (
            os.path.basename(path), s["n"], len({e.pt for e in events}),
            " ".join(name(c) for c in chans), min(T), max(T)))
        out("  bias (%+.3f, %+.3f) mm; error rms %.3f mm, 95 %% %.3f, worst %.3f; 1 sigma per axis "
            "%.3f mm" % (s["bx"], s["by"], s["rms"], s["p95"], s["worst"], s["sd1"]))
        verdict = ("within the goal's best" if s["sd1"] <= GOAL_MM[0] else
                   "within the goal" if s["sd1"] <= GOAL_MM[1] else "OVER the goal")
        out("  %s (%.1f-%.1f mm, 1 sigma)" % (verdict, *GOAL_MM))
        out("    point           n   bias x   bias y    sd     worst (mm)")
        by_pt = {}
        for e, a, b in errs:
            by_pt.setdefault(e.pt, []).append((e, a, b))
        for pt in sorted(by_pt, key=lambda p: (p[1], p[0])):
            p = summary(by_pt[pt])
            out("    (%6.1f,%6.1f) %3d %+8.3f %+8.3f %7.3f %7.3f" % (
                pt[0], pt[1], p["n"], p["bx"], p["by"], p["sd1"], p["worst"]))
        res = {"path": path, **s, "drop": {}}
        if drop and len(chans) >= 5:
            out("  with one channel left out:")
            for ch in chans:
                d = summary(errors(events, [c for c in chans if c != ch]))
                res["drop"][ch] = d
                out("    without %s: rms %.3f mm, worst %.3f (all: %.3f, %.3f)" % (
                    name(ch), d["rms"], d["worst"], s["rms"], s["worst"]))
        results.append(res)
    if len(results) > 1:
        out("")
        out("  across the logs: bias moves %.3f mm, rms from %.3f to %.3f mm" % (
            max(math.hypot(a["bx"] - b["bx"], a["by"] - b["by"]) for a in results for b in results),
            min(r["rms"] for r in results), max(r["rms"] for r in results)))
    return results


# ---- swap ----

def swap(events, ch, out=print):
    chans = channels(events, 0.5)
    if ch not in chans or len(chans) < 5 or len(events) < 10:
        out("swap: needs 10 or more sparks at known points with %s and 4 other channels" % name(ch))
        return None
    before = summary(errors(events, chans))
    # Channel ch's k and offset from the others: each spark's emission time from the other
    # channels (as they are), then t1 - emission - distance / c = k (t2 - t1) + offset.
    ys, us = [], []
    for e in events:
        if ch not in e.t0:
            continue
        taus = []
        for o in chans:
            if o == ch or o not in e.t0:
                continue
            x, y, h, _ = e.mic[o]
            taus.append(e.t0[o] - math.sqrt((e.pt[0] - x) ** 2 + (e.pt[1] - y) ** 2 + h * h) / e.c)
        x, y, h, _ = e.mic[ch]
        dist = math.sqrt((e.pt[0] - x) ** 2 + (e.pt[1] - y) ** 2 + h * h)
        ys.append(e.t1[ch] - np.mean(taus) - dist / e.c)
        us.append(e.t2[ch] - e.t1[ch])
    A = np.column_stack([us, np.ones(len(us))])
    (k, off), *_ = np.linalg.lstsq(A, np.array(ys), rcond=None)
    old = events[0].cfg["chans"][ch]
    for e in events:
        if ch in e.t0:
            e.t0[ch] = e.t1[ch] - k * (e.t2[ch] - e.t1[ch]) - off
    after = summary(errors(events, chans))
    out("swap %s: %d sparks at %d points" % (name(ch), len(events), len({e.pt for e in events})))
    out("  k %.4f -> %.4f, offset %+.1f -> %+.1f ns (the other channels as they are)" % (
        old[3], k, old[4], off))
    out("  error rms %.3f -> %.3f mm, worst %.3f -> %.3f mm, bias (%+.3f, %+.3f) -> (%+.3f, %+.3f)"
        % (before["rms"], after["rms"], before["worst"], after["worst"], before["bx"], before["by"],
           after["bx"], after["by"]))
    out("  to the target: `k %d %.5f` and `offset %d %.1f` (the calibration record goes stale: "
        "calibrate again when convenient)" % (ch + 1, k, ch + 1, off))
    return {"k": k, "off": off, "before": before, "after": after}


# ---- Synthetic logs for the self-test ----

def synth(path, points, per_point, true_k, true_off, noise, cfg_k=None, cfg_off=None,
          curve=None, temp=18.0, seed=1, n0=1, amp=6.0, tick=0.0):
    """A log of sparks: true k, offsets (ns) and timing noise (ns, 1 sigma) per channel; the
    cfg with cfg_k / cfg_off (default: the truth). curve: channels whose pulse rises like
    a sine (the straight-line k doesn't fit). Edges sampled at 100 ns, as the target."""
    rng = np.random.default_rng(seed)
    layout = model.LAYOUT_D2
    n = len(layout)
    chans = list(range(n))                     # CH1-CH6, as `mic default`
    h = 5.5
    cfg_k = true_k if cfg_k is None else cfg_k
    cfg_off = true_off if cfg_off is None else cfg_off
    # amp: the amplitude scale (6: t2 - t1 about 1-10 us; the simulator's amp 40 about 0.1-1.6);
    # tick: edges on a grid (ns), as the simulator's 25 ns (0: continuous, as real edges).
    cfgc = [None] * calibrate.MAX_MICS
    for j in chans:
        cfgc[j] = (layout[j][0], layout[j][1], h, cfg_k[j], cfg_off[j], 50.0)
    fields = calibrate.fields_text(cfgc) + " lim=1500 drop=1"
    cid = zlib.crc32(fields.encode()) & 0xFFFFFFFF
    c_ms = model.sound_speed(temp, 50, 101325)
    c = c_ms * 1e-6
    scale = 2 * 12000 / math.pi              # ns: the pulse's rise, as the simulator's
    lines = ["cfg id=%08X %s" % (cid, fields)]
    ev = n0
    for px, py in points:
        for _ in range(per_point):
            d = np.sqrt((px - layout[:, 0]) ** 2 + (py - layout[:, 1]) ** 2 + h * h)
            # Amplitude in threshold units (L = 1): about 4-20 at the mics, so t2 - t1 is
            # about 1-10 us, as the summary expects of the boards.
            a = amp * 100 / d * math.exp(rng.uniform(math.log(0.6), math.log(1.6)))
            T1, T2 = np.empty(n), np.empty(n)
            arrive = d / c + true_off + rng.normal(0, 1, n) * noise
            for j in range(n):              # a: each mic's amplitude (it falls with distance)
                L, H = 1.0, 1 + 1 / true_k[j]
                if curve and j in curve:       # A sin(pi t / 2T): trips later as it bends
                    # From barely over the high threshold to 5 times it: where the rise's
                    # bend shows (MEASUREMENT 2b, `sim model sine`).
                    aa = H / rng.uniform(0.2, 0.95)
                    T1[j] = arrive[j] + scale * math.asin(L / aa)
                    T2[j] = arrive[j] + scale * math.asin(H / aa)
                else:
                    T1[j] = arrive[j] + scale * L / a[j]
                    T2[j] = arrive[j] + scale * H / a[j]
            if tick:
                T1, T2 = np.round(T1 / tick) * tick, np.round(T2 / tick) * tick
            ph = rng.uniform(0, 100)
            s1 = np.ceil((T1 + ph) / 100) * 100
            s2 = np.ceil((T2 + ph) / 100) * 100
            ref = s1.min()
            edges = " ".join("CH%d=%d,%d" % (j + 1, s1[j] - ref, s2[j] - ref) for j in chans)
            lines.append("rec n=%d t=%.7f cfg=%08X T=%.2f RH=50.0 p=1013.25 meas=--- c=%.9g %s "
                         "pt=%.4f,%.4f cal=-" % (ev, ev * 0.2, cid, temp, c_ms, edges, px, py))
            ev += 1
    with open(path, "w") as f:
        f.write("\n".join(lines) + "\n")


def selftest(tmp):
    ok = True
    n = 6
    rng = np.random.default_rng(20261124)
    true_k = rng.uniform(0.28, 0.40, n)
    true_off = rng.uniform(-400, 400, n)
    true_off -= true_off.mean()
    noise = np.array([20, 30, 40, 25, 35, 50], float)
    grid = [(x, y) for y in (-75, -25, 25, 75) for x in (-75, -25, 25, 75)]
    lines = []

    def say(s=""):
        lines.append(s)
        print(s)

    # repeat: 200 sparks at one point. The channels' noise (with the 100 ns sampling) must
    # come back within 15 %, and the position scatter as predicted within 25 %.
    p = os.path.join(tmp, "repeat.log")
    synth(p, [(20, -30)], 200, true_k, true_off, noise, seed=2)
    r = repeat(load([p]), out=say)
    # The 100 ns sampling adds about 100/sqrt(12) (1 + 2k)-ish: measure it from a noiseless run.
    q = os.path.join(tmp, "repeat0.log")
    synth(q, [(20, -30)], 400, true_k, true_off, np.zeros(n), seed=3)
    r0 = repeat(load([q]), out=lambda s: None)
    expect = np.sqrt(noise ** 2 + np.array([r0["sigma"][ch] for ch in range(n)]) ** 2)
    got = np.array([r["sigma"][ch] for ch in range(n)])
    rel = np.abs(got / expect - 1).max()
    pos = abs(r["major"] / r["pred_major"] - 1)
    good = rel < 0.15 and pos < 0.25 and r["outliers"] == 0
    ok &= good
    say("selftest repeat: noise per channel within %.0f %% of the truth (with sampling), position "
        "scatter within %.0f %% of the prediction: %s" % (rel * 100, pos * 100, "PASS" if good else "FAIL"))

    # repeat with a half-cycle-late trip: found.
    p = os.path.join(tmp, "late.log")
    synth(p, [(20, -30)], 40, true_k, true_off, noise, seed=4)
    txt = open(p).read().splitlines()
    i = 20
    parts = txt[i].split()
    for j, f in enumerate(parts):
        if f.startswith("CH3="):
            a, b = (int(v) for v in f[4:].split(","))
            parts[j] = "CH3=%d,%d" % (a + 24000, b + 24000)
    txt[i] = " ".join(parts)
    open(p, "w").write("\n".join(txt) + "\n")
    seen = []
    r = repeat(load([p]), out=seen.append)
    good = r is not None and r["outliers"] == 1 and any("half-cycle" in s for s in seen)
    ok &= good
    say("selftest repeat, a trip 24 us late: found and left out: %s" % ("PASS" if good else "FAIL"))

    # kcheck: the cfg's k all 1/3 and offsets 0; the fit must find the true k and offsets
    # (offsets with their mean removed), and one channel with a curved rise.
    p = os.path.join(tmp, "kcheck.log")
    synth(p, grid, 15, true_k, true_off, noise, cfg_k=np.full(n, 1 / 3), cfg_off=np.zeros(n),
          curve={4}, seed=5)
    k = kcheck(load([p], need_pt=True), out=say)
    kerr = max(abs(k[ch]["k"] - true_k[ch]) for ch in range(n) if ch != 4)
    oerr = max(abs(k[ch]["off"] - true_off[ch]) for ch in range(n) if ch != 4)
    curved = [ch for ch in range(n) if k[ch]["curved"]]
    good = kerr < 0.01 and oerr < 60 and curved == [4] and k[0]["differ"] is True
    ok &= good
    say("selftest kcheck: k within %.4f, offsets within %.0f ns (straight channels); curved: %s; "
        "the boards differ: %s" % (kerr, oerr, " ".join(name(c) for c in curved) or "none",
                                   "PASS" if good else "FAIL"))
    # Equal boards: they must agree (t2 - t1 over 1-10 us), and over the simulator's short
    # range (0.1-1.6 us) no verdict at all.
    agree, short = [], []
    for seed in range(5):
        p = os.path.join(tmp, "equal.log")
        synth(p, grid, 10, np.full(n, 1 / 3), np.zeros(n), np.full(n, 30.0), seed=20 + seed)
        agree.append(kcheck(load([p], need_pt=True), out=lambda s: None)[0]["differ"])
        synth(p, grid, 10, np.full(n, 1 / 3), np.zeros(n), np.full(n, 30.0), seed=30 + seed,
              amp=40, tick=25)
        short.append(kcheck(load([p], need_pt=True), out=lambda s: None)[0]["differ"])
    good = agree.count(False) >= 4 and all(v is None for v in short)
    ok &= good
    say("selftest kcheck, equal boards: agree in %d of 5 runs; over a short range no verdict in "
        "%d of 5: %s" % (agree.count(False), short.count(None), "PASS" if good else "FAIL"))

    # verify: the exact calibration, 30 ns noise: the error as the noise predicts; a log
    # 2 C warmer with the air right: the same; --drop: every channel left out costs.
    p1 = os.path.join(tmp, "verify18.log")
    p2 = os.path.join(tmp, "verify20.log")
    synth(p1, grid, 10, true_k, true_off, np.full(n, 30.0), seed=6)
    synth(p2, grid, 10, true_k, true_off, np.full(n, 30.0), temp=20.0, seed=7)
    v = verify([p1, p2], drop=True, out=say)
    worse = all(v[0]["drop"][ch]["rms"] > v[0]["rms"] for ch in range(n))
    good = (len(v) == 2 and all(x["sd1"] < 0.15 and math.hypot(x["bx"], x["by"]) < 0.03 for x in v)
            and worse)
    ok &= good
    say("selftest verify: 1 sigma %.3f / %.3f mm, bias %.3f / %.3f mm, every channel left out "
        "costs: %s" % (v[0]["sd1"], v[1]["sd1"], math.hypot(v[0]["bx"], v[0]["by"]),
                       math.hypot(v[1]["bx"], v[1]["by"]), "PASS" if good else "FAIL"))

    # swap: CH4's board replaced (k and offset changed), the old calibration in the cfg.
    new_k, new_off = true_k.copy(), true_off.copy()
    new_k[3], new_off[3] = true_k[3] + 0.08, true_off[3] + 700
    p = os.path.join(tmp, "swap.log")
    synth(p, grid, 10, new_k, new_off, np.full(n, 30.0), cfg_k=true_k, cfg_off=true_off, seed=8)
    s = swap(load([p], need_pt=True), 3, out=say)
    good = (abs(s["k"] - new_k[3]) < 0.01 and abs(s["off"] - new_off[3]) < 60
            and s["before"]["rms"] > 3 * s["after"]["rms"] and s["after"]["sd1"] < 0.15)
    ok &= good
    say("selftest swap: k %.4f (true %.4f), offset %.0f ns (true %.0f); rms %.3f -> %.3f mm: %s" % (
        s["k"], new_k[3], s["off"], new_off[3], s["before"]["rms"], s["after"]["rms"],
        "PASS" if good else "FAIL"))
    say("selftest: %s" % ("PASS" if ok else "FAIL"))
    return 0 if ok else 1


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("cmd", nargs="?", choices=["repeat", "kcheck", "verify", "swap"])
    ap.add_argument("logs", nargs="*")
    ap.add_argument("--n", help="events n=A-B only")
    ap.add_argument("--cal", action="store_true", help="calibration mode's events only")
    ap.add_argument("--drop", action="store_true", help="verify: each channel left out in turn")
    ap.add_argument("--ch", type=int, help="swap: the channel whose board was replaced (1-8)")
    ap.add_argument("--jitter", type=float, default=0.0,
                    help="kcheck: each edge's own noise beyond the sampling, ns (from repeat)")
    ap.add_argument("--selftest", action="store_true")
    a = ap.parse_args()
    if a.selftest:
        import tempfile
        with tempfile.TemporaryDirectory() as tmp:
            return selftest(tmp)
    if not a.cmd or not a.logs:
        ap.error("a command and a log")
    nr = tuple(int(v) for v in a.n.split("-")) if a.n else None
    if a.cmd == "repeat":
        return 0 if repeat(load(a.logs, nr, a.cal)) else 1
    if a.cmd == "kcheck":
        return 0 if kcheck(load(a.logs, nr, a.cal, need_pt=True), jitter=a.jitter) else 1
    if a.cmd == "verify":
        return 0 if verify(a.logs, nr, a.cal, a.drop) else 1
    if a.cmd == "swap":
        if not a.ch:
            ap.error("swap needs --ch")
        return 0 if swap(load(a.logs, nr, a.cal, need_pt=True), a.ch - 1) else 1


if __name__ == "__main__":
    sys.exit(main())
