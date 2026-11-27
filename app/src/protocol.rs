//! BLE protocol constants and value encoding. Mirrors `protocol/PROTOCOL.md` and
//! `firmware/src/protocol.h`: change all three together.

use std::collections::HashMap;
use uuid::{Uuid, uuid};

pub const PROTOCOL_VERSION: u8 = 3;

pub const SERVICE_UUID: Uuid = uuid!("5631f100-5314-4f02-b637-0654ff6d409d");
pub const HEARTBEAT_UUID: Uuid = uuid!("5631f101-5314-4f02-b637-0654ff6d409d");
// 5631f102 was the LED (version 1); not reused.
pub const NAME_UUID: Uuid = uuid!("5631f103-5314-4f02-b637-0654ff6d409d");
pub const VERSION_UUID: Uuid = uuid!("5631f104-5314-4f02-b637-0654ff6d409d");
pub const PIN_UUID: Uuid = uuid!("5631f105-5314-4f02-b637-0654ff6d409d");
pub const SHOT_UUID: Uuid = uuid!("5631f106-5314-4f02-b637-0654ff6d409d");
pub const EVENT_UUID: Uuid = uuid!("5631f107-5314-4f02-b637-0654ff6d409d");
pub const STATUS_UUID: Uuid = uuid!("5631f108-5314-4f02-b637-0654ff6d409d");
pub const CONTROL_UUID: Uuid = uuid!("5631f109-5314-4f02-b637-0654ff6d409d");
pub const CALIBRATION_UUID: Uuid = uuid!("5631f10a-5314-4f02-b637-0654ff6d409d");
pub const SETTINGS_UUID: Uuid = uuid!("5631f10b-5314-4f02-b637-0654ff6d409d");

pub const MANUFACTURER_ID: u16 = 0xFFFF;
pub const ADV_FLAG_PAIRING: u8 = 0x01;

/// The pairing PIN of a new (or reset) sensor, shown as "000000".
pub const DEFAULT_PASSKEY: u32 = 0;
pub const PASSKEY_MAX: u32 = 999_999;
pub const PIN_DIGITS: usize = 6;
pub const NAME_MAX_LEN: usize = 20;

/// Heartbeat: `u32` little-endian, milliseconds since the sensor booted.
pub fn decode_heartbeat(bytes: &[u8]) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.try_into().ok()?))
}

fn is_name_char(c: char) -> bool {
    (' '..='~').contains(&c)
}

/// Name as sent by the sensor: 1–20 bytes of printable ASCII, taken as-is (no trimming).
pub fn decode_name(bytes: &[u8]) -> Option<String> {
    let s = std::str::from_utf8(bytes).ok()?;
    let ok = !s.is_empty() && s.len() <= NAME_MAX_LEN && s.chars().all(is_name_char);
    ok.then(|| s.to_owned())
}

/// Trims the user's input and checks it against the protocol rules:
/// 1–20 bytes of printable ASCII.
pub fn validate_name(input: &str) -> Result<String, String> {
    let name = input.trim();
    if name.is_empty() {
        return Err("The name can't be empty.".into());
    }
    if let Some(c) = name.chars().find(|&c| !is_name_char(c)) {
        return Err(format!(
            "“{c}” isn't allowed. Use letters, digits, spaces and basic punctuation."
        ));
    }
    if name.len() > NAME_MAX_LEN {
        return Err(format!("At most {NAME_MAX_LEN} characters ({} now).", name.len()));
    }
    Ok(name.to_owned())
}

/// A PIN as the user sees and types it: always 6 digits, with leading zeros.
pub fn format_pin(pin: u32) -> String {
    format!("{pin:0PIN_DIGITS$}")
}

/// The user's PIN input: exactly 6 digits (spaces around it are ignored).
pub fn validate_pin(input: &str) -> Result<u32, String> {
    let pin = input.trim();
    if pin.len() != PIN_DIGITS || !pin.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!("The PIN is exactly {PIN_DIGITS} digits."));
    }
    pin.parse().map_err(|_| "Not a number.".to_owned())
}

/// PIN characteristic: `u32` little-endian, 0–999 999.
pub fn encode_pin(pin: u32) -> [u8; 4] {
    debug_assert!(pin <= PASSKEY_MAX);
    pin.to_le_bytes()
}

