//! C2 (`app/CARDPHOTO.md`): the holes, and their match to the measured shots.
//!
//! A hole shows whatever is under the card. Where it is is found against what the card should
//! look like there, taken from the photo itself: the print is rings, so at each radius it
//! looks the same all round. Per 5° sector, the median colour at each radius is the expected
//! card (a hole is too small to move a median; the sector follows uneven light). The
//! difference from it, normalised by the paper–ink contrast, is high in a hole whatever the
//! table's colour, unless the table matches both the ink and the paper there.
//!
//! Each hole is a circle fitted to that difference (a soft edge, the radius shared by the
//! holes of a group); holes closer than about a diameter are fitted together. The shots settle
//! the card's quarter turn (all four tried) and its placement: the shift, the small angle and
//! a scale between the frame's coordinates and the card's.

use nalgebra::{DMatrix, DVector};

use crate::register::Registration;
use crate::{Mapping, Rgb};

/// A point in card mm.
type Pt = (f64, f64);
/// A pixel near a hole: its card point, its normalised colour, the card's expected colour.
type Sample = (Pt, [f32; 3], [f32; 3]);
/// A shot (frame mm) and its hole (card mm).
type Pair = (Pt, Pt);

/// The pellet's diameter (10 m air pistol), mm.
pub const HOLE_D_MM: f64 = 4.5;
/// How far from its shot (as placed so far) a hole is looked for, mm.
pub const SEARCH_MM: f64 = 4.0;
/// The matched filter's least contrast for a hole (the difference's units: the paper–ink
/// range).
const MIN_CONTRAST: f64 = 0.15;
/// The radius out to which holes are looked for without a shot (the scoring area), mm.
const SCORING_MM: f64 = 80.0;

/// What the card should look like, from the photo itself: the light (the paper's and the
/// ink's colour across the card, as smooth surfaces), and the print (one profile by radius,
/// normalised: 0 ink, 1 paper).
pub struct Expected {
    /// Per channel, quadratic surfaces in card mm / 100: 1, x, y, x², xy, y².
    paper: [[f64; 6]; 3],
    ink: [[f64; 6]; 3],
    rho_step: f64,
    profile: Vec<[f32; 3]>,
}

fn basis(q: (f64, f64)) -> [f64; 6] {
    let (x, y) = (q.0 / 100.0, q.1 / 100.0);
    [1.0, x, y, x * x, x * y, y * y]
}

fn eval6(c: &[f64; 6], q: (f64, f64)) -> f64 {
    basis(q).iter().zip(c).map(|(b, c)| b * c).sum()
}

/// A quadratic surface fitted robustly (Huber, a few rounds) to samples (point, value).
fn fit_surface(samples: &[((f64, f64), f64)]) -> [f64; 6] {
    let mut w = vec![1.0; samples.len()];
    let mut c = [0.0; 6];
    for _ in 0..5 {
        let mut a = nalgebra::Matrix6::<f64>::zeros();
        let mut b = nalgebra::Vector6::<f64>::zeros();
        for ((q, v), wi) in samples.iter().zip(&w) {
            let f = nalgebra::Vector6::from(basis(*q));
            a += f * f.transpose() * *wi;
            b += f * (*v * *wi);
        }
        let Some(sol) = a.lu().solve(&b) else { break };
        c = [sol[0], sol[1], sol[2], sol[3], sol[4], sol[5]];
        let r: Vec<f64> = samples.iter().map(|(q, v)| v - eval6(&c, *q)).collect();
        let mut abs: Vec<f64> = r.iter().map(|x| x.abs()).collect();
        abs.sort_by(|x, y| x.partial_cmp(y).unwrap());
        let k = (1.5 * 1.4826 * abs[abs.len() / 2]).max(1e-3);
        w = r
            .iter()
            .map(|r| if r.abs() <= k { 1.0 } else { k / r.abs() })
            .collect();
    }
    c
}

