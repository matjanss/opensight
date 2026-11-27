//! C1's tests (`app/CARDPHOTO.md`): registration on synthetic photos with a known truth.

use cardphoto::Mapping;
use cardphoto::card::issf_air_pistol;
use cardphoto::register::{Registration, register};
use cardphoto::synth::{Params, photo};

mod common;
use common::texture;

/// The worst error (mm) over the scoring area: each card point to the photo by the truth,
/// back by the fit, against the point turned by the best quarter turn (the outline fixes the
/// rotation only modulo 90°).
fn worst_error(reg: &Registration, truth: &Mapping) -> (f64, usize) {
    let mut best = (f64::INFINITY, 0);
    for k in 0..4 {
        let a = k as f64 * std::f64::consts::FRAC_PI_2;
        let (s, c) = a.sin_cos();
        let mut worst = 0.0f64;
        for i in -16..=16 {
            for j in -16..=16 {
                let q = (i as f64 * 5.0, j as f64 * 5.0);
                if q.0.hypot(q.1) > 78.0 {
                    continue;
                }
                let back = reg.map.to_card(truth.to_image(q)).unwrap();
                let turned = (c * q.0 - s * q.1, s * q.0 + c * q.1);
                worst = worst.max((back.0 - turned.0).hypot(back.1 - turned.1));
            }
        }
        if worst < best.0 {
            best = (worst, k);
        }
    }
    best
}

fn run(name: &str, p: &Params) -> (Registration, f64) {
    let (img, truth) = photo(texture(), p);
    let reg = register(&img, &issf_air_pistol()).unwrap_or_else(|e| panic!("{name}: {e}"));
    let (worst, k) = worst_error(&reg, &truth);
    println!(
        "{name}: worst {worst:.4} mm (quarter turn {k}), rms {:.4} mm, {} of {} points kept, axes {:?}, line offset {:+.3?} mm, k1 {:+.4} (true {:+.4}), contrast {:.2}-{:.2}",
        reg.rms_mm,
        reg.kept,
        reg.points,
        reg.axes,
        reg.offsets,
        reg.map.lens.k1,
        p.k1,
        reg.contrast.ink,
        reg.contrast.paper
    );
    // The app's card draws each 0.2 mm line with its outer edge on the stated radius, as
    // printed cards do: its centre 0.1 mm inside.
    assert!(
        (reg.offsets[0] + 0.1).abs() < 0.01,
        "{name}: line offset {:?}",
        reg.offsets
    );
    (reg, worst)
}

fn holes() -> Vec<(f64, f64)> {
    // In the black, in the white, cutting ring lines (ring 6's line at 37.75, the disc's edge
    // at 29.75, ring 2's at 69.75), and three overlapping.
    vec![
        (3.0, -2.0),
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

#[test]
fn easy() {
    let p = Params {
        rotation_deg: 10.0,
        tilt_x_deg: 5.0,
        tilt_y_deg: 3.0,
        blur_px: 1.0,
        noise: 0.01,
        light: 0.2,
        ..Params::default()
    };
    let (reg, worst) = run("easy", &p);
    assert!(reg.axes_known());
    assert!(worst < 0.02, "{worst}");
}

#[test]
fn hard() {
    let p = Params {
        rotation_deg: 137.0,
        tilt_x_deg: 12.0,
        tilt_y_deg: -9.0,
        offset: (0.05, -0.03),
        distance: 2.0,
        k1: 0.08,
        k2: -0.02,
        holes: holes(),
        light: 0.3,
        blur_px: 1.0,
        noise: 0.03,
        seed: 7,
        ..Params::default()
    };
    let (reg, worst) = run("hard", &p);
    assert!(reg.axes_known());
    assert!(worst < 0.02, "{worst}");
    assert!((reg.map.lens.k1 - p.k1).abs() < 0.01, "k1 {}", reg.map.lens.k1);
}

#[test]
fn light_table() {
    let p = Params {
        rotation_deg: -33.0,
        tilt_x_deg: -7.0,
        table: 0.72,
        k1: -0.05,
        blur_px: 1.0,
        noise: 0.02,
        holes: holes(),
        seed: 3,
        ..Params::default()
    };
    let (_, worst) = run("light table", &p);
    assert!(worst < 0.02, "{worst}");
}

#[test]
fn barrel_and_steep() {
    let p = Params {
        rotation_deg: 81.0,
        tilt_x_deg: 15.0,
        tilt_y_deg: 10.0,
        k1: -0.12,
        k2: 0.03,
        blur_px: 1.2,
        noise: 0.02,
        seed: 11,
        ..Params::default()
    };
    let (_, worst) = run("barrel, 15° tilt", &p);
    assert!(worst < 0.02, "{worst}");
}

/// A bent card isn't a plane: the fit's residual says so.
#[test]
fn bent_card_shows_in_the_residual() {
    let flat = Params {
        rotation_deg: 20.0,
        tilt_x_deg: 6.0,
        blur_px: 1.0,
        noise: 0.01,
        seed: 5,
        ..Params::default()
    };
    let bent = Params {
        bend_mm: 2.0,
        ..flat.clone()
    };
    let (rf, _) = run("flat", &flat);
    let (rb, wb) = run("bent 2 mm", &bent);
    println!("bent: worst {wb:.3} mm against the flat truth");
    assert!(
        rb.rms_mm > 2.0 * rf.rms_mm,
        "flat {} bent {}",
        rf.rms_mm,
        rb.rms_mm
    );
}

#[test]
fn no_card_is_an_error() {
    let p = Params::default();
    let (img, _) = photo(
        texture(),
        &Params {
            offset: (5.0, 5.0),
            ..p
        },
    ); // the card out of view
    assert!(register(&img, &issf_air_pistol()).is_err());
}

#[test]
fn too_small_is_an_error() {
    let p = Params {
        px_per_mm: 3.0,
        ..Params::default()
    };
    let (img, _) = photo(texture(), &p);
    let e = register(&img, &issf_air_pistol());
    println!("too small: {:?}", e.as_ref().err());
    assert!(e.is_err());
}
