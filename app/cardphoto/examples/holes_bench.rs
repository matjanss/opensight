//! The holes on a fixed set of photos, one JSON line per case: for comparing two versions of
//! the hole finding (speed and results), `app/CARDPHOTO.md`.
//!
//! Synthetic cases carry their truth (each shot's own hole), so each line also says how many
//! holes were found and how far off they are. Stored photos (`--photos <dir>`: the app's
//! `photos/`, each `<name>.jpg` with the shots from its `<name>.result.json`) are compared
//! between versions only.
//!
//! cargo run --release -p cardphoto --example holes_bench -- [--jobs N] [--only text]
//!     [--photos dir] > out.jsonl

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use cardphoto::card::issf_air_pistol;
use cardphoto::holes::{HoleFit, all_holes, ground_truth};
use cardphoto::register::register;
use cardphoto::synth::{CardTexture, Params, Rng, photo_rgb};
use cardphoto::{Gray, Mapping, Rgb};
use resvg::{tiny_skia, usvg};
use serde_json::{Value, json};

#[path = "../tests/common/layouts.rs"]
mod layouts;
use layouts::*;

const TEX_PX_PER_MM: f64 = 20.0;

fn texture() -> CardTexture {
    let dir = env!("CARGO_MANIFEST_DIR");
    let svg = std::fs::read_to_string(format!("{dir}/../assets/cards/issf-10m-air-pistol.svg")).unwrap();
    let mut opt = usvg::Options::default();
    let fonts = Arc::make_mut(&mut opt.fontdb);
    fonts.load_font_data(std::fs::read(format!("{dir}/../assets/fonts/IBMPlexSans-Regular.ttf")).unwrap());
    fonts.set_sans_serif_family("IBM Plex Sans");
    let tree = usvg::Tree::from_str(&svg, &opt).unwrap();
    let size = (170.0 * TEX_PX_PER_MM) as u32;
    let mut pm = tiny_skia::Pixmap::new(size, size).unwrap();
    let s = TEX_PX_PER_MM as f32;
    resvg::render(&tree, tiny_skia::Transform::from_scale(s, s), &mut pm.as_mut());
    let lum: Vec<u8> = pm
        .data()
        .chunks(4)
        .map(|p| (0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32) as u8)
        .collect();
    CardTexture {
        img: Gray::from_luma8(size as usize, size as usize, &lum),
        px_per_mm: TEX_PX_PER_MM,
        size_mm: 170.0,
    }
}

enum Source {
    /// Synthetic: the photo's parameters; per shot (frame mm), the index of its hole in
    /// `params.holes` (None: a false shot). No shots: unguided.
    Synth {
        params: Params,
        shots: Vec<(f64, f64)>,
        hole_of: Vec<Option<usize>>,
    },
    Stored {
        image: std::path::PathBuf,
        shots: Vec<(f64, f64)>,
    },
}

struct Case {
    name: String,
    source: Source,
}

fn guided(name: &str, params: Params, angle: f64, shift: (f64, f64), noise: f64, seed: u64) -> Case {
    let shots = shots_for(&params.holes, angle, shift, noise, seed);
    let hole_of = (0..shots.len()).map(Some).collect();
    Case {
        name: name.into(),
        source: Source::Synth {
            params,
            shots,
            hole_of,
        },
    }
}

