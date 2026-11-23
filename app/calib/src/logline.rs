//! Reading the firmware's `cfg` and `rec` lines (`firmware/MODEL.md` "Log lines"): enough to
//! fit a calibration from a log (serial, or the app's event log), as `calibrate.py` does.

use std::collections::HashMap;

use crate::fit::Spark;
use crate::text::crc32;
use crate::{Channel, MAX_MICS};

/// A `cfg` line: its ID and the channels (the policy isn't needed here).
#[derive(Clone, Debug)]
pub struct Cfg {
    pub id: u32,
    pub channels: [Channel; MAX_MICS],
}

/// A `rec` line: what the fit needs.
#[derive(Clone, Debug)]
pub struct Rec {
    pub n: u32,
    pub cfg: u32,
    pub c_ms: f32,
    /// Bit c: CH(c+1) was in the event.
    pub channels: u8,
    pub t1: [Option<i32>; MAX_MICS],
    pub t2: [Option<i32>; MAX_MICS],
    pub pt: Option<(f32, f32)>,
    pub cal_mode: bool,
}

const NO_MIC: Channel = Channel {
    mic: false,
    x: 0.0,
    y: 0.0,
    h: 0.0,
    k: 0.0,
    offset_ns: 0.0,
    sigma_ns: 0.0,
};

/// A `cfg` line (text before "cfg id=" is ignored); None if it isn't one, or its ID doesn't
/// match its fields.
pub fn parse_cfg(line: &str) -> Option<Cfg> {
    let rest = &line[line.find("cfg id=")? + 7..];
    let rest = rest.trim_end();
    let (id, fields) = rest.split_once(' ')?;
    let id = u32::from_str_radix(id, 16).ok()?;
    if crc32(fields.as_bytes()) != id {
        return None;
    }
    let mut channels = [NO_MIC; MAX_MICS];
    for kv in fields.split_whitespace() {
        let (key, val) = kv.split_once('=')?;
        if let Some(ch) = key.strip_prefix("CH") {
            let ch: usize = ch.parse().ok()?;
            if !(1..=MAX_MICS).contains(&ch) || val == "-" {
                continue;
            }
            let v: Vec<f32> = val.split(',').map(|s| s.parse().ok()).collect::<Option<_>>()?;
            if v.len() != 6 {
                return None;
            }
            channels[ch - 1] = Channel {
                mic: true,
                x: v[0],
                y: v[1],
                h: v[2],
                k: v[3],
                offset_ns: v[4],
                sigma_ns: v[5],
            };
        }
    }
    Some(Cfg { id, channels })
}

/// A `rec` line; None if it isn't one.
pub fn parse_rec(line: &str) -> Option<Rec> {
    let rest = &line[line.find("rec n=")? + 4..];
    let mut r = Rec {
        n: 0,
        cfg: 0,
        c_ms: 0.0,
        channels: 0,
        t1: [None; MAX_MICS],
        t2: [None; MAX_MICS],
        pt: None,
        cal_mode: false,
    };
    let (mut have_cfg, mut have_c) = (false, false);
    for kv in rest.split_whitespace() {
        let Some((key, val)) = kv.split_once('=') else {
            continue;
        };
        match key {
            "n" => r.n = val.parse().ok()?,
            "cfg" => {
                r.cfg = u32::from_str_radix(val, 16).ok()?;
                have_cfg = true;
            }
            "c" => {
                r.c_ms = val.parse().ok()?;
                have_c = true;
            }
            "pt" => {
                let (x, y) = val.split_once(',')?;
                r.pt = Some((x.parse().ok()?, y.parse().ok()?));
            }
            "mode" => r.cal_mode = val == "cal",
            _ => {
                if let Some(ch) = key.strip_prefix("CH").and_then(|c| c.parse::<usize>().ok())
                    && (1..=MAX_MICS).contains(&ch)
                {
                    let (a, b) = val.split_once(',')?;
                    r.channels |= 1 << (ch - 1);
                    r.t1[ch - 1] = a.parse().ok();
                    r.t2[ch - 1] = b.parse().ok();
                }
            }
        }
    }
    (have_cfg && have_c).then_some(r)
}

/// The `cfg` and `rec` lines of a log.
pub fn read_log(text: &str) -> (HashMap<u32, Cfg>, Vec<Rec>) {
    let mut cfgs = HashMap::new();
    let mut recs = Vec::new();
    for line in text.lines() {
        if let Some(c) = parse_cfg(line) {
            cfgs.insert(c.id, c);
        } else if let Some(r) = parse_rec(line) {
            recs.push(r);
        }
    }
    (cfgs, recs)
}

/// The sparks to fit from a log, as `calibrate.py` picks them: calibration mode's events
/// with a point (`any_pt`: every event with one), of the newest session (the cfg the last
/// one was made with). Returns that cfg and the sparks.
pub fn sparks_from_log(text: &str, any_pt: bool) -> Result<(Cfg, Vec<Spark>), String> {
    let (cfgs, recs) = read_log(text);
    let with_pt: Vec<&Rec> = recs
        .iter()
        .filter(|r| r.pt.is_some() && (any_pt || r.cal_mode))
        .collect();
    let last = with_pt
        .last()
        .ok_or("no events with a point in calibration mode")?;
    let cfg = cfgs
        .get(&last.cfg)
        .ok_or_else(|| format!("the log has no cfg line {:08X}", last.cfg))?
        .clone();
    let sparks = with_pt
        .iter()
        .filter(|r| r.cfg == cfg.id)
        .map(|r| Spark {
            t1: r.t1,
            t2: r.t2,
            pt: r.pt.unwrap(),
            c_ms: r.c_ms,
        })
        .collect();
    Ok((cfg, sparks))
}
