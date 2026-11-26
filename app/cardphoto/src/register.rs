//! The registration: one fit over the printed circles' edge points gives the homography and
//! the lens's radial distortion (`app/CARDPHOTO.md` C1). Circles fix everything but the card's
//! rotation about its centre; the printed numbers fix that modulo 90° (the print's axes), or,
//! where they don't show, the card's outline (C2's match with the shots, or the numbers' shapes
//! in the app, settle the quarter turn).
//!
//! Each edge point's residual: its distance from the centre on the card (through the mapping)
//! minus its circle's radius, plus an offset per kind of printed line (where on the line the
//! stated diameter sits). The disc's edge has none: it is the reference. The print's scale
//! is not fitted: a photo has no absolute scale, so the rings set it (their ISSF size).

use nalgebra::{DMatrix, DVector, Matrix3};

use crate::card::{CardModel, Kind};
use crate::detect::{Contrast, RingPoint, coarse, contrast, numbers_rotation, outline_rotation, ring_points};
use crate::{Gray, Lens, Mapping};

/// The registration's result.
#[derive(Clone, Debug)]
pub struct Registration {
    pub map: Mapping,
    /// The offsets of the dark lines (on the white) and the light lines (in the black), mm.
    pub d_dark: f64,
    pub d_light: f64,
    /// The rms residual of the edge points kept, mm on the card.
    pub rms_mm: f64,
    /// Edge points found, and kept (the others: holes, dust, print).
    pub points: usize,
    pub kept: usize,
    /// Edge points per printed circle (`CardModel::features`' order).
    pub per_feature: Vec<usize>,
    /// What fixed the card's axes (its rotation modulo 90°); None: the rotation is arbitrary.
    pub axes: Option<Axes>,
    pub contrast: Contrast,
}

/// What fixed the card's axes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axes {
    /// The ring numbers, printed on the axes: the print's own axes.
    Numbers,
    /// The paper's square outline (cards are cut up to about 1° off the print).
    Outline,
}

impl Registration {
    /// Whether the card's axes are known (modulo 90°).
    pub fn axes_known(&self) -> bool {
        self.axes.is_some()
    }
}

/// The parameters: h11 h12 h13 h21 h22 h23 h31 h32 (h33 = 1), k1, k2, d_dark, d_light.
const NP: usize = 12;

fn unpack(q: &[f64], lens0: &Lens) -> (Mapping, f64, f64) {
    let h = Matrix3::new(q[0], q[1], q[2], q[3], q[4], q[5], q[6], q[7], 1.0);
    let lens = Lens {
        k1: q[8],
        k2: q[9],
        ..*lens0
    };
    (Mapping { h, lens }, q[10], q[11])
}

fn pack(map: &Mapping, d_dark: f64, d_light: f64) -> Vec<f64> {
    let h = map.h / map.h[(2, 2)];
    vec![
        h[(0, 0)],
        h[(0, 1)],
        h[(0, 2)],
        h[(1, 0)],
        h[(1, 1)],
        h[(1, 2)],
        h[(2, 0)],
        h[(2, 1)],
        map.lens.k1,
        map.lens.k2,
        d_dark,
        d_light,
    ]
}

/// Each point's residual (mm), for parameters q.
fn residuals(q: &[f64], lens0: &Lens, pts: &[RingPoint], model: &CardModel) -> Vec<f64> {
    let (map, d_dark, d_light) = unpack(q, lens0);
    let Some(inv) = map.h.try_inverse() else {
        return vec![1e6; pts.len()];
    };
    pts.iter()
        .map(|rp| {
            let f = model.features[rp.feature];
            let c = map.to_card_with(&inv, rp.p);
            let off = match f.kind {
                Kind::DarkLine => d_dark,
                Kind::LightLine => d_light,
                Kind::Edge => 0.0,
            };
            c.0.hypot(c.1) - (f.radius_mm + off)
        })
        .collect()
}

