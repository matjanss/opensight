//! The target's position solver (`firmware/lib/measure/solver.cpp`, MODEL.md "Solver"),
//! ported line for line, in single precision like the target.
//!
//! Unknowns: the hole (x, y) and the emission time tau. Mic i's residual is
//! r_i = t0_i - tau - d_i / c, d_i = sqrt((x - x_i)^2 + (y - y_i)^2 + h_i^2), fitted by
//! weighted least squares (weights 1 / sigma_i^2) with Levenberg-Marquardt. Internally every
//! time is a range in mm relative to the first arrival, so float's digits leave 0.0001 mm.

// A line-for-line port of solver.cpp: its indexed loops, and its negated comparisons, which
// also reject NaN as the C++ does.
#![allow(clippy::needless_range_loop, clippy::neg_cmp_op_on_partial_ord)]

use crate::MAX_MICS;

pub const SOLVE_MAX_ITER: usize = 15;
pub const SOLVE_DONE_MM: f32 = 0.001;
pub const SOLVE_FAR_MM: f32 = 2000.0;

/// One mic: its position in the paper plane and its height behind the paper, mm.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Mic {
    pub x: f32,
    pub y: f32,
    pub h: f32,
}

/// The solver's input (`measure::SolveInput`).
#[derive(Clone, Debug, Default)]
pub struct SolveInput {
    /// Mics 0..n-1.
    pub n: usize,
    pub mic: [Mic; MAX_MICS],
    /// Arrival times, ns, any common origin.
    pub t0: [f32; MAX_MICS],
    /// 1σ timing error per mic, ns (> 0).
    pub sigma_ns: [f32; MAX_MICS],
    /// The speed of sound, mm/ns.
    pub c: f32,
}

/// The solver's output (`measure::Solution`).
#[derive(Clone, Debug, Default)]
pub struct Solution {
    pub converged: bool,
    pub iterations: u8,
    pub starts: u8,
    /// Bit i: mic i was in the fit.
    pub used: u8,
    /// The mic drop-one left out, -1 if none.
    pub dropped: i8,
    pub x: f32,
    pub y: f32,
    pub tau_ns: f32,
    pub rms_ns: f32,
    pub res_ns: [f32; MAX_MICS],
    pub sx: f32,
    pub sy: f32,
    pub rho: f32,
    pub major: f32,
}

struct Work {
    n: usize,
    mask: u8,
    mx: [f32; MAX_MICS],
    my: [f32; MAX_MICS],
    h2: [f32; MAX_MICS],
    s: [f32; MAX_MICS],
    w: [f32; MAX_MICS],
    wsum: f32,
}

impl Work {
    fn used(&self, i: usize) -> bool {
        (self.mask >> i) & 1 != 0
    }

    /// Distances, the best range offset D (weighted mean of s - d) and the weighted cost.
    fn eval_at(&self, x: f32, y: f32, d: &mut [f32; MAX_MICS]) -> (f32, f32) {
        let mut swr = 0.0f32;
        for i in 0..self.n {
            let (dx, dy) = (x - self.mx[i], y - self.my[i]);
            d[i] = (dx * dx + dy * dy + self.h2[i]).sqrt();
            if d[i] < 1e-3 {
                d[i] = 1e-3;
            }
            if self.used(i) {
                swr += self.w[i] * (self.s[i] - d[i]);
            }
        }
        let dd = swr / self.wsum;
        let mut cost = 0.0f32;
        for i in 0..self.n {
            if self.used(i) {
                let r = d[i] + dd - self.s[i];
                cost += self.w[i] * r * r;
            }
        }
        (cost, dd)
    }

    /// The normal matrix (upper triangle a11 a12 a13 a22 a23 a33) and the gradient.
    fn normal(&self, x: f32, y: f32, d: &[f32; MAX_MICS], dd: f32) -> ([f32; 6], [f32; 3]) {
        let mut a = [0.0f32; 6];
        let mut g = [0.0f32; 3];
        for i in 0..self.n {
            if !self.used(i) {
                continue;
            }
            let jx = (x - self.mx[i]) / d[i];
            let jy = (y - self.my[i]) / d[i];
            let r = d[i] + dd - self.s[i];
            let w = self.w[i];
            a[0] += w * jx * jx;
            a[1] += w * jx * jy;
            a[2] += w * jx;
            a[3] += w * jy * jy;
            a[4] += w * jy;
            a[5] += w;
            g[0] += w * jx * r;
            g[1] += w * jy * r;
            g[2] += w * r;
        }
        (a, g)
    }

