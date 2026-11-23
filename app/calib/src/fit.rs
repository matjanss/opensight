//! The calibration fit (`firmware/tools/calibrate.py`, MEASUREMENT 3e, PLAN D5).
//!
//! Per channel the mic position (x, y), the arrival-time factor k and the channel's own delay
//! (offset) are fitted to sparks made at known points; h stays as set (PLAN D3). Per spark e
//! and channel i:
//!     t1 - k_i (t2 - t1) - offset_i = tau_e + d(pt_e, mic_i, h_i) / c_e
//! with each spark's emission time tau_e unknown. A delay common to every channel only moves
//! every tau_e, so the offsets have mean 0.
//!
//! As `calibrate.py`: a robust first fit (soft L1, scale 50 ns, plain residuals), sparks with
//! a residual beyond 5 MADs left out, then the final fit with each channel's residual divided
//! by sqrt((1 + k)^2 + k^2) (both edges carry the sampling's noise: Deming regression, the
//! maximum-likelihood fit) and k within 0..2. sigma per channel = its rms residual.
//!
//! The emission times are solved exactly for each set of channel values (variable
//! projection: a weighted mean per spark), so the iteration runs over 4n - 1 values only. The
//! minimum is the same as `calibrate.py`'s joint fit.

use nalgebra::{DMatrix, DVector};

use crate::solver::{Mic, SolveInput, solve};
use crate::text::calibration_id;
use crate::{Channel, MAX_MICS};

/// One spark: its edges per channel (ns from the event's first edge), its true point (mm)
/// and the speed of sound the target used (m/s).
#[derive(Clone, Debug)]
pub struct Spark {
    pub t1: [Option<i32>; MAX_MICS],
    pub t2: [Option<i32>; MAX_MICS],
    pub pt: (f32, f32),
    pub c_ms: f32,
}

/// One fitted channel, with each value's 1σ uncertainty.
#[derive(Clone, Debug)]
pub struct FittedChannel {
    /// 0..7 (CH1..CH8).
    pub channel: usize,
    pub x: f64,
    pub y: f64,
    pub k: f64,
    pub offset_ns: f64,
    pub sigma_ns: f64,
    pub sd_x: f64,
    pub sd_y: f64,
    pub sd_k: f64,
    pub sd_offset: f64,
    /// The values the fit started from (the sensor's), for the change.
    pub before: Channel,
}

/// The position error at one template point, through the target's solver.
#[derive(Clone, Debug)]
pub struct PointError {
    pub x: f32,
    pub y: f32,
    pub sparks: usize,
    /// Mean and worst distance from the point, mm, with the values before and after.
    pub before_mean: f64,
    pub before_worst: f64,
    pub after_mean: f64,
    pub after_worst: f64,
}

/// The fit's result.
#[derive(Clone, Debug)]
pub struct FitResult {
    pub channels: Vec<FittedChannel>,
    /// The rms residual after the fit, ns.
    pub rms_ns: f64,
    /// Sparks used (after the outliers), and how many were left out as outliers.
    pub sparks: usize,
    pub outliers: usize,
    /// The outlier limit, ns.
    pub limit_ns: f64,
    pub points: Vec<PointError>,
    /// The position error over all sparks, rms, mm.
    pub before_rms_mm: f64,
    pub after_rms_mm: f64,
    /// What the sensor will hold: the fitted values as `calibrate.py` prints them (x, y to
    /// 0.0001 mm, k to 0.00001, offset to 0.01 ns, sigma to 0.1 ns), read as f32; channels
    /// without a mic as they were.
    pub record: [Channel; MAX_MICS],
    /// The record's calibration ID (the sensor's `measure::calibrationId`).
    pub id: u32,
}

/// What the fit needs, for the channels with a mic.
struct Data {
    n: usize,
    e: usize,
    h: Vec<f64>,
    t1: Vec<f64>,
    g: Vec<f64>,
    px: Vec<f64>,
    py: Vec<f64>,
    /// mm/ns
    c: Vec<f64>,
}

#[derive(Clone, Copy)]
enum Mode {
    /// Plain residuals, each weighted (the robust fit's reweighting).
    Plain,
    /// Divided by sqrt((1 + k)^2 + k^2) (the final fit).
    Scaled,
}