impl Expected {
    pub fn from_photo(rgb: &Rgb, reg: &Registration) -> Expected {
        let map = &reg.map;
        // Plain paper and plain black: the middles of the bands between the printed circles
        // (the ISSF air pistol card's).
        let band = |radii: &[f64]| -> Vec<((f64, f64), [f32; 3])> {
            let mut out = Vec::new();
            for &r in radii {
                for k in 0..360 {
                    let phi = k as f64 / 360.0 * std::f64::consts::TAU;
                    let q = (r * phi.cos(), r * phi.sin());
                    if let Some(c) = rgb.sample_at(map, q) {
                        out.push((q, c));
                    }
                }
            }
            out
        };
        let paper_s = band(&[33.75, 41.75, 49.75, 57.75, 65.75, 73.75]);
        let ink_s = band(&[9.75, 17.75, 25.75]);
        let surf = |s: &[((f64, f64), [f32; 3])], k: usize| {
            fit_surface(&s.iter().map(|(q, c)| (*q, f64::from(c[k]))).collect::<Vec<_>>())
        };
        let paper = [surf(&paper_s, 0), surf(&paper_s, 1), surf(&paper_s, 2)];
        let ink = [surf(&ink_s, 0), surf(&ink_s, 1), surf(&ink_s, 2)];
        let mut e = Expected {
            paper,
            ink,
            rho_step: 0.1,
            profile: Vec::new(),
        };
        // The print by radius: the median over the full circle (a hole is a small part of it).
        let n_rho = ((SCORING_MM + 4.0) / e.rho_step) as usize + 1;
        let mut profile = vec![[0.0f32; 3]; n_rho];
        let mut buf: [Vec<f32>; 3] = [Vec::new(), Vec::new(), Vec::new()];
        for (i, slot) in profile.iter_mut().enumerate() {
            let rho = i as f64 * e.rho_step;
            if rho < 2.0 {
                continue; // inside the inner ten: black (a hole there covers the whole circle)
            }
            for b in &mut buf {
                b.clear();
            }
            let n = 720;
            for j in 0..n {
                let phi = j as f64 / n as f64 * std::f64::consts::TAU;
                let q = (rho * phi.cos(), rho * phi.sin());
                if let Some(c) = rgb.sample_at(map, q) {
                    let v = e.normalize(c, q);
                    for k in 0..3 {
                        buf[k].push(v[k]);
                    }
                }
            }
            if buf[0].is_empty() {
                continue;
            }
            for k in 0..3 {
                buf[k].sort_by(|a, b| a.partial_cmp(b).unwrap());
                slot[k] = buf[k][buf[k].len() / 2];
            }
        }
        e.profile = profile;
        e
    }

    /// A colour at a card point, normalised by the light there: 0 ink, 1 paper.
    pub fn normalize(&self, c: [f32; 3], q: (f64, f64)) -> [f32; 3] {
        let mut out = [0.0; 3];
        for k in 0..3 {
            let (p, i) = (eval6(&self.paper[k], q), eval6(&self.ink[k], q));
            out[k] = ((f64::from(c[k]) - i) / (p - i).max(0.05)) as f32;
        }
        out
    }

    /// The print's expected normalised colour at a card point.
    pub fn at(&self, q: (f64, f64)) -> [f32; 3] {
        let f = (q.0.hypot(q.1) / self.rho_step).min((self.profile.len() - 1) as f64 - 1e-9);
        let i = f.floor() as usize;
        let t = (f - i as f64) as f32;
        let (a, b) = (self.profile[i], self.profile[i + 1]);
        [0, 1, 2].map(|k| a[k] * (1.0 - t) + b[k] * t)
    }

    /// How far a photo colour at a card point is from the expected card (0: as printed).
    pub fn diff(&self, c: [f32; 3], q: (f64, f64)) -> f32 {
        let (n, e) = (self.normalize(c, q), self.at(q));
        let s: f32 = (0..3).map(|k| (n[k] - e[k]) * (n[k] - e[k])).sum();
        (s / 3.0).sqrt()
    }
}

/// The difference from the expected card on a grid in card mm: `n` x `n` points from `origin`
/// (the lower left corner), `res` pixels per mm. Outside the photo: 0.
struct Field {
    origin: (f64, f64),
    res: f64,
    n: usize,
    d: Vec<f32>,
}

impl Field {
    fn new(rgb: &Rgb, map: &Mapping, exp: &Expected, centre: (f64, f64), half: f64, res: f64) -> Field {
        let n = (2.0 * half * res).ceil() as usize + 1;
        let origin = (centre.0 - half, centre.1 - half);
        let mut d = vec![0.0f32; n * n];
        for j in 0..n {
            for i in 0..n {
                let q = (origin.0 + i as f64 / res, origin.1 + j as f64 / res);
                if let Some(c) = rgb.sample_at(map, q) {
                    d[j * n + i] = exp.diff(c, q);
                }
            }
        }
        Field { origin, res, n, d }
    }

    fn point(&self, i: usize, j: usize) -> (f64, f64) {
        (
            self.origin.0 + i as f64 / self.res,
            self.origin.1 + j as f64 / self.res,
        )
    }

