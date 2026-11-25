//! OpenSights card photos (PLAN D13, `app/CARDPHOTO.md`): a phone photo of the shot card,
//! registered from its printed rings alone, so card millimetres map to pixels; then the
//! holes (step C2). Pure Rust, no UI, so the app uses it now and the range server later.
//!
//! - [`Gray`]: a greyscale image, 0..1.
//! - [`Lens`], [`Mapping`]: card mm ↔ image px: a homography and radial lens distortion.
//! - [`card`]: the printed features of the card (the rings, the black disc's edge).
//! - [`detect`]: the coarse find (the black disc), the ring edges, the card's outline.
//! - [`register`]: the fit, and rectified images.
//! - [`holes`]: the holes, and their match to the shots.
//! - [`meta`]: metadata (EXIF, GPS) out of an uploaded photo, the image untouched.
//! - [`synth`]: synthetic photos with a known truth (the tests, and the app's demo).
//!
//! Coordinates: **card** mm from the card's centre, x right, y up, as printed (the numbers
//! upright); **image** px from the top left, x right, y down.

pub mod card;
pub mod detect;
pub mod gauge;
pub mod holes;
pub mod meta;
pub mod register;
pub mod synth;
pub mod view;

use nalgebra::{Matrix3, Vector3};

/// `f` over `items` on the machine's cores, the results in the items' order. Each item is
/// worked on alone, so the results are the same as one by one.
pub(crate) fn par_map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let threads = std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .min(items.len());
    if threads <= 1 {
        return items.iter().map(f).collect();
    }
    let next = std::sync::atomic::AtomicUsize::new(0);
    let parts: Vec<Vec<(usize, R)>> = std::thread::scope(|s| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                s.spawn(|| {
                    let mut done = Vec::new();
                    loop {
                        let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        if i >= items.len() {
                            break;
                        }
                        done.push((i, f(&items[i])));
                    }
                    done
                })
            })
            .collect();
        workers.into_iter().map(|w| w.join().unwrap()).collect()
    });
    let mut out: Vec<Option<R>> = (0..items.len()).map(|_| None).collect();
    for (i, r) in parts.into_iter().flatten() {
        out[i] = Some(r);
    }
    out.into_iter().map(Option::unwrap).collect()
}

/// A greyscale image, values 0 (black) to 1 (white), row by row.
#[derive(Clone, Debug)]
pub struct Gray {
    pub w: usize,
    pub h: usize,
    pub px: Vec<f32>,
}

impl Gray {
    pub fn new(w: usize, h: usize) -> Gray {
        Gray {
            w,
            h,
            px: vec![0.0; w * h],
        }
    }

    /// From 8-bit greyscale.
    pub fn from_luma8(w: usize, h: usize, data: &[u8]) -> Gray {
        Gray {
            w,
            h,
            px: data.iter().map(|&v| f32::from(v) / 255.0).collect(),
        }
    }

    /// A photo (JPEG, PNG) to greyscale.
    pub fn decode(bytes: &[u8]) -> Result<Gray, String> {
        let img = image::load_from_memory(bytes).map_err(|e| format!("not a readable image: {e}"))?;
        let g = img.to_luma8();
        Ok(Gray::from_luma8(
            g.width() as usize,
            g.height() as usize,
            g.as_raw(),
        ))
    }

    /// 8-bit greyscale (rectified images, patches).
    pub fn to_luma8(&self) -> Vec<u8> {
        self.px
            .iter()
            .map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
            .collect()
    }

    pub fn get(&self, x: usize, y: usize) -> f32 {
        self.px[y * self.w + x]
    }

    pub fn set(&mut self, x: usize, y: usize, v: f32) {
        self.px[y * self.w + x] = v;
    }

