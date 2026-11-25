//! C5 (`app/CARDPHOTO.md`): a hole as the scoring gauge sees it.
//!
//! Pellets tear the card more often than they punch a clean disc. What a photo shows is a
//! small opening (often off-centre), torn paper and flaps around it, and a rim: in the white
//! the pellet's grey smear, in the black a halo where the ink flaked off. The rim's outer
//! edge is where the pellet's side cut the card: a circle of about the calibre. That edge is
//! what a plug gauge settles into, and what the ISSF's transparent overlay for torn holes is
//! laid over; so it is what is fitted here, not the opening.
//!
//! From a start, rays go out every 1°; on each, the damage (how far the photo is from the
//! expected card, `Expected::diff`) is followed out to where it ends: the last drop below
//! `LEVEL` within `REACH_MM` past the calibre's radius (the circle's once found; the first
//! round `SEED_SLACK_MM` further, for a start off the centre), with intact card after it for
//! `CLEAR_MM`, placed at half the rim's height. Flaps inside (card-coloured) are passed over; a neighbour's damage further out
//! is not reached. A circle is fitted to those edge points (robust weights: a tear sticking out
//! counts for nothing), the rays cast again from its centre until it settles. Overlapping holes
//! are fitted together: their circles' outer arcs make the outline.

use crate::holes::Expected;
use crate::{Mapping, Rgb};

/// A point in card mm.
type Pt = (f64, f64);

/// Rays per hole.
const RAYS: usize = 360;
/// The step along a ray, mm.
const STEP_MM: f64 = 0.04;
/// The damage level that finds an edge (`Expected::diff`'s units: the paper–ink range); the
/// edge is then placed at the rim's half height.
pub const LEVEL: f64 = 0.15;
/// How far a start may be off the hole's centre, mm (the first round looks this much further).
const SEED_SLACK_MM: f64 = 1.0;
/// How far past the calibre's radius (or the circle's, if larger) an edge is looked for, mm.
const REACH_MM: f64 = 1.0;
/// How far inside an edge the rim's peak is looked for, mm.
const PEAK_MM: f64 = 0.6;
/// How far the card must be intact after an edge, mm.
const CLEAR_MM: f64 = 0.4;
/// The least share of the rays with an edge for a fit.
const MIN_SHARE: f64 = 0.4;

#[derive(Clone, Debug)]
pub struct GaugeFit {
    /// The circle's centre, card mm, its radius fitted too (or, for overlapping holes, held at
    /// the card's).
    pub center: Pt,
    pub radius_mm: f64,
    /// The centre's 1σ (from the edge points' scatter; neighbouring rays are correlated, so a
    /// lower bound), mm.
    pub sd_mm: f64,
    /// The edge points on this circle (of `RAYS` per hole; overlapping holes share theirs).
    pub rays: usize,
    /// The edge points' robust spread about the circle (1.4826 · MAD), mm.
    pub rough_mm: f64,
    /// The mean damage inside 0.8 of the radius.
    pub damage: f64,
    /// How many holes were fitted together (overlapping), this one included: 1 alone.
    pub together: usize,
}

/// The damage along one ray from `c` at angle `a`, smoothed over three steps.
fn profile(rgb: &Rgb, map: &Mapping, exp: &Expected, c: Pt, a: f64, n: usize) -> Vec<f64> {
    let (ca, sa) = (a.cos(), a.sin());
    let raw: Vec<f64> = (0..n)
        .map(|i| {
            let r = i as f64 * STEP_MM;
            let q = (c.0 + r * ca, c.1 + r * sa);
            rgb.sample_at(map, q)
                .map_or(f64::NAN, |v| f64::from(exp.diff(v, q)))
        })
        .collect();
    (0..n)
        .map(|i| {
            let lo = i.saturating_sub(1);
            let hi = (i + 1).min(n - 1);
            raw[lo..=hi].iter().sum::<f64>() / (hi - lo + 1) as f64
        })
        .collect()
}