    /// The matched filter at a card point: the mean difference inside a hole there minus the
    /// mean in a ring around it.
    fn filter(&self, c: (f64, f64), r: f64) -> f64 {
        let (r_in, r_out) = (r + 0.35 * r, r + 0.75 * r);
        let (mut si, mut ni, mut so, mut no) = (0.0, 0, 0.0, 0);
        let reach = (r_out * self.res).ceil() as i64;
        let ci = ((c.0 - self.origin.0) * self.res).round() as i64;
        let cj = ((c.1 - self.origin.1) * self.res).round() as i64;
        for dj in -reach..=reach {
            for di in -reach..=reach {
                let (i, j) = (ci + di, cj + dj);
                if i < 0 || j < 0 || i >= self.n as i64 || j >= self.n as i64 {
                    continue;
                }
                let p = self.point(i as usize, j as usize);
                let dist = (p.0 - c.0).hypot(p.1 - c.1);
                let v = f64::from(self.d[j as usize * self.n + i as usize]);
                if dist <= r {
                    si += v;
                    ni += 1;
                } else if dist >= r_in && dist <= r_out {
                    so += v;
                    no += 1;
                }
            }
        }
        if ni == 0 || no == 0 {
            0.0
        } else {
            si / ni as f64 - so / no as f64
        }
    }

    /// The best filter response within `reach` of `c` (a grid search, then a finer one).
    fn best_near(&self, c: (f64, f64), reach: f64, r: f64) -> ((f64, f64), f64) {
        let mut best = (c, f64::NEG_INFINITY);
        let step = 0.25;
        let n = (reach / step) as i64;
        for j in -n..=n {
            for i in -n..=n {
                let p = (c.0 + i as f64 * step, c.1 + j as f64 * step);
                if (p.0 - c.0).hypot(p.1 - c.1) > reach {
                    continue;
                }
                let v = self.filter(p, r);
                if v > best.1 {
                    best = (p, v);
                }
            }
        }
        let c0 = best.0;
        for j in -5..=5 {
            for i in -5..=5 {
                let p = (c0.0 + i as f64 * 0.05, c0.1 + j as f64 * 0.05);
                let v = self.filter(p, r);
                if v > best.1 {
                    best = (p, v);
                }
            }
        }
        best
    }
}

/// One hole.
#[derive(Clone, Debug)]
pub struct HoleFit {
    /// Card mm (the registration's coordinates).
    pub center: (f64, f64),
    pub radius_mm: f64,
    /// The centre's 1σ uncertainty (the fit's; pixels are correlated, so a lower bound), mm.
    pub sd_mm: f64,
    /// How much it differs from the card (the paper–ink range = 1).
    pub contrast: f64,
}

fn sigmoid(x: f64) -> f64 {
    1.0 / (1.0 + (-x).exp())
}

/// Fits n circles together (a group), from starts `c0`, to the photo's normalised colours near
/// them: inside a hole the table's colour (one unknown colour), outside the card as expected
/// at each point, with a soft edge. That holds for any table, also for a hole across the
/// black disc's edge (half on expected black, half on expected white). Parameters: the
/// centres, the shared radius, the edge's width, the table's colour (3).
fn fit_group(rgb: &Rgb, map: &Mapping, exp: &Expected, c0: &[(f64, f64)], r0: f64, res: f64) -> Vec<HoleFit> {
    fit_group_cost(rgb, map, exp, c0, r0, res).0
}

