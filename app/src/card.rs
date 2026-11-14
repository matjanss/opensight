//! Target cards: an SVG for the picture plus JSON with the scoring geometry. Scoring only
//! uses the JSON, never the SVG (see FUTURE_ARCHITECTURE.md).
//!
//! SVG rules: 1 user unit = 1 mm, `width`/`height` without units. Text is drawn with the
//! app's embedded IBM Plex Sans (no system fonts), so a card looks the same everywhere.
//! resvg renders it statically: no scripts, no external files.

use std::sync::Arc;

use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions, Vec2};
use resvg::{tiny_skia, usvg};
use serde::Deserialize;

/// The card's JSON file.
#[derive(Deserialize)]
struct Meta {
    id: String,
    name: String,
    /// Where the target centre is, in SVG units (mm from the top left).
    centre_mm: [f32; 2],
    pellet_diameter_mm: f32,
    /// Decimal scoring (10.9 at best) or whole rings only.
    decimal: bool,
    inner_ten_diameter_mm: Option<f32>,
    /// Scoring rings, any order. Every ring's outer edge counts: a hole touching it scores it.
    rings: Vec<Ring>,
}

#[derive(Deserialize)]
struct Ring {
    value: u8,
    diameter_mm: f32,
}

pub struct Card {
    pub id: String,
    pub name: String,
    pub decimal: bool,
    pub pellet_diameter_mm: f32,
    centre: Vec2,
    inner_ten_r: Option<f32>,
    /// (value, outer radius), highest value (smallest ring) first.
    rings: Vec<(u8, f32)>,
    svg: usvg::Tree,
}

/// The score of one shot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Score {
    /// 0 = miss.
    pub ring: u8,
    /// Decimal score in tenths: 104 = 10.4. Equals `ring * 10` on whole-ring cards.
    pub tenths: u16,
    pub inner_ten: bool,
}

impl Score {
    pub fn text(&self, decimal: bool) -> String {
        if decimal {
            format!("{}.{}", self.tenths / 10, self.tenths % 10)
        } else {
            self.ring.to_string()
        }
    }
}

/// The 10 m air pistol card shipped with the app.
pub fn air_pistol() -> Card {
    Card::load(
        include_str!("../assets/cards/issf-10m-air-pistol.json"),
        include_str!("../assets/cards/issf-10m-air-pistol.svg"),
    )
    .expect("the built-in card is valid")
}

impl Card {
    pub fn load(json: &str, svg: &str) -> Result<Card, String> {
        let meta: Meta = serde_json::from_str(json).map_err(|e| format!("Card JSON: {e}"))?;
        let mut rings: Vec<(u8, f32)> = meta.rings.iter().map(|r| (r.value, r.diameter_mm / 2.0)).collect();
        rings.sort_by_key(|r| std::cmp::Reverse(r.0));
        let ok = !rings.is_empty()
            && rings.windows(2).all(|w| w[0].0 == w[1].0 + 1 && w[0].1 < w[1].1)
            && rings[0].1 > 0.0;
        if !ok {
            return Err("Card JSON: rings must be consecutive values, smaller for higher values.".into());
        }

        let mut options = usvg::Options::default();
        let fonts = Arc::make_mut(&mut options.fontdb);
        fonts.load_font_data(include_bytes!("../assets/fonts/IBMPlexSans-Regular.ttf").to_vec());
        fonts.set_sans_serif_family("IBM Plex Sans");
        let svg = usvg::Tree::from_str(svg, &options).map_err(|e| format!("Card SVG: {e}"))?;

        Ok(Card {
            id: meta.id,
            name: meta.name,
            decimal: meta.decimal,
            pellet_diameter_mm: meta.pellet_diameter_mm,
            centre: Vec2::from(meta.centre_mm),
            inner_ten_r: meta.inner_ten_diameter_mm.map(|d| d / 2.0),
            rings,
            svg,
        })
    }

    /// The card's size in mm.
    pub fn size_mm(&self) -> Vec2 {
        let s = self.svg.size();
        egui::vec2(s.width(), s.height())
    }

    /// A shot position (mm from the centre, y up) on the card (mm from the top left, y down).
    pub fn to_card(&self, x_mm: f32, y_mm: f32) -> Vec2 {
        self.centre + egui::vec2(x_mm, -y_mm)
    }