/// Where the damage ends on a profile, mm from its start. Found as the last drop below
/// `LEVEL` before `limit` samples with `clear` intact samples after it; then placed at half the
/// rim's height there: halfway between the damage's peak within `PEAK_MM` inside and the
/// intact card after it (the median), so a faint rim (a scan) and a strong one (a photo's
/// shadowed opening) give the same edge.
fn edge(d: &[f64], limit: usize, clear: usize) -> Option<f64> {
    let peak_n = (PEAK_MM / STEP_MM).round() as usize;
    let mut found = None;
    let mut seen = false;
    for i in 1..limit.min(d.len()) {
        if d[i - 1].is_nan() || d[i].is_nan() {
            break; // off the photo
        }
        if d[i - 1] > LEVEL {
            seen = true;
        }
        if seen && d[i - 1] > LEVEL && d[i] <= LEVEL {
            let after = &d[i..(i + clear).min(d.len())];
            if after.len() == clear && after.iter().all(|v| *v <= LEVEL) {
                found = Some(i);
            }
        }
    }
    let i = found?;
    let lo = i.saturating_sub(peak_n);
    let (ip, peak) = (lo..i)
        .map(|k| (k, d[k]))
        .fold((i - 1, f64::MIN), |a, b| if b.1 > a.1 { b } else { a });
    let mut after: Vec<f64> = d[i..i + clear].to_vec();
    after.sort_by(f64::total_cmp);
    let base = after[after.len() / 2];
    let half = 0.5 * (peak + base);
    let k = (ip + 1..i + clear).find(|&k| d[k] <= half)?;
    let t = (d[k - 1] - half) / (d[k - 1] - d[k]).max(1e-12);
    Some((k as f64 - 1.0 + t) * STEP_MM)
}

/// Circles fitted to points together (Gauss–Newton, Huber then Tukey weights), from `c0`, all of
/// radius
/// `r0`; with `fixed`, the radius held, else one shared radius fitted. Each point belongs to
/// the circle it is nearest to (the outline of overlapping holes is made of their circles'
/// outer arcs). Returns the centres, the radius, and per circle the centre's sd, the robust
/// spread of its points and their count.
fn circles(pts: &[Pt], c0: &[Pt], r0: f64, fixed: bool) -> (Vec<Pt>, f64, Vec<(f64, f64, usize)>) {
    let n = c0.len();
    let np = 2 * n + 1;
    let mut c: Vec<Pt> = c0.to_vec();
    let mut r = r0;
    let mut w = vec![1.0; pts.len()];
    let assign = |c: &[Pt], r: f64| -> Vec<(usize, f64)> {
        pts.iter()
            .map(|p| {
                (0..n)
                    .map(|k| (k, (p.0 - c[k].0).hypot(p.1 - c[k].1) - r))
                    .min_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
                    .unwrap()
            })
            .collect()
    };
    let mad = |res: &mut Vec<f64>| {
        if res.is_empty() {
            return 0.0;
        }
        res.iter_mut().for_each(|x| *x = x.abs());
        res.sort_by(f64::total_cmp);
        1.4826 * res[res.len() / 2]
    };
    for round in 0..4 {
        for _ in 0..20 {
            let mut a = nalgebra::DMatrix::<f64>::zeros(np, np);
            let mut b = nalgebra::DVector::<f64>::zeros(np);
            for ((p, wi), (k, res)) in pts.iter().zip(&w).zip(assign(&c, r)) {
                let (dx, dy) = (p.0 - c[k].0, p.1 - c[k].1);
                let d = dx.hypot(dy).max(1e-9);
                let j = [
                    (2 * k, -dx / d),
                    (2 * k + 1, -dy / d),
                    (2 * n, if fixed { 0.0 } else { -1.0 }),
                ];
                for &(ia, ga) in &j {
                    b[ia] += wi * ga * res;
                    for &(ib, gb) in &j {
                        a[(ia, ib)] += wi * ga * gb;
                    }
                }
            }
            for d in 0..np {
                if a[(d, d)] < 1e-9 {
                    a[(d, d)] = 1.0;
                }
            }
            let Some(s) = a.lu().solve(&(-b)) else { break };
            for k in 0..n {
                c[k].0 += s[2 * k];
                c[k].1 += s[2 * k + 1];
            }
            r += s[2 * n];
            if s.norm() < 1e-7 {
                break;
            }
        }
        if round < 3 {
            // Huber first; then Tukey's biweight, which drops a run of edges pulled off the
            // circle (a tear, a printed line's edge next to the hole) instead of only
            // weakening it.
            let res: Vec<f64> = assign(&c, r).into_iter().map(|x| x.1).collect();
            let m = mad(&mut res.clone()).max(0.005);
            w = if round == 0 {
                let k = 1.5 * m;
                res.iter()
                    .map(|x| if x.abs() <= k { 1.0 } else { k / x.abs() })
                    .collect()
            } else {
                let k = TUKEY * m;
                res.iter()
                    .map(|x| {
                        if x.abs() < k {
                            (1.0 - (x / k).powi(2)).powi(2)
                        } else {
                            0.0
                        }
                    })
                    .collect()
            };
        }
    }
    let asg = assign(&c, r);
    let per = (0..n)
        .map(|k| {
            let (mut sxx, mut syy, mut sxy, mut sw, mut s2) = (0.0, 0.0, 0.0, 0.0, 0.0);
            let mut res = Vec::new();
            for ((p, wi), (kk, e)) in pts.iter().zip(&w).zip(&asg) {
                if *kk != k {
                    continue;
                }
                let (dx, dy) = (p.0 - c[k].0, p.1 - c[k].1);
                let d2 = (dx * dx + dy * dy).max(1e-18);
                sxx += wi * dx * dx / d2;
                syy += wi * dy * dy / d2;
                sxy += wi * dx * dy / d2;
                sw += wi;
                s2 += wi * e * e;
                res.push(*e);
            }
            let var = s2 / (sw - 2.0).max(1.0);
            let det = (sxx * syy - sxy * sxy).max(1e-12);
            let count = res.len();
            ((var * sxx.max(syy) / det).sqrt(), mad(&mut res), count)
        })
        .collect();
    (c, r, per)
}