fn synthetic() -> Vec<Case> {
    let mut cases = Vec::new();
    // The tests' cards.
    for k in 0..4u64 {
        let p = Params {
            rotation_deg: 23.0 + 61.0 * k as f64,
            tilt_x_deg: 7.0,
            tilt_y_deg: -4.0,
            k1: 0.04,
            holes: nine(),
            table: 0.3,
            table_rgb: Some(BLUE),
            light: 0.25,
            blur_px: 1.0,
            noise: 0.02,
            seed: 20 + k,
            ..Params::default()
        };
        cases.push(guided(
            &format!("quarter{k}"),
            p,
            (k as f64 * 90.0 + 0.4).to_radians(),
            (0.8, -0.6),
            0.1,
            100 + k,
        ));
    }
    let p = Params {
        rotation_deg: -12.0,
        tilt_x_deg: 5.0,
        holes: nine(),
        table: 0.45,
        blur_px: 1.0,
        noise: 0.02,
        seed: 31,
        ..Params::default()
    };
    cases.push(guided("grey", p, 0.3f64.to_radians(), (-1.1, 0.4), 0.1, 7));
    {
        let angle = 180.2f64.to_radians();
        let mut holes = nine();
        holes.push((-40.0, -20.0));
        let p = Params {
            rotation_deg: 70.0,
            holes,
            table: 0.3,
            table_rgb: Some(BLUE),
            blur_px: 1.0,
            noise: 0.02,
            seed: 41,
            ..Params::default()
        };
        let mut shots = shots_for(&nine(), angle, (0.5, 0.5), 0.1, 9);
        shots.push(shots_for(&[(45.0, 45.0)], angle, (0.5, 0.5), 0.0, 1)[0]);
        let mut hole_of: Vec<Option<usize>> = (0..9).map(Some).collect();
        hole_of.push(None);
        cases.push(Case {
            name: "missing_extra".into(),
            source: Source::Synth {
                params: p,
                shots,
                hole_of,
            },
        });
    }
    let p = Params {
        rotation_deg: 5.0,
        holes: nine(),
        table: 0.1,
        blur_px: 1.0,
        noise: 0.02,
        seed: 71,
        ..Params::default()
    };
    cases.push(guided("dark", p, 0.0, (0.0, 0.0), 0.1, 3));
    for (name, holes, ragged) in [("unguided", nine(), 0.0), ("unguided_ragged", nine(), 0.15)] {
        let p = Params {
            rotation_deg: -40.0,
            tilt_x_deg: -6.0,
            holes,
            hole_ragged_mm: ragged,
            table: 0.3,
            table_rgb: Some(BLUE),
            blur_px: 1.0,
            noise: 0.02,
            seed: 61,
            ..Params::default()
        };
        cases.push(Case {
            name: name.into(),
            source: Source::Synth {
                params: p,
                shots: vec![],
                hole_of: vec![],
            },
        });
    }
    // The app's slow demo: 41 shots, as demo_photo makes them.
    {
        let (angle, shift) = ((90.5f64).to_radians(), (0.7, -0.4));
        let mut rng = Rng::new(7);
        let holes: Vec<(f64, f64)> = SHOTS41
            .iter()
            .map(|s| {
                let c = place(angle, shift, *s);
                (c.0 + 0.1 * rng.gauss(), c.1 + 0.1 * rng.gauss())
            })
            .collect();
        let p = demo_params(holes, 7);
        cases.push(Case {
            name: "demo41".into(),
            source: Source::Synth {
                params: p,
                shots: SHOTS41.to_vec(),
                hole_of: (0..41).map(Some).collect(),
            },
        });
    }
    // Series: tight and wider groups, 20 to 60 shots, every quarter.
    let mut seed = 200;
    for (n, sd) in [(20, 6.0), (40, 6.0), (40, 10.0), (60, 8.0)] {
        for ragged in [0.0, 0.15] {
            seed += 1;
            let k = seed % 4;
            let angle = (k as f64 * 90.0 - 0.6).to_radians();
            let holes = series(n, sd, (2.0, -3.0), seed);
            let mut p = demo_params(holes, seed);
            p.hole_ragged_mm = ragged;
            let name = format!("series{n}_sd{sd}{}", if ragged > 0.0 { "_ragged" } else { "" });
            cases.push(guided(&name, p, angle, (-0.9, 0.5), 0.15, seed + 1000));
        }
    }
    // Overlapping pairs and triples, round and ragged, two noise levels.
    for (ragged, noise, seed) in [
        (0.0, 0.1, 301),
        (0.0, 0.3, 302),
        (0.15, 0.1, 303),
        (0.25, 0.3, 304),
    ] {
        let mut p = demo_params(pairs(), seed);
        p.hole_ragged_mm = ragged;
        let name = format!(
            "pairs_n{noise}{}",
            if ragged > 0.0 {
                format!("_ragged{ragged}")
            } else {
                String::new()
            }
        );
        cases.push(guided(
            &name,
            p,
            (270.3f64).to_radians(),
            (0.4, 1.2),
            noise,
            seed + 1000,
        ));
    }
    // The nine, ragged.
    for ragged in [0.1, 0.25] {
        let p = Params {
            rotation_deg: 33.0,
            tilt_x_deg: 4.0,
            holes: nine(),
            hole_ragged_mm: ragged,
            table: 0.3,
            table_rgb: Some(BLUE),
            light: 0.2,
            blur_px: 1.0,
            noise: 0.02,
            seed: 81,
            ..Params::default()
        };
        cases.push(guided(
            &format!("nine_ragged{ragged}"),
            p,
            (90.4f64).to_radians(),
            (0.2, -0.3),
            0.1,
            82,
        ));
    }
    cases
}

