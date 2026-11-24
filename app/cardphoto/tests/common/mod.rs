//! What the tests share: the app's card, rendered once.

pub mod layouts;

use std::sync::{Arc, OnceLock};

use cardphoto::Gray;
use cardphoto::synth::CardTexture;
use resvg::{tiny_skia, usvg};

pub const TEX_PX_PER_MM: f64 = 20.0;

/// The app's card, rendered once.
pub fn texture() -> &'static CardTexture {
    static TEX: OnceLock<CardTexture> = OnceLock::new();
    TEX.get_or_init(|| {
        let dir = env!("CARGO_MANIFEST_DIR");
        let svg = std::fs::read_to_string(format!("{dir}/../assets/cards/issf-10m-air-pistol.svg")).unwrap();
        let mut opt = usvg::Options::default();
        let fonts = Arc::make_mut(&mut opt.fontdb);
        fonts
            .load_font_data(std::fs::read(format!("{dir}/../assets/fonts/IBMPlexSans-Regular.ttf")).unwrap());
        fonts.set_sans_serif_family("IBM Plex Sans");
        let tree = usvg::Tree::from_str(&svg, &opt).unwrap();
        let size = (170.0 * TEX_PX_PER_MM) as u32;
        let mut pm = tiny_skia::Pixmap::new(size, size).unwrap();
        let s = TEX_PX_PER_MM as f32;
        resvg::render(&tree, tiny_skia::Transform::from_scale(s, s), &mut pm.as_mut());
        let lum: Vec<u8> = pm
            .data()
            .chunks(4)
            .map(|p| (0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32) as u8)
            .collect();
        CardTexture {
            img: Gray::from_luma8(size as usize, size as usize, &lum),
            px_per_mm: TEX_PX_PER_MM,
            size_mm: 170.0,
        }
    })
}