    /// Levenberg-Marquardt from (x, y). True if converged; (x, y) the last accepted point.
    fn run_from(&self, x: &mut f32, y: &mut f32, iterations: &mut u32) -> bool {
        let mut d = [0.0f32; MAX_MICS];
        let (mut cost, mut dd) = self.eval_at(*x, *y, &mut d);
        let mut lambda = 1e-3f32;
        for _ in 0..SOLVE_MAX_ITER {
            *iterations += 1;
            let (a, g) = self.normal(*x, *y, &d, dd);
            let ad = [
                a[0] * (1.0 + lambda),
                a[1],
                a[2],
                a[3] * (1.0 + lambda),
                a[4],
                a[5] * (1.0 + lambda),
            ];
            let Some(inv) = invert3(&ad) else { return false };
            let sx = -(inv[0] * g[0] + inv[1] * g[1] + inv[2] * g[2]);
            let sy = -(inv[1] * g[0] + inv[3] * g[1] + inv[4] * g[2]);
            let (nx, ny) = (*x + sx, *y + sy);
            if !(nx.abs() < SOLVE_FAR_MM && ny.abs() < SOLVE_FAR_MM) {
                return false;
            }
            let small = sx * sx + sy * sy < SOLVE_DONE_MM * SOLVE_DONE_MM;
            let mut nd = [0.0f32; MAX_MICS];
            let (ncost, n_dd) = self.eval_at(nx, ny, &mut nd);
            // A tiny step is taken even if the cost doesn't drop: at the minimum the cost is
            // flat to float's precision, while the step (from the gradient) is still accurate.
            if ncost <= cost || small {
                *x = nx;
                *y = ny;
                cost = ncost;
                dd = n_dd;
                d = nd;
                lambda = (lambda * 0.1).max(1e-6);
                if small {
                    return true;
                }
            } else {
                lambda *= 10.0;
            }
        }
        false
    }
}

/// Inverse of a symmetric 3x3 (same packing); None if singular (mics in a line).
fn invert3(a: &[f32; 6]) -> Option<[f32; 6]> {
    let (a11, a12, a13, a22, a23, a33) = (a[0], a[1], a[2], a[3], a[4], a[5]);
    let c11 = a22 * a33 - a23 * a23;
    let c12 = a13 * a23 - a12 * a33;
    let c13 = a12 * a23 - a13 * a22;
    let det = a11 * c11 + a12 * c12 + a13 * c13;
    let scale = a11 * a22 * a33;
    if !(det.abs() > 1e-7 * scale.abs()) || !(scale > 0.0) {
        return None;
    }
    let c22 = a11 * a33 - a13 * a13;
    let c23 = a12 * a13 - a11 * a23;
    let c33 = a11 * a22 - a12 * a12;
    Some([c11 / det, c12 / det, c13 / det, c22 / det, c23 / det, c33 / det])
}

