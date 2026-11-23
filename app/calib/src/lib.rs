//! OpenSights calibration (MEASUREMENT step 5d, PLAN D5): the target's position solver and
//! the calibration fit, ported from `firmware/lib/measure` and `firmware/tools/calibrate.py`.
//! Pure computation, no BLE or UI, so the desktop app uses it now and the management UI
//! later (natively or as wasm).
//!
//! - [`solver`]: `measure::solve` and `solveDropOne`, in single precision like the target,
//!   checked against the firmware's test vectors.
//! - [`fit`]: the calibration fit over sparks at known points, checked against
//!   `calibrate.py` on the same log.
//! - [`text`]: the firmware's log-line formatting (`%.9g`, CRC-32, the `cfg` fields and IDs).
//! - [`logline`]: reading `cfg` and `rec` lines (tests, and fitting from a log).

pub mod fit;
pub mod logline;
pub mod solver;
pub mod text;

/// One channel's settings: its mic and its arrival-time calibration (`measure::ChannelCfg`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Channel {
    /// False: no mic on this channel, never in a fit.
    pub mic: bool,
    /// mm from the frame centre; h is the mic port behind the paper plane.
    pub x: f32,
    pub y: f32,
    pub h: f32,
    /// The arrival-time factor (MODEL.md "Arrival").
    pub k: f32,
    /// The channel's own delay, ns.
    pub offset_ns: f32,
    /// Its timing error, 1σ, ns (the solver's weight).
    pub sigma_ns: f32,
}

pub const MAX_MICS: usize = 8;