    /// Bilinear sample at (x, y) (pixel centres at integers); None outside.
    pub fn sample(&self, x: f64, y: f64) -> Option<f32> {
        if !(x >= 0.0 && y >= 0.0 && x <= (self.w - 1) as f64 && y <= (self.h - 1) as f64) {
            return None;
        }
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.w - 1), (y0 + 1).min(self.h - 1));
        let (fx, fy) = ((x - x0 as f64) as f32, (y - y0 as f64) as f32);
        let a = self.get(x0, y0) * (1.0 - fx) + self.get(x1, y0) * fx;
        let b = self.get(x0, y1) * (1.0 - fx) + self.get(x1, y1) * fx;
        Some(a * (1.0 - fy) + b * fy)
    }

    /// Smaller by an integer factor (box filter).
    pub fn downscale(&self, f: usize) -> Gray {
        let f = f.max(1);
        let (w, h) = (self.w / f, self.h / f);
        let mut out = Gray::new(w, h);
        let inv = 1.0 / (f * f) as f32;
        for y in 0..h {
            for x in 0..w {
                let mut s = 0.0;
                for dy in 0..f {
                    let row = (y * f + dy) * self.w + x * f;
                    s += self.px[row..row + f].iter().sum::<f32>();
                }
                out.set(x, y, s * inv);
            }
        }
        out
    }

    /// Gaussian blur (separable), sigma in pixels.
    pub fn blur(&self, sigma: f64) -> Gray {
        if sigma <= 0.0 {
            return self.clone();
        }
        let r = (3.0 * sigma).ceil() as isize;
        let k: Vec<f32> = (-r..=r)
            .map(|i| (-(i * i) as f64 / (2.0 * sigma * sigma)).exp() as f32)
            .collect();
        let ks: f32 = k.iter().sum();
        let k: Vec<f32> = k.iter().map(|v| v / ks).collect();
        let pass = |src: &Gray, horizontal: bool| -> Gray {
            let mut out = Gray::new(src.w, src.h);
            for y in 0..src.h {
                for x in 0..src.w {
                    let mut s = 0.0;
                    for (j, kv) in k.iter().enumerate() {
                        let d = j as isize - r;
                        let (xx, yy) = if horizontal {
                            ((x as isize + d).clamp(0, src.w as isize - 1) as usize, y)
                        } else {
                            (x, (y as isize + d).clamp(0, src.h as isize - 1) as usize)
                        };
                        s += kv * src.get(xx, yy);
                    }
                    out.set(x, y, s);
                }
            }
            out
        };
        pass(&pass(self, true), false)
    }
}

/// A colour image, values 0..1 per channel (R, G, B), row by row: the holes are found in
/// colour (a hole shows the table, which may be as dark as the ink or as light as the paper,
/// but rarely the same colour as both).
#[derive(Clone, Debug)]
pub struct Rgb {
    pub w: usize,
    pub h: usize,
    pub px: Vec<[f32; 3]>,
}

impl Rgb {
    pub fn new(w: usize, h: usize) -> Rgb {
        Rgb {
            w,
            h,
            px: vec![[0.0; 3]; w * h],
        }
    }

    /// A photo (JPEG, PNG).
    pub fn decode(bytes: &[u8]) -> Result<Rgb, String> {
        let img = image::load_from_memory(bytes).map_err(|e| format!("not a readable image: {e}"))?;
        let c = img.to_rgb8();
        let px = c
            .as_raw()
            .chunks(3)
            .map(|p| {
                [
                    f32::from(p[0]) / 255.0,
                    f32::from(p[1]) / 255.0,
                    f32::from(p[2]) / 255.0,
                ]
            })
            .collect();
        Ok(Rgb {
            w: c.width() as usize,
            h: c.height() as usize,
            px,
        })
    }

    /// Luminance (the registration works in grey).
    pub fn to_gray(&self) -> Gray {
        Gray {
            w: self.w,
            h: self.h,
            px: self
                .px
                .iter()
                .map(|p| 0.299 * p[0] + 0.587 * p[1] + 0.114 * p[2])
                .collect(),
        }
    }

    /// Bilinear sample; None outside.
    pub fn sample(&self, x: f64, y: f64) -> Option<[f32; 3]> {
        if !(x >= 0.0 && y >= 0.0 && x <= (self.w - 1) as f64 && y <= (self.h - 1) as f64) {
            return None;
        }
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.w - 1), (y0 + 1).min(self.h - 1));
        let (fx, fy) = ((x - x0 as f64) as f32, (y - y0 as f64) as f32);
        let g = |i: usize, j: usize| self.px[j * self.w + i];
        let (a, b, c, d) = (g(x0, y0), g(x1, y0), g(x0, y1), g(x1, y1));
        let mut out = [0.0; 3];
        for k in 0..3 {
            let top = a[k] * (1.0 - fx) + b[k] * fx;
            let bot = c[k] * (1.0 - fx) + d[k] * fx;
            out[k] = top * (1.0 - fy) + bot * fy;
        }
        Some(out)
    }

    /// The colour at a card point through a mapping.
    pub fn sample_at(&self, map: &Mapping, q: (f64, f64)) -> Option<[f32; 3]> {
        let p = map.to_image(q);
        self.sample(p.0, p.1)
    }

    /// Gaussian blur, channel by channel.
    pub fn blur(&self, sigma: f64) -> Rgb {
        if sigma <= 0.0 {
            return self.clone();
        }
        let mut out = self.clone();
        for k in 0..3 {
            let ch = Gray {
                w: self.w,
                h: self.h,
                px: self.px.iter().map(|p| p[k]).collect(),
            }
            .blur(sigma);
            for (o, v) in out.px.iter_mut().zip(ch.px) {
                o[k] = v;
            }
        }
        out
    }
}

