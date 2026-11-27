//! A card photo in the app (`CARDPHOTO.md` C4): the stored photo registered from its rings,
//! its holes matched to the series' shots (`cardphoto`), and the photo resampled onto the
//! card's own grid so the app paints it under the shots. Runs on a thread of its own (a
//! phone's photo takes seconds). The results go into a JSON beside the photo.

use std::path::PathBuf;
use std::time::Instant;

use cardphoto::card::issf_air_pistol;
use cardphoto::holes::{GroundTruth, HoleFit, Placement, all_gauges, ground_truth};
use cardphoto::register::{Registration, register};
use cardphoto::view::{TILT_OK_DEG, expected_error_mm};

/// The rings' registration over this rms (mm) is called poor: well above what real photos give.
const REG_RMS_WARN_MM: f64 = 0.12;
use cardphoto::{Gray, Rgb};
use eframe::egui::{Color32, ColorImage};

use crate::card::Card;
use crate::upload::{Photo, now_unix};

/// The photo on the card's grid: at most this many pixels per mm, and per side.
const GRID_PX_PER_MM: f32 = 10.0;
const GRID_MAX_PX: f32 = 2048.0;

/// The printed numbers decide the card's quarter turn when the best turn's match beats the
/// next one's by this much (normalised cross-correlation).
const MIN_PRINT_MARGIN: f64 = 0.1;

/// Where the printed card was in the sensor's frame: a printed point p (mm from the card's
/// centre, y up, the numbers upright) was at `centre + scale R(angle) p`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CardPose {
    pub angle: f64,
    pub centre: (f64, f64),
    pub scale: f64,
}

impl CardPose {
    pub const IDENTITY: CardPose = CardPose {
        angle: 0.0,
        centre: (0.0, 0.0),
        scale: 1.0,
    };

    /// From the view (frame → the registration's mm) and the print's quarter turn k (the
    /// registration's mm = R(k·90°) print).
    fn new(view: &Placement, k: u8) -> CardPose {
        CardPose {
            angle: f64::from(k) * std::f64::consts::FRAC_PI_2 - view.angle,
            centre: view.to_frame((0.0, 0.0)),
            scale: 1.0 / view.scale,
        }
    }

    pub fn apply(&self, p: (f64, f64)) -> (f64, f64) {
        let (sn, cs) = self.angle.sin_cos();
        (
            self.centre.0 + self.scale * (cs * p.0 - sn * p.1),
            self.centre.1 + self.scale * (sn * p.0 + cs * p.1),
        )
    }

    pub fn invert(&self, f: (f64, f64)) -> (f64, f64) {
        let (sn, cs) = self.angle.sin_cos();
        let (x, y) = (
            (f.0 - self.centre.0) / self.scale,
            (f.1 - self.centre.1) / self.scale,
        );
        (cs * x + sn * y, -sn * x + cs * y)
    }

    /// The turn, degrees, -180..180.
    pub fn angle_deg(&self) -> f64 {
        (self.angle.to_degrees() + 180.0).rem_euclid(360.0) - 180.0
    }
}

/// One shot against its hole, in the sensor's frame (mm, x right, y up).
#[derive(Clone, Debug)]
pub struct ShotCheck {
    pub shot: (f64, f64),
    /// The hole's centre, if one was found where the shot says.
    pub hole: Option<(f64, f64)>,
    /// The hole minus the shot: the sensor's error.
    pub diff: Option<(f64, f64)>,
    /// The hole centre's uncertainty from the photo (1σ, a lower bound), and its diameter.
    pub sd_mm: f64,
    pub hole_d_mm: f64,
    /// Whether its hole overlaps another (fitted together: less certain).
    pub overlap: bool,
    /// The hole on the paper: mm from the printed card's centre (y up, the print's way up
    /// when `print_quarter` is known), what a scorer would gauge. Scoring needs only its
    /// distance from the centre, so an unknown quarter turn doesn't change the score.
    pub paper: Option<(f64, f64)>,
}

/// What came out of one photo.
pub struct Processed {
    pub photo: Photo,
    pub result_file: PathBuf,
    pub seconds: f64,
    /// The registration: rms of the ring edge points (mm), points kept of found, the lens's
    /// k1, pixels per mm at the card, and whether the outline fixed the rotation.
    pub reg_rms_mm: f64,
    pub reg_points: (usize, usize),
    pub k1: f64,
    pub px_per_mm: f64,
    /// The print's quarter turn in the registration's axes (from the printed numbers; None
    /// when they didn't decide), and by how much the best turn won.
    pub print_quarter: Option<u8>,
    pub print_margin: f64,
    /// The camera's angle from straight above the card (at its centre), degrees, and the
    /// holes' expected error from this photo (rms, mm; `cardphoto::view`).
    pub camera_deg: f64,
    pub expected_mm: f64,
    /// Where the printed card was in the frame (with no shots: the frame is the print's).
    pub pose: CardPose,
    /// With shots: the card's placement in the frame, each shot against its hole, the rms,
    /// the holes no shot explains (frame mm). Without: `placement` None, `holes` every hole
    /// found (frame mm, here the print's).
    pub placement: Option<Placement>,
    pub scaled: bool,
    pub shots: Vec<ShotCheck>,
    pub rms_mm: Option<f64>,
    pub holes: Vec<(f64, f64)>,
    /// Without shots: whether each of `holes` overlaps another (fitted together).
    pub hole_overlap: Vec<bool>,
    /// The photo on the card's grid (as `Card::render`: the card's size, top left first).
    pub image: Option<ColorImage>,
}