/// fit_group, and the fit's mean squared residual (to compare counts of circles).
fn fit_group_cost(
    rgb: &Rgb,
    map: &Mapping,
    exp: &Expected,
    c0: &[(f64, f64)],
    r0: f64,
    res: f64,
) -> (Vec<HoleFit>, f64) {
    let n = c0.len();
    let np = 2 * n + 5;
    // The pixels near the circles: (x, y, normalised colour, expected colour).
    let reach = r0 + 1.5;
    let (mx, my) = (
        c0.iter().map(|c| c.0).sum::<f64>() / n as f64,
        c0.iter().map(|c| c.1).sum::<f64>() / n as f64,
    );
    let half = c0
        .iter()
        .map(|c| (c.0 - mx).abs().max((c.1 - my).abs()))
        .fold(0.0, f64::max)
        + reach;
    let g = (2.0 * half * res).ceil() as usize + 1;
    let mut pts: Vec<Sample> = Vec::new();
    for j in 0..g {
        for i in 0..g {
            let q = (mx - half + i as f64 / res, my - half + j as f64 / res);
            if !c0.iter().any(|c| (q.0 - c.0).hypot(q.1 - c.1) <= reach) {
                continue;
            }
            if let Some(c) = rgb.sample_at(map, q) {
                pts.push((q, exp.normalize(c, q), exp.at(q)));
            }
        }
    }
    let cover = |q: &[f64], p: (f64, f64)| -> f64 {
        let (r, w) = (q[2 * n], q[2 * n + 1].abs().max(0.01));
        let mut out = 1.0;
        for k in 0..n {
            let d = (p.0 - q[2 * k]).hypot(p.1 - q[2 * k + 1]);
            out *= 1.0 - sigmoid((r - d) / w);
        }
        1.0 - out
    };
    let resid = |q: &[f64], w: &[f64]| -> Vec<f64> {
        let mut out = Vec::with_capacity(3 * pts.len());
        for ((p, nv, ev), wi) in pts.iter().zip(w) {
            let s = cover(q, *p);
            for k in 0..3 {
                let m = f64::from(ev[k]) + s * (q[2 * n + 2 + k] - f64::from(ev[k]));
                out.push((m - f64::from(nv[k])) * wi.sqrt());
            }
        }
        out
    };
    let mut q: Vec<f64> = c0.iter().flat_map(|c| [c.0, c.1]).collect();
    // The table's colour at the start: the mean inside the starts.
    let inside: Vec<&[f32; 3]> = pts
        .iter()
        .filter(|p| c0.iter().any(|c| (p.0.0 - c.0).hypot(p.0.1 - c.1) < 0.6 * r0))
        .map(|p| &p.1)
        .collect();
    let t0: [f64; 3] =
        [0, 1, 2].map(|k| inside.iter().map(|c| f64::from(c[k])).sum::<f64>() / inside.len().max(1) as f64);
    q.extend([r0, 0.15, t0[0], t0[1], t0[2]]);
    let mut w = vec![1.0; pts.len()];
    let jacobian = |q: &[f64], w: &[f64]| -> DMatrix<f64> {
        let m = 3 * pts.len();
        let mut jac = DMatrix::zeros(m, np);
        for p in 0..np {
            let step = 1e-5;
            let mut qp = q.to_vec();
            qp[p] += step;
            let rp = resid(&qp, w);
            qp[p] = q[p] - step;
            let rm = resid(&qp, w);
            for i in 0..m {
                jac[(i, p)] = (rp[i] - rm[i]) / (2.0 * step);
            }
        }
        jac
    };
    for round in 0..3 {
        let mut lambda = 1e-3;
        let mut cost: f64 = resid(&q, &w).iter().map(|r| r * r).sum();
        for _ in 0..60 {
            let jac = jacobian(&q, &w);
            let fv = DVector::from_vec(resid(&q, &w));
            let a = jac.transpose() * &jac;
            let gv = jac.transpose() * fv;
            let mut improved = false;
            let mut done = false;
            for _ in 0..15 {
                let mut ad = a.clone();
                for d in 0..np {
                    ad[(d, d)] += lambda * a[(d, d)].max(1e-12);
                }
                let Some(step) = ad.lu().solve(&(-&gv)) else { break };
                let mut cand: Vec<f64> = q.iter().zip(step.iter()).map(|(a, b)| a + b).collect();
                cand[2 * n] = cand[2 * n].clamp(0.6 * r0, 1.3 * r0);
                cand[2 * n + 1] = cand[2 * n + 1].clamp(0.02, 0.8);
                let c: f64 = resid(&cand, &w).iter().map(|r| r * r).sum();
                if c <= cost {
                    done = cost - c <= 1e-10 * cost.max(1e-300);
                    q = cand;
                    cost = c;
                    lambda = (lambda / 3.0).max(1e-9);
                    improved = true;
                    break;
                }
                lambda *= 5.0;
            }
            if !improved || done {
                break;
            }
        }
        if round < 2 {
            // Robust, per pixel: Huber weights from the residuals' MAD (fibres, the print's
            // details, a textured table).
            let ones = vec![1.0; pts.len()];
            let r = resid(&q, &ones);
            let per: Vec<f64> = r
                .chunks(3)
                .map(|c| (c.iter().map(|x| x * x).sum::<f64>() / 3.0).sqrt())
                .collect();
            let mut a = per.clone();
            a.sort_by(|x, y| x.partial_cmp(y).unwrap());
            let k = (1.5 * 1.4826 * a[a.len() / 2]).max(0.01);
            w = per.iter().map(|r| if *r <= k { 1.0 } else { k / r }).collect();
        }
    }
    // Uncertainties from the unweighted Jacobian at the end.
    let ones = vec![1.0; pts.len()];
    let r_end = resid(&q, &ones);
    let s2 = r_end.iter().map(|r| r * r).sum::<f64>() / (r_end.len() as f64 - np as f64).max(1.0);
    let jac = jacobian(&q, &ones);
    let cov = (jac.transpose() * &jac)
        .pseudo_inverse(1e-12)
        .unwrap_or_else(|_| DMatrix::zeros(np, np))
        * s2;
    let table = [q[2 * n + 2], q[2 * n + 3], q[2 * n + 4]];
    let mse = r_end.iter().map(|r| r * r).sum::<f64>() / r_end.len().max(1) as f64;
    let fits = (0..n)
        .map(|k| {
            // How much the table differs from the card inside this hole (0 ink .. 1 paper units).
            let c = (q[2 * k], q[2 * k + 1]);
            let inner: Vec<f64> = pts
                .iter()
                .filter(|p| (p.0.0 - c.0).hypot(p.0.1 - c.1) < 0.7 * q[2 * n])
                .map(|p| {
                    ((0..3)
                        .map(|i| (table[i] - f64::from(p.2[i])).powi(2))
                        .sum::<f64>()
                        / 3.0)
                        .sqrt()
                })
                .collect();
            HoleFit {
                center: c,
                radius_mm: q[2 * n],
                sd_mm: cov[(2 * k, 2 * k)]
                    .max(cov[(2 * k + 1, 2 * k + 1)])
                    .max(0.0)
                    .sqrt(),
                contrast: inner.iter().sum::<f64>() / inner.len().max(1) as f64,
            }
        })
        .collect();
    (fits, mse)
}

