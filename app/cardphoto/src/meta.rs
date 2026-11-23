//! Metadata out of an uploaded photo (`app/CARDPHOTO.md` C3): a phone's JPEG carries where and
//! when it was taken (EXIF, GPS, XMP), the phone's make and serial, often a thumbnail. Those
//! segments are dropped from the file; the image data itself is copied byte for byte (no
//! re-encoding, so the pixels are the phone's). What the registration might use is read first
//! and returned: the EXIF orientation.
//!
//! JPEG: kept are the image segments (frames, tables, scans, restart intervals), APP0 (JFIF),
//! APP2 if it is an ICC colour profile, APP14 (Adobe: needed to read the colours right);
//! dropped are APP1 (EXIF, XMP), the other APPn (MPF and its appended images, maker data) and
//! comments, and anything after the end of the image. PNG: dropped are eXIf, tEXt, zTXt,
//! iTXt and tIME.

/// What was in the photo, and what was dropped.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stripped {
    /// "jpeg" or "png".
    pub format: &'static str,
    /// The EXIF orientation (1 upright, 3 upside down, 6 and 8 turned; 2, 4, 5, 7 mirrored),
    /// if the photo had one.
    pub orientation: Option<u16>,
    /// The dropped segments, by name ("APP1 Exif", "COM", "trailing 812 bytes", "tEXt").
    pub dropped: Vec<String>,
}

/// The photo without its metadata. Only JPEG and PNG are taken.
pub fn strip(bytes: &[u8]) -> Result<(Vec<u8>, Stripped), String> {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        strip_jpeg(bytes)
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        strip_png(bytes)
    } else {
        Err("not a JPEG or PNG photo".into())
    }
}

fn be16(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*b.get(i)?, *b.get(i + 1)?]))
}

fn strip_jpeg(b: &[u8]) -> Result<(Vec<u8>, Stripped), String> {
    let bad = |what: &str| format!("a damaged JPEG ({what})");
    let mut out = Vec::with_capacity(b.len());
    out.extend_from_slice(&b[..2]);
    let mut info = Stripped {
        format: "jpeg",
        ..Stripped::default()
    };
    let mut i = 2;
    let mut seen_scan = false;
    loop {
        // A marker: 0xFF, any fill 0xFF bytes, the code.
        if b.get(i) != Some(&0xFF) {
            return Err(bad("no marker where one should be"));
        }
        while b.get(i) == Some(&0xFF) {
            i += 1;
        }
        let m = *b.get(i).ok_or_else(|| bad("cut short"))?;
        i += 1;
        if m == 0xD9 {
            out.extend_from_slice(&[0xFF, 0xD9]);
            if i < b.len() {
                info.dropped.push(format!("trailing {} bytes", b.len() - i));
            }
            break;
        }
        if (0xD0..=0xD7).contains(&m) || m == 0x01 {
            out.extend_from_slice(&[0xFF, m]);
            continue;
        }
        let len = be16(b, i).ok_or_else(|| bad("cut short"))? as usize;
        if len < 2 || i + len > b.len() {
            return Err(bad("a segment runs past the end"));
        }
        let seg = &b[i - 2..i + len]; // FF, marker, length, data
        let data = &b[i + 2..i + len];
        i += len;
        let keep = match m {
            0xE0 => data.starts_with(b"JFIF\0") || data.starts_with(b"JFXX\0"),
            0xE2 => data.starts_with(b"ICC_PROFILE\0"),
            0xEE => data.starts_with(b"Adobe"),
            0xE1 => {
                if data.starts_with(b"Exif\0\0") && info.orientation.is_none() {
                    info.orientation = exif_orientation(&data[6..]);
                }
                false
            }
            0xE3..=0xEF | 0xFE => false,
            _ => true,
        };
        if keep {
            out.push(0xFF);
            out.extend_from_slice(&seg[1..]);
        } else {
            info.dropped.push(segment_name(m, data));
        }
        if m == 0xDA {
            // The entropy-coded scan: up to the next marker that isn't a stuffed byte (FF 00)
            // or a restart (FF D0..D7). Copied as it is.
            seen_scan = true;
            let start = i;
            loop {
                let Some(p) = b[i..].iter().position(|&v| v == 0xFF) else {
                    return Err(bad("the image data has no end"));
                };
                i += p;
                match b.get(i + 1) {
                    Some(0x00) | Some(0xD0..=0xD7) => i += 2,
                    Some(0xFF) => i += 1,
                    Some(_) => break,
                    None => return Err(bad("the image data has no end")),
                }
            }
            out.extend_from_slice(&b[start..i]);
        }
    }
    if !seen_scan {
        return Err(bad("no image data"));
    }
    Ok((out, info))
}