impl Processed {
    /// Warnings for the user: what makes this photo's truth weaker.
    pub fn warnings(&self) -> Vec<String> {
        self.warnings_with(true)
    }

    /// The warnings; `turn`: also whether the printed numbers showed which way up the card
    /// is (counting cards leaves it out: a score doesn't depend on it).
    pub fn warnings_with(&self, turn: bool) -> Vec<String> {
        let mut w = Vec::new();
        if self.px_per_mm < 8.0 {
            w.push(format!(
                "The card is small in the photo ({:.1} px per mm): hold the phone closer.",
                self.px_per_mm
            ));
        }
        // Real photos fit to 0.02 mm rms from straight above and 0.05–0.08 mm at 20–33° of
        // tilt (CARDPHOTO.md C5), so only a gross misfit is worth a word; the tilt has its own.
        if self.reg_rms_mm > REG_RMS_WARN_MM {
            w.push(format!(
                "The printed rings fit the photo poorly ({:.2} mm rms; real photos give 0.02–0.08): the card may \
                 be bent or partly hidden, or the photo blurred. The holes may be less exact than usual.",
                self.reg_rms_mm
            ));
        }
        // Tilted, when it matters for these holes (not for a clean card: no holes away from
        // the centre).
        if self.camera_deg > TILT_OK_DEG && self.expected_mm > 1.2 * expected_error_mm(0.0, 0.0) {
            w.push(format!(
                "Taken {:.0}° from straight above: the holes may be off by about {:.2} mm (rms), against about \
                 {:.2} mm from straight above (a card that doesn't lie quite flat shows when seen at an angle). \
                 This photo is used; take another from straight above for a closer check.",
                self.camera_deg,
                self.expected_mm,
                expected_error_mm(0.0, 0.0)
            ));
        }
        let overlapping = self
            .shots
            .iter()
            .filter(|s| s.hole.is_some() && s.overlap)
            .count()
            + self.hole_overlap.iter().filter(|&&o| o).count();
        if overlapping > 0 {
            w.push(format!(
                "{overlapping} hole{} overlap{} another: less certain than the single holes.",
                if overlapping == 1 { "" } else { "s" },
                if overlapping == 1 { "s" } else { "" }
            ));
        }
        if turn && self.print_quarter.is_none() {
            w.push("The printed numbers didn't show which way up the card is: the app's card may be drawn turned.".into());
        }
        let missing = self.shots.iter().filter(|s| s.hole.is_none()).count();
        if missing > 0 {
            w.push(format!(
                "{missing} shot{} with no hole where the sensor put {}.",
                if missing == 1 { "" } else { "s" },
                if missing == 1 { "it" } else { "them" }
            ));
        }
        if self.placement.is_some() && !self.holes.is_empty() {
            w.push(format!(
                "{} hole{} no shot explains.",
                self.holes.len(),
                if self.holes.len() == 1 { "" } else { "s" }
            ));
        }
        w
    }

    /// The worst shot: its index and its distance from its hole.
    pub fn worst(&self) -> Option<(usize, f64)> {
        self.shots
            .iter()
            .enumerate()
            .filter_map(|(i, s)| s.diff.map(|d| (i, d.0.hypot(d.1))))
            .max_by(|a, b| a.1.total_cmp(&b.1))
    }
}

/// Registers the stored photo and matches its holes to `shots` (frame mm), then puts it on
/// `card`'s grid. Writes `<photo>.result.json`.
pub fn process(photo: Photo, card: &Card, shots: &[(f64, f64)]) -> Result<Processed, String> {
    let t0 = Instant::now();
    let bytes = std::fs::read(&photo.image).map_err(|e| format!("could not read the photo: {e}"))?;
    let rgb = Rgb::decode(&bytes)?;
    let reg = register(&rgb.to_gray(), &issf_air_pistol())?;
    let (k, margin) = print_quarter(&rgb, &reg, card);
    let print_quarter = (margin >= MIN_PRINT_MARGIN).then_some(k);
    let k = print_quarter.unwrap_or(0);
    // The holes are gauged for the card's pellet (C5: a torn hole's mark is its circle).
    let diameter = f64::from(card.pellet_diameter_mm);
    let mut hole_overlap = Vec::new();
    let (placement, scaled, checks, rms, holes, view) = if shots.is_empty() {
        // No shots: the frame is the print's (the numbers upright).
        let view = Placement {
            angle: f64::from(k) * std::f64::consts::FRAC_PI_2,
            quarter: k,
            shift: (0.0, 0.0),
            scale: 1.0,
        };
        let found = all_gauges(&rgb, &reg, diameter);
        hole_overlap = found.iter().map(|(_, g)| g.together > 1).collect();
        let holes: Vec<(f64, f64)> = found.iter().map(|(_, g)| view.to_frame(g.center)).collect();
        (None, false, Vec::new(), None, holes, view)
    } else {
        let gt = ground_truth(&rgb, &reg, shots, diameter)?;
        let checks = shot_checks(&gt, shots);
        let extra = gt.extra.iter().map(|h| gt.placement.to_frame(h.center)).collect();
        let rms = rms_of(&checks);
        (
            Some(gt.placement),
            gt.scaled,
            checks,
            Some(rms),
            extra,
            gt.placement,
        )
    };
    // How far the holes are from the card's centre (rms), for the expected error.
    let rho = {
        let pts: Vec<(f64, f64)> = if shots.is_empty() {
            holes.clone()
        } else {
            shots.to_vec()
        };
        (pts.iter().map(|p| p.0 * p.0 + p.1 * p.1).sum::<f64>() / pts.len().max(1) as f64).sqrt()
    };
    let camera_deg = cardphoto::view::view(&reg.map).tilt_deg;
    let image = on_grid(&rgb, &reg, &view, card);
    let pose = CardPose::new(&view, k);
    let checks = checks
        .into_iter()
        .map(|c| ShotCheck {
            paper: c.hole.map(|h| pose.invert(h)),
            ..c
        })
        .collect();
    let result_file = photo.image.with_extension("result.json");
    let mut p = Processed {
        photo,
        result_file,
        seconds: 0.0,
        reg_rms_mm: reg.rms_mm,
        reg_points: (reg.kept, reg.points),
        k1: reg.map.lens.k1,
        px_per_mm: px_per_mm(&reg),
        print_quarter,
        print_margin: margin,
        camera_deg,
        expected_mm: expected_error_mm(camera_deg, rho),
        pose,
        placement,
        scaled,
        shots: checks,
        rms_mm: rms,
        holes,
        hole_overlap,
        image: Some(image),
    };
    p.seconds = t0.elapsed().as_secs_f64();
    std::fs::write(&p.result_file, result_json(&p, card))
        .map_err(|e| format!("could not save the results: {e}"))?;
    Ok(p)
}

