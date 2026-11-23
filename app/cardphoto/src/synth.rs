//! Synthetic photos of a card with a known truth (`app/CARDPHOTO.md` C1, C2): the card's
//! render seen by a pinhole camera (tilt, rotation, distance), radial lens distortion, an
//! optional bend, holes that show the table, a table texture, uneven light, blur and noise.
//! For the tests, and for the app's demo until there are real photos.

use nalgebra::{Matrix3, Rotation3, Vector3};

use crate::{Gray, Lens, Mapping};

/// The rendered card: `px_per_mm` pixels per mm, covering the whole card (side `size_mm`),
/// row 0 at the card's top. The card's centre is at its middle.
pub struct CardTexture {
    pub img: Gray,
    pub px_per_mm: f64,
    pub size_mm: f64,
}

/// What to make.
#[derive(Clone, Debug)]
pub struct Params {
    pub w: usize,
    pub h: usize,
    /// Pixels per mm at the card's centre.
    pub px_per_mm: f64,
    /// The card's rotation in the photo, degrees, counterclockwise as seen.
    pub rotation_deg: f64,
    /// Tilts about the card's x and y axes, degrees.
    pub tilt_x_deg: f64,
    pub tilt_y_deg: f64,
    /// Where the card's centre is, as a fraction of the image from its centre.
    pub offset: (f64, f64),
    /// The camera's distance, in units of the card's side (perspective: closer is stronger).
    pub distance: f64,
    /// Radial distortion (the `Lens` model).
    pub k1: f64,
    pub k2: f64,
    /// A bend: the card's edge this many mm off its plane (a cylinder along y).
    pub bend_mm: f64,
    /// Holes: card mm (centre origin, y up) and diameter mm.
    pub holes: Vec<(f64, f64)>,
    pub hole_d_mm: f64,
    /// The table's mean level (0..1) and its texture's strength.
    pub table: f32,
    pub table_texture: f32,
    /// Light: ±this fraction across the image.
    pub light: f32,
    pub blur_px: f64,
    pub noise: f32,
    pub seed: u64,
}

impl Default for Params {
    fn default() -> Params {
        Params {
            w: 2400,
            h: 1800,
            px_per_mm: 9.0,
            rotation_deg: 0.0,
            tilt_x_deg: 0.0,
            tilt_y_deg: 0.0,
            offset: (0.0, 0.0),
            distance: 2.5,
            k1: 0.0,
            k2: 0.0,
            bend_mm: 0.0,
            holes: vec![],
            hole_d_mm: 4.5,
            table: 0.2,
            table_texture: 0.06,
            light: 0.0,
            blur_px: 0.0,
            noise: 0.0,
            seed: 1,
        }
    }
}

