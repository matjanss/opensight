//! The calibration wizard and the field check (MEASUREMENT step 5d, PLAN D5), as state
//! machines without UI: they take the sensor's events, Status and Calibration, and say what to
//! send (`Action`). The app's window runs them over BLE, and so does the test client
//! (`examples/probe.rs`), so the logic is tested without clicking.
//!
//! Calibration: check the sensor → calibration mode on → per template point, set the point
//! (Control) and collect its sparks → fit (`calib::fit`) → review → write the record →
//! confirm the sensor's new calibration ID → calibration mode off. Cancel at any time: mode
//! off, nothing written.
//!
//! Field check: a few sparks at the template's check point, in calibration mode (so they
//! aren't scored); the sensor solves them with its calibration, and the wizard compares.

use calib::fit::{FitResult, Spark, calibrate};
use serde::Deserialize;

use crate::protocol::{Calibration, EV_CAL_MODE, EV_HAS_PT, EV_SOLVED, Event, Kind, NO_VALUE, Status};

/// A calibration template (`assets/templates/*.json`): the spark points on the card holder.
#[derive(Clone, Debug, Deserialize)]
pub struct Template {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// mm, card coordinates (x right, y up, as seen by the shooter).
    pub points: Vec<(f32, f32)>,
    pub sparks_per_point: usize,
    pub check_point: (f32, f32),
    pub check_sparks: usize,
    pub check_limit_mm: f64,
}

/// The built-in template: the 4 × 4 grid at ±25 and ±75 mm, a check point at the centre.
pub fn grid_template() -> Template {
    serde_json::from_str(include_str!("../assets/templates/grid-4x4.json")).expect("grid-4x4.json")
}

/// What to send to the sensor.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    CalMode(bool),
    Point(f32, f32),
    NoPoint,
    WriteCalibration(Box<Calibration>),
}

/// Two template points are the same to 0.01 mm (they go as f32 both ways).
fn same_point(a: (f32, f32), b: (f32, f32)) -> bool {
    (a.0 - b.0).abs() < 0.01 && (a.1 - b.1).abs() < 0.01
}

/// A spark for the fit from an event, if it is one: an acoustic event made in calibration
/// mode, with its point, and both edges on every channel in `mics`.
fn spark_of(e: &Event, mics: u8) -> Option<Spark> {
    if e.kind != Kind::Shot || e.flags & EV_CAL_MODE == 0 || e.flags & EV_HAS_PT == 0 {
        return None;
    }
    let edge = |t: i32| (t != NO_VALUE).then_some(t);
    let (t1, t2) = (e.t1.map(edge), e.t2.map(edge));
    let complete = (0..8)
        .filter(|i| mics >> i & 1 != 0)
        .all(|i| t1[i].is_some() && t2[i].is_some());
    complete.then_some(Spark {
        t1,
        t2,
        pt: (e.pt_x, e.pt_y),
        c_ms: e.c,
    })
}

#[derive(Clone, Debug)]
pub enum Phase {
    /// Not started: the template and the sensor's values are shown.
    Ready,
    /// Collecting sparks at template point `point`.
    Collecting {
        point: usize,
    },
    /// The fit, for review.
    Fitted(Box<FitResult>),
    /// The record was sent; waiting for the sensor to report its new calibration ID.
    Writing {
        expected: u32,
    },
    /// Written and confirmed.
    Done {
        id: u32,
    },
    Failed(String),
    Cancelled,
}

/// The calibration wizard.
pub struct Wizard {
    pub template: Template,
    pub phase: Phase,
    /// The sensor's calibration when the wizard started: the fit's start, and h and the
    /// channels without a mic as they are.
    pub nominal: Calibration,
    /// The cfg ID the sparks must carry (the sensor's settings when the wizard started).
    cfg: u32,
    mics: u8,
    /// Per template point, its sparks.
    pub sparks: Vec<Vec<Spark>>,
    /// Sparks that came but didn't count (another point, incomplete, other settings).
    pub ignored: usize,
    /// Test mode (the simulator, whose sparks carry its own grid's points): take each spark at
    /// the point it carries, and count them all towards the total.
    any_point: bool,
    actions: Vec<Action>,
}

