//! C2's tests (`app/CARDPHOTO.md`): the holes and their match to the shots, on synthetic
//! photos with a known truth.

use cardphoto::Mapping;
use cardphoto::card::issf_air_pistol;
use cardphoto::holes::{GroundTruth, all_holes, ground_truth};
use cardphoto::register::{Registration, register};
use cardphoto::synth::{Params, Rng, photo_rgb};

mod common;
use common::{layouts, texture};

/// In the black, in the white, on ring lines (ring 6 at 37.75, the disc's edge at 29.75, ring 2
/// at 69.75), near the centre, and three overlapping.
fn holes() -> Vec<(f64, f64)> {
    vec![
        (1.2, -0.8),
        (-12.0, 8.0),
        (37.75, 0.0),
        (0.0, -29.75),
        (-49.3, 49.3),
        (60.0, -30.0),
        (10.0, 15.0),
        (12.5, 16.5),
        (11.0, 18.5),
    ]
}

/// The card's placement in the frame (the truth): card = R(angle) frame + shift.
fn place(angle: f64, shift: (f64, f64), s: (f64, f64)) -> (f64, f64) {
    let (sn, cs) = angle.sin_cos();
    (cs * s.0 - sn * s.1 + shift.0, sn * s.0 + cs * s.1 + shift.1)
}

/// The shots the sensor would report for the holes: back through the placement, plus noise.
fn shots_for(holes: &[(f64, f64)], angle: f64, shift: (f64, f64), noise: f64, seed: u64) -> Vec<(f64, f64)> {
    let mut rng = Rng::new(seed);
    let (sn, cs) = (-angle).sin_cos();
    holes
        .iter()
        .map(|h| {
            let (x, y) = (h.0 - shift.0, h.1 - shift.1);
            (
                cs * x - sn * y + noise * rng.gauss(),
                sn * x + cs * y + noise * rng.gauss(),
            )
        })
        .collect()
}

/// A registration point (card mm in its coordinates) in the truth's card coordinates.
fn to_truth(reg: &Registration, truth: &Mapping, q: (f64, f64)) -> (f64, f64) {
    truth.to_card(reg.map.to_image(q)).unwrap()
}

struct Check {
    worst_hole: f64,
    /// The rms over the shots of where the placement puts them against the truth (the shots'
    /// noise makes the placement itself uncertain: with 9 shots of 0.1 mm noise and 4 placement
    /// values, about 0.067 mm rms is expected).
    rms_placed: f64,
}

/// Runs a photo with holes and shots; compares the holes and the placement with the truth.
fn check(
    name: &str,
    p: &Params,
    angle: f64,
    shift: (f64, f64),
    shots: &[(f64, f64)],
) -> (GroundTruth, Check) {
    let (rgb, truth) = photo_rgb(texture(), p);
    let reg = register(&rgb.to_gray(), &issf_air_pistol()).unwrap_or_else(|e| panic!("{name}: {e}"));
    let gt = ground_truth(&rgb, &reg, shots, 4.5).unwrap_or_else(|e| panic!("{name}: {e}"));
    let mut worst_hole = 0.0f64;
    let mut sum_placed = 0.0f64;
    for (s, r) in shots.iter().zip(&gt.shots) {
        // Where the placement puts the shot, in the truth's coordinates, against the truth.
        let est = to_truth(&reg, &truth, gt.placement.apply(*s));
        let want = place(angle, shift, *s);
        sum_placed += (est.0 - want.0).powi(2) + (est.1 - want.1).powi(2);
        if let Some(h) = &r.hole {
            let c = to_truth(&reg, &truth, h.center);
            let (d, t) = p
                .holes
                .iter()
                .map(|t| ((c.0 - t.0).hypot(c.1 - t.1), *t))
                .fold((f64::INFINITY, (0.0, 0.0)), |a, b| if b.0 < a.0 { b } else { a });
            if std::env::var("HOLES_VERBOSE").is_ok() {
                println!(
                    "  {name}: hole at ({:+.2}, {:+.2}): off {d:.4} mm, r {:.3}, contrast {:.2}, sd {:.4}",
                    t.0, t.1, h.radius_mm, h.contrast, h.sd_mm
                );
            }
            worst_hole = worst_hole.max(d);
        }
    }
    let rms_placed = (sum_placed / shots.len() as f64).sqrt();
    let found = gt.shots.iter().filter(|r| r.hole.is_some()).count();
    println!(
        "{name}: {found} of {} holes, worst hole {worst_hole:.4} mm, placement: quarter {} tilt {:+.3}° shift ({:+.3}, {:+.3}) scale {:.5}, placed rms {rms_placed:.4} mm, rms diff {:.4} mm, sd {:.4}, {} extra",
        shots.len(),
        gt.placement.quarter,
        gt.placement.tilt_deg(),
        gt.placement.shift.0,
        gt.placement.shift.1,
        gt.placement.scale,
        gt.rms_mm,
        gt.shots
            .iter()
            .filter_map(|r| r.hole.as_ref().map(|h| h.sd_mm))
            .fold(0.0, f64::max),
        gt.extra.len()
    );
    (
        gt,
        Check {
            worst_hole,
            rms_placed,
        },
    )
}