impl Data {
    fn subset(&self, keep: &[bool]) -> Data {
        let n = self.n;
        let idx: Vec<usize> = (0..self.e).filter(|&e| keep[e]).collect();
        let rows = |v: &Vec<f64>| idx.iter().flat_map(|&e| v[e * n..(e + 1) * n].to_vec()).collect();
        let col = |v: &Vec<f64>| idx.iter().map(|&e| v[e]).collect();
        Data {
            n,
            e: idx.len(),
            h: self.h.clone(),
            t1: rows(&self.t1),
            g: rows(&self.g),
            px: col(&self.px),
            py: col(&self.py),
            c: col(&self.c),
        }
    }

    /// (x, y, k, offsets) from the parameter vector: offsets n-1, the last minus their sum.
    fn unpack(&self, q: &[f64]) -> (Vec<f64>, Vec<f64>, Vec<f64>, Vec<f64>) {
        let n = self.n;
        let mut off = q[3 * n..4 * n - 1].to_vec();
        off.push(-off.iter().sum::<f64>());
        (
            q[..n].to_vec(),
            q[n..2 * n].to_vec(),
            q[2 * n..3 * n].to_vec(),
            off,
        )
    }

    fn pack(&self, x: &[f64], y: &[f64], k: &[f64], off: &[f64]) -> Vec<f64> {
        let mean = off.iter().sum::<f64>() / off.len() as f64;
        let mut q = Vec::with_capacity(4 * self.n - 1);
        q.extend_from_slice(x);
        q.extend_from_slice(y);
        q.extend_from_slice(k);
        q.extend(off[..self.n - 1].iter().map(|o| o - mean));
        q
    }

    /// The raw residuals (ns, e x n, each spark's emission time at its best for these values
    /// and weights) and the objective's residual vector.
    fn eval(&self, q: &[f64], mode: Mode, omega: Option<&[f64]>) -> (Vec<f64>, Vec<f64>) {
        let n = self.n;
        let (mx, my, k, off) = self.unpack(q);
        let scale: Vec<f64> = match mode {
            Mode::Plain => vec![1.0; n],
            Mode::Scaled => k.iter().map(|k| ((1.0 + k).powi(2) + k * k).sqrt()).collect(),
        };
        let mut raw = vec![0.0; self.e * n];
        let mut obj = vec![0.0; self.e * n];
        let mut base = vec![0.0; n];
        for e in 0..self.e {
            let (mut sw, mut swb) = (0.0, 0.0);
            for i in 0..n {
                let j = e * n + i;
                let d =
                    ((self.px[e] - mx[i]).powi(2) + (self.py[e] - my[i]).powi(2) + self.h[i].powi(2)).sqrt();
                base[i] = self.t1[j] - k[i] * self.g[j] - off[i] - d / self.c[e];
                let w = omega.map_or(1.0, |o| o[j]) / (scale[i] * scale[i]);
                sw += w;
                swb += w * base[i];
            }
            let tau = swb / sw;
            for i in 0..n {
                let j = e * n + i;
                raw[j] = base[i] - tau;
                obj[j] = omega.map_or(1.0, |o| o[j]).sqrt() * raw[j] / scale[i];
            }
        }
        (raw, obj)
    }

    /// The objective's Jacobian by central differences.
    fn jacobian(&self, q: &[f64], mode: Mode, omega: Option<&[f64]>) -> DMatrix<f64> {
        let m = self.e * self.n;
        let mut jac = DMatrix::zeros(m, q.len());
        let mut qq = q.to_vec();
        for p in 0..q.len() {
            let step = 1e-6 * q[p].abs().max(1.0);
            qq[p] = q[p] + step;
            let (_, fp) = self.eval(&qq, mode, omega);
            qq[p] = q[p] - step;
            let (_, fm) = self.eval(&qq, mode, omega);
            qq[p] = q[p];
            for r in 0..m {
                jac[(r, p)] = (fp[r] - fm[r]) / (2.0 * step);
            }
        }
        jac
    }