impl Wizard {
    /// Checks the sensor: at least 4 mics, and Status and Calibration read after the last
    /// change (the cfg ID). `any_point`: test mode, see the field.
    pub fn new(
        template: Template,
        status: &Status,
        nominal: &Calibration,
        any_point: bool,
    ) -> Result<Wizard, String> {
        let mics = nominal
            .ch
            .iter()
            .enumerate()
            .filter(|(_, c)| c.mic)
            .fold(0u8, |m, (i, _)| m | 1 << i);
        if mics.count_ones() < 4 {
            return Err(format!(
                "The sensor has {} mics set; calibration needs 4 or more.",
                mics.count_ones()
            ));
        }
        if status.mics != mics {
            return Err("The sensor's status and calibration don't agree yet: try again in a moment.".into());
        }
        if template.points.len() < 4 || template.sparks_per_point == 0 {
            return Err("The template needs 4 points or more.".into());
        }
        let n = template.points.len();
        Ok(Wizard {
            template,
            phase: Phase::Ready,
            nominal: nominal.clone(),
            cfg: status.cfg,
            mics,
            sparks: vec![Vec::new(); n],
            ignored: 0,
            any_point,
            actions: Vec::new(),
        })
    }

    /// Calibration mode on, the first point.
    pub fn start(&mut self) {
        self.actions.push(Action::CalMode(true));
        self.go_to(0);
    }

    fn go_to(&mut self, point: usize) {
        let p = self.template.points[point];
        self.actions.push(Action::Point(p.0, p.1));
        self.phase = Phase::Collecting { point };
    }

    /// The sparks counted so far, and how many the template asks for.
    pub fn progress(&self) -> (usize, usize) {
        let have = self.sparks.iter().map(|s| s.len()).sum();
        (have, self.template.points.len() * self.template.sparks_per_point)
    }

    /// Whether the current point has its sparks (test mode: whether all points have).
    pub fn point_done(&self) -> bool {
        match self.phase {
            Phase::Collecting { point } if self.any_point => {
                let (have, want) = self.progress();
                let _ = point;
                have >= want
            }
            Phase::Collecting { point } => self.sparks[point].len() >= self.template.sparks_per_point,
            _ => false,
        }
    }

    /// An event from the sensor. Returns true if it counted as a spark.
    pub fn on_event(&mut self, e: &Event) -> bool {
        let Phase::Collecting { point } = self.phase else {
            return false;
        };
        if e.kind != Kind::Shot || e.flags & EV_CAL_MODE == 0 {
            return false; // not a spark (a hole, a missed shot, made outside calibration mode)
        }
        let Some(s) = spark_of(e, self.mics).filter(|_| e.cfg == self.cfg) else {
            self.ignored += 1;
            return false;
        };
        let at = if self.any_point {
            self.template.points.iter().position(|&p| same_point(p, s.pt))
        } else {
            same_point(self.template.points[point], s.pt).then_some(point)
        };
        match at {
            Some(i) => {
                self.sparks[i].push(s);
                true
            }
            None => {
                self.ignored += 1;
                false
            }
        }
    }

    /// A Status from the sensor: if its settings changed while collecting, the sparks so far
    /// were made with others: the wizard stops.
    pub fn on_status(&mut self, st: &Status) {
        if matches!(self.phase, Phase::Collecting { .. }) && st.cfg != self.cfg {
            self.fail("The sensor's settings changed during the calibration: start again.");
        }
        if let Phase::Writing { expected } = self.phase
            && st.cal == expected
        {
            self.phase = Phase::Done { id: expected };
            self.actions.push(Action::NoPoint);
            self.actions.push(Action::CalMode(false));
        }
    }

    /// The next point, or the fit after the last one.
    pub fn next_point(&mut self) {
        let Phase::Collecting { point } = self.phase else {
            return;
        };
        if self.any_point || point + 1 >= self.template.points.len() {
            self.fit();
        } else {
            self.go_to(point + 1);
        }
    }

    /// The current point's sparks again.
    pub fn redo_point(&mut self) {
        if let Phase::Collecting { point } = self.phase {
            self.sparks[point].clear();
        }
    }

    /// Fits the sparks of every point.
    pub fn fit(&mut self) {
        let all: Vec<Spark> = self.sparks.iter().flatten().cloned().collect();
        self.phase = match calibrate(&self.nominal.ch, &all) {
            Ok(r) => Phase::Fitted(Box::new(r)),
            Err(e) => Phase::Failed(format!("The fit failed: {e}.")),
        };
    }