fn segment_name(m: u8, data: &[u8]) -> String {
    let base = match m {
        0xFE => return "COM".into(),
        0xE0..=0xEF => format!("APP{}", m - 0xE0),
        _ => format!("marker {m:02X}"),
    };
    // The segment's identifier: printable bytes up to the first NUL.
    let id: String = data
        .iter()
        .take(32)
        .take_while(|&&c| c != 0)
        .map(|&c| c as char)
        .filter(|c| c.is_ascii_graphic() || *c == ' ')
        .collect();
    if id.is_empty() {
        base
    } else {
        format!("{base} {id}")
    }
}

/// The orientation tag (0x0112) of IFD0 in an EXIF TIFF block.
fn exif_orientation(t: &[u8]) -> Option<u16> {
    let le = match t.get(..2)? {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let u16_at = |i: usize| -> Option<u16> {
        let v = [*t.get(i)?, *t.get(i + 1)?];
        Some(if le {
            u16::from_le_bytes(v)
        } else {
            u16::from_be_bytes(v)
        })
    };
    let u32_at = |i: usize| -> Option<u32> {
        let v = [*t.get(i)?, *t.get(i + 1)?, *t.get(i + 2)?, *t.get(i + 3)?];
        Some(if le {
            u32::from_le_bytes(v)
        } else {
            u32::from_be_bytes(v)
        })
    };
    if u16_at(2)? != 42 {
        return None;
    }
    let ifd = u32_at(4)? as usize;
    let n = u16_at(ifd)? as usize;
    (0..n.min(512)).find_map(|k| {
        let e = ifd + 2 + 12 * k;
        (u16_at(e)? == 0x0112 && u16_at(e + 2)? == 3).then(|| u16_at(e + 8))?
    })
}

fn strip_png(b: &[u8]) -> Result<(Vec<u8>, Stripped), String> {
    let mut out = b[..8].to_vec();
    let mut info = Stripped {
        format: "png",
        ..Stripped::default()
    };
    let mut i = 8;
    loop {
        let len = b
            .get(i..i + 4)
            .map(|v| u32::from_be_bytes([v[0], v[1], v[2], v[3]]) as usize)
            .ok_or("a damaged PNG (cut short)")?;
        let end = i + 12 + len;
        if end > b.len() {
            return Err("a damaged PNG (a chunk runs past the end)".into());
        }
        let kind = &b[i + 4..i + 8];
        let name = String::from_utf8_lossy(kind).into_owned();
        if matches!(kind, b"eXIf" | b"tEXt" | b"zTXt" | b"iTXt" | b"tIME") {
            info.dropped.push(name);
        } else {
            out.extend_from_slice(&b[i..end]);
        }
        i = end;
        if kind == b"IEND" {
            if i < b.len() {
                info.dropped.push(format!("trailing {} bytes", b.len() - i));
            }
            return Ok((out, info));
        }
    }
}

/// Test photos with metadata, as a phone writes it (for this crate's tests and the app's).
pub mod testdata {
    /// An EXIF APP1 segment (big-endian TIFF) with the orientation, the make, and a GPS IFD
    /// (latitude 52° 22' 12.5" N).
    pub fn exif_app1(orientation: u16) -> Vec<u8> {
        let mut t: Vec<u8> = Vec::new();
        t.extend_from_slice(b"MM\0\x2a\0\0\0\x08");
        // IFD0 at 8: 3 entries (orientation, make, GPS pointer), next IFD 0.
        let ifd0_len = 2 + 3 * 12 + 4;
        let make_off = 8 + ifd0_len;
        let make = b"PhoneMaker\0";
        let gps_off = make_off + make.len() + (make.len() & 1);
        let entry = |t: &mut Vec<u8>, tag: u16, typ: u16, count: u32, val: [u8; 4]| {
            t.extend_from_slice(&tag.to_be_bytes());
            t.extend_from_slice(&typ.to_be_bytes());
            t.extend_from_slice(&count.to_be_bytes());
            t.extend_from_slice(&val);
        };
        t.extend_from_slice(&3u16.to_be_bytes());
        let o = orientation.to_be_bytes();
        entry(&mut t, 0x0112, 3, 1, [o[0], o[1], 0, 0]);
        entry(
            &mut t,
            0x010F,
            2,
            make.len() as u32,
            (make_off as u32).to_be_bytes(),
        );
        entry(&mut t, 0x8825, 4, 1, (gps_off as u32).to_be_bytes());
        t.extend_from_slice(&0u32.to_be_bytes());
        t.extend_from_slice(make);
        if make.len() & 1 == 1 {
            t.push(0);
        }
        // The GPS IFD: latitude ref "N", latitude 52/1 22/1 125/10.
        let lat_off = gps_off + 2 + 2 * 12 + 4;
        t.extend_from_slice(&2u16.to_be_bytes());
        entry(&mut t, 0x0001, 2, 2, *b"N\0\0\0");
        entry(&mut t, 0x0002, 5, 3, (lat_off as u32).to_be_bytes());
        t.extend_from_slice(&0u32.to_be_bytes());
        for (n, d) in [(52u32, 1u32), (22, 1), (125, 10)] {
            t.extend_from_slice(&n.to_be_bytes());
            t.extend_from_slice(&d.to_be_bytes());
        }
        let mut seg = vec![0xFF, 0xE1];
        seg.extend_from_slice(&((2 + 6 + t.len()) as u16).to_be_bytes());
        seg.extend_from_slice(b"Exif\0\0");
        seg.extend_from_slice(&t);
        seg
    }

    /// A JPEG of the image with EXIF (orientation, GPS), XMP, a comment, an MPF segment, and
    /// a second image appended after the end, as phones do.
    pub fn phone_jpeg(img: &image::RgbImage, orientation: u16) -> Vec<u8> {
        let mut plain = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut plain, 90)
            .encode_image(img)
            .unwrap();
        // After SOI (and the encoder's APP0, if any): the phone's segments.
        let mut at = 2;
        if plain[2..4] == [0xFF, 0xE0] {
            at += 2 + u16::from_be_bytes([plain[4], plain[5]]) as usize;
        }
        let mut out = plain[..at].to_vec();
        out.extend(exif_app1(orientation));
        let xmp = b"http://ns.adobe.com/xap/1.0/\0<x:xmpmeta><exif:GPSLatitude>52,22.2N</exif:GPSLatitude></x:xmpmeta>";
        out.extend_from_slice(&[0xFF, 0xE1]);
        out.extend_from_slice(&((xmp.len() + 2) as u16).to_be_bytes());
        out.extend_from_slice(xmp);
        let mpf = b"MPF\0MM\0\x2a\0\0\0\x08";
        out.extend_from_slice(&[0xFF, 0xE2]);
        out.extend_from_slice(&((mpf.len() + 2) as u16).to_be_bytes());
        out.extend_from_slice(mpf);
        let com = b"taken at the range";
        out.extend_from_slice(&[0xFF, 0xFE]);
        out.extend_from_slice(&((com.len() + 2) as u16).to_be_bytes());
        out.extend_from_slice(com);
        out.extend_from_slice(&plain[at..]);
        // A depth map or the like, after the end.
        out.extend_from_slice(&plain[..plain.len().min(400)]);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture() -> image::RgbImage {
        image::RgbImage::from_fn(120, 80, |x, y| {
            image::Rgb([(x * 2) as u8, (y * 3) as u8, ((x + y) % 256) as u8])
        })
    }

    #[test]
    fn jpeg_metadata_gone_pixels_identical() {
        let phone = testdata::phone_jpeg(&picture(), 6);
        assert!(phone.windows(4).any(|w| w == b"Exif"));
        let (clean, info) = strip(&phone).unwrap();
        println!("{info:?}, {} -> {} bytes", phone.len(), clean.len());
        assert_eq!(info.orientation, Some(6));
        for needle in [
            &b"Exif"[..],
            b"GPS",
            b"PhoneMaker",
            b"xmpmeta",
            b"MPF",
            b"taken at",
        ] {
            assert!(
                !clean.windows(needle.len()).any(|w| w == needle),
                "{} left",
                String::from_utf8_lossy(needle)
            );
        }
        assert!(info.dropped.iter().any(|d| d.starts_with("trailing")));
        let a = image::load_from_memory(&phone).unwrap().to_rgb8();
        let b = image::load_from_memory(&clean).unwrap().to_rgb8();
        assert_eq!(a.dimensions(), b.dimensions());
        assert!(a.as_raw() == b.as_raw(), "pixels differ");
        // Stripping again changes nothing.
        let (again, info2) = strip(&clean).unwrap();
        assert_eq!(again, clean);
        assert!(info2.dropped.is_empty());
    }

    #[test]
    fn png_text_gone() {
        let mut png = Vec::new();
        picture()
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        // A tEXt chunk after IHDR (8 + 25 bytes).
        let text = b"Location\0Range 3";
        let mut chunk = (text.len() as u32).to_be_bytes().to_vec();
        chunk.extend_from_slice(b"tEXt");
        chunk.extend_from_slice(text);
        chunk.extend_from_slice(&[0; 4]); // the CRC isn't checked here
        let mut with = png[..33].to_vec();
        with.extend(chunk);
        with.extend_from_slice(&png[33..]);
        let (clean, info) = strip(&with).unwrap();
        assert_eq!(info.dropped, vec!["tEXt".to_string()]);
        assert_eq!(clean, png);
    }

    #[test]
    fn others_refused() {
        assert!(strip(b"GIF89a....").is_err());
        assert!(strip(&[0xFF, 0xD8, 0xFF, 0xE0, 0x00]).is_err()); // cut short
        let jpeg = testdata::phone_jpeg(&picture(), 1);
        assert!(strip(&jpeg[..jpeg.len() / 3]).is_err());
    }
}