/// Tukey's biweight cut, in robust sds (1.4826 · MAD) of the edge points about the circle.
const TUKEY: f64 = 4.685;
/// The least edge points a circle needs.
const MIN_POINTS: usize = 60;

/// The gauge's circles for overlapping holes near `starts` (one start: a single hole), for a
/// calibre of `diameter_mm`; None if too few rays find an edge for any of them. Rays go out
/// from every centre; the edges found are the outline of all of them together (a ray into a
/// neighbour runs on through its damage and finds no edge within reach). With `radius` (the
/// card's single holes'), the radius is held and only the centres are fitted.
pub fn gauge(
    rgb: &Rgb,
    map: &Mapping,
    exp: &Expected,
    starts: &[Pt],
    diameter_mm: f64,
    radius: Option<f64>,
) -> Option<Vec<GaugeFit>> {
    let r_cal = diameter_mm / 2.0;
    let fixed = radius.is_some();
    let clear = (CLEAR_MM / STEP_MM).ceil() as usize;
    let mut c: Vec<Pt> = starts.to_vec();
    let mut r = radius.unwrap_or(r_cal);
    let mut pts: Vec<Pt> = Vec::new();
    let mut per = Vec::new();
    for round in 0..8 {
        pts.clear();
        // How far out an edge is looked for: from a start up to `SEED_SLACK_MM` off the
        // centre, the far side of a mark larger than the calibre must be in reach; then out to
        // the circle found so far.
        let reach = if round == 0 {
            r_cal.max(r) + SEED_SLACK_MM
        } else {
            r.max(r_cal)
        } + REACH_MM;
        let limit = (reach / STEP_MM).ceil() as usize;
        let n = limit + clear + 1;
        for ck in &c {
            for k in 0..RAYS {
                let a = k as f64 / RAYS as f64 * std::f64::consts::TAU;
                let d = profile(rgb, map, exp, *ck, a, n);
                if let Some(e) = edge(&d, limit, clear) {
                    pts.push((ck.0 + e * a.cos(), ck.1 + e * a.sin()));
                }
            }
        }
        if (pts.len() as f64) < MIN_SHARE * (RAYS * c.len()) as f64 {
            return None;
        }
        let (cn, rn, pn) = circles(&pts, &c, r, fixed);
        if pn.iter().any(|p| p.2 < MIN_POINTS)
            || cn.iter().zip(&c).any(|(a, b)| (a.0 - b.0).hypot(a.1 - b.1) > 3.0)
            || !(0.3 * r_cal..2.0 * r_cal).contains(&rn)
        {
            return None;
        }
        let moved = cn
            .iter()
            .zip(&c)
            .map(|(a, b)| (a.0 - b.0).hypot(a.1 - b.1))
            .fold(0.0, f64::max);
        c = cn;
        r = rn;
        per = pn;
        if round >= 2 && moved < 0.005 {
            break;
        }
    }
    Some(
        c.iter()
            .zip(per)
            .map(|(center, (sd_mm, rough_mm, rays))| GaugeFit {
                center: *center,
                radius_mm: r,
                sd_mm,
                rays,
                rough_mm,
                damage: damage(rgb, map, exp, *center, 0.8 * r),
                together: starts.len(),
            })
            .collect(),
    )
}