/// Little-endian reader over a message of a known length.
struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn new(b: &[u8]) -> Reader<'_> {
        Reader { b, at: 0 }
    }
    fn take<const N: usize>(&mut self) -> [u8; N] {
        let out = self.b[self.at..self.at + N].try_into().unwrap();
        self.at += N;
        out
    }
    fn u8(&mut self) -> u8 {
        self.take::<1>()[0]
    }
    fn i8(&mut self) -> i8 {
        self.u8() as i8
    }
    fn u16(&mut self) -> u16 {
        u16::from_le_bytes(self.take())
    }
    fn u32(&mut self) -> u32 {
        u32::from_le_bytes(self.take())
    }
    fn i32(&mut self) -> i32 {
        i32::from_le_bytes(self.take())
    }
    fn u64(&mut self) -> u64 {
        u64::from_le_bytes(self.take())
    }
    fn f32(&mut self) -> f32 {
        f32::from_le_bytes(self.take())
    }
    fn skip(&mut self, n: usize) {
        self.at += n;
    }
}

/// What a hit is (byte 30 of the shot, byte 0 of the event).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Measured: it has a position.
    Shot,
    /// A curtain dip with no sound: a shot through an existing hole. No position.
    Hole,
    /// Sound in the dead time after a shot: a second shot, not measured. No position.
    Missed,
}

impl Kind {
    fn from_u8(v: u8) -> Option<Kind> {
        match v {
            0 => Some(Kind::Shot),
            1 => Some(Kind::Hole),
            2 => Some(Kind::Missed),
            _ => None,
        }
    }
}

/// Shot flags (byte 31).
pub const SHOT_DROPPED: u8 = 0x01;
pub const SHOT_AIR_ASSUMED: u8 = 0x02;
pub const SHOT_CAL_STALE: u8 = 0x04;
pub const SHOT_CURTAIN: u8 = 0x08;
pub const SIGMA_UNKNOWN: u16 = 0xFFFF;

/// One hit as the sensor reports it (version 3, 32 bytes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shot {
    /// Numbered from 1 for every boot of the sensor; every kind counts.
    pub seq: u32,
    /// Random per boot of the sensor, never 0: tells which boot `seq` belongs to.
    pub boot: u32,
    /// The sensor's `millis()` at the impact.
    pub time_ms: u32,
    /// Hundredths of a mm from the centre; x to the right, y up, as seen by the shooter.
    /// 0 for a hole or a missed shot.
    pub x: i32,
    pub y: i32,
    /// The event message it came from (its `seq`); 0 for none (a test shot).
    pub event: u32,
    /// The calibration ID it was measured with; 0 for none.
    pub cal: u32,
    /// 1σ along the error ellipse's long axis, 0.01 mm; `SIGMA_UNKNOWN` if not known.
    pub sigma: u16,
    pub kind: Kind,
    pub flags: u8,
}

impl Shot {
    pub fn x_mm(&self) -> f32 {
        self.x as f32 / 100.0
    }
    pub fn y_mm(&self) -> f32 {
        self.y as f32 / 100.0
    }
    /// Whether it has a position to score.
    pub fn scored(&self) -> bool {
        self.kind == Kind::Shot
    }
    pub fn sigma_mm(&self) -> Option<f32> {
        (self.sigma != SIGMA_UNKNOWN).then(|| self.sigma as f32 / 100.0)
    }
}

pub const SHOT_LEN: usize = 32;

/// Shot indication (PROTOCOL.md "Shot").
pub fn decode_shot(bytes: &[u8]) -> Option<Shot> {
    if bytes.len() != SHOT_LEN {
        return None;
    }
    let mut r = Reader::new(bytes);
    Some(Shot {
        seq: r.u32(),
        boot: r.u32(),
        time_ms: r.u32(),
        x: r.i32(),
        y: r.i32(),
        event: r.u32(),
        cal: r.u32(),
        sigma: r.u16(),
        kind: Kind::from_u8(r.u8())?,
        flags: r.u8(),
    })
}

/// Shot request: "send your buffered shots from `next_seq` of boot `boot` on". With a
/// boot the sensor doesn't have (0 when the app has no shots yet), it sends all it has.
pub fn encode_shot_request(boot: u32, next_seq: u32) -> [u8; 8] {
    let mut out = [0; 8];
    out[..4].copy_from_slice(&boot.to_le_bytes());
    out[4..].copy_from_slice(&next_seq.to_le_bytes());
    out
}

/// First byte of a clear request.
pub const SHOT_CLEAR: u8 = 0x01;

/// Clear request: "forget your buffered shots of boot `boot` up to and including `last_seq`",
/// so a later connect doesn't send them again. Shots after `last_seq` are kept.
pub fn encode_shot_clear(boot: u32, last_seq: u32) -> [u8; 9] {
    let mut out = [0; 9];
    out[0] = SHOT_CLEAR;
    out[1..5].copy_from_slice(&boot.to_le_bytes());
    out[5..].copy_from_slice(&last_seq.to_le_bytes());
    out
}

/// Event request: as the shot request, for the event stream.
pub fn encode_event_request(boot: u32, next_seq: u32) -> [u8; 8] {
    encode_shot_request(boot, next_seq)
}