/// Where the card lies in the frame: card point = scale · R(angle) · frame point + shift.
#[derive(Clone, Copy, Debug)]
pub struct Placement {
    /// The whole angle (radians), the quarter turn included.
    pub angle: f64,
    /// The quarter turn (0–3: 0, 90, 180, 270°) it is nearest to.
    pub quarter: u8,
    pub shift: (f64, f64),
    pub scale: f64,
}

impl Placement {
    pub fn apply(&self, s: (f64, f64)) -> (f64, f64) {
        let (sn, cs) = self.angle.sin_cos();
        (
            self.scale * (cs * s.0 - sn * s.1) + self.shift.0,
            self.scale * (sn * s.0 + cs * s.1) + self.shift.1,
        )
    }

    /// The inverse: a card point (the registration's mm) in the frame (the sensor's mm).
    pub fn to_frame(&self, c: (f64, f64)) -> (f64, f64) {
        let (sn, cs) = self.angle.sin_cos();
        let (x, y) = (
            (c.0 - self.shift.0) / self.scale,
            (c.1 - self.shift.1) / self.scale,
        );
        (cs * x + sn * y, -sn * x + cs * y)
    }

    /// The small angle beyond the quarter turn, degrees.
    pub fn tilt_deg(&self) -> f64 {
        let d = self.angle - f64::from(self.quarter) * std::f64::consts::FRAC_PI_2;
        (d + std::f64::consts::PI)
            .rem_euclid(std::f64::consts::TAU)
            .to_degrees()
            - 180.0
    }
}

/// The least-squares placement for pairs (frame point, card point): a rotation and shift, and
/// a scale if `with_scale`.
fn fit_placement(pairs: &[Pair], angle0: f64, with_scale: bool) -> Placement {
    let n = pairs.len() as f64;
    let quarter_of = |a: f64| ((a / std::f64::consts::FRAC_PI_2).round().rem_euclid(4.0)) as u8;
    if pairs.len() == 1 {
        let (s, h) = pairs[0];
        let (sn, cs) = angle0.sin_cos();
        return Placement {
            angle: angle0,
            quarter: quarter_of(angle0),
            shift: (h.0 - (cs * s.0 - sn * s.1), h.1 - (sn * s.0 + cs * s.1)),
            scale: 1.0,
        };
    }
    let ms = (
        pairs.iter().map(|p| p.0.0).sum::<f64>() / n,
        pairs.iter().map(|p| p.0.1).sum::<f64>() / n,
    );
    let mh = (
        pairs.iter().map(|p| p.1.0).sum::<f64>() / n,
        pairs.iter().map(|p| p.1.1).sum::<f64>() / n,
    );
    let (mut sxx, mut sxy, mut syx, mut syy, mut ss) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for &(s, h) in pairs {
        let (a, b) = (s.0 - ms.0, s.1 - ms.1);
        let (c, d) = (h.0 - mh.0, h.1 - mh.1);
        sxx += a * c;
        sxy += a * d;
        syx += b * c;
        syy += b * d;
        ss += a * a + b * b;
    }
    let angle = (sxy - syx).atan2(sxx + syy);
    let (sn, cs) = angle.sin_cos();
    let scale = if with_scale && ss > 0.0 {
        ((sxx + syy) * cs + (sxy - syx) * sn) / ss
    } else {
        1.0
    };
    let shift = (
        mh.0 - scale * (cs * ms.0 - sn * ms.1),
        mh.1 - scale * (sn * ms.0 + cs * ms.1),
    );
    Placement {
        angle,
        quarter: quarter_of(angle),
        shift,
        scale,
    }
}