/// The mean damage within `reach` of `c`.
fn damage(rgb: &Rgb, map: &Mapping, exp: &Expected, c: Pt, reach: f64) -> f64 {
    let mut sum = 0.0;
    let mut m = 0;
    let step = 0.1;
    let k = (reach / step) as i64;
    for j in -k..=k {
        for i in -k..=k {
            let (dx, dy) = (i as f64 * step, j as f64 * step);
            if dx.hypot(dy) > reach {
                continue;
            }
            let q = (c.0 + dx, c.1 + dy);
            if let Some(v) = rgb.sample_at(map, q) {
                sum += f64::from(exp.diff(v, q));
                m += 1;
            }
        }
    }
    sum / m.max(1) as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edge_passes_flaps_and_stops_before_a_neighbour() {
        let s = |mm: f64| (mm / STEP_MM).round() as usize;
        // Opening to 1.0, a flap (card) to 1.4, the rim to 2.3, card to 3.0, a neighbour after
        // (steps: the half-height edge is where the step is).
        let d: Vec<f64> = (0..s(4.0))
            .map(|i| {
                let r = i as f64 * STEP_MM;
                if r < 1.0 || (1.4..2.3).contains(&r) || r >= 3.0 {
                    0.8
                } else {
                    0.05
                }
            })
            .collect();
        let e = edge(&d, s(3.25), s(0.4)).unwrap();
        assert!((e - 2.3).abs() < 0.05, "{e}");
    }

    #[test]
    fn circle_ignores_a_tear() {
        let mut pts: Vec<Pt> = (0..360)
            .map(|k| {
                let a = f64::from(k).to_radians();
                (1.0 + 2.25 * a.cos(), -2.0 + 2.25 * a.sin())
            })
            .collect();
        for p in pts.iter_mut().take(30) {
            p.0 += 0.8; // a tear on one side
        }
        let (c, r, _) = circles(&pts, &[(0.5, -1.5)], 2.0, false);
        let c = c[0];
        assert!((c.0 - 1.0).hypot(c.1 + 2.0) < 0.03, "{c:?}");
        assert!((r - 2.25).abs() < 0.03, "{r}");
    }

    #[test]
    fn two_overlapping_circles() {
        // The outline of two circles 3 mm apart: the outer arcs only.
        let (a, b) = ((0.0, 0.0), (3.0, 0.5));
        let r = 2.3;
        let mut pts: Vec<Pt> = Vec::new();
        for c in [a, b] {
            for k in 0..360 {
                let t = f64::from(k).to_radians();
                let p = (c.0 + r * t.cos(), c.1 + r * t.sin());
                let other = if c == a { b } else { a };
                if (p.0 - other.0).hypot(p.1 - other.1) > r {
                    pts.push(p);
                }
            }
        }
        let (c, rr, per) = circles(&pts, &[(0.4, 0.3), (2.5, 0.0)], r, true);
        assert!((c[0].0 - a.0).hypot(c[0].1 - a.1) < 0.01, "{c:?}");
        assert!((c[1].0 - b.0).hypot(c[1].1 - b.1) < 0.01, "{c:?}");
        assert!((rr - r).abs() < 1e-9);
        assert!(per.iter().all(|p| p.2 > 200), "{per:?}");
    }
}
