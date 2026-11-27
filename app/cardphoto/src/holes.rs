//! C2 (`app/CARDPHOTO.md`): the holes, and their match to the measured shots.
//!
//! A hole shows whatever is under the card. Where it is is found against what the card should
//! look like there, taken from the photo itself: the print is rings, so at each radius it
//! looks the same all round. The light is smooth surfaces (paper and ink), the print the
//! median normalised colour at each radius over the full circle (a hole is too small to move a
//! median), with each printed circle where the photo has it (`Warp`: a card not quite flat
//! bends them). The difference from it, normalised by the paper–ink contrast, is high in a
//! hole whatever the table's colour, unless the table matches both the ink and the paper
//! there.
//!
//! Each hole is a circle fitted to that difference (a soft edge, the radius shared by the
//! holes of a group); holes closer than about a diameter are fitted together. The shots settle
//! the card's quarter turn (all four tried) and its placement: the shift, the small angle and
//! a scale between the frame's coordinates and the card's.

use nalgebra::{DMatrix, DVector};

use crate::gauge::{GaugeFit, gauge};
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
/// A gauge circle more than this smaller than the old model's opening saw something inside
/// the hole, not its edge, mm; so did one smaller than this share of the calibre's radius (a
/// pellet's mark is at least its own size: the C5 holes 2.29 mm and up for 4.5 mm, clean
/// synthetic ones 2.22–2.24 mm at the rim's half height).
const INSIDE_MM: f64 = 0.1;
const MARK_MIN: f64 = 0.97;
/// A gauge circle's radius, as a share of the calibre's: what a hole's mark can be.
const GAUGE_R_MIN: f64 = 0.8;
const GAUGE_R_MAX: f64 = 1.4;
/// The black disc's radius (the ISSF air pistol card: rings 7–10), mm.
const BLACK_MM: f64 = 29.75;
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
    /// Where the printed circles really lie in the photo, against the registration's circles.
    warp: Warp,
    /// The printed numbers (when the card's axes are known): per ring's number, its four
    /// copies' centres and their median look on a grid around them.
    numbers: Vec<NumberPatch>,
}

/// A ring's number as the photo shows it: the same digit, upright, on the card's four axes;
/// per point around it the median of the four copies (a hole on one copy doesn't move it).
struct NumberPatch {
    /// The copies that matched (those that didn't keep the rings' profile).
    centres: Vec<(f64, f64)>,
    grid: Vec<[f32; 3]>,
}

/// A number's patch: ±this around its centre, mm, on a grid of this step.
const NUMBER_HALF_MM: f64 = 2.6;
const NUMBER_STEP_MM: f64 = 0.05;
/// How far a copy may sit from its place on the axis, mm, either way: real cards print the
/// numbers on the horizontal axis about 0.6 mm off those on the vertical one (C7).
const NUMBER_SEARCH_MM: f64 = 1.0;
/// A copy is left out when it differs from the others' median by more than this many times
/// the second best copy's difference (mean per pixel, normalised grey), and more than the
/// floor.
const NUMBER_FIT_FACTOR: f32 = 2.0;
const NUMBER_FIT_MIN: f32 = 0.03;

impl NumberPatch {
    fn n() -> usize {
        (2.0 * NUMBER_HALF_MM / NUMBER_STEP_MM).round() as usize + 1
    }

    /// The patch's look at a card point, if the point is on one of its copies.
    fn at(&self, q: (f64, f64)) -> Option<[f32; 3]> {
        let n = Self::n();
        let c = self
            .centres
            .iter()
            .find(|c| (q.0 - c.0).abs() < NUMBER_HALF_MM && (q.1 - c.1).abs() < NUMBER_HALF_MM)?;
        let fx = ((q.0 - c.0 + NUMBER_HALF_MM) / NUMBER_STEP_MM).clamp(0.0, (n - 1) as f64 - 1e-9);
        let fy = ((q.1 - c.1 + NUMBER_HALF_MM) / NUMBER_STEP_MM).clamp(0.0, (n - 1) as f64 - 1e-9);
        let (i, j) = (fx.floor() as usize, fy.floor() as usize);
        let (tx, ty) = ((fx - i as f64) as f32, (fy - j as f64) as f32);
        let g = |i: usize, j: usize| self.grid[j * n + i];
        let (a, b, c2, d) = (g(i, j), g(i + 1, j), g(i, j + 1), g(i + 1, j + 1));
        Some(
            [0, 1, 2].map(|k| {
                (a[k] * (1.0 - tx) + b[k] * tx) * (1.0 - ty) + (c2[k] * (1.0 - tx) + d[k] * tx) * ty
            }),
        )
    }
}

/// How far the printed circles lie off the registration's circles in the photo, by angle: a
/// card not quite flat bends them (a wave around the card, up to 0.55 mm near an edge), and
/// a print compared at the wrong place shows a line's strip as damage. Per printed circle and
/// per angle (`WARP_ANGLES`), its measured point (`detect::ring_points`) less that circle's
/// median all round, smoothed: points more than 0.15 mm off the median over ±8 mm of arc are
/// dropped (a hole on the line), then the median over ±2.5 mm. Between circles linear in the
/// radius, between angles linear in the angle.
struct Warp {
    /// Per printed circle, from the centre outwards: its radius and its offset per angle (mm).
    rings: Vec<(f64, Vec<f64>)>,
}

/// The angles of `Warp` (as `ring_points` is asked for).
const WARP_ANGLES: usize = 720;
/// How far from the registration's circle a printed circle is looked for, mm (seen: 0.55 mm
/// near a card's edge; the circles are 3.25 mm apart or more).
const WARP_WINDOW_MM: f64 = 1.0;

impl Warp {
    fn measure(rgb: &Rgb, reg: &Registration) -> Warp {
        let model = crate::card::issf_air_pistol();
        let h = &reg.map.h;
        let ppm = (h[(0, 0)] * h[(1, 1)] - h[(0, 1)] * h[(1, 0)]).abs().sqrt();
        let step = (0.2 / ppm).min(0.05);
        let gray = rgb.to_gray();
        let pts = crate::detect::ring_points(
            &gray,
            &reg.map,
            &model,
            reg.contrast,
            WARP_ANGLES,
            WARP_WINDOW_MM,
            step,
        );
        let n = model.features.len();
        let mut raw: Vec<Vec<Option<f64>>> = vec![vec![None; WARP_ANGLES]; n];
        for p in &pts {
            if let Some(c) = reg.map.to_card(p.p) {
                let phi = c.1.atan2(c.0).rem_euclid(std::f64::consts::TAU);
                let k = (phi / std::f64::consts::TAU * WARP_ANGLES as f64).round() as usize % WARP_ANGLES;
                raw[p.feature][k] = Some(c.0.hypot(c.1) - model.features[p.feature].radius_mm);
            }
        }
        let median = |v: &mut Vec<f64>| -> Option<f64> {
            if v.len() < 3 {
                return None;
            }
            v.sort_by(f64::total_cmp);
            Some(v[v.len() / 2])
        };
        // The median over ±`half` angles around each angle, of the values `keep` lets through.
        let smooth = |vals: &[Option<f64>], half: usize, keep: &dyn Fn(usize, f64) -> bool| {
            (0..WARP_ANGLES)
                .map(|k| {
                    let mut w: Vec<f64> = (0..=2 * half)
                        .filter_map(|d| {
                            let j = (k + WARP_ANGLES + d - half) % WARP_ANGLES;
                            vals[j].filter(|&v| keep(j, v))
                        })
                        .collect();
                    median(&mut w)
                })
                .collect::<Vec<Option<f64>>>()
        };
        let mut rings = Vec::new();
        for (f, vals) in model.features.iter().zip(raw) {
            let mut all: Vec<f64> = vals.iter().flatten().copied().collect();
            if all.len() < WARP_ANGLES / 4 {
                continue;
            }
            let m = median(&mut all).unwrap();
            let arc = f.radius_mm * std::f64::consts::TAU / WARP_ANGLES as f64;
            let bins = |mm: f64| ((mm / arc).ceil() as usize).clamp(2, WARP_ANGLES / 4);
            let wide = smooth(&vals, bins(8.0), &|_, _| true);
            let near = smooth(&vals, bins(2.5), &|j, v| {
                wide[j].is_some_and(|w| (v - w).abs() <= 0.15)
            });
            // An angle with nothing near (a hole, a number) takes the nearest known on each side.
            let off: Vec<f64> = (0..WARP_ANGLES)
                .map(|k| {
                    let found = (0..WARP_ANGLES).find_map(|d| {
                        let v: Vec<f64> = [
                            near[(k + d) % WARP_ANGLES],
                            near[(k + WARP_ANGLES - d) % WARP_ANGLES],
                        ]
                        .into_iter()
                        .flatten()
                        .collect();
                        (!v.is_empty()).then(|| v.iter().sum::<f64>() / v.len() as f64)
                    });
                    (found.unwrap_or(m) - m).clamp(-WARP_WINDOW_MM, WARP_WINDOW_MM)
                })
                .collect();
            rings.push((f.radius_mm, off));
        }
        rings.sort_by(|a, b| a.0.total_cmp(&b.0));
        Warp { rings }
    }

