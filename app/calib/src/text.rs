//! The firmware's log-line formatting (`lib/measure/record.cpp`): its floats as C's `%.9g`,
//! the CRC-32 its IDs use, and the `cfg` line's channel fields.

use crate::{Channel, MAX_MICS};

/// C's `printf("%.*g", prec, v)`: `prec` significant digits, trailing zeros dropped, the
/// exponent form below 1e-4 and from 10^prec up.
pub fn fmt_g(v: f64, prec: usize) -> String {
    if v == 0.0 {
        return if v.is_sign_negative() {
            "-0".into()
        } else {
            "0".into()
        };
    }
    if v.is_nan() {
        return if v.is_sign_negative() {
            "-nan".into()
        } else {
            "nan".into()
        };
    }
    if v.is_infinite() {
        return if v < 0.0 { "-inf".into() } else { "inf".into() };
    }
    let p = prec.max(1);
    let sci = format!("{:.*e}", p - 1, v);
    let (mant, exp) = sci.split_once('e').unwrap();
    let x: i32 = exp.parse().unwrap();
    if x < -4 || x >= p as i32 {
        let sign = if x < 0 { '-' } else { '+' };
        format!("{}e{}{:02}", strip_zeros(mant), sign, x.abs())
    } else {
        strip_zeros(&format!("{:.*}", (p as i32 - 1 - x) as usize, v)).to_owned()
    }
}

fn strip_zeros(s: &str) -> &str {
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.')
    } else {
        s
    }
}

/// A float as the firmware prints it in its log lines (`%.9g`).
pub fn g9(v: f32) -> String {
    fmt_g(f64::from(v), 9)
}

/// CRC-32 (IEEE 802.3, as zlib's): the firmware's `cfg` and calibration IDs.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// The `cfg` line's channel fields, `CH1=x,y,h,k,offset,sigma ... CH8=-`.
pub fn channel_fields(ch: &[Channel; MAX_MICS]) -> String {
    let mut s = String::new();
    for (i, c) in ch.iter().enumerate() {
        if i > 0 {
            s.push(' ');
        }
        s += &format!("CH{}=", i + 1);
        if c.mic {
            s += &[c.x, c.y, c.h, c.k, c.offset_ns, c.sigma_ns].map(g9).join(",");
        } else {
            s.push('-');
        }
    }
    s
}

/// The calibration ID: the CRC-32 of the channel fields (`measure::calibrationId`).
pub fn calibration_id(ch: &[Channel; MAX_MICS]) -> u32 {
    crc32(channel_fields(ch).as_bytes())
}

#[cfg(test)]
// The cases carry the digits C printed, and one value near pi is a formatting case.
#[allow(clippy::excessive_precision, clippy::approx_constant)]
mod tests {
    use super::*;

    #[test]
    fn g_matches_c() {
        // gcc's printf("%.9g") of these floats.
        let cases: [(f32, &str); 20] = [
            (0.33333, "0.333330005"),
            (1.0 / 3.0, "0.333333343"),
            (-105.0, "-105"),
            (10.5, "10.5"),
            (0.0, "0"),
            (1500.0, "1500"),
            (1e-5, "9.99999975e-06"),
            (123_456_789.0, "123456792"),
            (1_234_567_890.0, "1.23456794e+09"),
            (0.0001, "9.99999975e-05"),
            (-0.5, "-0.5"),
            (42.5, "42.5"),
            (342.739_716, "342.739716"),
            (99_999.992_2, "99999.9922"),
            (1e10, "1e+10"),
            (0.000_123_456, "0.000123456004"),
            (-0.0, "-0"),
            (65.0, "65"),
            (100_000.0, "100000"),
            (3.141_592_65, "3.14159274"),
        ];
        for (v, want) in cases {
            assert_eq!(g9(v), want, "{v:e}");
        }
    }

    #[test]
    fn crc_is_zlibs() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }
}