/// An event's verdict (MODEL.md "Validity"); the event's byte 1.
pub const VERDICTS: [&str; 8] = [
    "ok", "few", "together", "nofit", "residual", "lost", "outside", "curtain",
];
#[allow(dead_code)] // mirrored from PROTOCOL.md; not read by the app yet
pub const VERDICT_NONE: u8 = 0xFF;
pub const NO_VALUE: i32 = i32::MIN;

/// Event flags (byte 36).
pub const EV_CAL_MODE: u8 = 0x01;
pub const EV_CAL_STALE: u8 = 0x02;
pub const EV_HAS_PT: u8 = 0x04;
pub const EV_CURTAIN_ON: u8 = 0x08;
#[allow(dead_code)] // mirrored from PROTOCOL.md; not read by the app yet
pub const EV_SOLVED: u8 = 0x10;
#[allow(dead_code)] // mirrored from PROTOCOL.md; not read by the app yet
pub const EV_SENT: u8 = 0x20;
/// Measured bits (events and Status).
pub const MEAS_T: u8 = 0x01;
pub const MEAS_RH: u8 = 0x02;
pub const MEAS_P: u8 = 0x04;

/// One event as the sensor captured and judged it (PROTOCOL.md "Event", 180 bytes):
/// everything the firmware's `rec` line has.
#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub kind: Kind,
    pub verdict: u8,
    pub acoustic: u8,
    /// 1 a dip in the window, 0 none, -1 not judged.
    pub curtain: i8,
    pub boot: u32,
    /// The event stream's number (requests use it; a shot's `event` points at it).
    pub seq: u32,
    /// The capture's event number (the `rec` line's `n=`).
    pub n: u32,
    pub time_ms: u32,
    /// The first edge, ns since the capture started.
    pub t_ns: u64,
    pub cfg: u32,
    pub cal: u32,
    pub flags: u8,
    pub channels: u8,
    pub fitted: u8,
    pub dropped: i8,
    pub x: i32,
    pub y: i32,
    pub rms_ns: f32,
    pub sigma_mm: f32,
    pub t_c: f32,
    pub rh: f32,
    pub hpa: f32,
    pub c: f32,
    pub measured: u8,
    pub dips: u8,
    pub t1: [i32; 8],
    pub t2: [i32; 8],
    pub dip: [i32; 4],
    pub dip_len: [i32; 4],
    pub pt_x: f32,
    pub pt_y: f32,
}

impl Event {
    /// The verdict's name, or None for a hole or missed shot.
    pub fn verdict_name(&self) -> Option<&'static str> {
        VERDICTS.get(self.verdict as usize).copied()
    }
}

pub const EVENT_LEN: usize = 180;

pub fn decode_event(bytes: &[u8]) -> Option<Event> {
    if bytes.len() != EVENT_LEN {
        return None;
    }
    let mut r = Reader::new(bytes);
    let kind = Kind::from_u8(r.u8())?;
    let mut e = Event {
        kind,
        verdict: r.u8(),
        acoustic: r.u8(),
        curtain: r.i8(),
        boot: r.u32(),
        seq: r.u32(),
        n: r.u32(),
        time_ms: r.u32(),
        t_ns: r.u64(),
        cfg: r.u32(),
        cal: r.u32(),
        flags: r.u8(),
        channels: r.u8(),
        fitted: r.u8(),
        dropped: r.i8(),
        x: r.i32(),
        y: r.i32(),
        rms_ns: r.f32(),
        sigma_mm: r.f32(),
        t_c: r.f32(),
        rh: r.f32(),
        hpa: r.f32(),
        c: r.f32(),
        measured: r.u8(),
        dips: r.u8(),
        t1: [0; 8],
        t2: [0; 8],
        dip: [0; 4],
        dip_len: [0; 4],
        pt_x: 0.0,
        pt_y: 0.0,
    };
    r.skip(2);
    for v in e.t1.iter_mut().chain(e.t2.iter_mut()) {
        *v = r.i32();
    }
    for v in e.dip.iter_mut().chain(e.dip_len.iter_mut()) {
        *v = r.i32();
    }
    e.pt_x = r.f32();
    e.pt_y = r.f32();
    (e.dips <= 4).then_some(e)
}

/// Status flags (byte 12).
pub const ST_CAL_MODE: u8 = 0x01;
pub const ST_CAL_STALE: u8 = 0x02;
pub const ST_POINT: u8 = 0x04;
#[allow(dead_code)] // mirrored from PROTOCOL.md; not read by the app yet
pub const ST_CURTAIN_ON: u8 = 0x08;
pub const ST_MTU_SMALL: u8 = 0x10;
#[allow(dead_code)] // mirrored from PROTOCOL.md; not read by the app yet
pub const ST_AIR_SENSOR: u8 = 0x20;

