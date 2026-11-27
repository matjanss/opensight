//! Each printed ring's line, measured again after the registration: where the photo's line
//! centre lies (mm from the card's centre, through the final mapping) against the ring's stated
//! radius, per ring. One line per file.
//!
//! `cargo run --release -p cardphoto --example ringcheck -- <photo>...`

use cardphoto::Rgb;
use cardphoto::card::issf_air_pistol;
use cardphoto::detect::ring_points;
use cardphoto::register::register;

fn median(mut v: Vec<f64>) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    v[v.len() / 2]
}

fn main() {
    let model = issf_air_pistol();
    let names: Vec<String> = model
        .features
        .iter()
        .map(|f| format!("{:>6.2}", f.radius_mm))
        .collect();
    println!(
        "{:<28} {:>6} {:>6} {:>7} {:>7} {:>6} | {}",
        "file",
        "px/mm",
        "rms",
        "k1",
        "k2",
        "offset",
        names.join(" ")
    );
    for path in std::env::args().skip(1) {
        let Ok(bytes) = std::fs::read(&path) else { continue };
        let Ok(rgb) = Rgb::decode(&bytes) else { continue };
        let gray = rgb.to_gray();
        let reg = match register(&gray, &model) {
            Ok(r) => r,
            Err(e) => {
                println!("{path}: {e}");
                continue;
            }
        };
        let h = &reg.map.h;
        let ppm = (h[(0, 0)] * h[(1, 1)] - h[(0, 1)] * h[(1, 0)]).abs().sqrt();
        let step = (0.2 / ppm).min(0.05);
        let pts = ring_points(&gray, &reg.map, &model, reg.contrast, 720, 0.5, step);
        let mut per: Vec<Vec<f64>> = vec![Vec::new(); model.features.len()];
        for p in &pts {
            if let Some(c) = reg.map.to_card(p.p) {
                per[p.feature].push(c.0.hypot(c.1) - model.features[p.feature].radius_mm);
            }
        }
        let cells: Vec<String> = per.into_iter().map(|v| format!("{:>+6.3}", median(v))).collect();
        let short = path.rsplit('/').take(2).collect::<Vec<_>>();
        println!(
            "{:<28} {:>6.1} {:>6.3} {:>+7.3} {:>+7.3} {:>+6.3} | {}",
            format!("{}/{}", short.get(1).unwrap_or(&""), short[0]),
            ppm,
            reg.rms_mm,
            reg.map.lens.k1,
            reg.map.lens.k2,
            reg.offsets.first().copied().unwrap_or(f64::NAN),
            cells.join(" ")
        );
    }
}