/// One shot's result.
#[derive(Clone, Debug)]
pub struct ShotResult {
    /// Its hole, if one was found where it should be.
    pub hole: Option<HoleFit>,
    /// The hole minus the placed shot, card mm: the sensor's error for this shot.
    pub diff_mm: Option<(f64, f64)>,
}

/// The ground truth of one card.
#[derive(Clone, Debug)]
pub struct GroundTruth {
    pub placement: Placement,
    pub shots: Vec<ShotResult>,
    /// The rms of the differences (the sensor's error after the placement), mm.
    pub rms_mm: f64,
    /// Holes that no shot explains.
    pub extra: Vec<HoleFit>,
    /// Whether the placement includes a fitted scale (5 or more shots spread out).
    pub scaled: bool,
}

/// Groups points closer than `gap` to another one (single linkage).
fn groups(points: &[(f64, f64)], gap: f64) -> Vec<Vec<usize>> {
    let n = points.len();
    let mut id: Vec<usize> = (0..n).collect();
    fn root(id: &mut [usize], i: usize) -> usize {
        let mut r = i;
        while id[r] != r {
            r = id[r];
        }
        id[i] = r;
        r
    }
    for i in 0..n {
        for j in i + 1..n {
            if (points[i].0 - points[j].0).hypot(points[i].1 - points[j].1) < gap {
                let (a, b) = (root(&mut id, i), root(&mut id, j));
                id[a] = b;
            }
        }
    }
    let mut out: Vec<Vec<usize>> = Vec::new();
    let mut seen: Vec<Option<usize>> = vec![None; n];
    for i in 0..n {
        let r = root(&mut id, i);
        match seen[r] {
            Some(g) => out[g].push(i),
            None => {
                seen[r] = Some(out.len());
                out.push(vec![i]);
            }
        }
    }
    out
}

/// Fits the holes at card points `starts` (grouped when close), each within its field.
fn fit_at(rgb: &Rgb, reg: &Registration, exp: &Expected, starts: &[(f64, f64)], res: f64) -> Vec<HoleFit> {
    let r0 = HOLE_D_MM / 2.0;
    let mut out = vec![None; starts.len()];
    for g in groups(starts, 2.0 * r0 + 1.0) {
        let cs: Vec<(f64, f64)> = g.iter().map(|&i| starts[i]).collect();
        for (k, h) in fit_group(rgb, &reg.map, exp, &cs, r0, res)
            .into_iter()
            .enumerate()
        {
            out[g[k]] = Some(h);
        }
    }
    out.into_iter().map(|h| h.unwrap()).collect()
}

/// Every hole in the scoring area, without shots (the test cards): the filter's peaks over
/// the card, then fitted (close ones together).
pub fn all_holes(rgb: &Rgb, reg: &Registration) -> Vec<HoleFit> {
    let exp = Expected::from_photo(rgb, reg);
    find_all(rgb, reg, &exp, &[])
}