    /// The offset at a card point: where the print's circle through it lies off the
    /// registration's, mm (outwards positive).
    fn at(&self, q: Pt) -> f64 {
        if self.rings.is_empty() {
            return 0.0;
        }
        let rho = q.0.hypot(q.1);
        let x = q.1.atan2(q.0).rem_euclid(std::f64::consts::TAU) / std::f64::consts::TAU * WARP_ANGLES as f64;
        let b0 = x.floor();
        let t = x - b0;
        let b0 = (b0 as usize) % WARP_ANGLES;
        let b1 = (b0 + 1) % WARP_ANGLES;
        let ring = |k: usize| self.rings[k].1[b0] * (1.0 - t) + self.rings[k].1[b1] * t;
        let last = self.rings.len() - 1;
        if rho <= self.rings[0].0 {
            return ring(0) * rho / self.rings[0].0;
        }
        if rho >= self.rings[last].0 {
            return ring(last);
        }
        let k = self.rings.iter().position(|r| r.0 > rho).unwrap();
        let (r0, r1) = (self.rings[k - 1].0, self.rings[k].0);
        let u = (rho - r0) / (r1 - r0);
        ring(k - 1) * (1.0 - u) + ring(k) * u
    }
}

impl Expected {
    /// A ring's number at radius `m`: its four copies, each found within
    /// `NUMBER_SEARCH_MM` of its place on the axis by matching it to the others (the absolute
    /// difference, each pixel's capped, so a hole on one copy doesn't pull it), and the
    /// median of the copies so aligned.
    fn number_patch(&self, rgb: &Rgb, map: &Mapping, m: f64) -> NumberPatch {
        let n = NumberPatch::n();
        let s = (NUMBER_SEARCH_MM / NUMBER_STEP_MM).round() as i64;
        let nb = n + 2 * s as usize;
        let places = [(m, 0.0), (-m, 0.0), (0.0, m), (0.0, -m)];
        // Each copy over its patch and the search around it.
        let big: Vec<Vec<Option<[f32; 3]>>> = places
            .iter()
            .map(|c| {
                let mut v = Vec::with_capacity(nb * nb);
                for j in 0..nb {
                    for i in 0..nb {
                        let q = (
                            c.0 - NUMBER_HALF_MM - NUMBER_SEARCH_MM + i as f64 * NUMBER_STEP_MM,
                            c.1 - NUMBER_HALF_MM - NUMBER_SEARCH_MM + j as f64 * NUMBER_STEP_MM,
                        );
                        v.push(rgb.sample_at(map, q).map(|c| self.normalize(c, q)));
                    }
                }
                v
            })
            .collect();
        // Copy c's value at patch pixel (i, j), shifted by d steps.
        let at = |c: usize, d: (i64, i64), i: usize, j: usize| -> Option<[f32; 3]> {
            let (x, y) = (i as i64 + s + d.0, j as i64 + s + d.1);
            big[c][y as usize * nb + x as usize]
        };
        let median = |v: &mut Vec<f32>| -> f32 {
            v.sort_by(f32::total_cmp);
            match v.len() {
                0 => 1.0,
                l if l % 2 == 1 => v[l / 2],
                l => 0.5 * (v[l / 2 - 1] + v[l / 2]),
            }
        };
        let grey = |c: [f32; 3]| (c[0] + c[1] + c[2]) / 3.0;
        // Copy c shifted by d against a reference (per patch pixel, None: not in the photo).
        let cost = |c: usize,
                    d: (i64, i64),
                    reference: &dyn Fn(usize, usize) -> Option<f32>,
                    stride: usize|
         -> f32 {
            let mut sum = 0.0;
            for j in (0..n).step_by(stride) {
                for i in (0..n).step_by(stride) {
                    sum += match (at(c, d, i, j), reference(i, j)) {
                        (Some(x), Some(r)) => (grey(x) - r).abs().min(0.5),
                        _ => 0.25,
                    };
                }
            }
            sum
        };
        // The best shift of copy c within `reach` steps of `from`.
        let search = |c: usize,
                      from: (i64, i64),
                      reach: i64,
                      stride: usize,
                      reference: &dyn Fn(usize, usize) -> Option<f32>| {
            let mut best = (f32::INFINITY, from);
            for dy in (-reach..=reach).step_by(stride) {
                for dx in (-reach..=reach).step_by(stride) {
                    let d = (from.0 + dx, from.1 + dy);
                    if d.0.abs() > s || d.1.abs() > s {
                        continue;
                    }
                    let k = cost(c, d, reference, stride);
                    if k < best.0 {
                        best = (k, d);
                    }
                }
            }
            best
        };
        // First against one copy: each copy in turn the reference (a hole on it leads astray),
        // the others found coarsely then finely; the reference the others fit best is kept.
        let mut shift = [(0i64, 0i64); 4];
        let mut best_total = f32::INFINITY;
        for r in 0..4 {
            let reference = |i: usize, j: usize| at(r, (0, 0), i, j).map(grey);
            let mut sh = [(0i64, 0i64); 4];
            let mut total = 0.0;
            for c in (0..4).filter(|&c| c != r) {
                let coarse = search(c, (0, 0), s, 2, &reference).1;
                let (k, d) = search(c, coarse, 2, 1, &reference);
                sh[c] = d;
                total += k;
            }
            if total < best_total {
                best_total = total;
                shift = sh;
            }
        }
        // Then each against the median of the copies so aligned.
        for _ in 0..2 {
            let mut med = vec![0.0f32; n * n];
            let mut v = Vec::with_capacity(4);
            for j in 0..n {
                for i in 0..n {
                    v.clear();
                    v.extend((0..4).filter_map(|c| at(c, shift[c], i, j)).map(grey));
                    med[j * n + i] = median(&mut v);
                }
            }
            let reference = |i: usize, j: usize| Some(med[j * n + i]);
            for (c, sh) in shift.iter_mut().enumerate() {
                *sh = search(c, *sh, 2, 1, &reference).1;
            }
        }
        // Each copy against the median of the others: one that doesn't match (a hole on it,
        // out of the photo, or found only at the search's edge) is left out, there the
        // expected card is the rings' profile alone.
        let fit: Vec<f32> = (0..4)
            .map(|c| {
                let reference = |i: usize, j: usize| {
                    let mut v: Vec<f32> = (0..4)
                        .filter(|&o| o != c)
                        .filter_map(|o| at(o, shift[o], i, j))
                        .map(grey)
                        .collect();
                    Some(median(&mut v))
                };
                cost(c, shift[c], &reference, 1) / (n * n) as f32
            })
            .collect();
        let mut sorted = fit.clone();
        sorted.sort_by(f32::total_cmp);
        let good: Vec<usize> = (0..4)
            .filter(|&c| {
                fit[c] <= (NUMBER_FIT_FACTOR * sorted[1]).max(NUMBER_FIT_MIN)
                    && shift[c].0.abs() < s
                    && shift[c].1.abs() < s
            })
            .collect();
        let mut grid = vec![[0.0f32; 3]; n * n];
        let mut v = Vec::with_capacity(4);
        for j in 0..n {
            for i in 0..n {
                for k in 0..3 {
                    v.clear();
                    v.extend(good.iter().filter_map(|&c| at(c, shift[c], i, j)).map(|x| x[k]));
                    grid[j * n + i][k] = median(&mut v);
                }
            }
        }
        let centres = if good.len() >= 2 {
            good.iter()
                .map(|&c| {
                    (
                        places[c].0 + shift[c].0 as f64 * NUMBER_STEP_MM,
                        places[c].1 + shift[c].1 as f64 * NUMBER_STEP_MM,
                    )
                })
                .collect()
        } else {
            Vec::new()
        };
        if std::env::var_os("CARDPHOTO_DEBUG").is_some() {
            let mm: Vec<String> = (0..4)
                .map(|c| {
                    format!(
                        "{:+.2},{:+.2} ({:.3}{})",
                        shift[c].0 as f64 * NUMBER_STEP_MM,
                        shift[c].1 as f64 * NUMBER_STEP_MM,
                        fit[c],
                        if good.contains(&c) { "" } else { " out" }
                    )
                })
                .collect();
            eprintln!("number at {m}: copies off by {}", mm.join(" "));
        }
        NumberPatch { centres, grid }
    }
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
            warp: Warp::measure(rgb, reg),
            numbers: Vec::new(),
        };
        // The print by radius: the median over the full circle (a hole is a small part of it).
        let n_rho = ((SCORING_MM + 4.0) / e.rho_step) as usize + 1;
        let radii: Vec<usize> = (0..n_rho).collect();
        let profile: Vec<[f32; 3]> = crate::par_map(&radii, |&i| {
            let mut slot = [0.0f32; 3];
            let rho = i as f64 * e.rho_step;
            if rho < 2.0 {
                return slot; // inside the inner ten: black (a hole there covers the whole circle)
            }
            let mut buf: [Vec<f32>; 3] = [Vec::new(), Vec::new(), Vec::new()];
            let n = 720;
            for j in 0..n {
                let phi = j as f64 / n as f64 * std::f64::consts::TAU;
                let q0 = (rho * phi.cos(), rho * phi.sin());
                // Where this radius of the print lies in the photo (the warp).
                let r = rho + e.warp.at(q0);
                let q = (r * phi.cos(), r * phi.sin());
                if let Some(c) = rgb.sample_at(map, q) {
                    let v = e.normalize(c, q);
                    for k in 0..3 {
                        buf[k].push(v[k]);
                    }
                }
            }
            if buf[0].is_empty() {
                return slot;
            }
            for k in 0..3 {
                buf[k].sort_by(|a, b| a.partial_cmp(b).unwrap());
                slot[k] = buf[k][buf[k].len() / 2];
            }
            slot
        });
        e.profile = profile;
        // The numbers, where the card's axes are known (the numbers or the outline fixed them).
        if reg.axes_known() {
            let radii = crate::card::issf_air_pistol().number_radii_mm;
            e.numbers = crate::par_map(&radii, |&m| e.number_patch(rgb, map, m));
        }
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
        if let Some(v) = self.numbers.iter().find_map(|p| p.at(q)) {
            return v;
        }
        let rho = (q.0.hypot(q.1) - self.warp.at(q)).max(0.0);
        let f = (rho / self.rho_step).min((self.profile.len() - 1) as f64 - 1e-9);
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
        let row = |j: &usize| -> Vec<f32> {
            (0..n)
                .map(|i| {
                    let q = (origin.0 + i as f64 / res, origin.1 + *j as f64 / res);
                    rgb.sample_at(map, q).map_or(0.0, |c| exp.diff(c, q))
                })
                .collect()
        };
        let rows: Vec<usize> = (0..n).collect();
        // A big field (the whole scoring area) row by row on the cores; a small one (a window
        // near a shot, already on a core of its own) here.
        let d: Vec<f32> = if n * n > 100_000 {
            crate::par_map(&rows, row).concat()
        } else {
            rows.iter().flat_map(row).collect()
        };
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

