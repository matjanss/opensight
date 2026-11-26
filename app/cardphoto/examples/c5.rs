//! CARDPHOTO C5: real photos. Each file given: the registration and every hole in the scoring
//! area (the unguided search: these cards were not shot on a sensor), the openings and the
//! gauge's circles (`C5_CAL=<mm>`, 4.5 by default), one JSON line each.
//!
//! `cargo run --release -p cardphoto --example c5 -- <photo>... > out.jsonl`

use std::time::Instant;

use cardphoto::Rgb;
use cardphoto::card::issf_air_pistol;
use cardphoto::holes::{all_gauges, all_holes, ground_truth};
use cardphoto::register::register;
use serde_json::json;

fn one(path: &str) -> serde_json::Value {
    let t0 = Instant::now();
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => return json!({ "file": path, "error": e.to_string() }),
    };
    let rgb = match Rgb::decode(&bytes) {
        Ok(r) => r,
        Err(e) => return json!({ "file": path, "error": e }),
    };
    let t_decode = t0.elapsed().as_secs_f64();
    let reg = match register(&rgb.to_gray(), &issf_air_pistol()) {
        Ok(r) => r,
        Err(e) => return json!({ "file": path, "w": rgb.w, "h": rgb.h, "error": e }),
    };
    let t_reg = t0.elapsed().as_secs_f64() - t_decode;
    // C5_NOHOLES=1: the registration alone.
    // C5_CAL=<mm>: the calibre (4.5 by default).
    let cal: f64 = std::env::var("C5_CAL")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(4.5);
    let (holes, gauges) = if std::env::var_os("C5_NOHOLES").is_some() {
        (vec![], vec![])
    } else {
        (all_holes(&rgb, &reg), all_gauges(&rgb, &reg, cal))
    };
    let t_holes = t0.elapsed().as_secs_f64() - t_decode - t_reg;
    // C5_GUIDED=1: the app's way, with shots made from the gauge's holes (turned 90.5°, shifted
    // (0.7, -0.4) mm, 0.3 mm of noise): per hole how far the matched hole is from the gauge's.
    let guided = std::env::var_os("C5_GUIDED").map(|_| {
        let (angle, shift) = (90.5f64.to_radians(), (0.7, -0.4));
        let (sn, cs) = (-angle).sin_cos();
        let mut rng = cardphoto::synth::Rng::new(9);
        let shots: Vec<(f64, f64)> = gauges
            .iter()
            .map(|(_, g)| {
                let (x, y) = (g.center.0 - shift.0, g.center.1 - shift.1);
                (
                    cs * x - sn * y + 0.3 * rng.gauss(),
                    sn * x + cs * y + 0.3 * rng.gauss(),
                )
            })
            .collect();
        let t = Instant::now();
        match ground_truth(&rgb, &reg, &shots, cal) {
            Ok(gt) => json!({
                "s": t.elapsed().as_secs_f64(),
                "extra": gt.extra.len(),
                "holes": gt.shots.iter().zip(&gauges).map(|(r, (_, g))| r.hole.as_ref().map(|h| json!({
                    "off": (h.center.0 - g.center.0).hypot(h.center.1 - g.center.1),
                    "r": h.radius_mm,
                    "overlap": r.overlap,
                }))).collect::<Vec<_>>(),
            }),
            Err(e) => json!({ "error": e }),
        }
    });
    if let Some(dir) = std::env::var_os("C5_RECT") {
        rectified(&rgb, &reg, &holes, &gauges, path, std::path::Path::new(&dir));
    }
    let v = cardphoto::view::view(&reg.map);
    let view = json!({
        "tilt": v.tilt_deg,
        "f_px": v.f_px,
        "f_from_photo": v.f_from_photo,
        "camera": [v.camera_mm.0, v.camera_mm.1, v.camera_mm.2],
    });
    let h = reg.map.h / reg.map.h[(2, 2)];
    let hm: Vec<Vec<f64>> = (0..3).map(|i| (0..3).map(|j| h[(i, j)]).collect()).collect();
    json!({
        "file": path,
        "w": rgb.w,
        "h": rgb.h,
        "s_decode": t_decode,
        "s_reg": t_reg,
        "s_holes": t_holes,
        "rms_mm": reg.rms_mm,
        "points": reg.points,
        "kept": reg.kept,
        "per_feature": reg.per_feature,
        "d_dark": reg.d_dark,
        "d_light": reg.d_light,
        "k1": reg.map.lens.k1,
        "k2": reg.map.lens.k2,
        "lens": [reg.map.lens.cx, reg.map.lens.cy, reg.map.lens.norm],
        "H": hm,
        "axes": format!("{:?}", reg.axes),
        "view": view,
        "contrast": [reg.contrast.paper, reg.contrast.ink],
        "holes": holes.iter().map(|f| json!({
            "c": [f.center.0, f.center.1],
            "r": f.radius_mm,
            "sd": f.sd_mm,
            "contrast": f.contrast,
        })).collect::<Vec<_>>(),
        "guided": guided,
        "gauges": gauges.iter().map(|(h, g)| json!({
            "c": [g.center.0, g.center.1],
            "r": g.radius_mm,
            "sd": g.sd_mm,
            "rays": g.rays,
            "rough": g.rough_mm,
            "damage": g.damage,
            "open": { "c": [h.center.0, h.center.1], "r": h.radius_mm, "contrast": h.contrast },
        })).collect::<Vec<_>>(),
    })
}