/// The shots against their holes, in the frame. The difference in the frame's axes (the
/// sensor's), not the card's.
fn shot_checks(gt: &GroundTruth, shots: &[(f64, f64)]) -> Vec<ShotCheck> {
    shots
        .iter()
        .zip(&gt.shots)
        .map(|(s, r)| {
            let hole = r.hole.as_ref().map(|h: &HoleFit| gt.placement.to_frame(h.center));
            ShotCheck {
                shot: *s,
                hole,
                diff: hole.map(|h| (h.0 - s.0, h.1 - s.1)),
                sd_mm: r.hole.as_ref().map_or(0.0, |h| h.sd_mm / gt.placement.scale),
                hole_d_mm: r.hole.as_ref().map_or(0.0, |h| 2.0 * h.radius_mm),
                overlap: r.overlap,
                paper: None, // set once the card's pose is known
            }
        })
        .collect()
}

fn rms_of(checks: &[ShotCheck]) -> f64 {
    let d: Vec<f64> = checks
        .iter()
        .filter_map(|c| c.diff.map(|d| d.0 * d.0 + d.1 * d.1))
        .collect();
    (d.iter().sum::<f64>() / d.len().max(1) as f64).sqrt()
}

fn px_per_mm(reg: &Registration) -> f64 {
    let a = reg.map.to_image((0.0, 0.0));
    let b = reg.map.to_image((1.0, 0.0));
    (b.0 - a.0).hypot(b.1 - a.1)
}

/// The photo resampled onto the card's grid: each pixel is a card point (mm from the top
/// left, y down), a frame point, the card point the placement makes of it, a photo pixel.
/// Outside the photo: transparent.
fn on_grid(rgb: &Rgb, reg: &Registration, view: &Placement, card: &Card) -> ColorImage {
    let size = card.size_mm();
    let ppm = GRID_PX_PER_MM.min(GRID_MAX_PX / size.x.max(size.y));
    let (w, h) = ((size.x * ppm).round() as usize, (size.y * ppm).round() as usize);
    let centre = card.to_card(0.0, 0.0);
    let mut px = vec![Color32::TRANSPARENT; w * h];
    for j in 0..h {
        let y = f64::from(centre.y) - (j as f64 + 0.5) / f64::from(ppm);
        for i in 0..w {
            let x = (i as f64 + 0.5) / f64::from(ppm) - f64::from(centre.x);
            let q = view.apply((x, y));
            if let Some(c) = rgb.sample_at(&reg.map, q) {
                let u = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
                px[j * w + i] = Color32::from_rgb(u(c[0]), u(c[1]), u(c[2]));
            }
        }
    }
    ColorImage::new([w, h], px)
}

/// The printed numbers' digits, each cropped to its ink and scaled, the same in x and y, so
/// its longer side spans this grid (`DIGIT_N` × `DIGIT_N`, the ink centred): a card printed in
/// another font or size still compares, and a digit read sideways stays wide.
const DIGIT_N: usize = 20;
/// Around each number: a square of ±2.4 mm (the app's digits are 3.2 mm tall, real cards'
/// about 2 mm), clear of the ring lines 4 mm either side. The axes are the print's (the
/// registration takes them from the numbers), so the digits sit in the middle.
const DIGIT_HALF_MM: f64 = 2.4;
const DIGIT_STEP_MM: f64 = 0.1;
/// Ink: this far from the window's background towards the ink, of the paper–ink contrast.
const DIGIT_INK: f64 = 0.35;
/// Inside this radius the numbers are light on the black.
const BLACK_MM: f64 = 29.75;