/// The sensor's state (PROTOCOL.md "Status", 52 bytes).
#[derive(Clone, Debug, PartialEq)]
pub struct Status {
    pub boot: u32,
    pub cfg: u32,
    pub cal: u32,
    pub flags: u8,
    pub measured: u8,
    pub mics: u8,
    pub pt_x: f32,
    pub pt_y: f32,
    pub t_c: f32,
    pub rh: f32,
    pub hpa: f32,
    pub c: f32,
    pub events: u32,
    pub missed: u32,
    pub shots: u32,
}

pub const STATUS_LEN: usize = 52;

pub fn decode_status(bytes: &[u8]) -> Option<Status> {
    if bytes.len() != STATUS_LEN {
        return None;
    }
    let mut r = Reader::new(bytes);
    let (boot, cfg, cal) = (r.u32(), r.u32(), r.u32());
    let (flags, measured, mics) = (r.u8(), r.u8(), r.u8());
    r.skip(1);
    Some(Status {
        boot,
        cfg,
        cal,
        flags,
        measured,
        mics,
        pt_x: r.f32(),
        pt_y: r.f32(),
        t_c: r.f32(),
        rh: r.f32(),
        hpa: r.f32(),
        c: r.f32(),
        events: r.u32(),
        missed: r.u32(),
        shots: r.u32(),
    })
}

/// Byte 0 of Settings and Calibration.
pub const LAYOUT: u8 = 1;

/// The solver's policy and the capture settings (PROTOCOL.md "Settings", 36 bytes).
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub drop_one: bool,
    /// Mic channels that may be left out of the fit; -1 off.
    pub max_lost: i8,
    pub mask: u8,
    pub res_limit_ns: f32,
    /// 0 off, for these four.
    pub area_mm: f32,
    pub together_ns: f32,
    pub pulse_max_ns: f32,
    /// 0, 0: the curtain's gate off.
    pub curtain_lo_ns: f32,
    pub curtain_hi_ns: f32,
    pub span_mm: u16,
    pub deadtime_ms: u16,
}

impl Settings {
    /// The curtain's window (low, high ns), or None when its gate is off (0, 0).
    pub fn curtain(&self) -> Option<(f32, f32)> {
        (self.curtain_hi_ns > 0.0).then_some((self.curtain_lo_ns, self.curtain_hi_ns))
    }

    /// These settings with only the curtain's window changed: None switches its gate off
    /// (and the sensor's emitter, IR_EN). Settings are written whole, so the rest stays as read.
    pub fn with_curtain(&self, window: Option<(f32, f32)>) -> Settings {
        let (lo, hi) = window.unwrap_or((0.0, 0.0));
        Settings {
            curtain_lo_ns: lo,
            curtain_hi_ns: hi,
            ..self.clone()
        }
    }
}

/// What is out of range in `s` (PROTOCOL.md "Settings", the firmware's `settingsValid`), or
/// None. The mic span's largest is the sensor's own (its capture buffer): not checked here,
/// the sensor drops a write over it.
pub fn settings_problem(s: &Settings) -> Option<&'static str> {
    let curtain_off = s.curtain_lo_ns == 0.0 && s.curtain_hi_ns == 0.0;
    if !(s.res_limit_ns > 0.0 && s.res_limit_ns <= 100_000.0) {
        Some("The residual limit must be over 0 and at most 100 000 ns.")
    } else if !(-1..=4).contains(&s.max_lost) {
        Some("Mics that may be lost: off, or 0 to 4.")
    } else if !(s.area_mm == 0.0 || (10.0..=1000.0).contains(&s.area_mm)) {
        Some("The area must be off, or ±10 to ±1000 mm.")
    } else if !(s.together_ns == 0.0 || (s.together_ns > 0.0 && s.together_ns <= 100_000.0)) {
        Some("The together limit must be off, or at most 100 µs.")
    } else if !(s.pulse_max_ns == 0.0 || (s.pulse_max_ns > 0.0 && s.pulse_max_ns <= 100_000.0)) {
        Some("The longest pulse must be off, or at most 100 µs.")
    } else if !(curtain_off
        || (s.curtain_lo_ns >= 0.0 && s.curtain_hi_ns > s.curtain_lo_ns && s.curtain_hi_ns <= 2_000_000.0))
    {
        Some("The curtain's window must end after it starts, and by 2000 µs.")
    } else if s.span_mm < 50 {
        Some("The mic span must be at least 50 mm.")
    } else if !(1..=1000).contains(&s.deadtime_ms) {
        Some("The dead time must be 1 to 1000 ms.")
    } else {
        None
    }
}

pub const SETTINGS_LEN: usize = 36;