    /// Levenberg-Marquardt over the channel values; k kept within 0..2 if `bounded`.
    fn minimize(&self, q0: &[f64], mode: Mode, omega: Option<&[f64]>, bounded: bool) -> Vec<f64> {
        let n = self.n;
        let clamp = |q: &mut Vec<f64>| {
            if bounded {
                for v in &mut q[2 * n..3 * n] {
                    *v = v.clamp(0.0, 2.0);
                }
            }
        };
        let cost = |q: &[f64]| self.eval(q, mode, omega).1.iter().map(|r| r * r).sum::<f64>();
        let mut q = q0.to_vec();
        clamp(&mut q);
        let mut c = cost(&q);
        let mut lambda = 1e-3;
        for _ in 0..500 {
            let jac = self.jacobian(&q, mode, omega);
            let f = DVector::from_vec(self.eval(&q, mode, omega).1);
            let a = jac.transpose() * &jac;
            let g = jac.transpose() * f;
            let mut improved = false;
            for _ in 0..30 {
                let mut ad = a.clone();
                for d in 0..ad.nrows() {
                    ad[(d, d)] += lambda * a[(d, d)].max(1e-12);
                }
                let Some(step) = ad.cholesky().map(|ch| ch.solve(&(-&g))) else {
                    lambda *= 10.0;
                    continue;
                };
                let mut cand: Vec<f64> = q.iter().zip(step.iter()).map(|(a, b)| a + b).collect();
                clamp(&mut cand);
                let cc = cost(&cand);
                if cc <= c {
                    let small = step.norm() < 1e-10 * (DVector::from_vec(q.clone()).norm() + 1e-10);
                    let done = c - cc <= 1e-15 * c.max(1e-300) || small;
                    q = cand;
                    c = cc;
                    lambda = (lambda / 3.0).max(1e-12);
                    improved = true;
                    if done {
                        return q;
                    }
                    break;
                }
                lambda *= 4.0;
            }
            if !improved {
                break; // no step lowers the cost: at the minimum to the arithmetic's precision
            }
        }
        q
    }
}

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let m = v.len() / 2;
    if v.len() % 2 == 1 {
        v[m]
    } else {
        0.5 * (v[m - 1] + v[m])
    }
}

/// The robust first fit: soft L1 (scale 50 ns) on the plain residuals, by iteratively
/// reweighted least squares (each spark's emission time weighted alike).
fn robust_fit(data: &Data, q0: &[f64]) -> (Vec<f64>, Vec<f64>) {
    let f_scale = 50.0f64;
    let mut omega = vec![1.0; data.e * data.n];
    let mut q = q0.to_vec();
    for _ in 0..30 {
        let next = data.minimize(&q, Mode::Plain, Some(&omega), false);
        let (raw, _) = data.eval(&next, Mode::Plain, Some(&omega));
        let moved = next
            .iter()
            .zip(&q)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0, f64::max);
        q = next;
        omega = raw
            .iter()
            .map(|r| 1.0 / (1.0 + (r / f_scale).powi(2)).sqrt())
            .collect();
        if moved < 1e-7 {
            break;
        }
    }
    let (raw, _) = data.eval(&q, Mode::Plain, Some(&omega));
    (q, raw)
}