    /// Scores a shot at (x, y) mm from the centre. A ring counts when the pellet hole
    /// touches its outer edge. Decimal: each ring is split into tenths from its outer edge
    /// inwards; the top ring continues with the same width to 10.9.
    pub fn score(&self, x_mm: f32, y_mm: f32) -> Score {
        let edge = x_mm.hypot(y_mm) - self.pellet_diameter_mm / 2.0; // hole edge nearest the centre
        let inner_ten = self.inner_ten_r.is_some_and(|r| edge <= r);
        let Some(i) = self.rings.iter().position(|&(_, r)| edge <= r) else {
            return Score { ring: 0, tenths: 0, inner_ten: false };
        };
        let (ring, outer) = self.rings[i];
        let inner = match i {
            0 => outer - (self.rings.get(1).map_or(outer, |r| r.1) - outer),
            _ => self.rings[i - 1].1,
        };
        let fraction = ((outer - edge) / (outer - inner)).clamp(0.0, 0.99);
        let tenths = if self.decimal {
            ring as u16 * 10 + (fraction * 10.0) as u16
        } else {
            ring as u16 * 10
        };
        Score { ring, tenths, inner_ten }
    }

    /// Renders the card `width` pixels wide.
    fn render(&self, width: u32) -> Option<ColorImage> {
        let size = self.svg.size();
        let scale = width as f32 / size.width();
        let height = (size.height() * scale).ceil() as u32;
        let mut pixmap = tiny_skia::Pixmap::new(width, height)?;
        resvg::render(&self.svg, tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
        Some(ColorImage::from_rgba_premultiplied(
            [width as usize, height as usize],
            pixmap.data(),
        ))
    }
}

/// The rendered card, re-rendered when the shown size changes enough to look blurry.
#[derive(Default)]
pub struct CardTexture {
    texture: Option<(String, u32, TextureHandle)>,
}

/// Renders in steps of this many pixels, so resizing the window doesn't re-render every frame.
const RENDER_STEP: u32 = 128;

impl CardTexture {
    pub fn get(&mut self, ctx: &egui::Context, card: &Card, width_px: f32) -> Option<&TextureHandle> {
        let width = (width_px.max(1.0) as u32).div_ceil(RENDER_STEP) * RENDER_STEP;
        let fresh = matches!(&self.texture, Some((id, w, _)) if *id == card.id && *w == width);
        if !fresh {
            let image = card.render(width)?;
            let tex = ctx.load_texture(format!("card-{}", card.id), image, TextureOptions::LINEAR);
            self.texture = Some((card.id.clone(), width, tex));
        }
        self.texture.as_ref().map(|(_, _, t)| t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(x: f32, y: f32) -> Score {
        air_pistol().score(x, y)
    }

    // 10 m air pistol: ring 10 has an 11.5 mm diameter (5.75 mm radius), rings are 8 mm
    // apart (radius), the pellet is 4.5 mm. A hole touches ring n while its centre is
    // within (radius n + 2.25 mm).

    #[test]
    fn centre_is_10_9_and_inner_ten() {
        assert_eq!(s(0.0, 0.0), Score { ring: 10, tenths: 109, inner_ten: true });
    }

    #[test]
    fn ring_edges() {
        assert_eq!(s(8.0, 0.0).tenths, 100); // just touches ring 10
        assert_eq!(s(0.0, 8.05).tenths, 99); // just outside: 9.9
        assert_eq!(s(16.0, 0.0).tenths, 90); // just touches ring 9
        assert_eq!(s(0.0, -80.0).tenths, 10); // ring 1 (77.75 + 2.25)
        assert_eq!(s(80.1, 0.0), Score { ring: 0, tenths: 0, inner_ten: false });
    }

    #[test]
    fn inner_ten() {
        assert!(s(4.75, 0.0).inner_ten); // 2.5 + 2.25
        assert!(!s(4.8, 0.0).inner_ten);
    }

    #[test]
    fn whole_numbers_match_decimals() {
        for i in 0..200 {
            let r = i as f32 * 0.43;
            let score = s(r * 0.6, r * 0.8);
            assert_eq!(score.tenths / 10, score.ring as u16, "at {r} mm");
        }
    }

    #[test]
    fn card_geometry() {
        let card = air_pistol();
        assert_eq!(card.size_mm(), egui::vec2(170.0, 170.0));
        assert_eq!(card.to_card(10.0, 5.0), egui::vec2(95.0, 80.0));
        assert!(card.render(256).is_some());
    }
}