/// Beyond this many edge widths outside a circle, its share of a pixel (sigmoid(-36), about
/// 2e-16) is below a double's resolution: the circle is left out there.
const EDGE_CUT: f64 = 36.0;

/// A circle's derivative at a pixel below this (by its edge's position, u) is left out of the
/// normal equations.
const GRAD_CUT: f64 = 1e-12;

/// A group's pixels: those within `reach` of the starts the group was made from, each with
/// its normalised colour and the card's expected one.
struct GroupPixels {
    pts: Vec<Sample>,
}

impl GroupPixels {
    fn new(
        rgb: &Rgb,
        map: &Mapping,
        exp: &Expected,
        area: &[(f64, f64)],
        reach: f64,
        res: f64,
    ) -> GroupPixels {
        let n = area.len();
        let (mx, my) = (
            area.iter().map(|c| c.0).sum::<f64>() / n as f64,
            area.iter().map(|c| c.1).sum::<f64>() / n as f64,
        );
        let half = area
            .iter()
            .map(|c| (c.0 - mx).abs().max((c.1 - my).abs()))
            .fold(0.0, f64::max)
            + reach;
        let g = (2.0 * half * res).ceil() as usize + 1;
        let mut pts: Vec<Sample> = Vec::new();
        for j in 0..g {
            for i in 0..g {
                let q = (mx - half + i as f64 / res, my - half + j as f64 / res);
                if !area.iter().any(|c| (q.0 - c.0).hypot(q.1 - c.1) <= reach) {
                    continue;
                }
                if let Some(c) = rgb.sample_at(map, q) {
                    pts.push((q, exp.normalize(c, q), exp.at(q)));
                }
            }
        }
        GroupPixels { pts }
    }
}

/// A group with more pixels than this has its sums worked out in chunks of this size on the
/// machine's cores (a big cluster of holes is otherwise one core's work).
const CHUNK: usize = 8192;
/// How far a centre may move in one step of the fit, mm: a circle hidden by its neighbours
/// has no say in any pixel, and the damping alone would let it run off (and drag the shared
/// radius with it).
const STEP_MAX_MM: f64 = 1.0;
/// How far from its shot a centre may go, mm (a hole beyond `SEARCH_MM` does not count).
const ANCHOR_MM: f64 = SEARCH_MM + 0.5;

/// The model of a group: n circles with a soft edge over the expected card, inside them the
/// table's colour. Parameters: the centres (2n), the radius (one shared, or one per hole),
/// the edge's width, the table's colour (3).
struct GroupModel<'a> {
    pts: &'a [Sample],
    n: usize,
}

/// A circle near a pixel: its index, sigma, u, d, dx, dy.
type Near = (usize, f64, f64, f64, f64, f64);

