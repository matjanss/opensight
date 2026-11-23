//! Finding the card in a photo: the coarse find (the black disc), the ring edges along lines
//! outwards from the centre, and the card's square outline (its rotation modulo 90°).

use nalgebra::Matrix3;

use crate::card::{CardModel, Kind};
use crate::{Gray, Lens, Mapping};

/// The paper's and the ink's grey levels near the centre (the contrast every threshold uses).
#[derive(Clone, Copy, Debug)]
pub struct Contrast {
    pub paper: f32,
    pub ink: f32,
}

impl Contrast {
    pub fn range(&self) -> f32 {
        self.paper - self.ink
    }
}

/// Otsu's threshold over values 0..1.
fn otsu(px: &[f32]) -> f32 {
    let mut hist = [0u64; 256];
    for &v in px {
        hist[(v.clamp(0.0, 1.0) * 255.0) as usize] += 1;
    }
    let total = px.len() as f64;
    let sum_all: f64 = hist.iter().enumerate().map(|(i, &c)| i as f64 * c as f64).sum();
    let (mut w0, mut sum0, mut best, mut best_t) = (0.0, 0.0, -1.0, 128);
    for (t, &c) in hist.iter().enumerate() {
        w0 += c as f64;
        sum0 += t as f64 * c as f64;
        let w1 = total - w0;
        if w0 == 0.0 || w1 == 0.0 {
            continue;
        }
        let (m0, m1) = (sum0 / w0, (sum_all - sum0) / w1);
        let between = w0 * w1 * (m0 - m1) * (m0 - m1);
        if between > best {
            best = between;
            best_t = t;
        }
    }
    (best_t as f32 + 0.5) / 255.0
}

/// The coarse find: the black aiming disc, the largest dark blob that is nearly an ellipse,
/// not touching the image's border, with light paper around it. Its ellipse gives a first
/// mapping (an affine one; the card's rotation is arbitrary here). Err if there is none.
pub fn coarse(img: &Gray, model: &CardModel) -> Result<Mapping, String> {
    let f = (img.w.max(img.h) / 800).max(1);
    let small = img.downscale(f);
    let (w, h) = (small.w, small.h);
    let t = otsu(&small.px);
    let dark: Vec<bool> = small.px.iter().map(|&v| v < t).collect();
    let mut label = vec![u32::MAX; w * h];
    let mut best: Option<(f64, [f64; 5])> = None; // area, (cx, cy, a, b, angle)
    let mut stack = Vec::new();
    for start in 0..w * h {
        if !dark[start] || label[start] != u32::MAX {
            continue;
        }
        label[start] = start as u32;
        stack.push(start);
        let (mut n, mut sx, mut sy, mut sxx, mut syy, mut sxy) = (0f64, 0f64, 0f64, 0f64, 0f64, 0f64);
        let mut border = false;
        while let Some(i) = stack.pop() {
            let (x, y) = (i % w, i / w);
            let (xf, yf) = (x as f64, y as f64);
            n += 1.0;
            sx += xf;
            sy += yf;
            sxx += xf * xf;
            syy += yf * yf;
            sxy += xf * yf;
            if x == 0 || y == 0 || x == w - 1 || y == h - 1 {
                border = true;
            }
            let mut push = |j: usize| {
                if dark[j] && label[j] == u32::MAX {
                    label[j] = start as u32;
                    stack.push(j);
                }
            };
            if x > 0 {
                push(i - 1);
            }
            if x + 1 < w {
                push(i + 1);
            }
            if y > 0 {
                push(i - w);
            }
            if y + 1 < h {
                push(i + w);
            }
        }
        if border || n < 0.002 * (w * h) as f64 {
            continue;
        }
        let (mx, my) = (sx / n, sy / n);
        let (vxx, vyy, vxy) = (sxx / n - mx * mx, syy / n - my * my, sxy / n - mx * my);
        let tr = vxx + vyy;
        let disc = ((vxx - vyy) * (vxx - vyy) / 4.0 + vxy * vxy).sqrt();
        let (l1, l2) = (tr / 2.0 + disc, tr / 2.0 - disc);
        if l2 <= 0.0 {
            continue;
        }
        let (a, b) = (2.0 * l1.sqrt(), 2.0 * l2.sqrt());
        let fill = n / (std::f64::consts::PI * a * b);
        if !(0.8..=1.12).contains(&fill) || b / a < 0.45 {
            continue;
        }
        let angle = 0.5 * (2.0 * vxy).atan2(vxx - vyy);
        // Light paper around it: most points on ellipses 1.25 and 1.6 times its size.
        let (ca, sa) = (angle.cos(), angle.sin());
        let mut light = 0;
        let mut seen = 0;
        for k in 0..64 {
            let phi = k as f64 / 64.0 * std::f64::consts::TAU;
            for scale in [1.25, 1.6] {
                let (ex, ey) = (a * scale * phi.cos(), b * scale * phi.sin());
                let (px, py) = (mx + ca * ex - sa * ey, my + sa * ex + ca * ey);
                if let Some(v) = small.sample(px, py) {
                    seen += 1;
                    if v >= t {
                        light += 1;
                    }
                }
            }
        }
        if seen < 64 || (light as f64) < 0.6 * seen as f64 {
            continue;
        }
        if best.as_ref().is_none_or(|(area, _)| n > *area) {
            best = Some((n, [mx, my, a, b, angle]));
        }
    }
    let (_, [mx, my, a, b, angle]) =
        best.ok_or("no card found: no black aiming disc with paper around it")?;
    // Back to full resolution (a pixel's centre: f x + (f - 1) / 2).
    let ff = f as f64;
    let (cx, cy) = (mx * ff + (ff - 1.0) / 2.0, my * ff + (ff - 1.0) / 2.0);
    let (a, b) = (a * ff / model.black_radius_mm, b * ff / model.black_radius_mm);
    let (c, s) = (angle.cos(), angle.sin());
    // Card mm (y up) -> image px (y down): R(angle) diag(a, b) diag(1, -1): never mirrored.
    let h = Matrix3::new(c * a, s * b, cx, s * a, -c * b, cy, 0.0, 0.0, 1.0);
    Ok(Mapping {
        h,
        lens: Lens::none(img.w, img.h),
    })
}

