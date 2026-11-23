//! The event log (protocol v3): every event the sensor sends, written as the firmware's
//! serial log lines (`cfg`, `rec`, `dip`), so `firmware/tools/replay` and `calibrate.py`
//! read an app's log like a serial one. See `firmware/MODEL.md` "Log lines".
//!
//! One file per day (UTC) in the app's data folder, `logs/YYYY-MM-DD.log`, appended to.

use std::collections::HashSet;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::protocol::{
    Calibration, EV_CAL_MODE, EV_CAL_STALE, EV_CURTAIN_ON, EV_HAS_PT, Event, Kind, MEAS_P, MEAS_RH, MEAS_T,
    NO_VALUE, Settings,
};

use calib::text::{crc32, g9};

/// The `cfg` line's fields, as `measure::formatFields` writes them.
fn cfg_fields(cal: &Calibration, set: &Settings) -> String {
    let mut s = calib::text::channel_fields(&cal.ch);
    s += &format!(
        " lim={} drop={} lost={} area={} tog={} pmax={}",
        g9(set.res_limit_ns),
        set.drop_one as u8,
        set.max_lost,
        g9(set.area_mm),
        g9(set.together_ns),
        g9(set.pulse_max_ns)
    );
    if set.curtain_hi_ns > 0.0 {
        s += &format!(" ir={},{}", g9(set.curtain_lo_ns), g9(set.curtain_hi_ns));
    } else {
        s += " ir=-";
    }
    s
}

/// The `cfg` line for the sensor's calibration and settings, and its ID (the firmware's: the
/// CRC-32 of the fields). It matches Status's cfg ID when both were read after the last change.
pub fn cfg_line(cal: &Calibration, set: &Settings) -> (u32, String) {
    let fields = cfg_fields(cal, set);
    let id = crc32(fields.as_bytes());
    (id, format!("cfg id={id:08X} {fields}"))
}

/// The calibration ID: the CRC-32 of the channel fields alone (`measure::calibrationId`).
#[allow(dead_code)] // the calibration wizard (step 5d)
pub fn calibration_id(cal: &Calibration, _set: &Settings) -> u32 {
    calib::text::calibration_id(&cal.ch)
}

fn ns_time(t_ns: u64, digits: usize) -> String {
    // As C's "%.7f" of the firmware's double seconds: whole seconds, then the fraction.
    let secs = (t_ns / 1_000_000_000) as f64 + (t_ns % 1_000_000_000) as f64 / 1e9;
    format!("{secs:.digits$}")
}

/// An acoustic event's `rec` line (`measure::formatRec`); None for a hole or missed shot.
pub fn rec_line(e: &Event) -> Option<String> {
    if e.kind != Kind::Shot {
        return None;
    }
    let meas = format!(
        "{}{}{}",
        if e.measured & MEAS_T != 0 { 'T' } else { '-' },
        if e.measured & MEAS_RH != 0 { 'R' } else { '-' },
        if e.measured & MEAS_P != 0 { 'P' } else { '-' }
    );
    let mut s = format!(
        "rec n={} t={} cfg={:08X} T={:.2} RH={:.1} p={:.2} meas={} c={}",
        e.n,
        ns_time(e.t_ns, 7),
        e.cfg,
        f64::from(e.t_c),
        f64::from(e.rh),
        f64::from(e.hpa),
        meas,
        g9(e.c)
    );
    let edge = |t: i32| {
        if t == NO_VALUE {
            "-".to_owned()
        } else {
            t.to_string()
        }
    };
    for c in 0..8 {
        if e.channels >> c & 1 != 0 {
            s += &format!(" CH{}={},{}", c + 1, edge(e.t1[c]), edge(e.t2[c]));
        }
    }
    if e.flags & EV_HAS_PT != 0 {
        s += &format!(" pt={:.4},{:.4}", f64::from(e.pt_x), f64::from(e.pt_y));
    }
    if e.flags & EV_CAL_MODE != 0 {
        s += " mode=cal";
    }
    if e.cal != 0 {
        s += &format!(" cal={:08X} stale={}", e.cal, (e.flags & EV_CAL_STALE != 0) as u8);
    } else {
        s += " cal=-";
    }
    if e.flags & EV_CURTAIN_ON != 0 {
        s += " ir=";
        if e.dips == 0 {
            s.push('-');
        }
        for i in 0..(e.dips as usize).min(4) {
            s += &format!(
                "{}{}/{}",
                if i > 0 { "," } else { "" },
                e.dip[i],
                edge(e.dip_len[i])
            );
        }
    }
    Some(s)
}

/// The log line for any event: `rec` for an acoustic one, a `dip` line for a hole (as the
/// firmware prints it; replay counts them), a `MISSED` line for a missed shot.
pub fn event_line(e: &Event) -> String {
    match e.kind {
        Kind::Shot => rec_line(e).unwrap(),
        Kind::Hole => {
            let len = if e.dip_len[0] == NO_VALUE {
                "-".to_owned()
            } else {
                e.dip_len[0].to_string()
            };
            format!(
                "dip n={} t={}.{:09} len={}",
                e.seq,
                e.t_ns / 1_000_000_000,
                e.t_ns % 1_000_000_000,
                len
            )
        }
        Kind::Missed => format!("MISSED: activity in the dead time while event {} was read", e.n),
    }
}