pub fn decode_settings(bytes: &[u8]) -> Option<Settings> {
    if bytes.len() != SETTINGS_LEN || bytes[0] != LAYOUT || bytes[1] > 1 {
        return None;
    }
    let mut r = Reader::new(&bytes[1..]);
    Some(Settings {
        drop_one: r.u8() == 1,
        max_lost: r.i8(),
        mask: r.u8(),
        res_limit_ns: r.f32(),
        area_mm: r.f32(),
        together_ns: r.f32(),
        pulse_max_ns: r.f32(),
        curtain_lo_ns: r.f32(),
        curtain_hi_ns: r.f32(),
        span_mm: r.u16(),
        deadtime_ms: r.u16(),
    })
}

#[allow(dead_code)] // the calibration wizard (step 5d)
pub fn encode_settings(s: &Settings) -> [u8; SETTINGS_LEN] {
    let mut out = Vec::with_capacity(SETTINGS_LEN);
    out.extend([LAYOUT, s.drop_one as u8, s.max_lost as u8, s.mask]);
    for v in [
        s.res_limit_ns,
        s.area_mm,
        s.together_ns,
        s.pulse_max_ns,
        s.curtain_lo_ns,
        s.curtain_hi_ns,
    ] {
        out.extend(v.to_le_bytes());
    }
    out.extend(s.span_mm.to_le_bytes());
    out.extend(s.deadtime_ms.to_le_bytes());
    out.extend([0; 4]);
    out.try_into().unwrap()
}

/// One channel of the calibration: its mic, and its arrival-time calibration (the
/// calibration crate's, so the fit's results go straight into a Calibration write).
pub use calib::Channel;

/// The calibration record (PROTOCOL.md "Calibration", 213 bytes).
#[derive(Clone, Debug, PartialEq)]
pub struct Calibration {
    /// yyyymmdd; 0 if there is no record (the values are the defaults or set by hand).
    pub date: u32,
    pub res_ns: f32,
    pub points: u16,
    pub sparks: u16,
    pub ch: [Channel; 8],
}

pub const CALIBRATION_LEN: usize = 213;

pub fn decode_calibration(bytes: &[u8]) -> Option<Calibration> {
    if bytes.len() != CALIBRATION_LEN || bytes[0] != LAYOUT {
        return None;
    }
    let mut r = Reader::new(&bytes[1..]);
    let (date, res_ns, points, sparks) = (r.u32(), r.f32(), r.u16(), r.u16());
    let mut ch = [Channel {
        mic: false,
        x: 0.0,
        y: 0.0,
        h: 0.0,
        k: 0.0,
        offset_ns: 0.0,
        sigma_ns: 0.0,
    }; 8];
    for c in ch.iter_mut() {
        let mic = r.u8();
        if mic > 1 {
            return None;
        }
        *c = Channel {
            mic: mic == 1,
            x: r.f32(),
            y: r.f32(),
            h: r.f32(),
            k: r.f32(),
            offset_ns: r.f32(),
            sigma_ns: r.f32(),
        };
    }
    Some(Calibration {
        date,
        res_ns,
        points,
        sparks,
        ch,
    })
}

pub fn encode_calibration(c: &Calibration) -> [u8; CALIBRATION_LEN] {
    let mut out = Vec::with_capacity(CALIBRATION_LEN);
    out.push(LAYOUT);
    out.extend(c.date.to_le_bytes());
    out.extend(c.res_ns.to_le_bytes());
    out.extend(c.points.to_le_bytes());
    out.extend(c.sparks.to_le_bytes());
    for ch in &c.ch {
        out.push(ch.mic as u8);
        for v in [ch.x, ch.y, ch.h, ch.k, ch.offset_ns, ch.sigma_ns] {
            out.extend(v.to_le_bytes());
        }
    }
    out.try_into().unwrap()
}

/// Control commands (PROTOCOL.md "Control").
pub fn encode_cal_mode(on: bool) -> [u8; 2] {
    [0x01, on as u8]
}

pub fn encode_point(x_mm: f32, y_mm: f32) -> [u8; 9] {
    let mut out = [0; 9];
    out[0] = 0x02;
    out[1..5].copy_from_slice(&x_mm.to_le_bytes());
    out[5..].copy_from_slice(&y_mm.to_le_bytes());
    out
}

pub fn encode_no_point() -> [u8; 1] {
    [0x03]
}

/// What the sensor puts in its advertisement's manufacturer data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AdvInfo {
    pub version: u8,
    pub pairing: bool,
}

/// `manufacturer_data` as btleplug reports it: company ID → the bytes after it.
pub fn decode_adv(manufacturer_data: &HashMap<u16, Vec<u8>>) -> Option<AdvInfo> {
    match manufacturer_data.get(&MANUFACTURER_ID)?.as_slice() {
        [version, flags, ..] => Some(AdvInfo {
            version: *version,
            pairing: flags & ADV_FLAG_PAIRING != 0,
        }),
        _ => None,
    }
}

