//! The Rust fit against `calibrate.py` on the same log (PLAN D5: "ported from calibrate.py and
//! checked against the same logs"), and both against the truth.
//!
//! `tests/data/selftest.log` and `.json` come from
//! `python3 firmware/tools/calibrate.py --selftest --write-log ... --json ...`: a simulated
//! calibration (16 points, 10 sparks each, mics up to 1.5 mm off, offsets up to ±500 ns,
//! k 0.25-0.6), and calibrate.py's fit of it.

use calib::fit::{calibrate, objective};
use calib::logline::sparks_from_log;
use serde_json::Value;

fn data(name: &str) -> String {
    std::fs::read_to_string(format!("{}/tests/data/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

fn floats(v: &Value, key: &str) -> Vec<f64> {
    v[key]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_f64().unwrap())
        .collect()
}

#[test]
fn selftest_matches_calibrate_py() {
    let (cfg, sparks) = sparks_from_log(&data("selftest.log"), false).unwrap();
    let want: Value = serde_json::from_str(&data("selftest.json")).unwrap();
    let fit = calibrate(&cfg.channels, &sparks).unwrap();

    let chans: Vec<usize> = fit.channels.iter().map(|c| c.channel + 1).collect();
    let want_chans: Vec<usize> = want["channels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_u64().unwrap() as usize)
        .collect();
    assert_eq!(chans, want_chans);
    assert_eq!(fit.sparks as u64, want["sparks"].as_u64().unwrap());
    assert_eq!(fit.outliers as u64, want["outliers"].as_u64().unwrap());
    assert_eq!(fit.points.len() as u64, want["points"].as_u64().unwrap());

    let (wx, wy, wk, wo, ws) = (
        floats(&want, "x"),
        floats(&want, "y"),
        floats(&want, "k"),
        floats(&want, "offset"),
        floats(&want, "sigma"),
    );
    let (sx, sy, sk, so) = (
        floats(&want, "sd_x"),
        floats(&want, "sd_y"),
        floats(&want, "sd_k"),
        floats(&want, "sd_offset"),
    );
    let mut worst = [0.0f64; 5];
    for (j, c) in fit.channels.iter().enumerate() {
        worst[0] = worst[0].max((c.x - wx[j]).abs()).max((c.y - wy[j]).abs());
        worst[1] = worst[1].max((c.k - wk[j]).abs());
        worst[2] = worst[2].max((c.offset_ns - wo[j]).abs());
        worst[3] = worst[3].max((c.sigma_ns - ws[j]).abs());
        for (a, b) in [
            (c.sd_x, sx[j]),
            (c.sd_y, sy[j]),
            (c.sd_k, sk[j]),
            (c.sd_offset, so[j]),
        ] {
            worst[4] = worst[4].max((a / b - 1.0).abs());
        }
    }
    println!(
        "against calibrate.py: mics within {:.2e} mm, k {:.2e}, offsets {:.2e} ns, sigma {:.2e} ns, uncertainties {:.2}% ; rms {:.3} (py {:.3}) ns; error after {:.5} (py {:.5}) mm; id {:08X} (py {})",
        worst[0],
        worst[1],
        worst[2],
        worst[3],
        worst[4] * 100.0,
        fit.rms_ns,
        want["rms"],
        fit.after_rms_mm,
        want["err_after_rms"],
        fit.id,
        want["id"]
    );
    // The same minimum. k and the offsets trade off along a very flat valley, where no
    // optimizer pins the last printed digits (scipy, even with tight tolerances and central
    // differences, stops a fraction of a ns short): so every value must agree to well under
    // a tenth of its uncertainty, the Rust fit's objective may not be higher than
    // calibrate.py's, and the shots must land the same.
    for (j, c) in fit.channels.iter().enumerate() {
        for (what, got, py, sd) in [
            ("x", c.x, wx[j], c.sd_x),
            ("y", c.y, wy[j], c.sd_y),
            ("k", c.k, wk[j], c.sd_k),
            ("offset", c.offset_ns, wo[j], c.sd_offset),
        ] {
            assert!(
                (got - py).abs() < 0.1 * sd,
                "CH{} {what}: {got} against {py} (sd {sd})",
                c.channel + 1
            );
        }
    }
    assert!(worst[3] < 0.05, "sigma {}", worst[3]);
    assert!(worst[4] < 0.05, "uncertainties {}", worst[4]);
    assert!((fit.rms_ns - want["rms"].as_f64().unwrap()).abs() < 0.01);
    assert!((fit.before_rms_mm - want["err_before_rms"].as_f64().unwrap()).abs() < 1e-3);
    assert!((fit.after_rms_mm - want["err_after_rms"].as_f64().unwrap()).abs() < 1e-4);
    let ours = objective(
        &cfg.channels,
        &sparks,
        &fit.channels.iter().map(|c| c.x).collect::<Vec<_>>(),
        &fit.channels.iter().map(|c| c.y).collect::<Vec<_>>(),
        &fit.channels.iter().map(|c| c.k).collect::<Vec<_>>(),
        &fit.channels.iter().map(|c| c.offset_ns).collect::<Vec<_>>(),
    );
    let theirs = objective(&cfg.channels, &sparks, &wx, &wy, &wk, &wo);
    println!("objective: ours {ours:.4}, calibrate.py's {theirs:.4}");
    assert!(
        ours <= theirs * (1.0 + 1e-9),
        "the Rust fit stopped short: {ours} > {theirs}"
    );
    // The record is what the sensor will hold, and its id is computed from it.
    assert_eq!(fit.id, calib::text::calibration_id(&fit.record));
    for c in &fit.channels {
        let r = fit.record[c.channel];
        assert_eq!(r.x, format!("{:.4}", c.x).parse::<f32>().unwrap());
        assert_eq!(r.k, format!("{:.5}", c.k).parse::<f32>().unwrap());
        assert_eq!(r.offset_ns, format!("{:.2}", c.offset_ns).parse::<f32>().unwrap());
    }
}

/// As calibrate.py's self-test: the values against the truth, within the fit's uncertainty.
#[test]
fn selftest_against_the_truth() {
    let (cfg, sparks) = sparks_from_log(&data("selftest.log"), false).unwrap();
    let truth: Value = serde_json::from_str::<Value>(&data("selftest.json")).unwrap()["truth"].clone();
    let fit = calibrate(&cfg.channels, &sparks).unwrap();
    let (tx, ty, tk, to) = (
        floats(&truth, "x"),
        floats(&truth, "y"),
        floats(&truth, "k"),
        floats(&truth, "offset"),
    );
    let (mut ex, mut zk, mut zo) = (0.0f64, 0.0f64, 0.0f64);
    for (j, c) in fit.channels.iter().enumerate() {
        ex = ex.max((c.x - tx[j]).abs()).max((c.y - ty[j]).abs());
        zk = zk.max((c.k - tk[j]).abs() / c.sd_k);
        zo = zo.max((c.offset_ns - to[j]).abs() / c.sd_offset);
    }
    println!(
        "against the truth: mics within {ex:.4} mm, k {zk:.1} sd, offsets {zo:.1} sd; error {:.3} -> {:.4} mm rms",
        fit.before_rms_mm, fit.after_rms_mm
    );
    assert!(ex < 0.05 && zk < 3.5 && zo < 3.5 && fit.after_rms_mm < 0.02);
    assert!(fit.before_rms_mm > 0.3); // the start was really off
}
