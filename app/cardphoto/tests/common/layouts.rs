//! Hole layouts and placements for the synthetic photos: shared by the tests and the
//! `holes_bench` example.
#![allow(dead_code)]

use cardphoto::synth::{Params, Rng};

pub const BLUE: [f32; 3] = [0.15, 0.3, 0.62];

/// card = R(angle) frame + shift
pub fn place(angle: f64, shift: (f64, f64), s: (f64, f64)) -> (f64, f64) {
    let (sn, cs) = angle.sin_cos();
    (cs * s.0 - sn * s.1 + shift.0, sn * s.0 + cs * s.1 + shift.1)
}

/// Shots for holes (card mm): back through the placement, plus the sensor's noise.
pub fn shots_for(
    holes: &[(f64, f64)],
    angle: f64,
    shift: (f64, f64),
    noise: f64,
    seed: u64,
) -> Vec<(f64, f64)> {
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

/// The tests' nine holes: black, white, on ring lines, near the centre, three overlapping.
pub fn nine() -> Vec<(f64, f64)> {
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

/// A series: n holes around a centre, sd mm (a shooter's group).
pub fn series(n: usize, sd: f64, centre: (f64, f64), seed: u64) -> Vec<(f64, f64)> {
    let mut rng = Rng::new(seed);
    (0..n)
        .map(|_| (centre.0 + sd * rng.gauss(), centre.1 + sd * rng.gauss()))
        .collect()
}

/// Pairs of holes at centre distances 0.5 to 4 mm, each set of six in the black, in the
/// white, and across the disc's edge (well apart from each other), and two triples 1.5 mm
/// apart (black, white).
pub fn pairs() -> Vec<(f64, f64)> {
    let seps = [0.5, 1.0, 1.5, 2.0, 3.0, 4.0];
    let black = [
        (-16.0, -14.0),
        (-2.0, -14.0),
        (12.0, -14.0),
        (-16.0, 2.0),
        (-2.0, 2.0),
        (12.0, 2.0),
    ];
    let polar = |r: f64, deg: f64| (r * deg.to_radians().cos(), r * deg.to_radians().sin());
    let white: Vec<(f64, f64)> = (0..6).map(|k| polar(50.0, 60.0 * k as f64)).collect();
    let edge: Vec<(f64, f64)> = (0..6).map(|k| polar(29.75, 30.0 + 60.0 * k as f64)).collect();
    let mut out = Vec::new();
    for (si, base) in [black.to_vec(), white, edge].iter().enumerate() {
        for (k, (c, s)) in base.iter().zip(seps).enumerate() {
            let a = (si * 6 + k) as f64 * 0.7;
            let (dx, dy) = (0.5 * s * a.cos(), 0.5 * s * a.sin());
            out.push((c.0 - dx, c.1 - dy));
            out.push((c.0 + dx, c.1 + dy));
        }
    }
    for c in [(-2.0, 16.0), (-45.0, -35.0)] {
        for k in 0..3 {
            let phi = k as f64 * std::f64::consts::TAU / 3.0;
            out.push((c.0 + 0.87 * phi.cos(), c.1 + 0.87 * phi.sin()));
        }
    }
    out
}

pub fn demo_params(holes: Vec<(f64, f64)>, seed: u64) -> Params {
    Params {
        w: 3000,
        h: 2250,
        px_per_mm: 11.0,
        rotation_deg: 8.0 + 40.0 * Rng::new(seed).uniform(),
        tilt_x_deg: 4.0,
        tilt_y_deg: -3.0,
        k1: 0.03,
        holes,
        table: 0.3,
        table_rgb: Some(BLUE),
        light: 0.2,
        blur_px: 1.0,
        noise: 0.015,
        seed,
        ..Params::default()
    }
}

/// The 41 shots of the app's slow demo photo (frame mm).
#[allow(clippy::approx_constant)] // -6.28 is a shot, not τ
pub const SHOTS41: [(f64, f64); 41] = [
    (24.67, 0.57),
    (42.53, 16.23),
    (8.5, 22.83),
    (6.86, 27.23),
    (-9.85, -11.96),
    (4.92, -5.16),
    (-6.28, -0.24),
    (12.83, 13.56),
    (-15.24, -3.41),
    (-9.23, -0.33),
    (3.75, -14.98),
    (-14.35, 16.85),
    (-7.1, -8.83),
    (-6.82, 12.32),
    (5.52, -2.33),
    (7.11, -2.14),
    (0.56, 8.96),
    (-9.73, -38.11),
    (-11.16, 11.77),
    (2.51, 24.74),
    (-16.45, 8.25),
    (-3.96, -9.65),
    (-19.72, -4.07),
    (0.2, -6.85),
    (-9.4, 7.39),
    (22.24, 19.69),
    (-17.64, 0.98),
    (-24.15, 7.19),
    (-16.95, 22.72),
    (-13.17, 6.48),
    (-30.14, 23.46),
    (3.09, -9.09),
    (-7.34, -3.01),
    (20.59, -12.86),
    (19.27, -3.38),
    (22.9, -9.55),
    (-18.66, 8.6),
    (-25.76, 6.65),
    (12.83, 17.43),
    (-6.5, -5.0),
    (-13.34, -1.41),
];