#[cfg(test)]
// The examples carry the digits C printed, and one value near pi is a formatting case.
#[allow(clippy::excessive_precision, clippy::approx_constant)]
pub(crate) mod tests {
    use super::*;

    // Examples from PROTOCOL.md.

    #[test]
    fn heartbeat_example() {
        assert_eq!(decode_heartbeat(&[0x39, 0x30, 0x00, 0x00]), Some(12_345));
        assert_eq!(decode_heartbeat(&[0x39, 0x30, 0x00]), None);
    }

    #[test]
    fn advertisement_example() {
        // On air: FF FF 03 01. btleplug strips the company ID into the map key.
        let data = HashMap::from([(0xFFFF, vec![0x03, 0x01])]);
        assert_eq!(
            decode_adv(&data),
            Some(AdvInfo {
                version: 3,
                pairing: true
            })
        );
        let data = HashMap::from([(0xFFFF, vec![0x03, 0x00])]);
        assert_eq!(
            decode_adv(&data),
            Some(AdvInfo {
                version: 3,
                pairing: false
            })
        );
        assert_eq!(decode_adv(&HashMap::new()), None);
    }

    #[test]
    fn pin_example() {
        assert_eq!(encode_pin(123_456), [0x40, 0xE2, 0x01, 0x00]);
        assert_eq!(format_pin(DEFAULT_PASSKEY), "000000");
    }

    #[test]
    fn pin_rules() {
        assert_eq!(validate_pin("001234"), Ok(1234));
        assert_eq!(validate_pin(" 999999 "), Ok(PASSKEY_MAX));
        assert!(validate_pin("1234").is_err()); // too short: 4 digits are not 001234
        assert!(validate_pin("1234567").is_err());
        assert!(validate_pin("12a456").is_err());
        assert!(validate_pin("+12345").is_err());
    }

    fn hex(s: &str) -> Vec<u8> {
        s.split_whitespace()
            .map(|b| u8::from_str_radix(b, 16).unwrap())
            .collect()
    }

    pub(crate) const SHOT_HEX: &str =
        "03 00 00 00 78 56 34 12 39 30 00 00 7D 00 00 00 06 FF FF FF 12 00 00 00 4D 3C 2B 1A 08 00 00 08";
    pub(crate) const HOLE_HEX: &str =
        "04 00 00 00 78 56 34 12 C8 32 00 00 00 00 00 00 00 00 00 00 13 00 00 00 4D 3C 2B 1A FF FF 01 00";
    pub(crate) const EVENT_HEX: &str = "00 00 00 01 78 56 34 12 12 00 00 00 15 00 00 00 39 30 00 00 D8 6F 70 36 07 00 00 00 9E 30 87 4A \
        00 00 00 00 3C FC FC FF B7 0B 00 00 B7 0B 00 00 E0 B7 98 41 FF F5 2F 3C 00 00 90 41 00 00 48 42 \
        00 50 7D 44 AF 5E AB 43 00 01 00 00 00 00 00 80 00 00 00 80 90 E3 04 00 54 44 03 00 A8 16 00 00 \
        54 44 03 00 BC 8C 02 00 00 00 00 00 00 00 00 80 00 00 00 80 DC E7 04 00 D8 47 03 00 38 18 00 00 \
        D8 47 03 00 DC 8F 02 00 F4 01 00 00 2B 8C FD FF 00 00 00 00 00 00 00 00 00 00 00 00 C3 69 00 00 \
        00 00 00 00 00 00 00 00 00 00 00 00 00 00 F0 41 00 00 F0 41";
    pub(crate) const STATUS_HEX: &str = "78 56 34 12 9E 30 87 4A 00 00 00 00 08 00 FC 00 00 00 00 00 00 00 00 00 00 00 90 41 00 00 48 42 \
        00 50 7D 44 AF 5E AB 43 15 00 00 00 00 00 00 00 15 00 00 00";
    pub(crate) const SETTINGS_HEX: &str = "01 01 01 3F 00 80 BB 44 00 00 D2 42 00 40 9C 46 00 40 1C 46 00 C8 2F 47 00 50 C3 47 2C 01 1E 00 \
        00 00 00 00";
    pub(crate) const CALIBRATION_HEX: &str = "01 03 29 35 01 00 00 2A 42 14 00 A0 00 00 00 00 00 00 00 00 00 00 00 00 28 41 3B AA AA 3E 00 00 \
        00 00 00 00 48 42 00 00 00 00 00 00 00 00 00 00 00 28 41 3B AA AA 3E 00 00 00 00 00 00 48 42 01 \
        00 00 D2 C2 00 00 D2 C2 00 00 28 41 3B AA AA 3E 00 00 00 00 00 00 48 42 01 00 00 D2 42 00 00 D2 \
        C2 00 00 28 41 3B AA AA 3E 00 00 00 00 00 00 48 42 01 00 00 82 42 00 00 D2 42 00 00 28 41 3B AA \
        AA 3E 00 00 00 00 00 00 48 42 01 00 00 D2 C2 00 00 D2 42 00 00 28 41 3B AA AA 3E 00 00 00 00 00 \
        00 48 42 01 00 00 00 00 00 00 D2 C2 00 00 28 41 3B AA AA 3E 00 00 00 00 00 00 48 42 01 00 00 D2 \
        42 00 00 00 00 00 00 28 41 3B AA AA 3E 00 00 00 00 00 00 48 42";

