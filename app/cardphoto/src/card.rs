//! The card's printed features the registration uses (`app/assets/cards`): circles about the
//! centre, of known radius (a line's: to its outer edge, ISSF). ISSF 10 m air pistol: ring lines 1–6 dark on the white, the black
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

/// One printed circle: its stated radius (mm), how it shows, and for a line, which of the
/// card's line offsets it shares (`CardModel::line_groups`).
#[derive(Clone, Copy, Debug)]
pub struct Feature {
    pub radius_mm: f64,
    pub kind: Kind,
    /// For a line: its offset group. None for the disc's edge (the reference, no offset).
    pub group: Option<usize>,
}

/// The printed card.
#[derive(Clone, Debug)]
pub struct CardModel {
    pub features: Vec<Feature>,
    /// How many line offsets the registration fits: where on a printed line its stated radius
    /// sits (ISSF: the line's outer edge, so its centre is half a line width inside). Lines
    /// printed alike share one; a card that prints its light and dark lines differently would
    /// have one per kind. One offset per group keeps the fit from moving a part of the card's
    /// rings on its own to hide a misfit elsewhere.
    pub line_groups: usize,
    /// The black disc's radius (the coarse find).
    pub black_radius_mm: f64,
    /// Half the card's side: its edges are at ±this from the centre (the outline).
    pub half_size_mm: f64,
    /// Where the white paper between the black disc and the next ring line is (the contrast).
    pub paper_band_mm: (f64, f64),
    /// Where the black between ring 8's line and the disc's edge is.
    pub ink_band_mm: (f64, f64),
    /// The printed ring numbers: upright, the same on the card's four axes (along its edges),
    /// centred at these distances from the centre.
    pub number_radii_mm: Vec<f64>,
}

/// The ISSF 10 m air pistol card (`assets/cards/issf-10m-air-pistol.*`): 170 × 170 mm.
pub fn issf_air_pistol() -> CardModel {
    let mut features: Vec<Feature> = [77.75, 69.75, 61.75, 53.75, 45.75, 37.75]
        .iter()
        .map(|&r| Feature {
            radius_mm: r,
            kind: Kind::DarkLine,
            group: Some(0),
        })
        .collect();
    features.push(Feature {
        radius_mm: 29.75,
        kind: Kind::Edge,
        group: None,
    });
    features.extend([21.75, 13.75, 5.75, 2.5].iter().map(|&r| Feature {
        radius_mm: r,
        kind: Kind::LightLine,
        group: Some(0),
    }));
    CardModel {
        features,
        // The light and dark lines are printed alike (a scan: both centred 0.09 mm inside
        // their stated radius): one offset.
        line_groups: 1,
        black_radius_mm: 29.75,
        half_size_mm: 85.0,
        paper_band_mm: (31.5, 36.0),
        ink_band_mm: (23.5, 28.0),
        // In the middle of the bands: 8 and 7 in the black, 6 to 1 on the white; 4.5 mm type
        // (a digit about 3.2 × 2.5 mm).
        number_radii_mm: vec![17.75, 25.75, 33.75, 41.75, 49.75, 57.75, 65.75, 73.75],
    }
}