/// A digit's shape: `lum(u, v)` the luminance at (u, v) mm from the number's place (print
/// axes, y up), `range` the paper–ink contrast in those units; 1 = ink. The background is the
/// window's median (the digit is a small part of it). None when there is no clear ink.
fn digit_shape(lum: impl Fn(f64, f64) -> Option<f64>, light_ink: bool, range: f64) -> Option<Vec<f64>> {
    let n = (2.0 * DIGIT_HALF_MM / DIGIT_STEP_MM).round() as usize + 1;
    let at = |i: usize| -DIGIT_HALF_MM + i as f64 * DIGIT_STEP_MM;
    let mut v = Vec::with_capacity(n * n);
    for j in 0..n {
        for i in 0..n {
            v.push(lum(at(i), at(n - 1 - j))?); // row 0 at the top
        }
    }
    let mut sorted = v.clone();
    sorted.sort_by(f64::total_cmp);
    let bg = sorted[n * n / 2];
    let ink: Vec<f64> = v
        .iter()
        .map(|&x| {
            let d = if light_ink { x - bg } else { bg - x };
            (d / range.max(1e-6) / DIGIT_INK).clamp(0.0, 1.0)
        })
        .collect();
    // The ink's box: the pixels clearly ink, and connected to the ink nearest the middle (a
    // hole's edge or a pen mark further out stays out).
    let solid = |k: usize| ink[k] >= 1.0;
    let mid = (n / 2) as f64;
    let seed = (0..n * n)
        .filter(|&k| solid(k))
        .min_by(|&a, &b| {
            let d = |k: usize| ((k % n) as f64 - mid).hypot((k / n) as f64 - mid);
            d(a).total_cmp(&d(b))
        })?;
    if ((seed % n) as f64 - mid).hypot((seed / n) as f64 - mid) > 1.2 / DIGIT_STEP_MM {
        return None;
    }
    let mut seen = vec![false; n * n];
    let mut stack = vec![seed];
    seen[seed] = true;
    let (mut x0, mut x1, mut y0, mut y1, mut count) = (n, 0, n, 0, 0);
    while let Some(k) = stack.pop() {
        let (i, j) = (k % n, k / n);
        (x0, x1, y0, y1) = (x0.min(i), x1.max(i), y0.min(j), y1.max(j));
        count += 1;
        // 8-connected, and one pixel's gap bridged (thin strokes, a little blur).
        for dj in -2i64..=2 {
            for di in -2i64..=2 {
                let (a, b) = (i as i64 + di, j as i64 + dj);
                if a < 0 || b < 0 || a >= n as i64 || b >= n as i64 {
                    continue;
                }
                let q = b as usize * n + a as usize;
                if !seen[q] && solid(q) {
                    seen[q] = true;
                    stack.push(q);
                }
            }
        }
    }
    if count < 10 {
        return None;
    }
    // The box, its longer side onto the grid, sampled bilinearly (outside: no ink).
    let (cx, cy) = ((x0 + x1) as f64 / 2.0, (y0 + y1) as f64 / 2.0);
    let side = ((x1 - x0).max(y1 - y0) + 1) as f64;
    let get = |x: f64, y: f64| {
        if x < 0.0 || y < 0.0 || x > (n - 1) as f64 || y > (n - 1) as f64 {
            return 0.0;
        }
        let (xi, yi) = (x.floor().min((n - 2) as f64), y.floor().min((n - 2) as f64));
        let (fx, fy) = (x - xi, y - yi);
        let (xi, yi) = (xi as usize, yi as usize);
        let p = |a: usize, b: usize| ink[b * n + a];
        (p(xi, yi) * (1.0 - fx) + p(xi + 1, yi) * fx) * (1.0 - fy)
            + (p(xi, yi + 1) * (1.0 - fx) + p(xi + 1, yi + 1) * fx) * fy
    };
    let mut out = Vec::with_capacity(DIGIT_N * DIGIT_N);
    for j in 0..DIGIT_N {
        for i in 0..DIGIT_N {
            let x = cx + ((i as f64 + 0.5) / DIGIT_N as f64 - 0.5) * side;
            let y = cy + ((j as f64 + 0.5) / DIGIT_N as f64 - 0.5) * side;
            out.push(get(x, y));
        }
    }
    Some(out)
}

fn ncc(a: &[f64], b: &[f64]) -> f64 {
    let n = a.len() as f64;
    let (ma, mb) = (a.iter().sum::<f64>() / n, b.iter().sum::<f64>() / n);
    let (mut sab, mut saa, mut sbb) = (0.0, 0.0, 0.0);
    for (x, y) in a.iter().zip(b) {
        sab += (x - ma) * (y - mb);
        saa += (x - ma) * (x - ma);
        sbb += (y - mb) * (y - mb);
    }
    sab / (saa * sbb).sqrt().max(1e-12)
}