/// Fits the mics in `mask` (bit i = mic i; at least 3, 4 or more to have a residual).
pub fn solve(input: &SolveInput, mask: u8) -> Solution {
    let mut out = Solution {
        dropped: -1,
        ..Default::default()
    };
    let n = input.n.min(MAX_MICS);
    let mut k = Work {
        n,
        mask: mask & ((1u16 << n) - 1) as u8,
        mx: [0.0; MAX_MICS],
        my: [0.0; MAX_MICS],
        h2: [0.0; MAX_MICS],
        s: [0.0; MAX_MICS],
        w: [0.0; MAX_MICS],
        wsum: 0.0,
    };
    let mut used = 0;
    let mut first: Option<usize> = None;
    let mut sigma_ref = 0.0f32;
    for i in 0..n {
        if !k.used(i) {
            continue;
        }
        if !(input.sigma_ns[i] > 0.0) {
            k.mask &= !(1 << i);
            continue;
        }
        used += 1;
        if first.is_none_or(|f| input.t0[i] < input.t0[f]) {
            first = Some(i);
        }
        if sigma_ref == 0.0 || input.sigma_ns[i] < sigma_ref {
            sigma_ref = input.sigma_ns[i];
        }
    }
    out.used = k.mask;
    let Some(first) = first else { return out };
    if used < 3 || !(input.c > 0.0) {
        return out;
    }
    for i in 0..n {
        k.mx[i] = input.mic[i].x;
        k.my[i] = input.mic[i].y;
        k.h2[i] = input.mic[i].h * input.mic[i].h;
        k.s[i] = input.c * (input.t0[i] - input.t0[first]);
        let q = if input.sigma_ns[i] > 0.0 {
            sigma_ref / input.sigma_ns[i]
        } else {
            0.0
        };
        k.w[i] = q * q;
        if k.used(i) {
            k.wsum += k.w[i];
        }
    }

    let mut iterations = 0u32;
    let (mut x, mut y) = (0.5 * k.mx[first], 0.5 * k.my[first]);
    out.starts = 1;
    let mut ok = k.run_from(&mut x, &mut y, &mut iterations);
    if !ok {
        out.starts = 2;
        x = 0.0;
        y = 0.0;
        ok = k.run_from(&mut x, &mut y, &mut iterations);
    }
    out.converged = ok;
    out.iterations = iterations.min(255) as u8;

    let mut d = [0.0f32; MAX_MICS];
    let (_, dd) = k.eval_at(x, y, &mut d);
    let mut ss = 0.0f32;
    for i in 0..n {
        out.res_ns[i] = (k.s[i] - d[i] - dd) / input.c;
        if k.used(i) {
            ss += out.res_ns[i] * out.res_ns[i];
        }
    }
    out.rms_ns = (ss / used as f32).sqrt();
    out.x = x;
    out.y = y;
    out.tau_ns = input.t0[first] + dd / input.c;

    let (a, _) = k.normal(x, y, &d, dd);
    if let Some(inv) = invert3(&a) {
        let s2 = (sigma_ref * input.c) * (sigma_ref * input.c);
        let (cxx, cxy, cyy) = (s2 * inv[0], s2 * inv[1], s2 * inv[3]);
        out.sx = cxx.sqrt();
        out.sy = cyy.sqrt();
        out.rho = if out.sx > 0.0 && out.sy > 0.0 {
            cxy / (out.sx * out.sy)
        } else {
            0.0
        };
        let (m, q) = (0.5 * (cxx + cyy), 0.5 * (cxx - cyy));
        out.major = (m + (q * q + cxy * cxy).sqrt()).sqrt();
    }
    out
}