    /// Writes the fitted record (date yyyymmdd); confirmed by the sensor's next Status.
    pub fn write(&mut self, date: u32) {
        let Phase::Fitted(r) = &self.phase else { return };
        let points = self.sparks.iter().filter(|s| !s.is_empty()).count() as u16;
        let record = Calibration {
            date,
            res_ns: r.rms_ns as f32,
            points,
            sparks: r.sparks as u16,
            ch: r.record,
        };
        let expected = r.id;
        self.actions.push(Action::WriteCalibration(Box::new(record)));
        self.phase = Phase::Writing { expected };
    }

    /// Stops: calibration mode off, nothing written (a write already sent stays).
    pub fn cancel(&mut self) {
        if !matches!(self.phase, Phase::Done { .. } | Phase::Cancelled) {
            self.actions.push(Action::NoPoint);
            self.actions.push(Action::CalMode(false));
            self.phase = Phase::Cancelled;
        }
    }

    fn fail(&mut self, why: &str) {
        self.actions.push(Action::NoPoint);
        self.actions.push(Action::CalMode(false));
        self.phase = Phase::Failed(why.into());
    }

    /// What to send to the sensor now, in order.
    pub fn take_actions(&mut self) -> Vec<Action> {
        std::mem::take(&mut self.actions)
    }

    /// The fit as text (the report saved next to the event logs).
    pub fn report(&self, sensor: &str) -> Option<String> {
        let Phase::Fitted(r) = &self.phase else {
            return None;
        };
        Some(report_text(&self.template, r, sensor, self.ignored))
    }
}

/// The fit as a text report, like `calibrate.py`'s.
pub fn report_text(t: &Template, r: &FitResult, sensor: &str, ignored: usize) -> String {
    let mut s = format!(
        "OpenSights calibration of {sensor}, template \"{}\"\n\
         {} sparks at {} points, {} left out as outliers (beyond {:.0} ns), {ignored} ignored\n\
         rms residual {:.1} ns after the fit\n\n\
         ch      x mm (±1 sd)        y mm (±1 sd)       k (±1 sd)        offset ns (±1 sd)  sigma ns  change\n",
        t.name,
        r.sparks,
        r.points.len(),
        r.outliers,
        r.limit_ns,
        r.rms_ns
    );
    for c in &r.channels {
        s += &format!(
            "CH{}  {:9.4} ({:.4})  {:9.4} ({:.4})  {:7.4} ({:.4})  {:8.1} ({:.1})  {:7.1}   x {:+.3} y {:+.3} mm, k {:+.4}, offset {:+.1} ns\n",
            c.channel + 1,
            c.x,
            c.sd_x,
            c.y,
            c.sd_y,
            c.k,
            c.sd_k,
            c.offset_ns,
            c.sd_offset,
            c.sigma_ns,
            c.x - f64::from(c.before.x),
            c.y - f64::from(c.before.y),
            c.k - f64::from(c.before.k),
            c.offset_ns - f64::from(c.before.offset_ns)
        );
    }
    s += "\npoint (mm)          before: mean, worst (mm)   after: mean, worst (mm)\n";
    for p in &r.points {
        s += &format!(
            "({:7.2}, {:7.2})   {:.3}, {:.3}                 {:.3}, {:.3}\n",
            p.x, p.y, p.before_mean, p.before_worst, p.after_mean, p.after_worst
        );
    }
    s += &format!(
        "all points:         rms {:.3}                   rms {:.3}\n\ncalibration ID {:08X}\n",
        r.before_rms_mm, r.after_rms_mm, r.id
    );
    s
}

/// The field check's result.
#[derive(Clone, Debug)]
pub struct CheckResult {
    pub sparks: usize,
    pub mean_mm: f64,
    pub worst_mm: f64,
    pub pass: bool,
}

/// The field check: sparks at the template's check point, solved by the sensor.
pub struct FieldCheck {
    pub point: (f32, f32),
    pub needed: usize,
    pub limit_mm: f64,
    /// Each spark's distance from the point, mm.
    pub errors: Vec<f64>,
    pub ignored: usize,
    actions: Vec<Action>,
    pub done: bool,
}

impl FieldCheck {
    pub fn new(t: &Template) -> FieldCheck {
        FieldCheck {
            point: t.check_point,
            needed: t.check_sparks.max(1),
            limit_mm: t.check_limit_mm,
            errors: Vec::new(),
            ignored: 0,
            actions: vec![
                Action::CalMode(true),
                Action::Point(t.check_point.0, t.check_point.1),
            ],
            done: false,
        }
    }