/// Fits the calibration. `nominal`: the sensor's values now (the start, and h and the
/// channels without a mic as they are). `sparks`: calibration mode's sparks at their points,
/// all made with these values. Sparks without both edges on every mic channel are skipped.
pub fn calibrate(nominal: &[Channel; MAX_MICS], sparks: &[Spark]) -> Result<FitResult, String> {
    let chans: Vec<usize> = (0..MAX_MICS).filter(|&i| nominal[i].mic).collect();
    let n = chans.len();
    if n < 4 {
        return Err(format!("{n} channels with a mic: the fit needs 4 or more"));
    }
    let usable: Vec<&Spark> = sparks
        .iter()
        .filter(|s| chans.iter().all(|&i| s.t1[i].is_some() && s.t2[i].is_some()))
        .collect();
    if usable.len() < 4 * n {
        return Err(format!(
            "only {} usable sparks for {n} channels: too few (at least {})",
            usable.len(),
            4 * n
        ));
    }
    let e = usable.len();
    let mut data = Data {
        n,
        e,
        h: chans.iter().map(|&i| f64::from(nominal[i].h)).collect(),
        t1: Vec::with_capacity(e * n),
        g: Vec::with_capacity(e * n),
        px: usable.iter().map(|s| f64::from(s.pt.0)).collect(),
        py: usable.iter().map(|s| f64::from(s.pt.1)).collect(),
        c: usable.iter().map(|s| f64::from(s.c_ms) * 1e-6).collect(),
    };
    for s in &usable {
        for &i in &chans {
            let (t1, t2) = (f64::from(s.t1[i].unwrap()), f64::from(s.t2[i].unwrap()));
            data.t1.push(t1);
            data.g.push(t2 - t1);
        }
    }
    let nom =
        |f: fn(&Channel) -> f32| -> Vec<f64> { chans.iter().map(|&i| f64::from(f(&nominal[i]))).collect() };
    let (x0, y0, k0, off0) = (nom(|c| c.x), nom(|c| c.y), nom(|c| c.k), nom(|c| c.offset_ns));
    let q0 = data.pack(&x0, &y0, &k0, &off0);

    // The robust fit, then the sparks beyond 5 MADs (at least 20 ns) out.
    let (q1, r1) = robust_fit(&data, &q0);
    let mut dev = r1.clone();
    let med = median(&mut dev);
    let mut absdev: Vec<f64> = r1.iter().map(|r| (r - med).abs()).collect();
    let mad = 1.4826 * median(&mut absdev);
    let limit = (5.0 * mad).max(20.0);
    let keep: Vec<bool> = (0..e)
        .map(|s| (0..n).all(|i| r1[s * n + i].abs() <= limit))
        .collect();
    let outliers = keep.iter().filter(|k| !**k).count();
    let data2 = data.subset(&keep);

    // The final fit: scaled, k within 0..2, from the robust fit's values.
    let mut q1 = q1;
    for v in &mut q1[2 * n..3 * n] {
        *v = v.clamp(0.01, 1.99);
    }
    let q = data2.minimize(&q1, Mode::Scaled, None, true);
    let (r2, f2) = data2.eval(&q, Mode::Scaled, None);
    let (mx, my, k, off) = data2.unpack(&q);

    // Uncertainties: the covariance s^2 (J^T J)^-1, with the degrees of freedom of the joint
    // fit (the emission times count as fitted values).
    let dof = (r2.len() as f64 - (4 * n - 1 + data2.e) as f64).max(1.0);
    let s2 = f2.iter().map(|r| r * r).sum::<f64>() / dof;
    let jac = data2.jacobian(&q, Mode::Scaled, None);
    let jtj = jac.transpose() * &jac;
    let cov = jtj.pseudo_inverse(1e-12).map_err(|e| e.to_string())? * s2;
    let sd = |p: usize| cov[(p, p)].max(0.0).sqrt();
    let mut sd_off: Vec<f64> = (0..n - 1).map(|i| sd(3 * n + i)).collect();
    let block = cov.view((3 * n, 3 * n), (n - 1, n - 1)).sum();
    sd_off.push(block.max(0.0).sqrt());
    let rms = (r2.iter().map(|r| r * r).sum::<f64>() / r2.len() as f64).sqrt();
    let sigma: Vec<f64> = (0..n)
        .map(|i| {
            ((0..data2.e).map(|s| r2[s * n + i].powi(2)).sum::<f64>() / data2.e as f64)
                .sqrt()
                .max(5.0)
        })
        .collect();

    // What the sensor will hold: the printed decimals, read as f32.
    let mut record = *nominal;
    let round = |v: f64, d: usize| format!("{v:.d$}").parse::<f32>().unwrap();
    for (j, &i) in chans.iter().enumerate() {
        record[i] = Channel {
            mic: true,
            x: round(mx[j], 4),
            y: round(my[j], 4),
            h: nominal[i].h,
            k: round(k[j], 5),
            offset_ns: round(off[j], 2),
            sigma_ns: round(sigma[j], 1),
        };
    }

    // The position error per point, before and after, through the target's solver.
    let solve_at = |s: &Spark, xs: &[f64], ys: &[f64], ks: &[f64], os: &[f64]| -> Option<(f32, f32)> {
        let mut input = SolveInput {
            n,
            c: s.c_ms * 1e-6,
            ..Default::default()
        };
        for (j, &i) in chans.iter().enumerate() {
            let (t1, t2) = (s.t1[i].unwrap() as f32, s.t2[i].unwrap() as f32);
            input.mic[j] = Mic {
                x: xs[j] as f32,
                y: ys[j] as f32,
                h: nominal[i].h,
            };
            input.t0[j] = t1 - ks[j] as f32 * (t2 - t1) - os[j] as f32;
            input.sigma_ns[j] = 50.0;
        }
        let sol = solve(&input, ((1u16 << n) - 1) as u8);
        sol.converged.then_some((sol.x, sol.y))
    };
    let mut keys: Vec<(i64, i64)> = usable.iter().map(|s| key(s.pt)).collect();
    keys.sort();
    keys.dedup();
    let (mut all_before, mut all_after) = (Vec::new(), Vec::new());
    let mut points = Vec::new();
    for pk in keys {
        let group: Vec<&&Spark> = usable.iter().filter(|s| key(s.pt) == pk).collect();
        let errs = |xs: &[f64], ys: &[f64], ks: &[f64], os: &[f64]| -> Vec<f64> {
            group
                .iter()
                .filter_map(|s| {
                    solve_at(s, xs, ys, ks, os)
                        .map(|(x, y)| f64::from(x - s.pt.0).hypot(f64::from(y - s.pt.1)))
                })
                .collect()
        };
        let b = errs(&x0, &y0, &k0, &off0);
        let a = errs(&mx, &my, &k, &off);
        let mean = |v: &[f64]| {
            if v.is_empty() {
                f64::NAN
            } else {
                v.iter().sum::<f64>() / v.len() as f64
            }
        };
        let worst = |v: &[f64]| v.iter().cloned().fold(f64::NAN, f64::max);
        points.push(PointError {
            x: group[0].pt.0,
            y: group[0].pt.1,
            sparks: group.len(),
            before_mean: mean(&b),
            before_worst: worst(&b),
            after_mean: mean(&a),
            after_worst: worst(&a),
        });
        all_before.extend(b);
        all_after.extend(a);
    }
    let rms_of = |v: &[f64]| (v.iter().map(|x| x * x).sum::<f64>() / v.len().max(1) as f64).sqrt();

    let channels = chans
        .iter()
        .enumerate()
        .map(|(j, &i)| FittedChannel {
            channel: i,
            x: mx[j],
            y: my[j],
            k: k[j],
            offset_ns: off[j],
            sigma_ns: sigma[j],
            sd_x: sd(j),
            sd_y: sd(n + j),
            sd_k: sd(2 * n + j),
            sd_offset: sd_off[j],
            before: nominal[i],
        })
        .collect();
    Ok(FitResult {
        channels,
        rms_ns: rms,
        sparks: data2.e,
        outliers,
        limit_ns: limit,
        points,
        before_rms_mm: rms_of(&all_before),
        after_rms_mm: rms_of(&all_after),
        id: calibration_id(&record),
        record,
    })
}