    pub(crate) fn example(h: &str) -> Vec<u8> {
        hex(h)
    }

    #[test]
    fn shot_example() {
        let shot = decode_shot(&hex(SHOT_HEX)).unwrap();
        assert_eq!(
            shot,
            Shot {
                seq: 3,
                boot: 0x1234_5678,
                time_ms: 12_345,
                x: 125,
                y: -250,
                event: 18,
                cal: 0x1A2B_3C4D,
                sigma: 8,
                kind: Kind::Shot,
                flags: SHOT_CURTAIN,
            }
        );
        assert_eq!((shot.x_mm(), shot.y_mm()), (1.25, -2.5));
        assert_eq!(shot.sigma_mm(), Some(0.08));
        assert!(shot.scored());
        let hole = decode_shot(&hex(HOLE_HEX)).unwrap();
        assert_eq!(
            (hole.seq, hole.kind, hole.sigma_mm(), hole.event),
            (4, Kind::Hole, None, 19)
        );
        assert!(!hole.scored());
        assert_eq!(decode_shot(&hex(SHOT_HEX)[..20]), None); // a v2 shot
        let mut bad = hex(SHOT_HEX);
        bad[30] = 3; // an unknown kind
        assert_eq!(decode_shot(&bad), None);
        assert_eq!(
            encode_shot_request(0x1234_5678, 4),
            [0x78, 0x56, 0x34, 0x12, 0x04, 0x00, 0x00, 0x00]
        );
        assert_eq!(
            encode_shot_clear(0x1234_5678, 3),
            [0x01, 0x78, 0x56, 0x34, 0x12, 0x03, 0x00, 0x00, 0x00]
        );
    }

    #[test]
    fn event_example() {
        let e = decode_event(&hex(EVENT_HEX)).unwrap();
        assert_eq!((e.kind, e.verdict_name(), e.curtain), (Kind::Shot, Some("ok"), 1));
        assert_eq!((e.boot, e.seq, e.n, e.time_ms), (0x1234_5678, 18, 21, 12_345));
        assert_eq!(e.t_ns, 30_978_109_400);
        assert_eq!((e.cfg, e.cal), (0x4A87_309E, 0));
        assert_eq!(e.flags, EV_HAS_PT | EV_CURTAIN_ON | EV_SOLVED | EV_SENT);
        assert_eq!((e.channels, e.fitted, e.dropped), (0xFC, 0xFC, -1));
        assert_eq!((e.x, e.y), (2999, 2999));
        assert_eq!((e.t_c, e.rh, e.hpa, e.c), (18.0, 50.0, 1013.25, 342.739_716));
        assert_eq!(
            e.t1,
            [NO_VALUE, NO_VALUE, 320_400, 214_100, 5800, 214_100, 167_100, 0]
        );
        assert_eq!(e.t2[7], 500);
        assert_eq!((e.dips, e.dip[0], e.dip_len[0]), (1, -160_725, 27_075));
        assert_eq!((e.pt_x, e.pt_y), (30.0, 30.0));
        assert_eq!(decode_event(&hex(EVENT_HEX)[..179]), None);
    }

    #[test]
    fn status_example() {
        let s = decode_status(&hex(STATUS_HEX)).unwrap();
        assert_eq!(
            (s.boot, s.cfg, s.cal, s.flags, s.mics),
            (0x1234_5678, 0x4A87_309E, 0, ST_CURTAIN_ON, 0xFC)
        );
        assert_eq!(
            (s.t_c, s.c, s.events, s.missed, s.shots),
            (18.0, 342.739_716, 21, 0, 21)
        );
    }

    #[test]
    fn settings_example() {
        let s = decode_settings(&hex(SETTINGS_HEX)).unwrap();
        assert_eq!(
            s,
            Settings {
                drop_one: true,
                max_lost: 1,
                mask: 0x3F,
                res_limit_ns: 1500.0,
                area_mm: 105.0,
                together_ns: 20000.0,
                pulse_max_ns: 10000.0,
                curtain_lo_ns: 45000.0,
                curtain_hi_ns: 100_000.0,
                span_mm: 300,
                deadtime_ms: 30,
            }
        );
        assert_eq!(encode_settings(&s).to_vec(), hex(SETTINGS_HEX));
        let mut other = hex(SETTINGS_HEX);
        other[0] = 2;
        assert_eq!(decode_settings(&other), None);
    }

