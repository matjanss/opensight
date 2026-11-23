//! The card's printed features the registration uses (`app/assets/cards`): circles about the
//! centre, of known radius. ISSF 10 m air pistol: ring lines 1–6 dark on the white, the black
//! aiming disc's edge (ring 7's outer edge, 59.5 mm), ring lines 8–10 and the inner ten light
//! in the black.

/// How a printed circle shows along a line outwards from the centre.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A thin dark line on light paper.
    DarkLine,
    /// A thin light line in the black.
    LightLine,
    /// The black disc's edge: dark inside, light outside.
    Edge,
}

/// One printed circle: its stated radius (mm) and how it shows.
#[derive(Clone, Copy, Debug)]
pub struct Feature {
    pub radius_mm: f64,
    pub kind: Kind,
}

/// The printed card.
#[derive(Clone, Debug)]
pub struct CardModel {
    pub features: Vec<Feature>,
    /// The black disc's radius (the coarse find).
    pub black_radius_mm: f64,
    /// Half the card's side: its edges are at ±this from the centre (the outline).
    pub half_size_mm: f64,
    /// Where the white paper between the black disc and the next ring line is (the contrast).
    pub paper_band_mm: (f64, f64),
    /// Where the black between ring 8's line and the disc's edge is.
    pub ink_band_mm: (f64, f64),
}

/// The ISSF 10 m air pistol card (`assets/cards/issf-10m-air-pistol.*`): 170 × 170 mm.
pub fn issf_air_pistol() -> CardModel {
    let mut features: Vec<Feature> = [77.75, 69.75, 61.75, 53.75, 45.75, 37.75]
        .iter()
        .map(|&r| Feature {
            radius_mm: r,
            kind: Kind::DarkLine,
        })
        .collect();
    features.push(Feature {
        radius_mm: 29.75,
        kind: Kind::Edge,
    });
    features.extend([21.75, 13.75, 5.75, 2.5].iter().map(|&r| Feature {
        radius_mm: r,
        kind: Kind::LightLine,
    }));
    CardModel {
        features,
        black_radius_mm: 29.75,
        half_size_mm: 85.0,
        paper_band_mm: (31.5, 36.0),
        ink_band_mm: (23.5, 28.0),
    }
}