/// A template point as a key (to 0.001 mm, as `calibrate.py` groups them).
fn key(p: (f32, f32)) -> (i64, i64) {
    (
        (f64::from(p.0) * 1000.0).round() as i64,
        (f64::from(p.1) * 1000.0).round() as i64,
    )
}

/// The final fit's objective (the sum of the squared scaled residuals, each spark's emission
/// time at its best) for given channel values over these sparks: to compare two fits of the
/// same sparks (the parity test against calibrate.py).
pub fn objective(
    nominal: &[Channel; MAX_MICS],
    sparks: &[Spark],
    x: &[f64],
    y: &[f64],
    k: &[f64],
    off: &[f64],
) -> f64 {
    let chans: Vec<usize> = (0..MAX_MICS).filter(|&i| nominal[i].mic).collect();
    let n = chans.len();
    let mut data = Data {
        n,
        e: sparks.len(),
        h: chans.iter().map(|&i| f64::from(nominal[i].h)).collect(),
        t1: vec![],
        g: vec![],
        px: sparks.iter().map(|s| f64::from(s.pt.0)).collect(),
        py: sparks.iter().map(|s| f64::from(s.pt.1)).collect(),
        c: sparks.iter().map(|s| f64::from(s.c_ms) * 1e-6).collect(),
    };
    for s in sparks {
        for &i in &chans {
            let (t1, t2) = (f64::from(s.t1[i].unwrap()), f64::from(s.t2[i].unwrap()));
            data.t1.push(t1);
            data.g.push(t2 - t1);
        }
    }
    let q = data.pack(x, y, k, off);
    data.eval(&q, Mode::Scaled, None).1.iter().map(|r| r * r).sum()
}
