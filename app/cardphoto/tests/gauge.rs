//! C5 (`app/CARDPHOTO.md`): the gauge on synthetic torn holes, with a known truth. The holes
//! as the real photos show them: an opening off the centre showing the table in shadow, torn
//! paper, a rim (a grey smear on paper, flaked ink on black); a light wooden table close to
//! the paper's colour; a tilted, rotated camera.

use cardphoto::Mapping;
use cardphoto::card::issf_air_pistol;
use cardphoto::holes::all_gauges;
use cardphoto::register::{Registration, register};
use cardphoto::synth::{Params, photo_rgb};

mod common;
use common::texture;

/// A light wooden table (the C5 photos').
const WOOD: [f32; 3] = [0.78, 0.66, 0.5];

/// Single holes: in the black, in the white, on ring lines, across the black's edge, far out.
fn singles() -> Vec<(f64, f64)> {
    vec![
        (1.2, -0.8),
        (-12.0, 8.0),
        (37.75, 0.0),
        (0.0, -29.75),
        (-49.3, 49.3),
        (60.0, -30.0),
        (15.0, 20.0),
        (-70.0, -20.0),
    ]
}

/// A registration's card point in the truth's card mm.
fn to_truth(reg: &Registration, truth: &Mapping, q: (f64, f64)) -> (f64, f64) {
    truth.to_card(reg.map.to_image(q)).unwrap()
}

fn torn(tilt: f64, seed: u64, holes: Vec<(f64, f64)>) -> Params {
    Params {
        rotation_deg: 25.0,
        tilt_x_deg: tilt,
        tilt_y_deg: 0.4 * tilt,
        holes,
        hole_ragged_mm: 0.08,
        torn: true,
        table: 0.66,
        table_rgb: Some(WOOD),
        table_texture: 0.04,
        k1: -0.05,
        blur_px: 1.0,
        noise: 0.02,
        seed,
        ..Params::default()
    }
}

/// Every single torn hole found, its centre within 0.05 mm and its radius within 0.1 mm of
/// the pellet's circle; nothing else found.
fn check(name: &str, p: &Params) {
    let (rgb, truth) = photo_rgb(texture(), p);
    let reg = register(&rgb.to_gray(), &issf_air_pistol()).unwrap_or_else(|e| panic!("{name}: {e}"));
    let found = all_gauges(&rgb, &reg, 4.5);
    if let Some(dir) = std::env::var_os("GAUGE_SAVE") {
        let img = image::RgbImage::from_fn(rgb.w as u32, rgb.h as u32, |x, y| {
            image::Rgb(rgb.px[y as usize * rgb.w + x as usize].map(|v| (v.clamp(0.0, 1.0) * 255.0) as u8))
        });
        img.save(std::path::Path::new(&dir).join(format!("{}.png", p.seed)))
            .unwrap();
    }
    let mut missing = Vec::new();
    let mut worst = 0.0f64;
    let mut worst_r = 0.0f64;
    for t in &p.holes {
        let (best, r) = found
            .iter()
            .map(|(_, g)| {
                let c = to_truth(&reg, &truth, g.center);
                ((c.0 - t.0).hypot(c.1 - t.1), g.radius_mm)
            })
            .fold((f64::INFINITY, 0.0), |a, b| if b.0 < a.0 { b } else { a });
        if best >= 0.5 {
            missing.push((*t, best));
            continue;
        }
        worst = worst.max(best);
        worst_r = worst_r.max((r - 2.25).abs());
    }
    println!(
        "{name}: {} found of {}, worst centre {worst:.4} mm, worst radius off {worst_r:.3} mm",
        found.len(),
        p.holes.len()
    );
    assert!(
        missing.is_empty(),
        "{name}: not found (and the nearest): {missing:?}"
    );
    assert_eq!(found.len(), p.holes.len(), "{name}");
    assert!(worst < 0.05, "{name}: {worst}");
    assert!(worst_r < 0.1, "{name}: {worst_r}");
}

#[test]
fn torn_holes_straight_down() {
    check("torn, 3°", &torn(3.0, 11, singles()));
}

#[test]
fn torn_holes_tilted() {
    check("torn, 25°", &torn(25.0, 12, singles()));
}

#[test]
fn clean_card_on_wood() {
    let p = torn(15.0, 13, vec![]);
    let (rgb, _) = photo_rgb(texture(), &p);
    let reg = register(&rgb.to_gray(), &issf_air_pistol()).unwrap();
    let found = all_gauges(&rgb, &reg, 4.5);
    assert!(
        found.is_empty(),
        "{:?}",
        found.iter().map(|(_, g)| g.center).collect::<Vec<_>>()
    );
}