const BLUE: [f32; 3] = [0.15, 0.3, 0.62];

#[test]
fn every_quarter_turn() {
    for k in 0..4 {
        let angle = (k as f64 * 90.0 + 0.4).to_radians();
        let shift = (0.8, -0.6);
        let p = Params {
            rotation_deg: 23.0 + 61.0 * k as f64,
            tilt_x_deg: 7.0,
            tilt_y_deg: -4.0,
            k1: 0.04,
            holes: holes(),
            table: 0.3,
            table_rgb: Some(BLUE),
            light: 0.25,
            blur_px: 1.0,
            noise: 0.02,
            seed: 20 + k,
            ..Params::default()
        };
        let shots = shots_for(&p.holes, angle, shift, 0.1, 100 + k);
        let (gt, c) = check(&format!("quarter {k}"), &p, angle, shift, &shots);
        assert!(
            gt.shots.iter().all(|r| r.hole.is_some()),
            "quarter {k}: a hole not found"
        );
        assert!(c.worst_hole < 0.05, "quarter {k}: hole {}", c.worst_hole);
        assert!(c.rms_placed < 0.1, "quarter {k}: placed {}", c.rms_placed);
        assert!(gt.extra.is_empty(), "quarter {k}: extra {:?}", gt.extra);
    }
}

#[test]
fn grey_table() {
    let angle = 0.3f64.to_radians();
    let p = Params {
        rotation_deg: -12.0,
        tilt_x_deg: 5.0,
        holes: holes(),
        table: 0.45,
        blur_px: 1.0,
        noise: 0.02,
        seed: 31,
        ..Params::default()
    };
    let shots = shots_for(&p.holes, angle, (-1.1, 0.4), 0.1, 7);
    let (gt, c) = check("grey table", &p, angle, (-1.1, 0.4), &shots);
    assert!(gt.shots.iter().all(|r| r.hole.is_some()));
    assert!(
        c.worst_hole < 0.05 && c.rms_placed < 0.1,
        "{} {}",
        c.worst_hole,
        c.rms_placed
    );
}

/// A shot with no hole (a false shot), and a hole with no shot (a shot the sensor missed).
#[test]
fn missing_and_extra() {
    let angle = 180.2f64.to_radians();
    let mut card_holes = holes();
    let extra_hole = (-40.0, -20.0);
    card_holes.push(extra_hole);
    let p = Params {
        rotation_deg: 70.0,
        holes: card_holes.clone(),
        table: 0.3,
        table_rgb: Some(BLUE),
        blur_px: 1.0,
        noise: 0.02,
        seed: 41,
        ..Params::default()
    };
    let mut shots = shots_for(&holes(), angle, (0.5, 0.5), 0.1, 9);
    shots.push(shots_for(&[(45.0, 45.0)], angle, (0.5, 0.5), 0.0, 1)[0]); // no hole there
    let (gt, _) = check("missing and extra", &p, angle, (0.5, 0.5), &shots);
    assert!(
        gt.shots.last().unwrap().hole.is_none(),
        "the false shot got a hole"
    );
    assert_eq!(gt.shots.iter().filter(|r| r.hole.is_none()).count(), 1);
    assert_eq!(gt.extra.len(), 1, "{:?}", gt.extra);
}