impl GroupModel<'_> {
    fn np(&self) -> usize {
        2 * self.n + 5
    }
    fn ir(&self) -> usize {
        2 * self.n
    }
    fn iw(&self) -> usize {
        self.np() - 4
    }
    fn it(&self) -> usize {
        self.np() - 3
    }

    /// How much of a pixel the circles cover (0..1), and with `grad`, its derivatives by the
    /// geometric parameters (centres, radii, edge width) as (index, value).
    fn cover(&self, q: &[f64], p: Pt, grad: Option<&mut Vec<(usize, f64)>>) -> f64 {
        let wq = q[self.iw()];
        let w = wq.abs().max(0.01);
        let dw = if wq.abs() > 0.01 { wq.signum() } else { 0.0 };
        let mut buf: [Near; 16] = [(0, 0.0, 0.0, 0.0, 0.0, 0.0); 16];
        let mut many: Vec<Near> = Vec::new();
        let mut m = 0;
        let mut add = |e: Near| {
            if m < buf.len() {
                buf[m] = e;
            } else {
                if many.is_empty() {
                    many.extend_from_slice(&buf);
                }
                many.push(e);
            }
            m += 1;
        };
        let r = q[self.ir()];
        for k in 0..self.n {
            let (dx, dy) = (p.0 - q[2 * k], p.1 - q[2 * k + 1]);
            let d = dx.hypot(dy);
            let u = (r - d) / w;
            if u < -EDGE_CUT {
                continue;
            }
            add((k, sigmoid(u), u, d, dx, dy));
        }
        let near: &[Near] = if many.is_empty() { &buf[..m] } else { &many };
        let mut out = 1.0;
        for e in near {
            out *= 1.0 - e.1;
        }
        if let Some(g) = grad {
            g.clear();
            // Each circle's share of the derivative needs the product over the others: from
            // running products from both ends (no division: 1 - sigma can be 0).
            let m = near.len();
            let mut before = [1.0; 17];
            let mut before_v: Vec<f64> = Vec::new();
            let before: &mut [f64] = if m < before.len() {
                &mut before[..m + 1]
            } else {
                before_v.resize(m + 1, 1.0);
                &mut before_v
            };
            for (j, e) in near.iter().enumerate() {
                before[j + 1] = before[j] * (1.0 - e.1);
            }
            let (mut gr, mut gw) = (0.0, 0.0);
            let mut after = 1.0;
            for j in (0..m).rev() {
                let (k, s, u, d, dx, dy) = near[j];
                let others = before[j] * after;
                after *= 1.0 - s;
                let ds_du = others * s * (1.0 - s);
                // Deep inside another circle, or far outside this one: no say here.
                if ds_du.abs() < GRAD_CUT {
                    continue;
                }
                gw += ds_du * (-u / w) * dw;
                gr += ds_du / w;
                if d > 1e-12 {
                    g.push((2 * k, ds_du * dx / (d * w)));
                    g.push((2 * k + 1, ds_du * dy / (d * w)));
                }
            }
            if gr != 0.0 {
                g.push((self.ir(), gr));
            }
            if gw != 0.0 {
                g.push((self.iw(), gw));
            }
        }
        1.0 - out
    }

    /// The model's colour at a pixel minus the photo's, per channel (unweighted).
    fn misfit(&self, q: &[f64], s: f64, nv: &[f32; 3], ev: &[f32; 3]) -> [f64; 3] {
        let t = self.it();
        [0, 1, 2].map(|k| {
            let e = f64::from(ev[k]);
            e + s * (q[t + k] - e) - f64::from(nv[k])
        })
    }

    /// The pixel ranges the sums are worked out in (one, or chunks on the cores).
    fn chunks(&self) -> Vec<std::ops::Range<usize>> {
        let n = self.pts.len();
        (0..n.div_ceil(CHUNK).max(1))
            .map(|i| i * CHUNK..((i + 1) * CHUNK).min(n))
            .collect()
    }

    fn sum_chunks<R: Send>(&self, f: impl Fn(std::ops::Range<usize>) -> R + Sync) -> Vec<R> {
        let chunks = self.chunks();
        if chunks.len() == 1 {
            vec![f(chunks[0].clone())]
        } else {
            crate::par_map(&chunks, |c| f(c.clone()))
        }
    }

    /// The weighted sum of squared residuals.
    fn cost(&self, q: &[f64], w: &[f64]) -> f64 {
        self.sum_chunks(|range| {
            let mut sum = 0.0;
            for i in range {
                let (p, nv, ev) = &self.pts[i];
                let s = self.cover(q, *p, None);
                let r = self.misfit(q, s, nv, ev);
                let sw = w[i].sqrt();
                sum += r.iter().map(|x| (x * sw) * (x * sw)).sum::<f64>();
            }
            sum
        })
        .into_iter()
        .sum()
    }

    /// Per pixel, the rms of its three residuals (unweighted).
    fn per_pixel(&self, q: &[f64]) -> Vec<f64> {
        self.sum_chunks(|range| {
            range
                .map(|i| {
                    let (p, nv, ev) = &self.pts[i];
                    let r = self.misfit(q, self.cover(q, *p, None), nv, ev);
                    (r.iter().map(|x| x * x).sum::<f64>() / 3.0).sqrt()
                })
                .collect::<Vec<f64>>()
        })
        .concat()
    }

    /// The normal equations of the weighted residuals: JᵀJ and Jᵀr, built pixel by pixel
    /// from each pixel's few nonzero derivatives.
    fn normal(&self, q: &[f64], w: &[f64]) -> (DMatrix<f64>, DVector<f64>) {
        let np = self.np();
        let t = self.it();
        let parts = self.sum_chunks(|range| {
            let mut a = DMatrix::<f64>::zeros(np, np);
            let mut b = DVector::<f64>::zeros(np);
            let mut g: Vec<(usize, f64)> = Vec::with_capacity(4 * self.n + 2);
            for i in range {
                let (p, nv, ev) = &self.pts[i];
                let wi = w[i];
                let s = self.cover(q, *p, Some(&mut g));
                let r = self.misfit(q, s, nv, ev);
                // Channel k's derivative row: (t_k - e_k)·g over the geometry, s at t + k.
                let mut geo2 = 0.0;
                let mut geo_r = 0.0;
                for k in 0..3 {
                    let f = q[t + k] - f64::from(ev[k]);
                    geo2 += f * f;
                    geo_r += f * r[k];
                    a[(t + k, t + k)] += wi * s * s;
                    b[t + k] += wi * s * r[k];
                    for &(ia, ga) in &g {
                        let v = wi * f * s * ga;
                        a[(ia, t + k)] += v;
                        a[(t + k, ia)] += v;
                    }
                }
                for &(ia, ga) in &g {
                    b[ia] += wi * geo_r * ga;
                    for &(ib, gb) in &g {
                        a[(ia, ib)] += wi * geo2 * ga * gb;
                    }
                }
            }
            (a, b)
        });
        let mut it = parts.into_iter();
        let (mut a, mut b) = it.next().unwrap();
        for (pa, pb) in it {
            a += pa;
            b += pb;
        }
        (a, b)
    }
}