fn find_all(rgb: &Rgb, reg: &Registration, exp: &Expected, skip: &[(f64, f64)]) -> Vec<HoleFit> {
    let r0 = HOLE_D_MM / 2.0;
    let res = 5.0;
    let f = Field::new(rgb, &reg.map, exp, (0.0, 0.0), SCORING_MM + 4.0, res);
    // Blobs: where the photo differs from the card, in the scoring area, away from the holes
    // already found.
    let on: Vec<bool> = (0..f.n * f.n)
        .map(|k| {
            let p = f.point(k % f.n, k / f.n);
            f.d[k] > 0.2
                && p.0.hypot(p.1) <= SCORING_MM
                && skip.iter().all(|s| (s.0 - p.0).hypot(s.1 - p.1) > r0 + 0.6)
        })
        .collect();
    let mut seen = vec![false; on.len()];
    let disc = std::f64::consts::PI * r0 * r0;
    let mut out = Vec::new();
    for start in 0..on.len() {
        if !on[start] || seen[start] {
            continue;
        }
        let mut blob = vec![start];
        seen[start] = true;
        let mut i = 0;
        while i < blob.len() {
            let k = blob[i];
            i += 1;
            let (x, y) = (k % f.n, k / f.n);
            for (dx, dy) in [(-1i64, 0i64), (1, 0), (0, -1), (0, 1)] {
                let (nx, ny) = (x as i64 + dx, y as i64 + dy);
                if nx < 0 || ny < 0 || nx >= f.n as i64 || ny >= f.n as i64 {
                    continue;
                }
                let j = ny as usize * f.n + nx as usize;
                if on[j] && !seen[j] {
                    seen[j] = true;
                    blob.push(j);
                }
            }
        }
        let area = blob.len() as f64 / (res * res);
        if area < 0.5 * disc {
            continue; // the print's details (a number's strokes), dust
        }
        let pts: Vec<(f64, f64)> = blob.iter().map(|&k| f.point(k % f.n, k / f.n)).collect();
        // How many holes: from the area, then one or two more if they clearly fit better.
        let n0 = ((area / disc).round() as usize).clamp(1, 6);
        let mut best: Option<(Vec<HoleFit>, f64)> = None;
        for n in n0..=(n0 + 2).min(8) {
            let starts = farthest_points(&pts, n);
            let (fits, mse) = fit_group_cost(rgb, &reg.map, exp, &starts, r0, 10.0);
            let apart = fits.iter().enumerate().all(|(i, a)| {
                fits[i + 1..]
                    .iter()
                    .all(|b| (a.center.0 - b.center.0).hypot(a.center.1 - b.center.1) > 1.0)
            });
            let better = match &best {
                None => true,
                Some((_, m)) => apart && mse < 0.95 * m,
            };
            if better {
                best = Some((fits, mse));
            } else {
                break;
            }
        }
        out.extend(best.unwrap().0);
    }
    // A hole is round and clearly different from the card.
    out.into_iter()
        .filter(|h| {
            h.contrast >= MIN_CONTRAST
                && (h.radius_mm - r0).abs() < 0.5
                && h.center.0.hypot(h.center.1) <= SCORING_MM
        })
        .collect()
}

/// n starts spread over a blob's points: k-means from the farthest-point choice.
fn farthest_points(pts: &[(f64, f64)], n: usize) -> Vec<(f64, f64)> {
    let m = (
        pts.iter().map(|p| p.0).sum::<f64>() / pts.len() as f64,
        pts.iter().map(|p| p.1).sum::<f64>() / pts.len() as f64,
    );
    let d2 = |a: (f64, f64), b: (f64, f64)| (a.0 - b.0).powi(2) + (a.1 - b.1).powi(2);
    let mut c: Vec<(f64, f64)> = vec![
        *pts.iter()
            .max_by(|a, b| d2(**a, m).partial_cmp(&d2(**b, m)).unwrap())
            .unwrap(),
    ];
    while c.len() < n {
        let next = pts
            .iter()
            .max_by(|a, b| {
                let da = c.iter().map(|x| d2(**a, *x)).fold(f64::INFINITY, f64::min);
                let db = c.iter().map(|x| d2(**b, *x)).fold(f64::INFINITY, f64::min);
                da.partial_cmp(&db).unwrap()
            })
            .unwrap();
        c.push(*next);
    }
    if n == 1 {
        return vec![m];
    }
    for _ in 0..20 {
        let mut sum = vec![(0.0, 0.0, 0usize); n];
        for p in pts {
            let k = (0..n)
                .min_by(|&i, &j| d2(*p, c[i]).partial_cmp(&d2(*p, c[j])).unwrap())
                .unwrap();
            sum[k].0 += p.0;
            sum[k].1 += p.1;
            sum[k].2 += 1;
        }
        for (ci, s) in c.iter_mut().zip(&sum) {
            if s.2 > 0 {
                *ci = (s.0 / s.2 as f64, s.1 / s.2 as f64);
            }
        }
    }
    c
}