/// No holes: the printed numbers and lines are not holes.
#[test]
fn clean_card_has_no_holes() {
    let p = Params {
        rotation_deg: 15.0,
        tilt_y_deg: 6.0,
        table: 0.3,
        table_rgb: Some(BLUE),
        blur_px: 1.0,
        noise: 0.02,
        seed: 51,
        ..Params::default()
    };
    let (rgb, _) = photo_rgb(texture(), &p);
    let reg = register(&rgb.to_gray(), &issf_air_pistol()).unwrap();
    let found = all_holes(&rgb, &reg);
    println!("clean card: {found:?}");
    assert!(found.is_empty());
}

/// Without shots (a test card): every hole found by its shape.
#[test]
fn unguided() {
    let p = Params {
        rotation_deg: -40.0,
        tilt_x_deg: -6.0,
        holes: holes(),
        table: 0.3,
        table_rgb: Some(BLUE),
        blur_px: 1.0,
        noise: 0.02,
        seed: 61,
        ..Params::default()
    };
    let (rgb, truth) = photo_rgb(texture(), &p);
    let reg = register(&rgb.to_gray(), &issf_air_pistol()).unwrap();
    let found = all_holes(&rgb, &reg);
    let mut worst = 0.0f64;
    let mut matched = 0;
    for t in &p.holes {
        let best = found
            .iter()
            .map(|h| {
                let c = to_truth(&reg, &truth, h.center);
                (c.0 - t.0).hypot(c.1 - t.1)
            })
            .fold(f64::INFINITY, f64::min);
        if best < 0.5 {
            matched += 1;
            worst = worst.max(best);
        } else {
            println!(
                "  unguided: not found: ({:+.2}, {:+.2}) (nearest found {best:.2} mm)",
                t.0, t.1
            );
        }
    }
    println!(
        "unguided: {} found, {matched} of {} matched, worst {worst:.4} mm",
        found.len(),
        p.holes.len()
    );
    assert!(matched >= p.holes.len() - 1, "{matched}");
    assert!(worst < 0.05, "{worst}");
}

/// A table as dark as the ink: holes in the black hardly show. They must be reported as not
/// found, never placed wrong.
#[test]
fn dark_table_never_misplaces() {
    let angle = 0.0;
    let p = Params {
        rotation_deg: 5.0,
        holes: holes(),
        table: 0.1,
        blur_px: 1.0,
        noise: 0.02,
        seed: 71,
        ..Params::default()
    };
    let shots = shots_for(&p.holes, angle, (0.0, 0.0), 0.1, 3);
    let (gt, c) = check("dark table", &p, angle, (0.0, 0.0), &shots);
    let missing = gt.shots.iter().filter(|r| r.hole.is_none()).count();
    println!("dark table: {missing} of {} holes not found", shots.len());
    assert!(c.worst_hole < 0.1, "a hole misplaced by {}", c.worst_hole);
}

/// Each shot's own hole (`p.holes[i]` for shot i): how many found, the worst error against
/// the truth (mm), and how long the holes took.
fn own_holes(name: &str, p: &Params, shots: &[(f64, f64)]) -> (usize, f64, f64) {
    let (rgb, truth) = photo_rgb(texture(), p);
    let reg = register(&rgb.to_gray(), &issf_air_pistol()).unwrap_or_else(|e| panic!("{name}: {e}"));
    let t = std::time::Instant::now();
    let gt = ground_truth(&rgb, &reg, shots, 4.5).unwrap_or_else(|e| panic!("{name}: {e}"));
    let secs = t.elapsed().as_secs_f64();
    let mut worst = 0.0f64;
    let mut found = 0;
    for (r, want) in gt.shots.iter().zip(&p.holes) {
        if let Some(h) = &r.hole {
            let c = to_truth(&reg, &truth, h.center);
            worst = worst.max((c.0 - want.0).hypot(c.1 - want.1));
            if std::env::var_os("HOLES_DEBUG").is_some() {
                println!(
                    "  {want:?} off {:.3} r {:.2} overlap {}",
                    (c.0 - want.0).hypot(c.1 - want.1),
                    h.radius_mm,
                    r.overlap
                );
            }
            found += 1;
        }
    }
    println!(
        "{name}: {found} of {} found, worst {worst:.3} mm, {secs:.1} s",
        shots.len()
    );
    (found, worst, secs)
}