/// The print's quarter turn k in the registration's axes (its mm = R(k·90°) print), and the
/// margin it won by. The numbers are printed upright on all four axes: each one in the photo,
/// read as if the card were turned k, against the app's card's digit (both cropped to their
/// ink and scaled alike, `digit_shape`), by normalised cross-correlation, averaged over every
/// number found; the best turn against the next. A wrong turn shows the digits sideways or
/// upside down. Needs the print's axes (`Registration::axes`); without them, (0, 0).
fn print_quarter(rgb: &Rgb, reg: &Registration, card: &Card) -> (u8, f64) {
    if !reg.axes_known() {
        return (0, 0.0);
    }
    let ppm = 12.0f32;
    let Some(printed) = card.render((card.size_mm().x * ppm).round() as u32) else {
        return (0, 0.0);
    };
    let lum8 = |c: Color32| (0.299 * f64::from(c.r()) + 0.587 * f64::from(c.g()) + 0.114 * f64::from(c.b())) / 255.0;
    let radii = cardphoto::card::issf_air_pistol().number_radii_mm;
    let range = f64::from(reg.contrast.range());
    // The app's digits, from the number on the right of the centre at each radius.
    let templates: Vec<Option<Vec<f64>>> = radii
        .iter()
        .map(|&r| {
            digit_shape(
                |u, v| {
                    let at = card.to_card((r + u) as f32, v as f32) * ppm;
                    let (x, y) = (at.x as usize, at.y as usize);
                    (x < printed.size[0] && y < printed.size[1]).then(|| lum8(printed.pixels[y * printed.size[0] + x]))
                },
                r < BLACK_MM,
                0.8,
            )
        })
        .collect();
    let mut scores: Vec<(f64, u8)> = (0..4u8)
        .map(|k| {
            let (sn, cs) = (f64::from(k) * std::f64::consts::FRAC_PI_2).sin_cos();
            let (mut sum, mut count) = (0.0, 0);
            for (r, t) in radii.iter().zip(&templates) {
                let Some(t) = t else { continue };
                for (ax, ay) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
                    let shape = digit_shape(
                        |u, v| {
                            let p = (ax * r + u, ay * r + v);
                            let q = (cs * p.0 - sn * p.1, sn * p.0 + cs * p.1);
                            rgb.sample_at(&reg.map, q)
                                .map(|c| 0.299 * f64::from(c[0]) + 0.587 * f64::from(c[1]) + 0.114 * f64::from(c[2]))
                        },
                        *r < BLACK_MM,
                        range,
                    );
                    if let Some(sh) = shape {
                        sum += ncc(&sh, t);
                        count += 1;
                    }
                }
            }
            (if count > 0 { sum / f64::from(count) } else { 0.0 }, k)
        })
        .collect();
    scores.sort_by(|x, y| y.0.total_cmp(&x.0));
    if std::env::var_os("COUNT_DEBUG").is_some() {
        eprintln!("  print quarter scores {scores:.3?}");
    }
    (scores[0].1, scores[0].0 - scores[1].0)
}

fn result_json(p: &Processed, card: &Card) -> String {
    let pair = |v: (f64, f64)| serde_json::json!([round(v.0), round(v.1)]);
    // Scored by the card's rule (the pellet's diameter, decimal in tenths).
    let score = |v: (f64, f64)| {
        let s = card.score(v.0 as f32, v.1 as f32);
        serde_json::json!({ "ring": s.ring, "tenths": s.tenths, "inner_ten": s.inner_ten })
    };
    let shots: Vec<_> = p
        .shots
        .iter()
        .enumerate()
        .map(|(i, s)| {
            serde_json::json!({
                "n": i + 1,
                "shot_mm": pair(s.shot),
                "hole_mm": s.hole.map(pair),
                "diff_mm": s.diff.map(pair),
                "hole_sd_mm": s.hole.map(|_| round(s.sd_mm)),
                "hole_d_mm": s.hole.map(|_| round(s.hole_d_mm)),
                "overlap": s.hole.map(|_| s.overlap),
                "paper_mm": s.paper.map(pair),
                "sensor_score": score(s.shot),
                "paper_score": s.paper.map(score),
            })
        })
        .collect();
    let v = serde_json::json!({
        "photo": p.photo.meta.image,
        "series": p.photo.meta.series,
        "processed_unix": now_unix(),
        "seconds": (p.seconds * 10.0).round() / 10.0,
        "registration": {
            "rms_mm": round(p.reg_rms_mm),
            "points_kept": p.reg_points.0,
            "points": p.reg_points.1,
            "k1": round(p.k1),
            "px_per_mm": (p.px_per_mm * 100.0).round() / 100.0,
            "camera_deg": (p.camera_deg * 10.0).round() / 10.0,
            "expected_mm": round(p.expected_mm),
        },
        "placement": p.placement.as_ref().map(|pl| serde_json::json!({
            "quarter_turns": pl.quarter,
            "tilt_deg": round(pl.tilt_deg()),
            "shift_mm": pair(pl.shift),
            "scale": (pl.scale * 1e6).round() / 1e6,
            "scale_fitted": p.scaled,
        })),
        "print_quarter": p.print_quarter,
        "print_margin": round(p.print_margin),
        "card_pose": {
            "angle_deg": round(p.pose.angle_deg()),
            "centre_mm": pair(p.pose.centre),
            "scale": (p.pose.scale * 1e6).round() / 1e6,
        },
        "rms_mm": p.rms_mm.map(round),
        "shots": shots,
        "holes_mm": p.holes.iter().map(|h| pair(*h)).collect::<Vec<_>>(),
        "holes_overlap": (!p.hole_overlap.is_empty()).then_some(&p.hole_overlap),
        "holes_are": if p.placement.is_some() { "unexplained, frame mm" } else { "all found, card mm (quarter turn unknown)" },
    });
    serde_json::to_string_pretty(&v).unwrap_or_default()
}