fn stored(dir: &str) -> Vec<Case> {
    let mut out = Vec::new();
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .collect();
    entries.sort();
    for p in entries {
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        let Some(stem) = name.strip_suffix(".result.json") else {
            continue;
        };
        let Ok(text) = std::fs::read_to_string(&p) else {
            continue;
        };
        let Ok(v) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        let shots: Vec<(f64, f64)> = v["shots"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|s| Some((s["shot_mm"][0].as_f64()?, s["shot_mm"][1].as_f64()?)))
                    .collect()
            })
            .unwrap_or_default();
        let image = ["jpg", "jpeg", "png"]
            .iter()
            .map(|e| p.with_file_name(format!("{stem}.{e}")))
            .find(|f| f.exists());
        if let Some(image) = image {
            out.push(Case {
                name: format!("stored:{stem}"),
                source: Source::Stored { image, shots },
            });
        }
    }
    out
}

/// A case's photo, its truth (synthetic), the shots, each shot's hole, the true holes.
type Loaded = (
    Rgb,
    Option<Mapping>,
    Vec<(f64, f64)>,
    Vec<Option<usize>>,
    Vec<(f64, f64)>,
);

fn hole_json(h: &HoleFit) -> Value {
    json!({"c": [h.center.0, h.center.1], "r": h.radius_mm, "sd": h.sd_mm, "contrast": h.contrast})
}