/// Today's date (UTC), "YYYY-MM-DD".
pub fn utc_date() -> String {
    let days = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() / 86_400)
        .unwrap_or(0) as i64;
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Year, month, day of a day number since 1970-01-01 (Howard Hinnant's civil_from_days).
pub fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

/// Appends lines to the day's file in `dir`. Each `cfg` line is written once per file.
pub struct EventLog {
    dir: PathBuf,
    written: HashSet<(String, u32)>,
}

impl EventLog {
    pub fn new(dir: PathBuf) -> EventLog {
        EventLog {
            dir,
            written: HashSet::new(),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn append(&self, date: &str, line: &str) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.dir.join(format!("{date}.log")))?;
        writeln!(f, "{line}")
    }

    /// A comment line (replay ignores lines that aren't log lines).
    pub fn note(&mut self, text: &str) -> std::io::Result<()> {
        self.append(&utc_date(), &format!("# {text}"))
    }

    /// The sensor's settings as a `cfg` line, if this file doesn't have it yet. Returns its ID.
    pub fn config(&mut self, cal: &Calibration, set: &Settings) -> std::io::Result<u32> {
        let (id, line) = cfg_line(cal, set);
        let date = utc_date();
        if !self.written.contains(&(date.clone(), id)) {
            self.append(&date, &line)?;
            self.written.insert((date, id));
        }
        Ok(id)
    }

    /// Whether today's file has the `cfg` line with this ID.
    #[allow(dead_code)] // the tests; the wizard (5d)
    pub fn has_config(&self, id: u32) -> bool {
        self.written.contains(&(utc_date(), id))
    }

    pub fn event(&mut self, e: &Event) -> std::io::Result<()> {
        self.append(&utc_date(), &event_line(e))
    }
}

#[cfg(test)]
// The examples carry the digits C printed, and one value near pi is a formatting case.
#[allow(clippy::excessive_precision, clippy::approx_constant)]
mod tests {
    use super::*;
    use crate::protocol::{decode_calibration, decode_event, decode_settings, tests as p};

    // From firmware/test/test_proto: the 4b bench log's cfg line and event 21.
    const CFG_LINE: &str = "cfg id=4A87309E CH1=- CH2=- CH3=-105,-105,10.5,0.333330005,0,50 CH4=105,-105,10.5,0.333330005,0,50 \
        CH5=65,105,10.5,0.333330005,0,50 CH6=-105,105,10.5,0.333330005,0,50 CH7=0,-105,10.5,0.333330005,0,50 \
        CH8=105,0,10.5,0.333330005,0,50 lim=1500 drop=1 lost=1 area=105 tog=20000 pmax=10000 ir=45000,100000";
    const REC_LINE: &str = "rec n=21 t=30.9781094 cfg=4A87309E T=18.00 RH=50.0 p=1013.25 meas=--- c=342.739716 \
        CH3=320400,321500 CH4=214100,215000 CH5=5800,6200 CH6=214100,215000 CH7=167100,167900 CH8=0,500 \
        pt=30.0000,30.0000 cal=- ir=-160725/27075";

    #[test]
    fn cfg_line_as_the_firmware() {
        let cal = decode_calibration(&p::example(p::CALIBRATION_HEX)).unwrap();
        let set = decode_settings(&p::example(p::SETTINGS_HEX)).unwrap();
        let (id, line) = cfg_line(&cal, &set);
        assert_eq!(line, CFG_LINE);
        assert_eq!(id, 0x4A87_309E);
        assert_eq!(calibration_id(&cal, &set), 0x746A_AA08); // as the firmware's test
    }

    #[test]
    fn rec_line_as_the_firmware() {
        let e = decode_event(&p::example(p::EVENT_HEX)).unwrap();
        assert_eq!(rec_line(&e).unwrap(), REC_LINE);
        assert_eq!(event_line(&e), REC_LINE);
    }

    #[test]
    fn hole_and_missed_lines() {
        let mut e = decode_event(&p::example(p::EVENT_HEX)).unwrap();
        e.kind = Kind::Hole;
        e.seq = 19;
        e.t_ns = 31_000_123_456;
        e.dip_len[0] = NO_VALUE;
        assert_eq!(event_line(&e), "dip n=19 t=31.000123456 len=-");
        e.kind = Kind::Missed;
        assert_eq!(
            event_line(&e),
            "MISSED: activity in the dead time while event 21 was read"
        );
        assert_eq!(rec_line(&e), None);
    }

    #[test]
    fn writes_each_cfg_once() {
        let dir = std::env::temp_dir().join(format!("opensights-eventlog-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut log = EventLog::new(dir.clone());
        let cal = decode_calibration(&p::example(p::CALIBRATION_HEX)).unwrap();
        let set = decode_settings(&p::example(p::SETTINGS_HEX)).unwrap();
        assert_eq!(log.config(&cal, &set).unwrap(), 0x4A87_309E);
        log.config(&cal, &set).unwrap();
        assert!(log.has_config(0x4A87_309E));
        log.event(&decode_event(&p::example(p::EVENT_HEX)).unwrap())
            .unwrap();
        let text = std::fs::read_to_string(dir.join(format!("{}.log", utc_date()))).unwrap();
        assert_eq!(text, format!("{CFG_LINE}\n{REC_LINE}\n"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn utc_date_shape() {
        let d = utc_date();
        assert_eq!(d.len(), 10);
        assert!(d.starts_with("20"));
    }
}