/// The paper's and the ink's levels: medians in a band of white paper and a band of black.
pub fn contrast(img: &Gray, map: &Mapping, model: &CardModel) -> Option<Contrast> {
    let band = |(r0, r1): (f64, f64)| -> Option<f32> {
        let mut v = Vec::new();
        for k in 0..360 {
            let phi = k as f64 / 360.0 * std::f64::consts::TAU;
            for j in 0..5 {
                let r = r0 + (r1 - r0) * j as f64 / 4.0;
                if let Some(s) = img.sample_at(map, (r * phi.cos(), r * phi.sin())) {
                    v.push(s);
                }
            }
        }
        if v.len() < 500 {
            return None;
        }
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        Some(v[v.len() / 2])
    };
    let c = Contrast {
        paper: band(model.paper_band_mm)?,
        ink: band(model.ink_band_mm)?,
    };
    (c.range() > 0.1).then_some(c)
}

impl Gray {
    /// The value at a card point through a mapping.
    pub fn sample_at(&self, map: &Mapping, q: (f64, f64)) -> Option<f32> {
        let p = map.to_image(q);
        self.sample(p.0, p.1)
    }
}

/// An edge point of one printed circle, found in the photo.
#[derive(Clone, Copy, Debug)]
pub struct RingPoint {
    /// Image px.
    pub p: (f64, f64),
    /// The feature (index into `CardModel::features`).
    pub feature: usize,
}

fn median(v: &[f32]) -> f32 {
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    s[s.len() / 2]
}

/// Along `n_angles` lines outwards from the centre (through `map`), every printed circle
/// within ±`window` mm of where `map` puts it, to a fraction of a sample (`step` mm apart).
pub fn ring_points(
    img: &Gray,
    map: &Mapping,
    model: &CardModel,
    contrast: Contrast,
    n_angles: usize,
    window: f64,
    step: f64,
) -> Vec<RingPoint> {
    let range = contrast.range();
    let n = (2.0 * window / step).round() as usize + 1;
    let mut out = Vec::new();
    let mut v = vec![0.0f32; n];
    for k in 0..n_angles {
        let phi = k as f64 / n_angles as f64 * std::f64::consts::TAU;
        let (c, s) = (phi.cos(), phi.sin());
        'feature: for (fi, f) in model.features.iter().enumerate() {
            let w = window.min(f.radius_mm * 0.6);
            let start = f.radius_mm - w;
            let count = ((2.0 * w / step).round() as usize + 1).min(n);
            for (i, vi) in v.iter_mut().enumerate().take(count) {
                let r = start + i as f64 * step;
                match img.sample_at(map, (r * c, r * s)) {
                    Some(x) => *vi = x,
                    None => continue 'feature,
                }
            }
            let v = &v[..count];
            let rho = match f.kind {
                Kind::DarkLine | Kind::LightLine => {
                    let base = median(v);
                    let sign = if f.kind == Kind::DarkLine { 1.0 } else { -1.0 };
                    let d: Vec<f32> = v.iter().map(|x| sign * (base - x)).collect();
                    let (imax, &dmax) = d
                        .iter()
                        .enumerate()
                        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
                        .unwrap();
                    let need = if f.kind == Kind::DarkLine { 0.25 } else { 0.15 };
                    if dmax < need * range || imax == 0 || imax + 1 == count {
                        continue;
                    }
                    // The centroid of the dip above half its depth, around its deepest point.
                    let half = 0.5 * dmax;
                    let (mut lo, mut hi) = (imax, imax);
                    while lo > 0 && d[lo - 1] > half {
                        lo -= 1;
                    }
                    while hi + 1 < count && d[hi + 1] > half {
                        hi += 1;
                    }
                    if lo == 0 || hi + 1 == count {
                        continue; // not a thin line inside the window
                    }
                    let (mut sw, mut swr) = (0.0f64, 0.0f64);
                    for (i, di) in d.iter().enumerate().take(hi + 1).skip(lo) {
                        let wgt = f64::from(di - half);
                        sw += wgt;
                        swr += wgt * i as f64;
                    }
                    start + swr / sw * step
                }
                Kind::Edge => {
                    // Dark inside, light outside: the steepest rise, to a parabola's top.
                    let g: Vec<f32> = (1..count - 1).map(|i| v[i + 1] - v[i - 1]).collect();
                    let (j, &gmax) = g
                        .iter()
                        .enumerate()
                        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
                        .unwrap();
                    let third = count / 3;
                    let inner: f32 = v[..third].iter().sum::<f32>() / third as f32;
                    let outer: f32 = v[count - third..].iter().sum::<f32>() / third as f32;
                    if outer - inner < 0.5 * range || j == 0 || j + 1 == g.len() || gmax <= 0.0 {
                        continue;
                    }
                    let (a, b, cc) = (g[j - 1] as f64, gmax as f64, g[j + 1] as f64);
                    let den = a - 2.0 * b + cc;
                    let off = if den.abs() > 1e-12 {
                        0.5 * (a - cc) / den
                    } else {
                        0.0
                    };
                    start + (j as f64 + 1.0 + off.clamp(-0.5, 0.5)) * step
                }
            };
            out.push(RingPoint {
                p: map.to_image((rho * c, rho * s)),
                feature: fi,
            });
        }
    }
    out
}

