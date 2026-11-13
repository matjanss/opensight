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

pub const MANUFACTURER_ID: u16 = 0xFFFF;
pub const ADV_FLAG_PAIRING: u8 = 0x01;

pub const PASSKEY: &str = "1234";
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
    fn name_rules() {
        assert_eq!(validate_name("  Kitchen  "), Ok("Kitchen".into()));
        assert!(validate_name("   ").is_err());
        assert!(validate_name("abcdefghijklmnopqrstu").is_err()); // 21
        assert!(validate_name("abcdefghijklmnopqrst").is_ok()); // 20
        assert!(validate_name("Küche").is_err());
        assert_eq!(decode_name(b"ESPSensor-a1f"), Some("ESPSensor-a1f".into()));
        assert_eq!(decode_name(b" padded "), Some(" padded ".into()));
        assert_eq!(decode_name(b""), None);
        assert_eq!(decode_name("Küche".as_bytes()), None);
    }
}