/// Weighted Levenberg-Marquardt from q0; weights per point (the robust reweighting). With
/// `lens` false, the lens's k1 and k2 stay as they are.
fn minimize(
    q0: &[f64],
    lens0: &Lens,
    pts: &[RingPoint],
    model: &CardModel,
    w: &[f64],
    lens: bool,
) -> Vec<f64> {
    // Typical sizes, for the finite differences' steps.
    let scale = |q: &[f64], i: usize| -> f64 {
        let lin = q[0]
            .abs()
            .max(q[1].abs())
            .max(q[3].abs())
            .max(q[4].abs())
            .max(1e-3);
        match i {
            0 | 1 | 3 | 4 => lin,
            2 | 5 => lin * 100.0,
            6 | 7 => lin * 1e-3 / q[2].abs().max(q[5].abs()).max(1.0) + 1e-9,
            8 | 9 => 0.01,
            _ => 0.01,
        }
    };
    let cost = |q: &[f64]| -> f64 {
        residuals(q, lens0, pts, model)
            .iter()
            .zip(w)
            .map(|(r, w)| w * r * r)
            .sum()
    };
    let mut q = q0.to_vec();
    let mut c = cost(&q);
    let mut lambda = 1e-3;
    for _ in 0..100 {
        let r0 = residuals(&q, lens0, pts, model);
        let m = pts.len();
        let mut jac = DMatrix::zeros(m, NP);
        for p in 0..NP {
            let step = 1e-6 * scale(&q, p);
            let mut qp = q.clone();
            qp[p] += step;
            let rp = residuals(&qp, lens0, pts, model);
            qp[p] = q[p] - step;
            let rm = residuals(&qp, lens0, pts, model);
            for i in 0..m {
                jac[(i, p)] = (rp[i] - rm[i]) / (2.0 * step) * w[i].sqrt();
            }
        }
        let f = DVector::from_iterator(m, r0.iter().zip(w).map(|(r, w)| r * w.sqrt()));
        let mut a = jac.transpose() * &jac;
        let mut g = jac.transpose() * f;
        if !lens {
            for p in 8..10 {
                a.row_mut(p).fill(0.0);
                a.column_mut(p).fill(0.0);
                a[(p, p)] = 1.0;
                g[p] = 0.0;
            }
        }
        let mut improved = false;
        for _ in 0..20 {
            let mut ad = a.clone();
            for d in 0..NP {
                ad[(d, d)] += lambda * a[(d, d)].max(1e-30);
            }
            // The card's rotation about its centre is free (circles): the damping keeps it.
            let Some(step) = ad
                .clone()
                .cholesky()
                .map(|ch| ch.solve(&(-&g)))
                .or_else(|| ad.lu().solve(&(-&g)))
            else {
                lambda *= 10.0;
                continue;
            };
            let cand: Vec<f64> = q.iter().zip(step.iter()).map(|(a, b)| a + b).collect();
            let cc = cost(&cand);
            if cc.is_finite() && cc <= c {
                let done = c - cc <= 1e-12 * c.max(1e-300);
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
            break;
        }
    }
    q
}

fn mad_sigma(r: &[f64]) -> f64 {
    let mut a: Vec<f64> = r.iter().map(|x| x.abs()).collect();
    a.sort_by(|x, y| x.partial_cmp(y).unwrap());
    1.4826 * a[a.len() / 2]
}

/// Fits the mapping to edge points, robustly (Huber weights from the residuals' MAD, a few
/// rounds). Returns the parameters and the final residuals. With `lens` false, the lens stays
/// as `map` has it.
fn fit(
    map: &Mapping,
    d: (f64, f64),
    pts: &[RingPoint],
    model: &CardModel,
    lens: bool,
) -> (Vec<f64>, Vec<f64>) {
    let lens0 = map.lens;
    let mut q = pack(map, d.0, d.1);
    let mut w = vec![1.0; pts.len()];
    for _ in 0..6 {
        q = minimize(&q, &lens0, pts, model, &w, lens);
        let r = residuals(&q, &lens0, pts, model);
        let k = (1.5 * mad_sigma(&r)).max(0.002);
        w = r
            .iter()
            .map(|r| if r.abs() <= k { 1.0 } else { k / r.abs() })
            .collect();
    }
    let r = residuals(&q, &lens0, pts, model);
    (q, r)
}

/// Registers a photo: coarse find, then edge points and the fit three times, each with a
/// narrower window, then the axes (the numbers', else the outline's rotation).
pub fn register(img: &Gray, model: &CardModel) -> Result<Registration, String> {
    let mut map = coarse(img, model)?;
    let con = contrast(img, &map, model).ok_or("too little contrast between the paper and the black")?;
    // Samples about every 0.2 px.
    let px_per_mm = (map.h[(0, 0)] * map.h[(1, 1)] - map.h[(0, 1)] * map.h[(1, 0)])
        .abs()
        .sqrt();
    if px_per_mm < 4.0 {
        return Err(format!(
            "the card is too small in the photo ({px_per_mm:.1} px per mm; 8 or more is best)"
        ));
    }
    let step = (0.2 / px_per_mm).min(0.05);
    let mut d = (0.0, 0.0);
    let mut pts = Vec::new();
    let mut res = Vec::new();
    // From the disc outwards: the coarse mapping (from the disc alone) can be several mm off
    // at the outer rings of a close, tilted photo, more than the window and up to the next
    // ring (8 mm away). So the inner rings first, the lens held, then further out, each
    // stage's guess reaching at most about 40 % beyond the rings already fitted; then the
    // whole card, the lens free, in narrower windows.
    for (reach, window, lens) in [
        (38.0, 2.0, false),
        (54.0, 2.0, false),
        (f64::INFINITY, 2.0, true),
        (f64::INFINITY, 0.8, true),
        (f64::INFINITY, 0.5, true),
    ] {
        let part = CardModel {
            features: model
                .features
                .iter()
                .copied()
                .filter(|f| f.radius_mm <= reach)
                .collect(),
            ..model.clone()
        };
        pts = ring_points(img, &map, &part, con, 720, window, step);
        if pts.len() < 500 {
            return Err(format!(
                "only {} edge points on the rings: not enough of the card in focus",
                pts.len()
            ));
        }
        let (q, r) = fit(&map, d, &pts, &part, lens);
        let (m, dd, dl) = unpack(&q, &map.lens);
        map = m;
        d = (dd, dl);
        res = r;
    }
    let sigma = mad_sigma(&res);
    let keep: Vec<bool> = res.iter().map(|r| r.abs() <= (3.0 * sigma).max(0.005)).collect();
    let kept = keep.iter().filter(|k| **k).count();
    let rms = (res
        .iter()
        .zip(&keep)
        .filter(|(_, k)| **k)
        .map(|(r, _)| r * r)
        .sum::<f64>()
        / kept.max(1) as f64)
        .sqrt();
    let mut per_feature = vec![0; model.features.len()];
    for (p, k) in pts.iter().zip(&keep) {
        if *k {
            per_feature[p.feature] += 1;
        }
    }
    let axes = match numbers_rotation(img, &map, model, con) {
        Some((a, _)) => Some((a, Axes::Numbers)),
        None => outline_rotation(img, &map, model, con).map(|a| (a, Axes::Outline)),
    };
    if let Some((a, _)) = axes {
        map = map.rotated(a);
    }
    Ok(Registration {
        map,
        d_dark: d.0,
        d_light: d.1,
        rms_mm: rms,
        points: pts.len(),
        kept,
        per_feature,
        axes: axes.map(|(_, f)| f),
        contrast: con,
    })
}

/// A region of the card, rectified: `x` and `y` ranges in card mm, `px_per_mm` pixels per mm,
/// the top row at the top of the region (y up on the card, as printed). Outside the photo: 0.
pub fn rectify(img: &Gray, map: &Mapping, x: (f64, f64), y: (f64, f64), px_per_mm: f64) -> Gray {
    let w = ((x.1 - x.0) * px_per_mm).round().max(1.0) as usize;
    let h = ((y.1 - y.0) * px_per_mm).round().max(1.0) as usize;
    let mut out = Gray::new(w, h);
    for j in 0..h {
        let cy = y.1 - (j as f64 + 0.5) / px_per_mm;
        for i in 0..w {
            let cx = x.0 + (i as f64 + 0.5) / px_per_mm;
            out.set(i, j, img.sample_at(map, (cx, cy)).unwrap_or(0.0));
        }
    }
    out
}