/// The card's rotation in `map`'s card coordinates, modulo 90° (radians, in -45°..45°), from
/// its square outline: along lines outwards, where the paper ends. None if the outline
/// doesn't show well enough (a table as light as the paper, the card cut off).
pub fn outline_rotation(img: &Gray, map: &Mapping, model: &CardModel, contrast: Contrast) -> Option<f64> {
    let half = model.half_size_mm;
    let (r0, r1, step) = (half - 2.0, half * std::f64::consts::SQRT_2 + 4.0, 0.1);
    let n = ((r1 - r0) / step) as usize;
    let mut pts: Vec<(f64, f64)> = Vec::new(); // (angle, radius)
    let n_angles = 1440;
    let mut v = vec![0.0f32; n];
    'ray: for k in 0..n_angles {
        let phi = k as f64 / n_angles as f64 * std::f64::consts::TAU;
        let (c, s) = (phi.cos(), phi.sin());
        for (i, vi) in v.iter_mut().enumerate() {
            let r = r0 + i as f64 * step;
            match img.sample_at(map, (r * c, r * s)) {
                Some(x) => *vi = x,
                None => continue 'ray,
            }
        }
        // The first strong change from the paper's level, outwards.
        let g: Vec<f32> = (1..n - 1).map(|i| v[i + 1] - v[i - 1]).collect();
        let Some((j, gm)) = g
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.abs().partial_cmp(&b.1.abs()).unwrap())
        else {
            continue;
        };
        if gm.abs() < 0.08 * contrast.range() || j == 0 || j + 1 == g.len() {
            continue;
        }
        let (a, b, cc) = (g[j - 1].abs() as f64, gm.abs() as f64, g[j + 1].abs() as f64);
        let den = a - 2.0 * b + cc;
        let off = if den.abs() > 1e-12 {
            0.5 * (a - cc) / den
        } else {
            0.0
        };
        pts.push((phi, r0 + (j as f64 + 1.0 + off.clamp(-0.5, 0.5)) * step));
    }
    if pts.len() < n_angles / 4 {
        return None;
    }
    // A square of half side `half` turned by alpha: its edge at angle phi is at
    // half / max(|cos(phi - alpha)|, |sin(phi - alpha)|). Truncated cost against stray points.
    let cost = |alpha: f64| -> f64 {
        pts.iter()
            .map(|&(phi, r)| {
                let (c, s) = ((phi - alpha).cos().abs(), (phi - alpha).sin().abs());
                (r - half / c.max(s)).abs().min(1.0)
            })
            .sum()
    };
    let quarter = std::f64::consts::FRAC_PI_4;
    let mut best = (f64::INFINITY, 0.0);
    let steps = 1800;
    for i in 0..steps {
        let a = -quarter + i as f64 / steps as f64 * 2.0 * quarter;
        let c = cost(a);
        if c < best.0 {
            best = (c, a);
        }
    }
    let mut span = 2.0 * quarter / steps as f64;
    let mut a = best.1;
    for _ in 0..30 {
        let (l, r) = (cost(a - span / 2.0), cost(a + span / 2.0));
        if l < r {
            a -= span / 4.0;
        } else {
            a += span / 4.0;
        }
        span /= 2.0;
    }
    let inliers = pts
        .iter()
        .filter(|&&(phi, r)| {
            let (c, s) = ((phi - a).cos().abs(), (phi - a).sin().abs());
            (r - half / c.max(s)).abs() < 0.5
        })
        .count();
    (inliers >= n_angles / 4).then_some(a)
}
