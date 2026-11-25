//! C5 (`app/CARDPHOTO.md`): where the camera was, from the registration, and what that means
//! for the holes.
//!
//! A card that doesn't lie quite flat (curled at the edges, a bump where it was shot) is
//! still mapped as a plane. Seen from straight above that hardly matters; seen at an angle, a
//! part of the card 0.5 mm off the table appears tan θ · 0.5 mm off. The first real photos
//! (2026-11-25) showed it: a hole's position repeated to about 0.045 mm per axis from straight
//! above, and worse the more the photo was tilted and the further the hole was from the
//! centre. `expected_error_mm` is that fit.

use nalgebra::Vector3;

use crate::Mapping;

/// The camera as the registration's homography places it.
#[derive(Clone, Copy, Debug)]
pub struct View {
    /// The focal length, px: from the homography when it tells (a tilted photo), else a phone's
    /// main lens (26 mm equivalent).
    pub f_px: f64,
    pub f_from_photo: bool,
    /// The camera's centre in card mm (x, y; height above the card).
    pub camera_mm: (f64, f64, f64),
    /// The angle between the card's normal and the line from the card's centre to the camera,
    /// degrees.
    pub tilt_deg: f64,
}

/// Up to this tilt a photo counts as taken from straight above, degrees.
pub const TILT_OK_DEG: f64 = 10.0;

/// A phone's main lens: 26 mm equivalent, f = 26 / 43.27 of the image's diagonal.
const F_DEFAULT_OF_DIAGONAL: f64 = 26.0 / 43.27;

/// The camera's pose from a registration's mapping (the lens's centre as the principal point,
/// square pixels).
pub fn view(map: &Mapping) -> View {
    let (cx, cy) = (map.lens.cx, map.lens.cy);
    let h = map.h;
    let col = |j: usize| (h[(0, j)] - cx * h[(2, j)], h[(1, j)] - cy * h[(2, j)], h[(2, j)]);
    let (a1, a2) = (col(0), col(1));
    // r1 ⟂ r2 and |r1| = |r2| for the columns K⁻¹h: two equations in 1/f², least squares.
    let dot = a1.0 * a2.0 + a1.1 * a2.1;
    let (n1, n2) = (a1.0 * a1.0 + a1.1 * a1.1, a2.0 * a2.0 + a2.1 * a2.1);
    let (u, v) = (a1.2 * a2.2, a1.2 * a1.2 - a2.2 * a2.2);
    let den = u * u + v * v;
    let diag = 2.0 * map.lens.norm;
    let f2 = if den > 0.0 {
        -(dot * u + (n1 - n2) * v) / den
    } else {
        -1.0
    };
    let f_est = f2.sqrt();
    let f0 = F_DEFAULT_OF_DIAGONAL * diag;
    let pose = |f: f64| {
        let m = |a: (f64, f64, f64)| Vector3::new(a.0 / f, a.1 / f, a.2);
        let (m1, m2, m3) = (m(a1), m(a2), m(col(2)));
        let lam = 0.5 * (m1.norm() + m2.norm());
        let (r1, r2, t) = (m1 / lam, m2 / lam, m3 / lam);
        let r3 = r1.cross(&r2);
        // The camera's centre: −Rᵀt.
        let c = -(Vector3::new(r1.dot(&t), r2.dot(&t), r3.dot(&t)));
        (c, c.x.hypot(c.y).atan2(c.z.abs()).to_degrees())
    };
    // Near straight above the equations say little about f: a phone's main lens then.
    let f_from_photo = f2 > 0.0 && (0.3 * diag..4.0 * diag).contains(&f_est) && pose(f0).1 > 8.0;
    let f = if f_from_photo { f_est } else { f0 };
    let (c, tilt_deg) = pose(f);
    View {
        f_px: f,
        f_from_photo,
        camera_mm: (c.x, c.y, c.z.abs()),
        tilt_deg,
    }
}

/// From straight above: a hole's scatter per axis, mm.
const ERR_FLAT_MM: f64 = 0.045;
/// Tilted: per axis, this times tan θ times the hole's distance from the centre over 80 mm
/// (fitted 0.34 mm on the first photos; taken a little larger, their cards vary).
const ERR_TILT_MM: f64 = 0.4;

/// The expected error of a hole's position from a photo at `tilt_deg`, for holes at an rms
/// distance `rho_mm` from the card's centre: the rms distance from the truth (both axes), mm.
pub fn expected_error_mm(tilt_deg: f64, rho_mm: f64) -> f64 {
    let t = tilt_deg.to_radians().tan();
    let axis = ERR_FLAT_MM.hypot(ERR_TILT_MM * t * rho_mm / 80.0);
    std::f64::consts::SQRT_2 * axis
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Lens;
    use nalgebra::{Matrix3, Rotation3};

    /// A camera at a known tilt: the view finds the tilt and the focal length.
    #[test]
    fn finds_the_tilt() {
        let (w, h) = (4032usize, 3024usize);
        let lens = Lens::none(w, h);
        let f = 0.6 * 2.0 * lens.norm;
        for tilt in [0.0f64, 5.0, 15.0, 30.0] {
            let r = Rotation3::from_axis_angle(&Vector3::z_axis(), 0.7)
                * Rotation3::from_axis_angle(&Vector3::x_axis(), tilt.to_radians());
            // The camera 300 mm from the card's centre, looking at it.
            let t = Vector3::new(0.0, 0.0, 300.0);
            let k = Matrix3::new(f, 0.0, lens.cx, 0.0, f, lens.cy, 0.0, 0.0, 1.0);
            let rm = r.matrix();
            let hm = k * Matrix3::from_columns(&[rm.column(0).into(), rm.column(1).into(), t]);
            let map = Mapping {
                h: hm / hm[(2, 2)],
                lens,
            };
            let v = view(&map);
            assert!((v.tilt_deg - tilt).abs() < 0.05, "{tilt}: {v:?}");
            assert!(
                (v.camera_mm.0.hypot(v.camera_mm.1).hypot(v.camera_mm.2) - 300.0).abs() < 1.0,
                "{v:?}"
            );
            if tilt >= 15.0 {
                assert!(v.f_from_photo && (v.f_px / f - 1.0).abs() < 1e-3, "{tilt}: {v:?}");
            }
        }
    }

    #[test]
    fn error_grows_with_the_tilt() {
        assert!((expected_error_mm(0.0, 50.0) - 0.0636).abs() < 0.001);
        assert!(expected_error_mm(25.0, 50.0) > 2.0 * expected_error_mm(5.0, 50.0));
    }
}