/// Pairs 0.5 to 4 mm apart (in the black, the white, across the disc's edge) and two
/// triples 1.5 mm apart: every hole found, on its own shot's hole, with the shots' noise at
/// 0.1 and at 0.3 mm (where a pair's two shots can swap places).
#[test]
fn overlapping_pairs() {
    for (noise, seed) in [(0.1, 301), (0.3, 302)] {
        let p = layouts::demo_params(layouts::pairs(), seed);
        let shots = layouts::shots_for(&p.holes, 270.3f64.to_radians(), (0.4, 1.2), noise, seed + 1000);
        let (found, worst, _) = own_holes(&format!("pairs, noise {noise} mm"), &p, &shots);
        assert_eq!(found, shots.len(), "noise {noise}: a hole not found");
        assert!(worst < 0.25, "noise {noise}: a hole {worst} mm off");
    }
}

/// Torn edges (the radius varying round each hole by 0.1 and 0.25 mm rms): every hole found,
/// the centre off by about the tear.
#[test]
fn ragged_holes() {
    for (ragged, limit) in [(0.1, 0.12), (0.25, 0.35)] {
        let p = Params {
            rotation_deg: 33.0,
            tilt_x_deg: 4.0,
            holes: layouts::nine(),
            hole_ragged_mm: ragged,
            table: 0.3,
            table_rgb: Some(BLUE),
            light: 0.2,
            blur_px: 1.0,
            noise: 0.02,
            seed: 81,
            ..Params::default()
        };
        let shots = layouts::shots_for(&p.holes, 90.4f64.to_radians(), (0.2, -0.3), 0.1, 82);
        let (found, worst, _) = own_holes(&format!("ragged {ragged} mm"), &p, &shots);
        assert_eq!(found, shots.len(), "ragged {ragged}: a hole not found");
        assert!(worst < limit, "ragged {ragged}: a hole {worst} mm off");
    }
}

/// A tight group (20 shots, 6 mm sd): in its shot-out middle the paper shows no single holes.
/// Those are not found; the holes found are on their own shots' holes.
#[test]
fn dense_group_never_misplaced() {
    let p = layouts::demo_params(layouts::series(20, 6.0, (2.0, -3.0), 201), 201);
    let shots = layouts::shots_for(&p.holes, 89.4f64.to_radians(), (-0.9, 0.5), 0.15, 1201);
    let (found, worst, _) = own_holes("dense group", &p, &shots);
    assert!(found >= 6, "only {found} found");
    assert!(worst < 0.2, "a hole {worst} mm off");
}

/// The app's demo of 41 shots, which once took 85 s: every hole, and well within the time.
#[test]
fn demo_41_shots_in_time() {
    let (angle, shift) = (90.5f64.to_radians(), (0.7, -0.4));
    let mut rng = Rng::new(7);
    let holes = layouts::SHOTS41
        .iter()
        .map(|s| {
            let c = layouts::place(angle, shift, *s);
            (c.0 + 0.1 * rng.gauss(), c.1 + 0.1 * rng.gauss())
        })
        .collect();
    let p = layouts::demo_params(holes, 7);
    let (found, worst, secs) = own_holes("demo, 41 shots", &p, &layouts::SHOTS41);
    assert_eq!(found, 41);
    assert!(worst < 0.06, "a hole {worst} mm off");
    assert!(secs < 30.0, "{secs} s");
}