/// The photo on the card's grid, beside the photo (`<photo>.grid.png`), so a window holding
/// many cards keeps only the one it shows in memory.
pub fn save_grid(img: &ColorImage, photo: &std::path::Path) -> Result<PathBuf, String> {
    let path = photo.with_extension("grid.png");
    let raw: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_srgba_unmultiplied()).collect();
    image::RgbaImage::from_raw(img.size[0] as u32, img.size[1] as u32, raw)
        .ok_or("bad image size")?
        .save(&path)
        .map_err(|e| format!("could not save the card's image: {e}"))?;
    Ok(path)
}

pub fn load_grid(path: &std::path::Path) -> Result<ColorImage, String> {
    let img = image::open(path).map_err(|e| e.to_string())?.to_rgba8();
    Ok(ColorImage::from_rgba_unmultiplied(
        [img.width() as usize, img.height() as usize],
        img.as_raw(),
    ))
}

fn round(v: f64) -> f64 {
    (v * 1e4).round() / 1e4
}

/// The card as a texture for synthetic photos (`cardphoto::synth`), `px_per_mm` sharp.
pub fn synth_texture(card: &Card, px_per_mm: f32) -> Option<cardphoto::synth::CardTexture> {
    let size = card.size_mm();
    let img = card.render((size.x * px_per_mm).round() as u32)?;
    let lum: Vec<u8> = img
        .pixels
        .iter()
        .map(|c| (0.299 * f32::from(c.r()) + 0.587 * f32::from(c.g()) + 0.114 * f32::from(c.b())) as u8)
        .collect();
    Some(cardphoto::synth::CardTexture {
        img: Gray::from_luma8(img.size[0], img.size[1], &lum),
        px_per_mm: f64::from(px_per_mm),
        size_mm: f64::from(size.x),
    })
}