/// A small deterministic generator (xorshift64*).
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    pub fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    /// Uniform in [0, 1).
    pub fn uniform(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
    pub fn gauss(&mut self) -> f64 {
        let (u1, u2) = (self.uniform().max(1e-300), self.uniform());
        (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
    }
}

/// A smooth texture (value noise over a few scales), about -1..1.
fn table_noise(x: f64, y: f64, seed: u64) -> f64 {
    let hash = |i: i64, j: i64, o: u64| -> f64 {
        let mut h = (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
            ^ (j as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
            ^ seed
            ^ o;
        h ^= h >> 31;
        h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        h ^= h >> 29;
        (h >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
    };
    let mut v = 0.0;
    let mut amp = 0.5;
    let mut scale = 1.0 / 60.0;
    for o in 0..4u64 {
        let (fx, fy) = (x * scale, y * scale);
        let (i, j) = (fx.floor() as i64, fy.floor() as i64);
        let (tx, ty) = (fx - i as f64, fy - j as f64);
        let (sx, sy) = (tx * tx * (3.0 - 2.0 * tx), ty * ty * (3.0 - 2.0 * ty));
        let a = hash(i, j, o) * (1.0 - sx) + hash(i + 1, j, o) * sx;
        let b = hash(i, j + 1, o) * (1.0 - sx) + hash(i + 1, j + 1, o) * sx;
        v += amp * (a * (1.0 - sy) + b * sy);
        amp *= 0.5;
        scale *= 2.3;
    }
    v
}

/// The camera: card point (mm, y up, z towards the camera) -> undistorted image px.
struct Camera {
    r: Rotation3<f64>,
    t: Vector3<f64>,
    f: f64,
    c: (f64, f64),
}

impl Camera {
    fn project(&self, q: Vector3<f64>) -> (f64, f64) {
        let x = self.r * q + self.t;
        (self.c.0 + self.f * x.x / x.z, self.c.1 - self.f * x.y / x.z)
    }

    /// The homography of the card's plane (z = 0).
    fn homography(&self) -> Matrix3<f64> {
        let k = Matrix3::new(self.f, 0.0, self.c.0, 0.0, -self.f, self.c.1, 0.0, 0.0, 1.0);
        let m = self.r.matrix();
        let rt = Matrix3::from_columns(&[m.column(0).into(), m.column(1).into(), self.t]);
        k * rt
    }
}

/// A synthetic photo and its truth: the mapping of the flat card (exact unless bent).
pub fn photo(tex: &CardTexture, p: &Params) -> (Gray, Mapping) {
    let dist = p.distance * tex.size_mm;
    let f = p.px_per_mm * dist;
    let cx = (p.w as f64 - 1.0) / 2.0 + p.offset.0 * p.w as f64;
    let cy = (p.h as f64 - 1.0) / 2.0 + p.offset.1 * p.h as f64;
    let r = Rotation3::from_axis_angle(&Vector3::z_axis(), p.rotation_deg.to_radians())
        * Rotation3::from_axis_angle(&Vector3::x_axis(), p.tilt_x_deg.to_radians())
        * Rotation3::from_axis_angle(&Vector3::y_axis(), p.tilt_y_deg.to_radians());
    // The camera looks along +z at the card; the card's centre straight ahead, then offset.
    let cam = Camera {
        r,
        t: Vector3::new(0.0, 0.0, dist),
        f,
        c: (cx, cy),
    };
    let lens = Lens {
        k1: p.k1,
        k2: p.k2,
        ..Lens::none(p.w, p.h)
    };
    let map = Mapping {
        h: cam.homography(),
        lens,
    };
    let inv = map.h.try_inverse().expect("a camera's homography");
    let half = tex.size_mm / 2.0;
    let bend = |x: f64| p.bend_mm * (x / half) * (x / half);
    let mut out = Gray::new(p.w, p.h);
    let hole_r2 = (p.hole_d_mm / 2.0) * (p.hole_d_mm / 2.0);
    // Each pixel averages 3 x 3 points over its area, as a sensor's pixel does (no aliasing
    // of the thin ring lines).
    const SS: usize = 3;
    for y in 0..p.h {
        for x in 0..p.w {
            let mut sum = 0.0f32;
            for sj in 0..SS {
                for si in 0..SS {
                    let (px, py) = (
                        x as f64 + (si as f64 + 0.5) / SS as f64 - 0.5,
                        y as f64 + (sj as f64 + 0.5) / SS as f64 - 0.5,
                    );
                    let u = lens.undistort((px, py));
                    let flat = Mapping::apply_pub(&inv, u);
                    let mut q = flat;
                    if p.bend_mm != 0.0 {
                        // The bent card's point that lands here: fixed-point iteration.
                        for _ in 0..4 {
                            let qb = Mapping::apply_pub(&inv, cam.project(Vector3::new(q.0, q.1, bend(q.0))));
                            q = (q.0 + flat.0 - qb.0, q.1 + flat.1 - qb.1);
                        }
                    }
                    let table =
                        (p.table as f64 + p.table_texture as f64 * table_noise(px, py, p.seed)) as f32;
                    let in_hole = p
                        .holes
                        .iter()
                        .any(|h| (q.0 - h.0).powi(2) + (q.1 - h.1).powi(2) < hole_r2);
                    sum += if q.0.abs() < half && q.1.abs() < half && !in_hole {
                        let tx = (q.0 + half) * tex.px_per_mm - 0.5;
                        let ty = (half - q.1) * tex.px_per_mm - 0.5;
                        tex.img.sample(tx, ty).unwrap_or(table)
                    } else {
                        table
                    };
                }
            }
            let v = sum / (SS * SS) as f32;
            let light = 1.0 + p.light * ((x as f32 / p.w as f32 - 0.5) + 0.6 * (y as f32 / p.h as f32 - 0.5));
            out.set(x, y, v * light);
        }
    }
    let mut out = out.blur(p.blur_px);
    if p.noise > 0.0 {
        let mut rng = Rng::new(p.seed);
        for v in &mut out.px {
            *v = (*v + p.noise * rng.gauss() as f32).clamp(0.0, 1.0);
        }
    }
    (out, map)
}

impl Mapping {
    /// A homography applied to a point (for the synthetic photos).
    pub fn apply_pub(m: &Matrix3<f64>, p: (f64, f64)) -> (f64, f64) {
        let v = m * Vector3::new(p.0, p.1, 1.0);
        (v.x / v.z, v.y / v.z)
    }
}