    /// The ranges, at their edges (PROTOCOL.md "Settings").
    #[test]
    fn settings_ranges() {
        let ok = decode_settings(&hex(SETTINGS_HEX)).unwrap();
        assert_eq!(settings_problem(&ok), None);
        type Edge = (fn(&mut Settings), bool); // a change, and whether it stays valid
        let edges: [Edge; 16] = [
            (|s| s.res_limit_ns = 100_000.0, true),
            (|s| s.res_limit_ns = 0.0, false),
            (|s| s.max_lost = -1, true),
            (|s| s.max_lost = 5, false),
            (|s| s.area_mm = 0.0, true),
            (|s| s.area_mm = 9.5, false),
            (|s| s.area_mm = 1000.0, true),
            (|s| s.together_ns = 100_001.0, false),
            (|s| s.pulse_max_ns = 0.0, true),
            (|s| s.pulse_max_ns = -1.0, false),
            (|s| *s = s.with_curtain(None), true),
            (|s| *s = s.with_curtain(Some((0.0, 2_000_000.0))), true),
            (|s| *s = s.with_curtain(Some((50_000.0, 50_000.0))), false),
            (|s| s.span_mm = 49, false),
            (|s| s.deadtime_ms = 1000, true),
            (|s| s.deadtime_ms = 0, false),
        ];
        for (i, (change, valid)) in edges.iter().enumerate() {
            let mut s = ok.clone();
            change(&mut s);
            assert_eq!(settings_problem(&s).is_none(), *valid, "edge {i}: {s:?}");
        }
    }

    /// The curtain switched off and on again: bytes 20–27 change, nothing else.
    #[test]
    fn curtain_switch_changes_only_the_window() {
        let s = decode_settings(&hex(SETTINGS_HEX)).unwrap();
        assert_eq!(s.curtain(), Some((45000.0, 100_000.0)));
        let off = s.with_curtain(None);
        assert_eq!(off.curtain(), None);
        let (a, b) = (encode_settings(&s), encode_settings(&off));
        let changed: Vec<usize> = (0..SETTINGS_LEN).filter(|&i| a[i] != b[i]).collect();
        assert!(changed.iter().all(|i| (20..28).contains(i)), "{changed:?}");
        assert_eq!(&b[20..28], &[0; 8]);
        let on = off.with_curtain(Some((50000.0, 90000.0)));
        assert_eq!(on.curtain(), Some((50000.0, 90000.0)));
        assert_eq!(on.with_curtain(s.curtain()), s);
    }

    #[test]
    fn calibration_example() {
        let c = decode_calibration(&hex(CALIBRATION_HEX)).unwrap();
        assert_eq!(
            (c.date, c.res_ns, c.points, c.sparks),
            (20_261_123, 42.5, 20, 160)
        );
        assert!(!c.ch[0].mic);
        assert_eq!(
            c.ch[2],
            Channel {
                mic: true,
                x: -105.0,
                y: -105.0,
                h: 10.5,
                k: 0.333_330_005,
                offset_ns: 0.0,
                sigma_ns: 50.0
            }
        );
        assert_eq!(encode_calibration(&c).to_vec(), hex(CALIBRATION_HEX));
    }

    #[test]
    fn control_examples() {
        assert_eq!(encode_cal_mode(true), [0x01, 0x01]);
        assert_eq!(
            encode_point(50.0, -30.0),
            [0x02, 0x00, 0x00, 0x48, 0x42, 0x00, 0x00, 0xF0, 0xC1]
        );
        assert_eq!(encode_no_point(), [0x03]);
        assert_eq!(
            encode_event_request(0x1234_5678, 19),
            [0x78, 0x56, 0x34, 0x12, 0x13, 0x00, 0x00, 0x00]
        );
    }

    #[test]
    fn name_rules() {
        assert_eq!(validate_name("  Kitchen  "), Ok("Kitchen".into()));
        assert!(validate_name("   ").is_err());
        assert!(validate_name("abcdefghijklmnopqrstu").is_err()); // 21
        assert!(validate_name("abcdefghijklmnopqrst").is_ok()); // 20
        assert!(validate_name("Küche").is_err());
        assert_eq!(decode_name(b"OpenSights-a1f"), Some("OpenSights-a1f".into()));
        assert_eq!(decode_name(b" padded "), Some(" padded ".into()));
        assert_eq!(decode_name(b""), None);
        assert_eq!(decode_name("Küche".as_bytes()), None);
    }
}