fn run(case: &Case, tex: &CardTexture) -> Value {
    let (rgb, truth, shots, hole_of, truth_holes): Loaded = match &case.source {
        Source::Synth {
            params,
            shots,
            hole_of,
        } => {
            let (rgb, map) = photo_rgb(tex, params);
            (
                rgb,
                Some(map),
                shots.clone(),
                hole_of.clone(),
                params.holes.clone(),
            )
        }
        Source::Stored { image, shots } => {
            let bytes = std::fs::read(image).unwrap();
            (Rgb::decode(&bytes).unwrap(), None, shots.clone(), vec![], vec![])
        }
    };
    let t0 = Instant::now();
    let reg = match register(&rgb.to_gray(), &issf_air_pistol()) {
        Ok(r) => r,
        Err(e) => return json!({"name": case.name, "error": e}),
    };
    let t_reg = t0.elapsed().as_secs_f64();
    let to_truth =
        |q: (f64, f64)| -> Option<(f64, f64)> { truth.as_ref().and_then(|t| t.to_card(reg.map.to_image(q))) };
    let t1 = Instant::now();
    if shots.is_empty() {
        let found = all_holes(&rgb, &reg);
        let secs = t1.elapsed().as_secs_f64();
        let mut matched = 0;
        let mut worst = 0.0f64;
        for t in &truth_holes {
            let best = found
                .iter()
                .filter_map(|h| to_truth(h.center))
                .map(|c| (c.0 - t.0).hypot(c.1 - t.1))
                .fold(f64::INFINITY, f64::min);
            if best < 0.5 {
                matched += 1;
                worst = worst.max(best);
            }
        }
        return json!({
            "name": case.name, "reg_s": t_reg, "holes_s": secs, "unguided": true,
            "holes": found.iter().map(hole_json).collect::<Vec<_>>(),
            "truth": truth.as_ref().map(|_| json!({"holes": truth_holes.len(), "matched": matched, "worst": worst})),
        });
    }
    let gt = match ground_truth(&rgb, &reg, &shots) {
        Ok(g) => g,
        Err(e) => return json!({"name": case.name, "reg_s": t_reg, "error": e}),
    };
    let secs = t1.elapsed().as_secs_f64();
    let mut per = Vec::new();
    let (mut found, mut should, mut wrong, mut false_found) = (0, 0, 0, 0);
    let mut errs = Vec::new();
    for (i, r) in gt.shots.iter().enumerate() {
        let mut e = Value::Null;
        if truth.is_some() {
            match hole_of.get(i).copied().flatten() {
                Some(t) => {
                    should += 1;
                    if let Some(h) = &r.hole {
                        found += 1;
                        let c = to_truth(h.center).unwrap();
                        let d = (c.0 - truth_holes[t].0).hypot(c.1 - truth_holes[t].1);
                        if d > 0.2 {
                            wrong += 1;
                        }
                        errs.push(d);
                        e = json!(d);
                    }
                }
                None => {
                    if r.hole.is_some() {
                        false_found += 1;
                    }
                }
            }
        }
        per.push(json!({"hole": r.hole.as_ref().map(hole_json), "truth_err": e}));
    }
    let rms = (errs.iter().map(|d| d * d).sum::<f64>() / errs.len().max(1) as f64).sqrt();
    let worst = errs.iter().copied().fold(0.0, f64::max);
    json!({
        "name": case.name, "reg_s": t_reg, "holes_s": secs, "shots_n": shots.len(),
        "quarter": gt.placement.quarter, "angle": gt.placement.angle, "shift": [gt.placement.shift.0, gt.placement.shift.1],
        "scale": gt.placement.scale, "rms_mm": gt.rms_mm, "scaled": gt.scaled, "refits": gt.refits, "quarter_all": gt.quarter_all,
        "shots": per,
        "extra": gt.extra.iter().map(hole_json).collect::<Vec<_>>(),
        "truth": truth.as_ref().map(|_| json!({"should": should, "found": found, "wrong": wrong, "false_found": false_found, "rms": rms, "worst": worst})),
    })
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let arg = |k: &str| args.iter().position(|a| a == k).map(|i| args[i + 1].clone());
    let jobs: usize = arg("--jobs").map_or(1, |v| v.parse().unwrap());
    let only = arg("--only");
    let mut cases = synthetic();
    if let Some(d) = arg("--photos") {
        cases.extend(stored(&d));
    }
    if let Some(o) = &only {
        cases.retain(|c| o.split(',').any(|w| c.name.contains(w)));
    }
    let tex = texture();
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<(usize, Value)>> = Mutex::new(Vec::new()); // a lock for the output
    std::thread::scope(|s| {
        for _ in 0..jobs {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    if i >= cases.len() {
                        break;
                    }
                    let v = run(&cases[i], &tex);
                    eprintln!(
                        "{}: {}",
                        cases[i].name,
                        v.get("holes_s").map_or("error".into(), |s| format!("{s:.2} s"))
                    );
                    // Each line as soon as it is done (a slow case doesn't hold back the others).
                    let mut r = results.lock().unwrap();
                    println!("{v}");
                    r.push((i, v));
                }
            });
        }
    });
}
