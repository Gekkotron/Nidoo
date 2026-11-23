//! What the field tracks draw, as plain data: the shared x scale, the bands of a steps track,
//! their colours, the lines of a number track, and the words. The component only renders it.

use nidoo_api::FieldTrack;

use crate::components::history_chart::geom::{YScale, y_scale};

pub use crate::components::history_chart::periods::{
    Band, PALETTE, band_title, bands, number_text, scalar_text, track_label, x_frac,
};

/// Width of every strip's viewBox; the strip is stretched to its box.
pub const VIEW_W: f64 = 1000.0;
/// Height of a number strip's viewBox.
pub const NUMBER_H: f64 = 40.0;
/// A band narrower than this share of the window carries no text.
const MIN_LABEL_FRAC: f64 = 0.08;
/// A single bucket is drawn at least this wide (viewBox units), so it stays visible.
const MIN_TICK_W: f64 = 3.0;

/// Whether the band is wide enough to write its value inside.
pub fn fits_label(band: &Band) -> bool {
    band.x1 - band.x0 >= MIN_LABEL_FRAC
}

/// The line and band paths of a number track, in strip viewBox coordinates.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct NumberPaths {
    /// The mean line, one sub-path per run of consecutive buckets.
    pub line: String,
    /// The min–max band, one closed shape per run; empty when `spread` is false.
    pub band: String,
}

/// The y extent of a number track: a percentage always reads 0 to 100, anything else fits the
/// data with headroom.
pub fn track_scale(unit: Option<&str>, points: &[(i64, f64, f64, f64)]) -> YScale {
    if unit == Some("%") {
        YScale {
            min: 0.0,
            max: 100.0,
        }
    } else {
        y_scale(points)
    }
}

/// Lines for `points` (`[t, mean, min, max]`, one per `window` seconds) over `from..to`. A gap
/// of more than one and a half windows between two buckets breaks the line. A lone bucket is
/// drawn as a short flat stroke. `spread` also builds the min–max band.
pub fn number_paths(
    points: &[(i64, f64, f64, f64)],
    window: i64,
    from: i64,
    to: i64,
    scale: YScale,
    spread: bool,
) -> NumberPaths {
    let span = (scale.max - scale.min).max(f64::EPSILON);
    let y_of = |v: f64| ((1.0 - (v - scale.min) / span) * NUMBER_H).clamp(0.0, NUMBER_H);
    let x_of = |t: i64| x_frac(t, from, to) * VIEW_W;
    let mut runs: Vec<Vec<&(i64, f64, f64, f64)>> = Vec::new();
    for p in points {
        match runs.last_mut() {
            Some(run) if run.last().is_some_and(|q| p.0 - q.0 <= window + window / 2) => {
                run.push(p);
            }
            _ => runs.push(vec![p]),
        }
    }
    let mut out = NumberPaths::default();
    for run in runs {
        // (x, mean y, min y, max y)
        let mut pts: Vec<(f64, f64, f64, f64)> = run
            .iter()
            .map(|p| (x_of(p.0), y_of(p.1), y_of(p.2), y_of(p.3)))
            .collect();
        if pts.len() == 1 {
            let last = run[0];
            let w = (x_of(last.0 + window) - pts[0].0).max(MIN_TICK_W);
            let mut tail = pts[0];
            tail.0 += w;
            pts.push(tail);
        }
        for (i, p) in pts.iter().enumerate() {
            out.line.push_str(&format!(
                "{}{:.1},{:.1}",
                if i == 0 { 'M' } else { 'L' },
                p.0,
                p.1
            ));
        }
        if spread && pts.iter().any(|p| (p.2 - p.3).abs() > f64::EPSILON) {
            for (i, p) in pts.iter().enumerate() {
                out.band.push_str(&format!(
                    "{}{:.1},{:.1}",
                    if i == 0 { 'M' } else { 'L' },
                    p.0,
                    p.3
                ));
            }
            for p in pts.iter().rev() {
                out.band.push_str(&format!("L{:.1},{:.1}", p.0, p.2));
            }
            out.band.push('Z');
        }
    }
    out
}

/// Whether the reply asks the main chart to step aside: it carries the headline value itself.
pub fn has_headline(tracks: &[FieldTrack]) -> bool {
    tracks.iter().any(|t| t.field.is_none())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nidoo_core::flow::Scalar;

    fn text(s: &str) -> Option<Scalar> {
        Some(Scalar::Text(s.into()))
    }

    #[test]
    fn narrow_bands_carry_no_text() {
        let b = bands(&[(0, text("a")), (5, text("b"))], 0, 100);
        assert!(!fits_label(&b[0]));
        assert!(fits_label(&b[1]));
    }

    #[test]
    fn number_line_breaks_at_a_gap() {
        let scale = YScale {
            min: 0.0,
            max: 100.0,
        };
        let pts = [
            (0, 50.0, 50.0, 50.0),
            (60, 50.0, 50.0, 50.0),
            (600, 100.0, 100.0, 100.0),
            (660, 100.0, 100.0, 100.0),
        ];
        let p = number_paths(&pts, 60, 0, 1000, scale, false);
        assert_eq!(p.line.matches('M').count(), 2);
        assert_eq!(p.line, "M0.0,20.0L60.0,20.0M600.0,0.0L660.0,0.0");
        assert!(p.band.is_empty());
    }

    #[test]
    fn a_lone_bucket_is_a_flat_stroke() {
        let scale = YScale {
            min: 0.0,
            max: 100.0,
        };
        let p = number_paths(&[(500, 0.0, 0.0, 0.0)], 60, 0, 1000, scale, false);
        assert_eq!(p.line, "M500.0,40.0L560.0,40.0");
    }

    #[test]
    fn the_band_follows_max_then_min_back() {
        let scale = YScale {
            min: 0.0,
            max: 100.0,
        };
        let pts = [(0, 50.0, 25.0, 75.0), (500, 50.0, 25.0, 75.0)];
        let p = number_paths(&pts, 500, 0, 1000, scale, true);
        assert_eq!(p.band, "M0.0,10.0L500.0,10.0L500.0,30.0L0.0,30.0Z");
        let flat = number_paths(&[(0, 5.0, 5.0, 5.0)], 60, 0, 1000, scale, true);
        assert!(flat.band.is_empty());
    }

    #[test]
    fn no_points_no_paths() {
        let p = number_paths(&[], 60, 0, 100, YScale { min: 0.0, max: 1.0 }, true);
        assert_eq!(p, NumberPaths::default());
    }

    #[test]
    fn a_percentage_reads_zero_to_one_hundred() {
        let pts = [(0, 40.0, 40.0, 40.0)];
        let s = track_scale(Some("%"), &pts);
        assert_eq!((s.min, s.max), (0.0, 100.0));
        let other = track_scale(Some("°C"), &pts);
        assert!(other.min < 40.0 && other.max > 40.0);
    }

    #[test]
    fn a_headline_track_is_detected() {
        use nidoo_api::TrackData;
        let steps = |field: Option<&str>| FieldTrack {
            field: field.map(Into::into),
            unit: None,
            data: TrackData::Steps { changes: vec![] },
        };
        assert!(has_headline(&[steps(Some("battery")), steps(None)]));
        assert!(!has_headline(&[steps(Some("battery"))]));
        assert!(!has_headline(&[]));
    }
}
