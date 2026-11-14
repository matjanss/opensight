//! BLE protocol constants and value encoding. Mirrors `protocol/PROTOCOL.md` and
//! `firmware/src/protocol.h`: change all three together.

use std::collections::HashMap;
use uuid::{Uuid, uuid};

pub const PROTOCOL_VERSION: u8 = 1;

pub const SERVICE_UUID: Uuid = uuid!("5631f100-5314-4f02-b637-0654ff6d409d");
pub const HEARTBEAT_UUID: Uuid = uuid!("5631f101-5314-4f02-b637-0654ff6d409d");
pub const LED_UUID: Uuid = uuid!("5631f102-5314-4f02-b637-0654ff6d409d");
pub const NAME_UUID: Uuid = uuid!("5631f103-5314-4f02-b637-0654ff6d409d");
pub const VERSION_UUID: Uuid = uuid!("5631f104-5314-4f02-b637-0654ff6d409d");
pub const PIN_UUID: Uuid = uuid!("5631f105-5314-4f02-b637-0654ff6d409d");
pub const SHOT_UUID: Uuid = uuid!("5631f106-5314-4f02-b637-0654ff6d409d");

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

/// LED: one byte, 0 = off, 1 = on.
pub fn decode_led(bytes: &[u8]) -> Option<bool> {
    match bytes {
        [0] => Some(false),
        [1] => Some(true),
        _ => None,
    }
}

pub fn encode_led(on: bool) -> [u8; 1] {
    [on as u8]
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

/// One shot as the sensor reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shot {
    /// Numbered from 1 for every boot of the sensor.
    pub seq: u32,
    /// Random per boot of the sensor, never 0: tells which boot `seq` belongs to.
    pub boot: u32,
    /// The sensor's `millis()` at the impact.
    pub time_ms: u32,
    /// Hundredths of a mm from the centre; x to the right, y up, as seen by the shooter.
    pub x: i32,
    pub y: i32,
}

impl Shot {
    pub fn x_mm(&self) -> f32 {
        self.x as f32 / 100.0
    }
    pub fn y_mm(&self) -> f32 {
        self.y as f32 / 100.0
    }
}

/// Shot indication: seq `u32`, boot `u32`, time `u32`, x `i32`, y `i32`, little-endian.
pub fn decode_shot(bytes: &[u8]) -> Option<Shot> {
    let b: &[u8; 20] = bytes.try_into().ok()?;
    let word = |i: usize| [b[i], b[i + 1], b[i + 2], b[i + 3]];
    Some(Shot {
        seq: u32::from_le_bytes(word(0)),
        boot: u32::from_le_bytes(word(4)),
        time_ms: u32::from_le_bytes(word(8)),
        x: i32::from_le_bytes(word(12)),
        y: i32::from_le_bytes(word(16)),
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
mod tests {
    use super::*;

    // Examples from PROTOCOL.md.

    #[test]
    fn heartbeat_example() {
        assert_eq!(decode_heartbeat(&[0x39, 0x30, 0x00, 0x00]), Some(12_345));
        assert_eq!(decode_heartbeat(&[0x39, 0x30, 0x00]), None);
    }

    #[test]
    fn advertisement_example() {
        // On air: FF FF 01 01. btleplug strips the company ID into the map key.
        let data = HashMap::from([(0xFFFF, vec![0x01, 0x01])]);
        assert_eq!(
            decode_adv(&data),
            Some(AdvInfo {
                version: 1,
                pairing: true
            })
        );
        let data = HashMap::from([(0xFFFF, vec![0x01, 0x00])]);
        assert_eq!(
            decode_adv(&data),
            Some(AdvInfo {
                version: 1,
                pairing: false
            })
        );
        assert_eq!(decode_adv(&HashMap::new()), None);
    }

    #[test]
    fn led_values() {
        assert_eq!(decode_led(&encode_led(true)), Some(true));
        assert_eq!(decode_led(&encode_led(false)), Some(false));
        assert_eq!(decode_led(&[2]), None);
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

    #[test]
    fn shot_example() {
        let bytes = [
            0x03, 0x00, 0x00, 0x00, 0x78, 0x56, 0x34, 0x12, 0x39, 0x30, 0x00, 0x00, 0x7D, 0x00, 0x00, 0x00,
            0x06, 0xFF, 0xFF, 0xFF,
        ];
        let shot = decode_shot(&bytes).unwrap();
        assert_eq!(
            shot,
            Shot {
                seq: 3,
                boot: 0x1234_5678,
                time_ms: 12_345,
                x: 125,
                y: -250
            }
        );
        assert_eq!((shot.x_mm(), shot.y_mm()), (1.25, -2.5));
        assert_eq!(decode_shot(&bytes[..19]), None);
        assert_eq!(
            encode_shot_request(0x1234_5678, 4),
            [0x78, 0x56, 0x34, 0x12, 0x04, 0x00, 0x00, 0x00]
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