/// A synthetic phone photo (JPEG) of the card with holes where `shots` (frame mm) went,
/// placed as a card in a holder would be (turned a quarter and a little, shifted), each hole
/// `error_mm` off at random (the sensor's error): for trying the app without real photos.
pub fn demo_photo(card: &Card, shots: &[(f64, f64)], error_mm: f64, seed: u64) -> Result<Vec<u8>, String> {
    use cardphoto::synth::{Params, Rng, photo_rgb};
    let tex = synth_texture(card, 12.0).ok_or("could not render the card")?;
    let placement = Placement {
        angle: (90.0f64 + 0.5).to_radians(),
        quarter: 1,
        shift: (0.7, -0.4),
        scale: 1.0,
    };
    let mut rng = Rng::new(seed);
    let holes = shots
        .iter()
        .map(|s| {
            let c = placement.apply(*s);
            (c.0 + error_mm * rng.gauss(), c.1 + error_mm * rng.gauss())
        })
        .collect();
    let p = Params {
        w: 3000,
        h: 2250,
        px_per_mm: 11.0,
        rotation_deg: 8.0 + 40.0 * rng.uniform(),
        tilt_x_deg: 4.0,
        tilt_y_deg: -3.0,
        k1: 0.03,
        holes,
        table: 0.3,
        table_rgb: Some([0.15, 0.3, 0.62]),
        light: 0.2,
        blur_px: 1.0,
        noise: 0.015,
        seed,
        ..Params::default()
    };
    let (rgb, _) = photo_rgb(&tex, &p);
    let raw: Vec<u8> = rgb
        .px
        .iter()
        .flat_map(|c| c.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8))
        .collect();
    let img = image::RgbImage::from_raw(rgb.w as u32, rgb.h as u32, raw).ok_or("bad image size")?;
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 92)
        .encode_image(&img)
        .map_err(|e| e.to_string())?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::upload::store;

    fn shots() -> Vec<(f64, f64)> {
        vec![
            (1.0, -2.0),
            (-11.0, 7.0),
            (8.0, 12.0),
            (24.0, -18.0),
            (-35.0, -25.0),
            (40.0, 30.0),
            (-3.0, 33.0),
            (15.0, -40.0),
        ]
    }

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("opensights-photo-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Through the app's path: a synthetic JPEG with holes 0.1 mm off the shots, stored as an
    /// upload, processed with the shots; the quarter turn, the differences, the photo on the
    /// card's grid (a hole shows the table under the shot), the results file.
    #[test]
    fn synthetic_photo_with_shots() {
        let card = crate::card::air_pistol();
        let shots = shots();
        let jpeg = demo_photo(&card, &shots, 0.1, 5).unwrap();
        let d = dir("shots");
        let photo = store(&d, "demo", &jpeg, now_unix()).unwrap();
        let p = process(photo, &card, &shots).unwrap();
        let pl = p.placement.as_ref().unwrap();
        println!(
            "{:.1} s, reg rms {:.4} mm, {} of {} points, {:.1} px/mm, quarter {} tilt {:+.3}° shift ({:+.3}, {:+.3}) scale {:.5}, rms {:.4} mm, worst {:?}, warnings {:?}",
            p.seconds,
            p.reg_rms_mm,
            p.reg_points.0,
            p.reg_points.1,
            p.px_per_mm,
            pl.quarter,
            pl.tilt_deg(),
            pl.shift.0,
            pl.shift.1,
            pl.scale,
            p.rms_mm.unwrap(),
            p.worst(),
            p.warnings()
        );
        // The quarter turn is relative to the registration's axes (the outline fixes them only
        // modulo 90°), so only the shift's length and the tilt compare with the truth.
        assert!(
            (pl.shift.0.hypot(pl.shift.1) - 0.7f64.hypot(0.4)).abs() < 0.15,
            "{:?}",
            pl.shift
        );
        assert!((pl.tilt_deg() - 0.5).abs() < 0.2, "{}", pl.tilt_deg());
        assert!(p.shots.iter().all(|s| s.hole.is_some()));
        // The holes are 0.1 mm (per axis) off the shots; the placement takes up some of it.
        let rms = p.rms_mm.unwrap();
        assert!((0.04..0.2).contains(&rms), "{rms}");
        assert!(p.holes.is_empty());
        assert!(p.warnings().is_empty(), "{:?}", p.warnings());
        // On the grid, at each shot: the table (blue) shows through the hole.
        let img = p.image.as_ref().unwrap();
        let ppm = img.size[0] as f32 / card.size_mm().x;
        for s in &shots {
            let at = card.to_card(s.0 as f32, s.1 as f32) * ppm;
            let c = img.pixels[at.y as usize * img.size[0] + at.x as usize];
            assert!(i32::from(c.b()) > i32::from(c.r()) + 40, "shot {s:?}: {c:?}");
        }
        // Away from the holes: the card (paper is not blue).
        let at = card.to_card(-60.0, 60.0) * ppm;
        let c = img.pixels[at.y as usize * img.size[0] + at.x as usize];
        assert!(c.r() > 150 && i32::from(c.b()) < i32::from(c.r()) + 20, "{c:?}");
        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&p.result_file).unwrap()).unwrap();
        assert_eq!(json["shots"].as_array().unwrap().len(), shots.len());
        assert_eq!(json["placement"]["quarter_turns"], pl.quarter);
        // The clean-up leaves the results alone.
        assert!(crate::upload::clean_up(&d, now_unix()).unwrap().is_empty());
        std::fs::remove_dir_all(&d).unwrap();
    }

    /// The black disc's centre on the card's grid (frame mm): its edge along 360 rays from
    /// `near`, a least-squares circle through the edge points (Kåsa), the points more than
    /// 0.3 mm off it dropped (holes, the numbers next to the disc), fitted again.
    fn disc_centre(img: &ColorImage, card: &Card, near: (f64, f64)) -> (f64, f64) {
        let ppm = f64::from(img.size[0] as f32 / card.size_mm().x);
        let dark = |x: f64, y: f64| {
            let at = card.to_card(x as f32, y as f32) * ppm as f32;
            let c = img.pixels[at.y as usize * img.size[0] + at.x as usize];
            c.r() < 90 && c.g() < 90 && c.b() < 90
        };
        let mut pts: Vec<(f64, f64)> = (0..360)
            .filter_map(|k| {
                let t = f64::from(k).to_radians();
                (0..400).map(|i| 25.0 + i as f64 * 0.025).find_map(|rr| {
                    let p = (near.0 + rr * t.cos(), near.1 + rr * t.sin());
                    (!dark(p.0, p.1)).then_some(p)
                })
            })
            .collect();
        let mut c = near;
        for _ in 0..3 {
            // x² + y² = 2 a x + 2 b y + k, least squares.
            let mut m = nalgebra::Matrix3::<f64>::zeros();
            let mut v = nalgebra::Vector3::<f64>::zeros();
            for p in &pts {
                let row = nalgebra::Vector3::new(2.0 * p.0, 2.0 * p.1, 1.0);
                m += row * row.transpose();
                v += row * (p.0 * p.0 + p.1 * p.1);
            }
            let s = m.lu().solve(&v).unwrap();
            c = (s[0], s[1]);
            let r = (s[2] + c.0 * c.0 + c.1 * c.1).sqrt();
            pts.retain(|p| ((p.0 - c.0).hypot(p.1 - c.1) - r).abs() < 0.3);
        }
        c
    }

    /// The photo lies on the grid where the placement says the card is: the photo's black
    /// disc centred on the card's centre in the frame (the demo's card sits about 0.8 mm off
    /// the sensor's centre in the holder, turned 90.5°).
    #[test]
    fn photo_on_the_grid_where_the_card_was() {
        let card = crate::card::air_pistol();
        let shots = shots();
        let jpeg = demo_photo(&card, &shots, 0.0, 5).unwrap();
        let d = dir("grid");
        let photo = store(&d, "demo", &jpeg, now_unix()).unwrap();
        let p = process(photo, &card, &shots).unwrap();
        let pl = p.placement.as_ref().unwrap();
        let want = pl.to_frame((0.0, 0.0));
        // The truth: the demo's card centre in the frame (card = R(90.5°) frame + (0.7, -0.4)).
        let truth = {
            let a = (90.0f64 + 0.5).to_radians();
            let (x, y) = (-0.7, 0.4);
            (a.cos() * x + a.sin() * y, -a.sin() * x + a.cos() * y)
        };
        // The measurement itself, on the printed card: its centre.
        let printed = disc_centre(&card.render(1700).unwrap(), &card, (0.3, -0.2));
        // The photo on the grid without the placement: on the printed card's rings.
        let rgb = Rgb::decode(&std::fs::read(&p.photo.image).unwrap()).unwrap();
        let reg = register(&rgb.to_gray(), &issf_air_pistol()).unwrap();
        let none = Placement {
            angle: 0.0,
            quarter: 0,
            shift: (0.0, 0.0),
            scale: 1.0,
        };
        let plain = disc_centre(&on_grid(&rgb, &reg, &none, &card), &card, (0.3, -0.2));
        let got = disc_centre(p.image.as_ref().unwrap(), &card, want);
        println!("the disc's centre: printed {printed:?}, the photo without the placement {plain:?}");
        println!(
            "card centre in the frame: true ({:+.3}, {:+.3}), placement ({:+.3}, {:+.3}), the photo's disc on the grid ({:+.3}, {:+.3}); rms {:.4}",
            truth.0,
            truth.1,
            want.0,
            want.1,
            got.0,
            got.1,
            p.rms_mm.unwrap()
        );
        assert!(printed.0.hypot(printed.1) < 0.03, "{printed:?}");
        assert!(plain.0.hypot(plain.1) < 0.05, "{plain:?}");
        assert!((got.0 - want.0).hypot(got.1 - want.1) < 0.05);
        assert!((want.0 - truth.0).hypot(want.1 - truth.1) < 0.05);
        // The printed card's pose: turned -90.5°, centred where the card was.
        println!(
            "print quarter {:?} (margin {:.3}), pose {:+.3}° at ({:+.3}, {:+.3}) scale {:.5}",
            p.print_quarter,
            p.print_margin,
            p.pose.angle_deg(),
            p.pose.centre.0,
            p.pose.centre.1,
            p.pose.scale
        );
        assert!(p.print_quarter.is_some());
        assert!((p.pose.angle_deg() + 90.5).abs() < 0.2, "{}", p.pose.angle_deg());
        assert!((p.pose.centre.0 - truth.0).hypot(p.pose.centre.1 - truth.1) < 0.05);
        // A printed point through the pose lands where the photo shows it: each hole of the
        // demo (print mm) at its shot (frame mm).
        let print_holes: Vec<(f64, f64)> = shots
            .iter()
            .map(|s| {
                let a = (90.0f64 + 0.5).to_radians();
                (
                    a.cos() * s.0 - a.sin() * s.1 + 0.7,
                    a.sin() * s.0 + a.cos() * s.1 - 0.4,
                )
            })
            .collect();
        for (h, s) in print_holes.iter().zip(&shots) {
            let f = p.pose.apply(*h);
            assert!((f.0 - s.0).hypot(f.1 - s.1) < 0.1, "{h:?} -> {f:?}, shot {s:?}");
        }
        // The paper: each hole where the demo punched it on the print, scored as the print
        // scores it; the card 0.8 mm off in the holder, some differ from the sensor's.
        let mut differ = 0;
        for ((c, h), s) in p.shots.iter().zip(&print_holes).zip(&shots) {
            let q = c.paper.unwrap();
            assert!((q.0 - h.0).hypot(q.1 - h.1) < 0.1, "paper {q:?}, punched {h:?}");
            let paper = card.score(q.0 as f32, q.1 as f32);
            assert_eq!(paper, card.score(h.0 as f32, h.1 as f32), "shot {s:?}");
            differ += usize::from(paper != card.score(s.0 as f32, s.1 as f32));
        }
        println!("{differ} of {} shots score differently on paper", shots.len());
        std::fs::remove_dir_all(&d).unwrap();
    }

    /// No shots: every hole found, in the registration's coordinates.
    #[test]
    fn synthetic_photo_without_shots() {
        let card = crate::card::air_pistol();
        let jpeg = demo_photo(&card, &shots(), 0.0, 9).unwrap();
        let d = dir("noshots");
        let photo = store(&d, "demo", &jpeg, now_unix()).unwrap();
        let p = process(photo, &card, &[]).unwrap();
        println!(
            "{} holes found: {:?}; print quarter {:?} (margin {:.3})",
            p.holes.len(),
            p.holes,
            p.print_quarter,
            p.print_margin
        );
        assert_eq!(p.holes.len(), shots().len());
        assert!(p.placement.is_none() && p.shots.is_empty());
        // The frame is the print's: the holes where the demo punched them (print mm), the
        // printed card drawn as it is.
        assert!(p.print_quarter.is_some());
        assert_eq!(p.pose, CardPose::IDENTITY);
        for s in shots() {
            let a = (90.0f64 + 0.5).to_radians();
            let h = (
                a.cos() * s.0 - a.sin() * s.1 + 0.7,
                a.sin() * s.0 + a.cos() * s.1 - 0.4,
            );
            let d = p
                .holes
                .iter()
                .map(|f| (f.0 - h.0).hypot(f.1 - h.1))
                .fold(f64::INFINITY, f64::min);
            assert!(d < 0.05, "hole {h:?}: nearest {d}");
        }
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn not_a_card() {
        let card = crate::card::air_pistol();
        let img = image::RgbImage::from_fn(800, 600, |x, y| {
            image::Rgb([(x % 200) as u8, (y % 150) as u8, 90])
        });
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 90)
            .encode_image(&img)
            .unwrap();
        let d = dir("nocard");
        let photo = store(&d, "demo", &jpeg, now_unix()).unwrap();
        let e = process(photo, &card, &shots()).err();
        println!("{e:?}");
        assert!(e.is_some());
        std::fs::remove_dir_all(&d).unwrap();
    }
}
