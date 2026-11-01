//! Colour conversions to the app's hue/saturation: `h` in degrees `0..360`, `s` in `0..=100`.

use crate::model::Hs;

/// Hue and saturation of an sRGB colour.
pub fn rgb_to_hs(r: u8, g: u8, b: u8) -> Hs {
    let (r, g, b) = (
        f64::from(r) / 255.0,
        f64::from(g) / 255.0,
        f64::from(b) / 255.0,
    );
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    if delta <= f64::EPSILON || max <= f64::EPSILON {
        return Hs { h: 0.0, s: 0.0 };
    }
    let sector = if max == r {
        (g - b) / delta
    } else if max == g {
        (b - r) / delta + 2.0
    } else {
        (r - g) / delta + 4.0
    };
    let h = (60.0 * sector).rem_euclid(360.0);
    let s = delta / max * 100.0;
    Hs {
        h: round3(h),
        s: round3(s.clamp(0.0, 100.0)),
    }
}

/// Hue and saturation of a CIE 1931 `xy` chromaticity: the point is converted to linear
/// sRGB at full luminance (wide RGB D65 matrix), gamma-corrected, normalised so the largest
/// component is 1, then read as RGB. Degenerate input (`y <= 0`, non-finite) is white.
pub fn xy_to_hs(x: f64, y: f64) -> Hs {
    if !x.is_finite() || !y.is_finite() || y <= 0.0 {
        return Hs { h: 0.0, s: 0.0 };
    }
    let big_y = 1.0;
    let big_x = big_y / y * x;
    let big_z = big_y / y * (1.0 - x - y);
    let r = big_x * 1.656_492 - big_y * 0.354_851 - big_z * 0.255_038;
    let g = -big_x * 0.707_196 + big_y * 1.655_397 + big_z * 0.036_152;
    let b = big_x * 0.051_713 - big_y * 0.121_364 + big_z * 1.011_530;
    let gamma = |c: f64| {
        let c = if c <= 0.003_130_8 {
            12.92 * c
        } else {
            1.055 * c.powf(1.0 / 2.4) - 0.055
        };
        c.max(0.0)
    };
    let (r, g, b) = (gamma(r), gamma(g), gamma(b));
    let max = r.max(g).max(b);
    let (r, g, b) = if max > 1.0 {
        (r / max, g / max, b / max)
    } else {
        (r, g, b)
    };
    let to_u8 = |c: f64| (c.clamp(0.0, 1.0) * 255.0).round() as u8;
    rgb_to_hs(to_u8(r), to_u8(g), to_u8(b))
}

fn round3(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}