/// The ground truth of a card: the holes of the shots (frame mm, as the sensor measured
/// them), the card's placement, the differences, and holes no shot explains.
pub fn ground_truth(rgb: &Rgb, reg: &Registration, shots: &[(f64, f64)]) -> Result<GroundTruth, String> {
    if shots.is_empty() {
        return Err("no shots to match".into());
    }
    let exp = Expected::from_photo(rgb, reg);
    let r0 = HOLE_D_MM / 2.0;
    let quarter = std::f64::consts::FRAC_PI_2;
    // The quarter turn: where the shots' holes show best.
    let window = |c: (f64, f64)| Field::new(rgb, &reg.map, &exp, c, SEARCH_MM + 2.0 * r0, 5.0);
    let mut best: Option<(f64, u8)> = None;
    for k in 0..4u8 {
        let a = f64::from(k) * quarter;
        let (sn, cs) = a.sin_cos();
        let score: f64 = shots
            .iter()
            .map(|s| {
                let c = (cs * s.0 - sn * s.1, sn * s.0 + cs * s.1);
                if c.0.hypot(c.1) > SCORING_MM + SEARCH_MM {
                    return 0.0;
                }
                window(c).best_near(c, SEARCH_MM, r0).1.max(0.0)
            })
            .sum();
        if best.is_none_or(|(b, _)| score > b) {
            best = Some((score, k));
        }
    }
    let k = best.unwrap().1;
    let mut placement = Placement {
        angle: f64::from(k) * quarter,
        quarter: k,
        shift: (0.0, 0.0),
        scale: 1.0,
    };
    // Coarse holes near each shot, then the placement, then the holes fitted from the placed
    // shots (close ones together), then the placement again, twice.
    let mut pairs: Vec<Pair> = Vec::new();
    for s in shots {
        let c = placement.apply(*s);
        let (p, v) = window(c).best_near(c, SEARCH_MM, r0);
        if v >= MIN_CONTRAST {
            pairs.push((*s, p));
        }
    }
    if pairs.is_empty() {
        return Err("no hole found near any shot".into());
    }
    placement = fit_placement(&pairs, placement.angle, false);
    let spread = {
        let n = shots.len() as f64;
        let m = (
            shots.iter().map(|s| s.0).sum::<f64>() / n,
            shots.iter().map(|s| s.1).sum::<f64>() / n,
        );
        (shots
            .iter()
            .map(|s| (s.0 - m.0).powi(2) + (s.1 - m.1).powi(2))
            .sum::<f64>()
            / n)
            .sqrt()
    };
    let with_scale = shots.len() >= 5 && spread > 20.0;
    let mut fits = Vec::new();
    for _ in 0..2 {
        let starts: Vec<(f64, f64)> = shots.iter().map(|s| placement.apply(*s)).collect();
        fits = fit_at(rgb, reg, &exp, &starts, 10.0);
        // A hole counts if it is clear, round, and near where its shot says.
        let ok: Vec<bool> = fits
            .iter()
            .zip(&starts)
            .map(|(h, s)| {
                h.contrast >= MIN_CONTRAST
                    && (h.radius_mm - r0).abs() < 0.6
                    && (h.center.0 - s.0).hypot(h.center.1 - s.1) < SEARCH_MM
            })
            .collect();
        let mut pairs: Vec<Pair> = shots
            .iter()
            .zip(&fits)
            .zip(&ok)
            .filter(|(_, k)| **k)
            .map(|((s, h), _)| (*s, h.center))
            .collect();
        if pairs.is_empty() {
            return Err("no hole fitted where the shots are".into());
        }
        placement = fit_placement(&pairs, placement.angle, with_scale);
        // Robust: leave out pairs far beyond the rest, once.
        let r: Vec<f64> = pairs
            .iter()
            .map(|(s, h)| {
                let p = placement.apply(*s);
                (h.0 - p.0).hypot(h.1 - p.1)
            })
            .collect();
        let mut sorted = r.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let limit = (4.0 * sorted[sorted.len() / 2]).max(1.0);
        if r.iter().any(|x| *x > limit) && pairs.len() > 2 {
            pairs = pairs
                .into_iter()
                .zip(&r)
                .filter(|(_, x)| **x <= limit)
                .map(|(p, _)| p)
                .collect();
            placement = fit_placement(&pairs, placement.angle, with_scale);
        }
    }
    let starts: Vec<(f64, f64)> = shots.iter().map(|s| placement.apply(*s)).collect();
    let results: Vec<ShotResult> = shots
        .iter()
        .zip(fits)
        .zip(&starts)
        .map(|((_, h), st)| {
            let good = h.contrast >= MIN_CONTRAST
                && (h.radius_mm - r0).abs() < 0.6
                && (h.center.0 - st.0).hypot(h.center.1 - st.1) < SEARCH_MM;
            if good {
                ShotResult {
                    diff_mm: Some((h.center.0 - st.0, h.center.1 - st.1)),
                    hole: Some(h),
                }
            } else {
                ShotResult {
                    hole: None,
                    diff_mm: None,
                }
            }
        })
        .collect();
    let d: Vec<f64> = results
        .iter()
        .filter_map(|r| r.diff_mm.map(|d| d.0 * d.0 + d.1 * d.1))
        .collect();
    let rms = (d.iter().sum::<f64>() / d.len().max(1) as f64).sqrt();
    let found: Vec<(f64, f64)> = results
        .iter()
        .filter_map(|r| r.hole.as_ref().map(|h| h.center))
        .collect();
    let extra = find_all(rgb, reg, &exp, &found);
    Ok(GroundTruth {
        placement,
        shots: results,
        rms_mm: rms,
        extra,
        scaled: with_scale,
    })
}