/// C5_RECT=<dir>: the photo on the card's grid, 6 px/mm, card x right and y up (the
/// registration's axes, so turned by a quarter turn or not as it found them), the holes found
/// in red (their fitted circle, and the centre), the gauge's circles in blue.
fn rectified(
    rgb: &Rgb,
    reg: &cardphoto::register::Registration,
    holes: &[cardphoto::holes::HoleFit],
    gauges: &[(cardphoto::holes::HoleFit, cardphoto::gauge::GaugeFit)],
    path: &str,
    dir: &std::path::Path,
) {
    let ppm = 6.0;
    let half = 86.0;
    let n = (2.0 * half * ppm) as u32;
    let mut img = image::RgbImage::new(n, n);
    for j in 0..n {
        for i in 0..n {
            let q = (
                -half + (f64::from(i) + 0.5) / ppm,
                half - (f64::from(j) + 0.5) / ppm,
            );
            let c = rgb.sample_at(&reg.map, q).unwrap_or([0.0, 0.0, 0.0]);
            img.put_pixel(i, j, image::Rgb(c.map(|v| (v.clamp(0.0, 1.0) * 255.0) as u8)));
        }
    }
    let mut dot = |x: f64, y: f64, c: [u8; 3]| {
        let (i, j) = (((x + half) * ppm) as i64, ((half - y) * ppm) as i64);
        if (0..i64::from(n)).contains(&i) && (0..i64::from(n)).contains(&j) {
            img.put_pixel(i as u32, j as u32, image::Rgb(c));
        }
    };
    let circles = holes
        .iter()
        .map(|h| (h.center, h.radius_mm, [255, 0, 0]))
        .chain(gauges.iter().map(|(_, g)| (g.center, g.radius_mm, [0, 90, 255])));
    for (c, r, col) in circles {
        for k in 0..360 {
            let a = f64::from(k).to_radians();
            dot(c.0 + r * a.cos(), c.1 + r * a.sin(), col);
        }
        for d in -3..=3 {
            dot(c.0 + f64::from(d) / ppm, c.1, col);
            dot(c.0, c.1 + f64::from(d) / ppm, col);
        }
    }
    let p = std::path::Path::new(path);
    let name = format!(
        "{}_{}.png",
        p.parent()
            .and_then(|d| d.file_name())
            .map_or("x".into(), |d| d.to_string_lossy()),
        p.file_stem().map_or("x".into(), |f| f.to_string_lossy())
    );
    img.save(dir.join(name)).unwrap();
}

fn main() {
    for path in std::env::args().skip(1) {
        let v = one(&path);
        println!("{v}");
        eprintln!(
            "{path}: {}",
            v.get("error").map_or_else(
                || format!(
                    "rms {:.4} mm, {} holes, {} gauges, {:.1} s",
                    v["rms_mm"].as_f64().unwrap(),
                    v["holes"].as_array().unwrap().len(),
                    v["gauges"].as_array().unwrap().len(),
                    v["s_decode"].as_f64().unwrap()
                        + v["s_reg"].as_f64().unwrap()
                        + v["s_holes"].as_f64().unwrap()
                ),
                |e| format!("ERROR {e}")
            )
        );
    }
}