    /// An event: a solved spark at the check point counts. Returns true if it counted.
    pub fn on_event(&mut self, e: &Event) -> bool {
        if self.done || e.kind != Kind::Shot || e.flags & EV_CAL_MODE == 0 {
            return false;
        }
        if e.flags & EV_HAS_PT == 0 || e.flags & EV_SOLVED == 0 || !same_point(self.point, (e.pt_x, e.pt_y)) {
            self.ignored += 1;
            return false;
        }
        let (x, y) = (f64::from(e.x) / 100.0, f64::from(e.y) / 100.0);
        self.errors
            .push((x - f64::from(self.point.0)).hypot(y - f64::from(self.point.1)));
        if self.errors.len() >= self.needed {
            self.done = true;
            self.actions.push(Action::NoPoint);
            self.actions.push(Action::CalMode(false));
        }
        true
    }

    pub fn result(&self) -> Option<CheckResult> {
        if self.errors.is_empty() {
            return None;
        }
        let mean = self.errors.iter().sum::<f64>() / self.errors.len() as f64;
        let worst = self.errors.iter().cloned().fold(0.0, f64::max);
        Some(CheckResult {
            sparks: self.errors.len(),
            mean_mm: mean,
            worst_mm: worst,
            pass: mean <= self.limit_mm,
        })
    }

    /// Stops: calibration mode off.
    pub fn cancel(&mut self) {
        if !self.done {
            self.done = true;
            self.actions.push(Action::NoPoint);
            self.actions.push(Action::CalMode(false));
        }
    }

