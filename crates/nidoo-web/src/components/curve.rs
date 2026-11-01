//! A monotone cubic line as SVG path text: shared by the solar curve and the history chart.

pub(crate) fn pt(x: f64, y: f64) -> String {
    format!("{x:.1} {y:.1}")
}

pub(crate) fn finite(v: f64) -> f64 {
    if v.is_finite() { v } else { 0.0 }
}

/// The tangent at the middle point of three (d3's `curveMonotoneX`): zero at a peak or a trough,
/// never steeper than either side, so the curve stays between its points.
fn inner_slope(a: (f64, f64), b: (f64, f64), c: (f64, f64)) -> f64 {
    let (h0, h1) = (b.0 - a.0, c.0 - b.0);
    if h0 <= 0.0 || h1 <= 0.0 {
        return 0.0;
    }
    let (s0, s1) = ((b.1 - a.1) / h0, (c.1 - b.1) / h1);
    if s0 * s1 <= 0.0 {
        return 0.0;
    }
    let p = (s0 * h1 + s1 * h0) / (h0 + h1);
    finite(2.0 * s0.signum() * s0.abs().min(s1.abs()).min(0.5 * p.abs()))
}

/// The tangent at an end of the line, from its neighbour's tangent `t`.
fn end_slope(a: (f64, f64), b: (f64, f64), t: f64) -> f64 {
    let h = b.0 - a.0;
    if h > 0.0 {
        finite((3.0 * (b.1 - a.1) / h - t) / 2.0)
    } else {
        t
    }
}

/// The tangent at every point of `pts` (x ascending): a monotone cubic's slopes.
fn slopes(pts: &[(f64, f64)]) -> Vec<f64> {
    let n = pts.len();
    let mut t = vec![0.0; n];
    for i in 1..n.saturating_sub(1) {
        t[i] = inner_slope(pts[i - 1], pts[i], pts[i + 1]);
    }
    if n == 2 {
        let h = pts[1].0 - pts[0].0;
        let s = if h > 0.0 {
            (pts[1].1 - pts[0].1) / h
        } else {
            0.0
        };
        t = vec![finite(s); 2];
    } else if n > 2 {
        t[0] = end_slope(pts[0], pts[1], t[1]);
        t[n - 1] = end_slope(pts[n - 2], pts[n - 1], t[n - 2]);
    }
    t
}

/// The smooth line through `pts` after its first point, as ` C …` segments: a monotone cubic,
/// so it passes through every point and never overshoots them (no dip below zero, no false peak).
pub(crate) fn smooth_tail(pts: &[(f64, f64)]) -> String {
    let n = pts.len();
    if n < 2 {
        return String::new();
    }
    let t = slopes(pts);
    let mut out = String::new();
    for i in 0..n - 1 {
        let (a, b) = (pts[i], pts[i + 1]);
        let dx = (b.0 - a.0) / 3.0;
        out.push_str(&format!(
            " C {} {} {}",
            pt(a.0 + dx, a.1 + dx * t[i]),
            pt(b.0 - dx, b.1 - dx * t[i + 1]),
            pt(b.0, b.1)
        ));
    }
    out
}

/// The same curve as [`smooth_tail`] walked backwards, from the last point of `pts` (x ascending)
/// to the first, so a closed shape can come back along its lower edge. The path is expected to
/// already be at the last point.
pub(crate) fn smooth_tail_rev(pts: &[(f64, f64)]) -> String {
    let n = pts.len();
    if n < 2 {
        return String::new();
    }
    let t = slopes(pts);
    let mut out = String::new();
    for i in (0..n - 1).rev() {
        let (a, b) = (pts[i], pts[i + 1]);
        let dx = (b.0 - a.0) / 3.0;
        out.push_str(&format!(
            " C {} {} {}",
            pt(b.0 - dx, b.1 - dx * t[i + 1]),
            pt(a.0 + dx, a.1 + dx * t[i]),
            pt(a.0, a.1)
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn numbers(s: &str) -> Vec<f64> {
        s.replace(['M', 'L', 'C', 'Z'], " ")
            .split_whitespace()
            .map(|n| n.parse().unwrap())
            .collect()
    }

    #[test]
    fn a_single_point_or_none_has_no_tail() {
        assert_eq!(smooth_tail(&[(0.0, 1.0)]), "");
        assert_eq!(smooth_tail_rev(&[]), "");
    }

    #[test]
    fn the_backward_tail_ends_at_the_first_point_through_the_same_points() {
        let pts = [(0.0, 40.0), (25.0, 10.0), (50.0, 30.0), (75.0, 5.0)];
        let back = smooth_tail_rev(&pts);
        assert_eq!(back.matches(" C ").count(), 3, "{back}");
        assert!(back.trim_end().ends_with(&pt(0.0, 40.0)), "{back}");
        for (x, y) in &pts[..3] {
            assert!(back.contains(&pt(*x, *y)), "{back}");
        }
    }

    #[test]
    fn the_backward_tail_never_overshoots_its_points() {
        let pts = [(0.0, 40.0), (10.0, 40.0), (20.0, 5.0), (30.0, 5.0)];
        let back = smooth_tail_rev(&pts);
        for (k, seg) in back.split(" C ").skip(1).enumerate() {
            let i = pts.len() - 2 - k;
            let n = numbers(seg);
            let (lo, hi) = (pts[i].1.min(pts[i + 1].1), pts[i].1.max(pts[i + 1].1));
            for y in [n[1], n[3]] {
                assert!(y >= lo - 0.05 && y <= hi + 0.05, "segment {i}: {seg}");
            }
        }
    }
}