/// Fits n circles together (a group), from starts `c0`, to the photo's normalised colours near
/// them: inside a hole the table's colour (one unknown colour), outside the card as expected
/// at each point, with a soft edge. That holds for any table, also for a hole across the
/// black disc's edge (half on expected black, half on expected white). Parameters: the
/// centres, the radius shared by the group, the edge's width, the table's colour (3). Each
/// centre stays within `ANCHOR_MM` of its `anchor` (its shot), and moves at most
/// `STEP_MAX_MM` a step. Returns the holes and the fit's mean squared residual (to compare
/// counts of circles on the same pixels).
fn fit_group(px: &GroupPixels, c0: &[(f64, f64)], anchor: &[(f64, f64)], r0: f64) -> (Vec<HoleFit>, f64) {
    let pts = &px.pts;
    let n = c0.len();
    let model = GroupModel { pts, n };
    let np = model.np();
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
    let limit = |q: &[f64], c: &mut [f64]| {
        for k in 0..n {
            let (mut x, mut y) = (c[2 * k], c[2 * k + 1]);
            let (sx, sy) = (x - q[2 * k], y - q[2 * k + 1]);
            let step = sx.hypot(sy);
            if step > STEP_MAX_MM {
                x = q[2 * k] + sx * STEP_MAX_MM / step;
                y = q[2 * k + 1] + sy * STEP_MAX_MM / step;
            }
            let (ax, ay) = (x - anchor[k].0, y - anchor[k].1);
            let off = ax.hypot(ay);
            if off > ANCHOR_MM {
                x = anchor[k].0 + ax * ANCHOR_MM / off;
                y = anchor[k].1 + ay * ANCHOR_MM / off;
            }
            c[2 * k] = x;
            c[2 * k + 1] = y;
        }
        let i = model.ir();
        c[i] = c[i].clamp(0.6 * r0, 1.3 * r0);
        let i = model.iw();
        c[i] = c[i].clamp(0.02, 0.8);
    };
    let mut w = vec![1.0; pts.len()];
    for round in 0..3 {
        let mut lambda = 1e-3;
        let mut cost = model.cost(&q, &w);
        for _ in 0..60 {
            let (a, gv) = model.normal(&q, &w);
            let mut improved = false;
            let mut done = false;
            for _ in 0..15 {
                let mut ad = a.clone();
                for d in 0..np {
                    ad[(d, d)] += lambda * a[(d, d)].max(1e-12);
                }
                let Some(step) = ad.lu().solve(&(-&gv)) else { break };
                let mut cand: Vec<f64> = q.iter().zip(step.iter()).map(|(a, b)| a + b).collect();
                limit(&q, &mut cand);
                let c = model.cost(&cand, &w);
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
            let per = model.per_pixel(&q);
            let mut a = per.clone();
            a.sort_by(|x, y| x.partial_cmp(y).unwrap());
            let k = (1.5 * 1.4826 * a[a.len() / 2]).max(0.01);
            w = per.iter().map(|r| if *r <= k { 1.0 } else { k / r }).collect();
        }
    }
    // Uncertainties from the unweighted fit at the end.
    let ones = vec![1.0; pts.len()];
    let sum2 = model.cost(&q, &ones);
    let m = 3 * pts.len();
    let s2 = sum2 / (m as f64 - np as f64).max(1.0);
    let (jtj, _) = model.normal(&q, &ones);
    let cov = jtj
        .pseudo_inverse(1e-12)
        .unwrap_or_else(|_| DMatrix::zeros(np, np))
        * s2;
    let t = model.it();
    let table = [q[t], q[t + 1], q[t + 2]];
    let mse = sum2 / m.max(1) as f64;
    let fits = (0..n)
        .map(|k| {
            // How much the table differs from the card inside this hole (0 ink .. 1 paper units).
            let c = (q[2 * k], q[2 * k + 1]);
            let r = q[model.ir()];
            let inner: Vec<f64> = pts
                .iter()
                .filter(|p| (p.0.0 - c.0).hypot(p.0.1 - c.1) < 0.7 * r)
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
                radius_mm: r,
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
    /// Whether its hole overlaps others (fitted together with them: less certain).
    pub overlap: bool,
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
    /// Whether the quarter turn was judged on all the shots (not only on a part).
    pub quarter_all: bool,
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

/// A hole counts as ground truth only if its fit pins it down: its centre known to this,
/// mm (deep in a shot-out cluster a circle can slide where the paper is gone), and (the old
/// model's circles) its radius within `RADIUS_SPREAD_MM` of the photo's other holes (the
/// median).
const SD_MAX_MM: f64 = 0.25;
const RADIUS_SPREAD_MM: f64 = 0.2;
/// The quarter turn counts a shot's coarse hole only if it agrees with one placement for all
/// of them within this, mm: at the wrong turn the holes found near the turned shots (in a
/// tight group, the group's own) are scattered.
const QUARTER_AGREE_MM: f64 = 2.0;

/// Per hole of the old model, whether it is pinned down (`SD_MAX_MM`, `RADIUS_SPREAD_MM`)
/// among the holes that count for their starts (`hole_ok`); those that don't count, false.
fn pinned(fits: &[HoleFit], starts: &[(f64, f64)], r0: f64) -> Vec<bool> {
    let ok: Vec<bool> = fits.iter().zip(starts).map(|(h, s)| hole_ok(h, *s, r0)).collect();
    let mut radii: Vec<f64> = fits
        .iter()
        .zip(&ok)
        .filter(|(_, k)| **k)
        .map(|(h, _)| h.radius_mm)
        .collect();
    if radii.is_empty() {
        return ok;
    }
    radii.sort_by(|a, b| a.total_cmp(b));
    let median = radii[radii.len() / 2];
    fits.iter()
        .zip(ok)
        .map(|(h, k)| k && h.sd_mm <= SD_MAX_MM && (h.radius_mm - median).abs() < RADIUS_SPREAD_MM)
        .collect()
}

/// Whether an old model's hole counts for its start (its shot as placed): clear, round, and
/// near.
fn hole_ok(h: &HoleFit, start: (f64, f64), r0: f64) -> bool {
    h.contrast >= MIN_CONTRAST
        && (h.radius_mm - r0).abs() < 0.6
        && (h.center.0 - start.0).hypot(h.center.1 - start.1) < SEARCH_MM
}

/// Fits the holes at card points `starts` (grouped when close; the groups on the machine's
/// cores, the biggest first).
fn fit_at(
    rgb: &Rgb,
    reg: &Registration,
    exp: &Expected,
    starts: &[(f64, f64)],
    res: f64,
    r0: f64,
) -> Vec<HoleFit> {
    let mut groups = groups(starts, 2.0 * r0 + 1.0);
    groups.sort_by_key(|g| std::cmp::Reverse(g.len()));
    let fitted: Vec<Vec<HoleFit>> = crate::par_map(&groups, |g| {
        let cs: Vec<(f64, f64)> = g.iter().map(|&i| starts[i]).collect();
        let px = GroupPixels::new(rgb, &reg.map, exp, &cs, r0 + 1.5, res);
        fit_group(&px, &cs, &cs, r0).0
    });
    let mut out = vec![None; starts.len()];
    for (g, fits) in groups.iter().zip(fitted) {
        for (k, h) in fits.into_iter().enumerate() {
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
    // A hole is round and clearly different from the card.
    candidates(rgb, reg, exp, skip, SCORING_MM, 0.5)
        .into_iter()
        .filter(|h| {
            h.contrast >= MIN_CONTRAST
                && (h.radius_mm - r0).abs() < 0.5
                && h.center.0.hypot(h.center.1) <= SCORING_MM
        })
        .collect()
}

/// Every hole in the scoring area, without shots, as the gauge sees it (C5): the blobs' fits
/// (the openings), each with the gauge's circle from there, for a calibre of `diameter_mm`.
/// Kept: those whose circle is near the calibre's size and clearly damaged inside, one per
/// hole; overlapping holes fitted again together (`refit_overlaps`).
pub fn all_gauges(rgb: &Rgb, reg: &Registration, diameter_mm: f64) -> Vec<(HoleFit, GaugeFit)> {
    let exp = Expected::from_photo(rgb, reg);
    unguided_gauges(rgb, reg, &exp, diameter_mm)
}

fn unguided_gauges(
    rgb: &Rgb,
    reg: &Registration,
    exp: &Expected,
    diameter_mm: f64,
) -> Vec<(HoleFit, GaugeFit)> {
    let r_cal = diameter_mm / 2.0;
    // A torn hole's opening and its rim can be separate blobs, each smaller than the pellet's
    // disc: smaller blobs are tried too (the gauge rejects what has no round edge).
    let found = candidates(rgb, reg, exp, &[], SCORING_MM + 2.0, 0.2);
    let fits: Vec<Option<GaugeFit>> =
        crate::par_map(&found, |h| single_gauge(rgb, reg, exp, h.center, diameter_mm));
    let mut out: Vec<(HoleFit, GaugeFit)> = found
        .into_iter()
        .zip(fits)
        .filter_map(|(h, g)| g.map(|g| (h, g)))
        .collect();
    // Two starts in one hole (a torn one fitted as two openings) give two circles on the same
    // damage: the smoother one is kept.
    out.sort_by(|a, b| a.1.rough_mm.total_cmp(&b.1.rough_mm));
    let mut keep: Vec<(HoleFit, GaugeFit)> = Vec::new();
    for (h, g) in out.drain(..) {
        if keep
            .iter()
            .all(|(_, k)| (k.center.0 - g.center.0).hypot(k.center.1 - g.center.1) > r_cal)
        {
            keep.push((h, g));
        }
    }
    let seeds: Vec<(f64, f64)> = keep.iter().map(|(_, g)| g.center).collect();
    let mut fits: Vec<Option<GaugeFit>> = keep.iter().map(|(_, g)| Some(g.clone())).collect();
    refit_overlaps(rgb, reg, exp, &seeds, &mut fits, diameter_mm);
    keep.into_iter()
        .zip(fits)
        .filter_map(|((h, _), g)| g.map(|g| (h, g)))
        .collect()
}

/// Whether a gauge circle can be a hole's mark: the calibre's circle or a little outside it (a
/// smear, a halo), not the print's details (a ring's number: radius 0.9–1.6 mm), and clearly
/// damaged inside.
fn gauge_ok(g: &GaugeFit, r_cal: f64) -> bool {
    (GAUGE_R_MIN * r_cal..GAUGE_R_MAX * r_cal).contains(&g.radius_mm) && g.damage >= MIN_CONTRAST
}

/// One hole's gauge circle from `seed`, if it can be a hole's mark.
fn single_gauge(
    rgb: &Rgb,
    reg: &Registration,
    exp: &Expected,
    seed: (f64, f64),
    diameter_mm: f64,
) -> Option<GaugeFit> {
    gauge(rgb, &reg.map, exp, &[seed], diameter_mm, None)
        .and_then(|g| g.into_iter().next())
        .filter(|g| gauge_ok(g, diameter_mm / 2.0))
}

/// Overlapping holes (`seeds` closer than two typical radii and a margin) fitted again
/// together: their circles' outer arcs make the outline, the radius held at the median of the
/// single holes in the same zone (black or white: the mark's edge differs), or one shared
/// radius fitted if there are none. If the joint fit fails, none of them counts.
fn refit_overlaps(
    rgb: &Rgb,
    reg: &Registration,
    exp: &Expected,
    seeds: &[(f64, f64)],
    fits: &mut [Option<GaugeFit>],
    diameter_mm: f64,
) {
    let r_cal = diameter_mm / 2.0;
    let r_typ = {
        let mut r: Vec<f64> = fits.iter().flatten().map(|g| g.radius_mm).collect();
        r.sort_by(f64::total_cmp);
        r.get(r.len() / 2).copied().unwrap_or(r_cal)
    };
    let groups = groups(seeds, 2.0 * r_typ + 0.5);
    let in_black = |c: (f64, f64)| c.0.hypot(c.1) < BLACK_MM;
    let zone_r = |black: bool| {
        let mut r: Vec<f64> = groups
            .iter()
            .filter(|g| g.len() == 1 && in_black(seeds[g[0]]) == black)
            .filter_map(|g| fits[g[0]].as_ref().map(|f| f.radius_mm))
            .collect();
        r.sort_by(f64::total_cmp);
        r.get(r.len() / 2).copied()
    };
    let radius = [zone_r(false), zone_r(true)];
    let multi: Vec<&Vec<usize>> = groups.iter().filter(|g| g.len() > 1).collect();
    let again: Vec<Option<Vec<GaugeFit>>> = crate::par_map(&multi, |g| {
        let starts: Vec<(f64, f64)> = g.iter().map(|&i| seeds[i]).collect();
        let black = starts.iter().filter(|c| in_black(**c)).count() * 2 > starts.len();
        gauge(
            rgb,
            &reg.map,
            exp,
            &starts,
            diameter_mm,
            radius[usize::from(black)],
        )
        .filter(|f| f.iter().all(|f| gauge_ok(f, r_cal)))
    });
    for (g, joint) in multi.iter().zip(again) {
        for (k, &i) in g.iter().enumerate() {
            fits[i] = joint.as_ref().map(|f| f[k].clone());
        }
    }
}

/// The blobs where the photo differs from the card within `reach` of the centre, of at least
/// `min_area` of the pellet's disc, each fitted as one or more holes (unfiltered).
fn candidates(
    rgb: &Rgb,
    reg: &Registration,
    exp: &Expected,
    skip: &[(f64, f64)],
    reach: f64,
    min_area: f64,
) -> Vec<HoleFit> {
    let r0 = HOLE_D_MM / 2.0;
    let res = 5.0;
    let f = Field::new(rgb, &reg.map, exp, (0.0, 0.0), SCORING_MM + 4.0, res);
    // Blobs: where the photo differs from the card, in the scoring area, away from the holes
    // already found.
    let on: Vec<bool> = (0..f.n * f.n)
        .map(|k| {
            let p = f.point(k % f.n, k / f.n);
            f.d[k] > 0.2
                && p.0.hypot(p.1) <= reach
                && skip.iter().all(|s| (s.0 - p.0).hypot(s.1 - p.1) > r0 + 0.6)
        })
        .collect();
    let mut seen = vec![false; on.len()];
    let disc = std::f64::consts::PI * r0 * r0;
    let mut blobs: Vec<Vec<(f64, f64)>> = Vec::new();
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
        if area < min_area * disc {
            continue; // the print's details (a number's strokes), dust
        }
        blobs.push(blob.iter().map(|&k| f.point(k % f.n, k / f.n)).collect());
    }
    // Each blob on its own core: how many holes, from the area, then one or two more if they
    // clearly fit better.
    crate::par_map(&blobs, |pts| {
        let area = pts.len() as f64 / (res * res);
        let n0 = ((area / disc).round() as usize).clamp(1, 6);
        let mut best: Option<(Vec<HoleFit>, f64)> = None;
        for n in n0..=(n0 + 2).min(8) {
            let starts = farthest_points(pts, n);
            let px = GroupPixels::new(rgb, &reg.map, exp, &starts, r0 + 1.5, 10.0);
            let (fits, mse) = fit_group(&px, &starts, &starts, r0);
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
        best.unwrap().0
    })
    .into_iter()
    .flatten()
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

/// The quarter turn is first judged on this many shots, spread over the series (the outer
/// shots tell the turns apart best); on all of them when it is not clear.
const QUARTER_SHOTS: usize = 10;
/// The quarter turn is clear when the runner-up scores less than this share of the best.
const QUARTER_CLEAR: f64 = 0.8;

/// `n` of the points, spread out: the farthest from the mean first, then each the farthest
/// from those chosen. Indices, in the points' order.
fn spread(points: &[(f64, f64)], n: usize) -> Vec<usize> {
    if points.len() <= n {
        return (0..points.len()).collect();
    }
    let m = (
        points.iter().map(|p| p.0).sum::<f64>() / points.len() as f64,
        points.iter().map(|p| p.1).sum::<f64>() / points.len() as f64,
    );
    let d2 = |a: (f64, f64), b: (f64, f64)| (a.0 - b.0).powi(2) + (a.1 - b.1).powi(2);
    let mut near: Vec<f64> = points.iter().map(|p| d2(*p, m)).collect();
    let mut chosen = Vec::new();
    for _ in 0..n {
        let i = (0..points.len())
            .filter(|i| !chosen.contains(i))
            .max_by(|&a, &b| near[a].total_cmp(&near[b]).then(b.cmp(&a)))
            .unwrap();
        chosen.push(i);
        for (k, p) in points.iter().enumerate() {
            near[k] = if chosen.len() == 1 {
                d2(*p, points[i])
            } else {
                near[k].min(d2(*p, points[i]))
            };
        }
    }
    chosen.sort();
    chosen
}

/// The quarter turn, from where the shots' holes show best and agree on one placement
/// (`QUARTER_AGREE_MM`), and the best filter response near each shot at that turn (position,
/// value): the coarse holes.
struct Quarter {
    k: u8,
    near: Vec<((f64, f64), f64)>,
    /// Judged on a part of the shots only.
    part: bool,
}

fn find_quarter(
    rgb: &Rgb,
    reg: &Registration,
    exp: &Expected,
    shots: &[(f64, f64)],
    all: bool,
    r0: f64,
) -> Quarter {
    let quarter = std::f64::consts::FRAC_PI_2;
    let turn = |k: u8, s: (f64, f64)| {
        let (sn, cs) = (f64::from(k) * quarter).sin_cos();
        (cs * s.0 - sn * s.1, sn * s.0 + cs * s.1)
    };
    let near = |c: (f64, f64)| {
        Field::new(rgb, &reg.map, exp, c, SEARCH_MM + 2.0 * r0, 5.0).best_near(c, SEARCH_MM, r0)
    };
    let idx = if all {
        (0..shots.len()).collect()
    } else {
        spread(shots, QUARTER_SHOTS)
    };
    let part = idx.len() < shots.len();
    let tasks: Vec<(u8, usize)> = (0..4u8).flat_map(|k| idx.iter().map(move |&i| (k, i))).collect();
    let found: Vec<Option<((f64, f64), f64)>> = crate::par_map(&tasks, |&(k, i)| {
        let c = turn(k, shots[i]);
        (c.0.hypot(c.1) <= SCORING_MM + SEARCH_MM).then(|| near(c))
    });
    // Per turn: the clear coarse holes, the one placement they make, and the sum of the
    // filter over those that agree with it.
    let mut scores = [0.0f64; 4];
    for (k, score) in scores.iter_mut().enumerate() {
        let clear: Vec<(Pair, f64)> = tasks
            .iter()
            .zip(&found)
            .filter(|(t, _)| t.0 as usize == k)
            .filter_map(|(t, f)| {
                f.filter(|f| f.1 >= MIN_CONTRAST)
                    .map(|f| ((shots[t.1], f.0), f.1))
            })
            .collect();
        let pairs: Vec<Pair> = clear.iter().map(|c| c.0).collect();
        if pairs.is_empty() {
            continue;
        }
        let pl = fit_placement(&pairs, k as f64 * quarter, false);
        *score = clear
            .iter()
            .filter(|((s, h), _)| {
                let p = pl.apply(*s);
                (h.0 - p.0).hypot(h.1 - p.1) < QUARTER_AGREE_MM
            })
            .map(|(_, v)| *v)
            .sum();
    }
    let mut best: Option<(f64, u8)> = None;
    for (k, &score) in scores.iter().enumerate() {
        if best.is_none_or(|(b, _)| score > b) {
            best = Some((score, k as u8));
        }
    }
    let (top, k) = best.unwrap();
    let second = (0..4)
        .filter(|&j| j != k as usize)
        .map(|j| scores[j])
        .fold(f64::NEG_INFINITY, f64::max);
    if part && !(top > 0.0 && second < QUARTER_CLEAR * top) {
        return find_quarter(rgb, reg, exp, shots, true, r0);
    }
    // The coarse holes at that turn: those already found, then the rest.
    let mut at: Vec<Option<((f64, f64), f64)>> = vec![None; shots.len()];
    for (t, f) in tasks.iter().zip(found) {
        if t.0 == k {
            at[t.1] = f;
        }
    }
    let missing: Vec<usize> = (0..shots.len()).filter(|&i| at[i].is_none()).collect();
    for (i, f) in missing
        .iter()
        .zip(crate::par_map(&missing, |&i| near(turn(k, shots[i]))))
    {
        at[*i] = Some(f);
    }
    Quarter {
        k,
        near: at.into_iter().map(Option::unwrap).collect(),
        part,
    }
}

/// The holes and the placement from a quarter turn: coarse holes near each shot, then the
/// placement, then the holes fitted from the placed shots (close ones together), then the
/// placement again, twice.
struct Matched {
    placement: Placement,
    shots: Vec<ShotResult>,
    rms_mm: f64,
    scaled: bool,
    /// Pairs left out as far beyond the rest, in the last round.
    dropped: usize,
}

fn match_from(
    rgb: &Rgb,
    reg: &Registration,
    exp: &Expected,
    shots: &[(f64, f64)],
    q: &Quarter,
    diameter_mm: f64,
) -> Result<Matched, String> {
    let r0 = diameter_mm / 2.0;
    let mut placement = Placement {
        angle: f64::from(q.k) * std::f64::consts::FRAC_PI_2,
        quarter: q.k,
        shift: (0.0, 0.0),
        scale: 1.0,
    };
    let pairs: Vec<Pair> = shots
        .iter()
        .zip(&q.near)
        .filter(|(_, (_, v))| *v >= MIN_CONTRAST)
        .map(|(s, (p, _))| (*s, *p))
        .collect();
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
    // The holes near the placed shots. The first round places the card from the old model's
    // circles (quick; the openings are near enough for that). The second: a hole that
    // overlaps another is the old model's (the table seen through the circles' union: better
    // for overlapping holes with shots, the synthetic pairs show; flagged), a single hole the
    // gauge's circle (the mark's edge, C5), seeded by the old model's circle where it found
    // one near the shot.
    let holes_at = |starts: &[(f64, f64)], gauge: bool| -> Vec<Option<(HoleFit, bool)>> {
        let opening = fit_at(rgb, reg, exp, starts, 10.0, r0);
        let good = pinned(&opening, starts, r0);
        let at: Vec<(f64, f64)> = opening
            .iter()
            .zip(starts)
            .map(|(h, s)| {
                let near = (h.center.0 - s.0).hypot(h.center.1 - s.1) < SEARCH_MM;
                if near && h.contrast >= MIN_CONTRAST {
                    h.center
                } else {
                    *s
                }
            })
            .collect();
        let overlap: Vec<bool> = (0..at.len())
            .map(|i| (0..at.len()).any(|j| j != i && (at[i].0 - at[j].0).hypot(at[i].1 - at[j].1) < 2.0 * r0))
            .collect();
        let singles: Vec<usize> = (0..at.len()).filter(|&i| gauge && !overlap[i]).collect();
        // Seeded at the old model's circle and at the shot (a long torn mark can give a circle on
        // one lobe from one seed and on the whole mark from another): the smoother kept.
        let gauged: Vec<Option<GaugeFit>> = crate::par_map(&singles, |&i| {
            let seeds: &[(f64, f64)] = if (at[i].0 - starts[i].0).hypot(at[i].1 - starts[i].1) > 0.3 {
                &[at[i], starts[i]]
            } else {
                &[at[i]]
            };
            seeds
                .iter()
                .filter_map(|&s| single_gauge(rgb, reg, exp, s, diameter_mm))
                .filter(|g| {
                    (g.center.0 - starts[i].0).hypot(g.center.1 - starts[i].1) < SEARCH_MM
                        && g.sd_mm <= SD_MAX_MM
                })
                .min_by(|a, b| a.rough_mm.total_cmp(&b.rough_mm))
        });
        let mut out: Vec<Option<(HoleFit, bool)>> = opening
            .iter()
            .cloned()
            .zip(&good)
            .zip(&overlap)
            .map(|((h, g), o)| g.then_some((h, *o)))
            .collect();
        // The gauge's circle, unless it is smaller than the pellet or than the opening the old
        // model pinned down: then it didn't see the hole's edge (a table as dark as the ink)
        // but something inside, and the old model's circle stays.
        let inside = |i: usize, g: &GaugeFit| {
            g.radius_mm < MARK_MIN * r0 || (good[i] && g.radius_mm < opening[i].radius_mm - INSIDE_MM)
        };
        for (&i, g) in singles.iter().zip(gauged) {
            let g = g.filter(|g| !inside(i, g));
            if let Some(g) = g {
                out[i] = Some((
                    HoleFit {
                        center: g.center,
                        radius_mm: g.radius_mm,
                        sd_mm: g.sd_mm,
                        contrast: g.damage,
                    },
                    false,
                ));
            } else if !good[i] {
                out[i] = None;
            }
        }
        // Holes whose circles cross overlap too, however far apart their openings were.
        let circles: Vec<Option<((f64, f64), f64)>> = out
            .iter()
            .map(|h| h.as_ref().map(|h| (h.0.center, h.0.radius_mm)))
            .collect();
        for i in 0..out.len() {
            let Some((ci, ri)) = circles[i] else { continue };
            let crossed = circles
                .iter()
                .enumerate()
                .any(|(j, c)| j != i && c.is_some_and(|(cj, rj)| (ci.0 - cj.0).hypot(ci.1 - cj.1) < ri + rj));
            if crossed {
                out[i].as_mut().unwrap().1 = true;
            }
        }
        out
    };
    let mut fits: Vec<Option<(HoleFit, bool)>> = Vec::new();
    let mut dropped = 0;
    let pairs_of = |fits: &[Option<(HoleFit, bool)>]| -> Vec<Pair> {
        shots
            .iter()
            .zip(fits)
            .filter_map(|(s, h)| h.as_ref().map(|h| (*s, h.0.center)))
            .collect()
    };
    for round in 0..2 {
        let starts: Vec<(f64, f64)> = shots.iter().map(|s| placement.apply(*s)).collect();
        fits = holes_at(&starts, round == 1);
        if round == 0 && pairs_of(&fits).is_empty() {
            // No hole the old model sees (a scan: the lid's white in the holes): the gauge's.
            fits = holes_at(&starts, true);
        }
        let mut pairs = pairs_of(&fits);
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
        dropped = 0;
        if r.iter().any(|x| *x > limit) && pairs.len() > 2 {
            let before = pairs.len();
            pairs = pairs
                .into_iter()
                .zip(&r)
                .filter(|(_, x)| **x <= limit)
                .map(|(p, _)| p)
                .collect();
            dropped = before - pairs.len();
            placement = fit_placement(&pairs, placement.angle, with_scale);
        }
    }
    let starts: Vec<(f64, f64)> = shots.iter().map(|s| placement.apply(*s)).collect();
    let results: Vec<ShotResult> = fits
        .into_iter()
        .zip(&starts)
        .map(
            |(h, st)| match h.filter(|h| (h.0.center.0 - st.0).hypot(h.0.center.1 - st.1) < SEARCH_MM) {
                Some((h, overlap)) => ShotResult {
                    diff_mm: Some((h.center.0 - st.0, h.center.1 - st.1)),
                    overlap,
                    hole: Some(h),
                },
                None => ShotResult {
                    hole: None,
                    overlap: false,
                    diff_mm: None,
                },
            },
        )
        .collect();
    let d: Vec<f64> = results
        .iter()
        .filter_map(|r| r.diff_mm.map(|d| d.0 * d.0 + d.1 * d.1))
        .collect();
    let rms = (d.iter().sum::<f64>() / d.len().max(1) as f64).sqrt();
    Ok(Matched {
        placement,
        shots: results,
        rms_mm: rms,
        scaled: with_scale,
        dropped,
    })
}

/// The ground truth of a card: the holes of the shots (frame mm, as the sensor measured
/// them), the card's placement, the differences, and holes no shot explains.
///
/// The quarter turn is judged on part of the shots when there are many; if then a shot finds
/// no hole, or pairs are left out as outliers, it is judged again on all of them, and if
/// that gives another turn, the turn that matches more holes (then the smaller rms) wins.
pub fn ground_truth(
    rgb: &Rgb,
    reg: &Registration,
    shots: &[(f64, f64)],
    diameter_mm: f64,
) -> Result<GroundTruth, String> {
    if shots.is_empty() {
        return Err("no shots to match".into());
    }
    let r0 = diameter_mm / 2.0;
    let exp = Expected::from_photo(rgb, reg);
    let q = find_quarter(rgb, reg, &exp, shots, false, r0);
    let first = match_from(rgb, reg, &exp, shots, &q, diameter_mm);
    let doubtful = match &first {
        Ok(m) => m.dropped > 0 || m.shots.iter().any(|r| r.hole.is_none()),
        Err(_) => true,
    };
    let mut quarter_all = !q.part;
    let mut best = first;
    if q.part && doubtful {
        quarter_all = true;
        let q2 = find_quarter(rgb, reg, &exp, shots, true, r0);
        if q2.k != q.k {
            let second = match_from(rgb, reg, &exp, shots, &q2, diameter_mm);
            let found = |m: &Matched| m.shots.iter().filter(|r| r.hole.is_some()).count();
            let take = match (&best, &second) {
                (Err(_), Ok(_)) => true,
                (Ok(a), Ok(b)) => found(b) > found(a) || (found(b) == found(a) && b.rms_mm < a.rms_mm),
                _ => false,
            };
            if take {
                best = second;
            }
        }
    }
    let m = best?;
    let found: Vec<(f64, f64)> = m
        .shots
        .iter()
        .filter_map(|r| r.hole.as_ref().map(|h| h.center))
        .collect();
    // Holes no shot explains: the unguided search's, away from those found.
    let extra = unguided_gauges(rgb, reg, &exp, diameter_mm)
        .into_iter()
        .map(|(_, g)| g)
        .filter(|g| {
            g.center.0.hypot(g.center.1) <= SCORING_MM
                && found
                    .iter()
                    .all(|f| (f.0 - g.center.0).hypot(f.1 - g.center.1) > r0)
        })
        .map(|g| HoleFit {
            center: g.center,
            radius_mm: g.radius_mm,
            sd_mm: g.sd_mm,
            contrast: g.damage,
        })
        .collect();
    Ok(GroundTruth {
        placement: m.placement,
        shots: m.shots,
        rms_mm: m.rms_mm,
        extra,
        scaled: m.scaled,
        quarter_all,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Three overlapping circles' pixels (no photo needed): the expected card a smooth
    /// pattern, the photo the model at slightly other parameters plus a ripple.
    fn pixels() -> Vec<Sample> {
        let mut pts = Vec::new();
        for j in 0..90 {
            for i in 0..110 {
                let q = (-4.0 + i as f64 * 0.1, -4.0 + j as f64 * 0.1);
                let e = [0.2 + 0.1 * (q.0).sin() as f32, 0.6, 0.9 - 0.05 * q.1 as f32];
                let n = [
                    e[0] + 0.3 * (q.0 * 3.0).cos() as f32,
                    e[1] - 0.2,
                    e[2] + 0.1 * (q.1 * 2.0).sin() as f32,
                ];
                pts.push((q, n, e));
            }
        }
        pts
    }

    #[test]
    fn derivatives() {
        let pts = pixels();
        let model = GroupModel { pts: &pts, n: 3 };
        let mut q = vec![-1.0, 0.2, 1.1, -0.3, 0.4, 1.6];
        q.extend([2.2, 0.17, 0.3, 0.5, 0.4]);
        assert_eq!(q.len(), model.np());
        let w: Vec<f64> = (0..pts.len()).map(|i| 0.5 + (i % 7) as f64 / 7.0).collect();
        let resid = |q: &[f64]| -> Vec<f64> {
            let mut out = Vec::new();
            for ((p, nv, ev), wi) in pts.iter().zip(&w) {
                let r = model.misfit(q, model.cover(q, *p, None), nv, ev);
                out.extend(r.iter().map(|x| x * wi.sqrt()));
            }
            out
        };
        let np = model.np();
        let r0 = resid(&q);
        let mut jac = DMatrix::<f64>::zeros(r0.len(), np);
        for p in 0..np {
            let h = 1e-6;
            let (mut a, mut b) = (q.clone(), q.clone());
            a[p] += h;
            b[p] -= h;
            let (ra, rb) = (resid(&a), resid(&b));
            for i in 0..r0.len() {
                jac[(i, p)] = (ra[i] - rb[i]) / (2.0 * h);
            }
        }
        let want_a = jac.transpose() * &jac;
        let want_b = jac.transpose() * DVector::from_vec(r0.clone());
        let (a, b) = model.normal(&q, &w);
        let scale = want_a.amax();
        assert!(
            (&a - &want_a).amax() < 1e-6 * scale,
            "JᵀJ off by {}",
            (&a - &want_a).amax() / scale
        );
        assert!(
            (&b - &want_b).amax() < 1e-6 * want_b.amax(),
            "Jᵀr off by {}",
            (&b - &want_b).amax()
        );
        let cost: f64 = r0.iter().map(|x| x * x).sum();
        assert!((model.cost(&q, &w) - cost).abs() < 1e-9 * cost);
    }

    #[test]
    fn spread_takes_the_outer_points() {
        let pts = [(0.0, 0.0), (10.0, 0.0), (0.1, 0.1), (-10.0, 0.0), (0.0, 9.0)];
        assert_eq!(spread(&pts, 3), vec![1, 3, 4]);
        assert_eq!(spread(&pts, 9), vec![0, 1, 2, 3, 4]);
    }
}