/// Radial lens distortion about the image centre: an image point p (as photographed) is,
/// without the lens's distortion, at c + (p - c) (1 + k1 r^2 + k2 r^4), r = |p - c| / norm.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lens {
    pub cx: f64,
    pub cy: f64,
    /// Half the image's diagonal: r is 1 in the corners.
    pub norm: f64,
    pub k1: f64,
    pub k2: f64,
}

impl Lens {
    /// No distortion, for an image of this size.
    pub fn none(w: usize, h: usize) -> Lens {
        let (cx, cy) = ((w as f64 - 1.0) / 2.0, (h as f64 - 1.0) / 2.0);
        Lens {
            cx,
            cy,
            norm: (cx * cx + cy * cy).sqrt(),
            k1: 0.0,
            k2: 0.0,
        }
    }

    fn factor(&self, dx: f64, dy: f64) -> f64 {
        let r2 = (dx * dx + dy * dy) / (self.norm * self.norm);
        1.0 + self.k1 * r2 + self.k2 * r2 * r2
    }

    /// The photographed point without the lens's distortion.
    pub fn undistort(&self, p: (f64, f64)) -> (f64, f64) {
        let (dx, dy) = (p.0 - self.cx, p.1 - self.cy);
        let f = self.factor(dx, dy);
        (self.cx + dx * f, self.cy + dy * f)
    }

    /// Where the lens puts an undistorted point (the inverse, by fixed-point iteration).
    pub fn distort(&self, u: (f64, f64)) -> (f64, f64) {
        let (ux, uy) = (u.0 - self.cx, u.1 - self.cy);
        let (mut dx, mut dy) = (ux, uy);
        for _ in 0..30 {
            let f = self.factor(dx, dy);
            let (nx, ny) = (ux / f, uy / f);
            let done = (nx - dx).abs() + (ny - dy).abs() < 1e-9;
            dx = nx;
            dy = ny;
            if done {
                break;
            }
        }
        (self.cx + dx, self.cy + dy)
    }
}

/// Card mm ↔ image px: a homography from card mm to the undistorted image, then the lens.
#[derive(Clone, Debug, PartialEq)]
pub struct Mapping {
    pub h: Matrix3<f64>,
    pub lens: Lens,
}

impl Mapping {
    /// A card point (mm) in the photo (px).
    pub fn to_image(&self, q: (f64, f64)) -> (f64, f64) {
        let v = self.h * Vector3::new(q.0, q.1, 1.0);
        self.lens.distort((v.x / v.z, v.y / v.z))
    }

    /// A photo point (px) on the card (mm); None if the homography is singular.
    pub fn to_card(&self, p: (f64, f64)) -> Option<(f64, f64)> {
        let inv = self.h.try_inverse()?;
        Some(Self::apply(&inv, self.lens.undistort(p)))
    }

    /// to_card with the inverse homography given (many points).
    pub fn to_card_with(&self, h_inv: &Matrix3<f64>, p: (f64, f64)) -> (f64, f64) {
        Self::apply(h_inv, self.lens.undistort(p))
    }

    fn apply(m: &Matrix3<f64>, p: (f64, f64)) -> (f64, f64) {
        let v = m * Vector3::new(p.0, p.1, 1.0);
        (v.x / v.z, v.y / v.z)
    }

    /// The same mapping with the card turned by `angle` (radians, counterclockwise) in its own
    /// plane: card coordinates q here are R(angle) q in the old mapping.
    pub fn rotated(&self, angle: f64) -> Mapping {
        let (s, c) = angle.sin_cos();
        let r = Matrix3::new(c, -s, 0.0, s, c, 0.0, 0.0, 0.0, 1.0);
        Mapping {
            h: self.h * r,
            lens: self.lens,
        }
    }
}