/// The drop-one search: the fit over `mask`; if its rms residual is over `limit_ns` and it
/// used 5 or more mics, the best fit with one of them left out.
pub fn solve_drop_one(input: &SolveInput, limit_ns: f32, mask: u8) -> Solution {
    let full = solve(input, mask);
    let used = full.used.count_ones();
    if (full.converged && full.rms_ns <= limit_ns) || used < 5 {
        return full;
    }
    let mut best: Option<Solution> = None;
    for i in 0..input.n.min(MAX_MICS) {
        if (full.used >> i) & 1 == 0 {
            continue;
        }
        let mut s = solve(input, full.used & !(1 << i));
        s.dropped = i as i8;
        // A converged fit beats one that didn't; then the lower residual.
        let better = match &best {
            None => true,
            Some(b) => (s.converged && !b.converged) || (s.converged == b.converged && s.rms_ns < b.rms_ns),
        };
        if better {
            best = Some(s);
        }
    }
    best.unwrap_or(full)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Case {
        name: String,
        kind: String,
        limit: f32,
        mask: u8,
        input: SolveInput,
        ok: bool,
        dropped: i8,
        x: f32,
        y: f32,
        tau: f32,
        rms: f32,
        res: Vec<f32>,
        sx: f32,
        sy: f32,
        rho: f32,
        major: f32,
    }

    /// The firmware's shared vectors (`firmware/test/vectors/solve.txt`, from tools/model.py).
    fn cases() -> Vec<Case> {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../firmware/test/vectors/solve.txt"
        );
        let text = std::fs::read_to_string(path).expect("firmware/test/vectors/solve.txt");
        let mut out = Vec::new();
        for line in text.lines().filter(|l| !l.is_empty() && !l.starts_with('#')) {
            let mut t = line.split_whitespace();
            let mut next = || t.next().unwrap().to_owned();
            let name = next();
            let kind = next();
            let limit: f32 = next().parse().unwrap();
            let n: usize = next().parse().unwrap();
            let mask = u8::from_str_radix(&next(), 16).unwrap();
            let c = next().parse::<f64>().unwrap() as f32;
            let mut input = SolveInput {
                n,
                c,
                ..Default::default()
            };
            for i in 0..n {
                input.mic[i] = Mic {
                    x: next().parse().unwrap(),
                    y: next().parse().unwrap(),
                    h: next().parse().unwrap(),
                };
            }
            for i in 0..n {
                input.t0[i] = next().parse().unwrap();
            }
            for i in 0..n {
                input.sigma_ns[i] = next().parse().unwrap();
            }
            assert_eq!(next(), "|");
            let ok = next() == "1";
            let mut c = Case {
                name,
                kind,
                limit,
                mask,
                input,
                ok,
                dropped: 0,
                x: 0.0,
                y: 0.0,
                tau: 0.0,
                rms: 0.0,
                res: vec![],
                sx: 0.0,
                sy: 0.0,
                rho: 0.0,
                major: 0.0,
            };
            if ok {
                c.dropped = next().parse().unwrap();
                c.x = next().parse().unwrap();
                c.y = next().parse().unwrap();
                c.tau = next().parse().unwrap();
                c.rms = next().parse().unwrap();
                c.res = (0..n).map(|_| next().parse().unwrap()).collect();
                c.sx = next().parse().unwrap();
                c.sy = next().parse().unwrap();
                c.rho = next().parse().unwrap();
                c.major = next().parse().unwrap();
            }
            out.push(c);
        }
        out
    }

    /// As the firmware's test_vectors: every fit and drop-one case to 0.001 mm, the outside
    /// points converge only to the truth, and no blast scores.
    #[test]
    fn firmware_vectors() {
        let (mut fits, mut drops, mut outside, mut blasts) = (0, 0, 0, 0);
        let (mut worst_pos, mut worst_res, mut worst_tau, mut worst_sig) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
        for c in cases() {
            match c.kind.as_str() {
                "fit" | "drop" => {
                    let s = if c.kind == "fit" {
                        solve(&c.input, c.mask)
                    } else {
                        solve_drop_one(&c.input, c.limit, c.mask)
                    };
                    let msg = format!(
                        "{}: x {} y {} (want {} {}), dropped {}",
                        c.name, s.x, s.y, c.x, c.y, s.dropped
                    );
                    assert!(c.ok && s.converged, "{msg}");
                    assert_eq!(c.dropped, s.dropped, "{msg}");
                    let e = (s.x - c.x).hypot(s.y - c.y);
                    assert!(e < 0.001, "{msg}");
                    worst_pos = worst_pos.max(e);
                    worst_tau = worst_tau.max((s.tau_ns - c.tau).abs());
                    worst_res = worst_res.max((s.rms_ns - c.rms).abs());
                    for i in 0..c.input.n {
                        worst_res = worst_res.max((s.res_ns[i] - c.res[i]).abs());
                    }
                    worst_sig = worst_sig
                        .max((s.sx / c.sx - 1.0).abs())
                        .max((s.sy / c.sy - 1.0).abs())
                        .max((s.major / c.major - 1.0).abs());
                    assert!((s.rho - c.rho).abs() < 0.001, "{msg}");
                    if c.kind == "fit" { fits += 1 } else { drops += 1 }
                }
                "outside" => {
                    let s = solve(&c.input, c.mask);
                    outside += 1;
                    if s.converged && s.rms_ns < 10.0 {
                        assert!((s.x - c.x).hypot(s.y - c.y) < 0.01, "{}", c.name);
                    }
                }
                "blast" => {
                    for drop in [false, true] {
                        let s = if drop {
                            solve_drop_one(&c.input, c.limit, c.mask)
                        } else {
                            solve(&c.input, c.mask)
                        };
                        let hit =
                            s.converged && s.rms_ns <= c.limit && s.x.abs() <= 100.0 && s.y.abs() <= 100.0;
                        assert!(!hit, "{} fits as a hit at ({}, {})", c.name, s.x, s.y);
                    }
                    blasts += 1;
                }
                _ => {}
            }
        }
        println!(
            "vectors: {fits} fits, {drops} drop-one, worst position {worst_pos} mm, residual {worst_res} ns, tau {worst_tau} ns, sigma {worst_sig:e}"
        );
        assert!(fits >= 300 && drops >= 100 && outside >= 50 && blasts >= 50);
        // As the firmware: arrival times near 1e6 ns are float steps of 0.06 ns apart, so the
        // residuals agree to a few tenths of a ns.
        assert!(
            worst_res < 0.5 && worst_tau < 0.5 && worst_sig < 1e-3,
            "res {worst_res} tau {worst_tau} sig {worst_sig}"
        );
    }
}