    pub fn take_actions(&mut self) -> Vec<Action> {
        std::mem::take(&mut self.actions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{Channel, decode_event, tests as p};

    /// The self-test log's sparks (calib/tests/data, from calibrate.py) as v3 events.
    fn selftest_events() -> (Calibration, Status, Vec<Event>) {
        let text = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/calib/tests/data/selftest.log"
        ))
        .unwrap();
        let (cfg, sparks) = calib::logline::sparks_from_log(&text, false).unwrap();
        let mut template = decode_event(&p::example(p::EVENT_HEX)).unwrap();
        template.flags = EV_CAL_MODE | EV_HAS_PT | EV_SOLVED;
        template.cfg = cfg.id;
        let events = sparks
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let mut e = template.clone();
                e.seq = i as u32 + 1;
                e.t1 = s.t1.map(|t| t.unwrap_or(NO_VALUE));
                e.t2 = s.t2.map(|t| t.unwrap_or(NO_VALUE));
                e.pt_x = s.pt.0;
                e.pt_y = s.pt.1;
                e.c = s.c_ms;
                e.channels = 0xFC;
                e
            })
            .collect();
        let mics = cfg
            .channels
            .iter()
            .enumerate()
            .filter(|(_, c)| c.mic)
            .fold(0u8, |m, (i, _)| m | 1 << i);
        let status = Status {
            boot: 1,
            cfg: cfg.id,
            cal: 0,
            flags: 0,
            measured: 0,
            mics,
            pt_x: 0.0,
            pt_y: 0.0,
            t_c: 18.0,
            rh: 50.0,
            hpa: 1013.25,
            c: 342.0,
            events: 0,
            missed: 0,
            shots: 0,
        };
        let cal = Calibration {
            date: 0,
            res_ns: 0.0,
            points: 0,
            sparks: 0,
            ch: cfg.channels,
        };
        (cal, status, events)
    }

    #[test]
    fn template_loads() {
        let t = grid_template();
        assert_eq!(t.points.len(), 16);
        assert_eq!(
            (t.sparks_per_point, t.check_point, t.check_sparks),
            (10, (0.0, 0.0), 5)
        );
        assert_eq!(t.points[0], (-75.0, -75.0));
        assert_eq!(t.points[15], (75.0, 75.0));
    }

    /// The whole run, point by point, as the UI drives it: the same result as calib's fit.
    #[test]
    fn calibration_point_by_point() {
        let (cal, mut status, events) = selftest_events();
        let mut w = Wizard::new(grid_template(), &status, &cal, false).unwrap();
        w.start();
        assert_eq!(
            w.take_actions(),
            vec![Action::CalMode(true), Action::Point(-75.0, -75.0)]
        );
        let mut sent = 0;
        while let Phase::Collecting { point } = w.phase {
            // The sensor's sparks come in the template's order; a spark at another point and
            // one with another cfg don't count.
            let mut stray = events[0].clone();
            stray.pt_x = 999.0;
            assert!(!w.on_event(&stray));
            let mut other_cfg = events[0].clone();
            other_cfg.cfg ^= 1;
            assert!(!w.on_event(&other_cfg));
            let here = w.template.points[point];
            for e in events.iter().filter(|e| same_point((e.pt_x, e.pt_y), here)) {
                assert!(w.on_event(e));
                sent += 1;
            }
            assert!(w.point_done());
            w.next_point();
            if let Phase::Collecting { point } = w.phase {
                let p = w.template.points[point];
                assert_eq!(w.take_actions(), vec![Action::Point(p.0, p.1)]);
            }
        }
        assert_eq!(sent, 160);
        assert_eq!(w.ignored, 32);
        let Phase::Fitted(r) = &w.phase else {
            panic!("{:?}", w.phase)
        };
        assert!(r.after_rms_mm < 0.02 && r.before_rms_mm > 0.3);
        let id = r.id;
        let report = w.report("test").unwrap();
        assert!(report.contains(&format!("calibration ID {id:08X}")));
        w.write(20261123);
        let acts = w.take_actions();
        let Action::WriteCalibration(rec) = &acts[0] else {
            panic!()
        };
        assert_eq!((rec.date, rec.points, rec.sparks), (20261123, 16, 160));
        assert_eq!(calib::text::calibration_id(&rec.ch), id);
        // Confirmed when the sensor reports the new calibration ID.
        status.cal = id;
        w.on_status(&status);
        assert!(matches!(w.phase, Phase::Done { id: i } if i == id));
        assert_eq!(w.take_actions(), vec![Action::NoPoint, Action::CalMode(false)]);
    }

    #[test]
    fn redo_cancel_and_changed_settings() {
        let (cal, mut status, events) = selftest_events();
        let mut w = Wizard::new(grid_template(), &status, &cal, false).unwrap();
        w.start();
        w.take_actions();
        assert!(w.on_event(&events[0]));
        w.redo_point();
        assert_eq!(w.sparks[0].len(), 0);
        w.cancel();
        assert!(matches!(w.phase, Phase::Cancelled));
        assert_eq!(w.take_actions(), vec![Action::NoPoint, Action::CalMode(false)]);
        // The settings change mid-way: stop.
        let mut w = Wizard::new(grid_template(), &status, &cal, false).unwrap();
        w.start();
        status.cfg ^= 1;
        w.on_status(&status);
        assert!(matches!(w.phase, Phase::Failed(_)));
        // Too few mics.
        let mut few = cal.clone();
        for c in &mut few.ch[..6] {
            *c = Channel { mic: false, ..*c };
        }
        assert!(Wizard::new(grid_template(), &status, &few, false).is_err());
    }

    #[test]
    fn any_point_mode_takes_each_spark_at_its_point() {
        let (cal, status, events) = selftest_events();
        let mut w = Wizard::new(grid_template(), &status, &cal, true).unwrap();
        w.start();
        for e in &events {
            assert!(w.on_event(e));
        }
        assert!(w.point_done());
        w.next_point();
        assert!(matches!(w.phase, Phase::Fitted(_)));
    }

    #[test]
    fn field_check() {
        let (_, _, events) = selftest_events();
        let mut c = FieldCheck::new(&grid_template());
        assert_eq!(
            c.take_actions(),
            vec![Action::CalMode(true), Action::Point(0.0, 0.0)]
        );
        // The sensor's solved positions: 0.2 mm off, then one 0.6 mm off.
        let mut e = events[0].clone();
        e.pt_x = 0.0;
        e.pt_y = 0.0;
        for (i, x) in [20, -20, 0, 20, 60].iter().enumerate() {
            e.x = *x;
            e.y = 0;
            assert!(c.on_event(&e), "{i}");
        }
        assert!(c.done);
        let r = c.result().unwrap();
        assert_eq!(r.sparks, 5);
        assert!((r.mean_mm - 0.24).abs() < 1e-9 && (r.worst_mm - 0.6).abs() < 1e-9 && r.pass);
        assert_eq!(c.take_actions(), vec![Action::NoPoint, Action::CalMode(false)]);
        assert!(!c.on_event(&e)); // done
    }
}
