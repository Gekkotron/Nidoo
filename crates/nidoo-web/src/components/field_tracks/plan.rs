//! What the field tracks draw, as plain data: the shared x scale, the bands of a steps track,
//! their colours, the lines of a number track, and the words. The component only renders it.

use nidoo_api::{FieldTrack, HistoryRange};
use nidoo_core::flow::Scalar;
use nidoo_core::model::Kind;

use crate::components::history_chart::format::tooltip_time;
use crate::components::history_chart::geom::{YScale, y_scale};
use crate::format::DASH;
use crate::i18n::Lang;
use crate::i18n::flows::field_label;
use crate::i18n::format as fmt;
use crate::i18n::history::{HistoryText, history_text};
use crate::i18n::states::{StateCtx, on_off, state_word};

/// Width of every strip's viewBox; the strip is stretched to its box.
pub const VIEW_W: f64 = 1000.0;
/// Height of a number strip's viewBox.
pub const NUMBER_H: f64 = 40.0;
/// How many accent colours a steps track cycles through.
pub const PALETTE: usize = 5;
/// A band narrower than this share of the window carries no text.
const MIN_LABEL_FRAC: f64 = 0.08;
/// A single bucket is drawn at least this wide (viewBox units), so it stays visible.
const MIN_TICK_W: f64 = 3.0;

/// Where `t` falls across the window `from..to`, as a fraction clamped to `0..=1`.
pub fn x_frac(t: i64, from: i64, to: i64) -> f64 {
    ((t - from) as f64 / (to - from).max(1) as f64).clamp(0.0, 1.0)
}

/// One stretch of a steps track over which the value holds.
#[derive(Clone, Debug, PartialEq)]
pub struct Band {
    /// Start and end as fractions of the window.
    pub x0: f64,
    pub x1: f64,
    /// Start and end, unix seconds, clipped to the window.
    pub start: i64,
    pub end: i64,
    pub value: Option<Scalar>,
    /// Index into the track's palette; `None` for an unknown stretch.
    pub color: Option<usize>,
}

impl Band {
    /// Whether the band is wide enough to write its value inside.
    pub fn fits_label(&self) -> bool {
        self.x1 - self.x0 >= MIN_LABEL_FRAC
    }
}

/// The key two values share when they are the same value.
fn value_key(v: &Scalar) -> String {
    match v {
        Scalar::Bool(b) => format!("b:{b}"),
        Scalar::Number(n) => format!("n:{n}"),
        Scalar::Text(t) => format!("t:{t}"),
    }
}

/// The bands of `changes` over `from..to`: each value lasts until the next change or `to`. A
/// change before `from` starts at `from`, one at or after `to` or out of order is dropped, and
/// a stretch with no width is skipped. Colours go by first appearance of each distinct value,
/// so they stay stable within a track; an unknown stretch has none.
pub fn bands(changes: &[(i64, Option<Scalar>)], from: i64, to: i64) -> Vec<Band> {
    let mut seen: Vec<String> = Vec::new();
    let mut out: Vec<Band> = Vec::new();
    for (i, (t, value)) in changes.iter().enumerate() {
        let start = (*t).max(from);
        let end = changes.get(i + 1).map_or(to, |next| next.0.min(to));
        if start >= end {
            continue;
        }
        let color = value.as_ref().map(|v| {
            let key = value_key(v);
            let at = seen.iter().position(|k| *k == key).unwrap_or_else(|| {
                seen.push(key);
                seen.len() - 1
            });
            at % PALETTE
        });
        out.push(Band {
            x0: x_frac(start, from, to),
            x1: x_frac(end, from, to),
            start,
            end,
            value: value.clone(),
            color,
        });
    }
    out
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

/// A number as a track words it: whole when it reads whole, else one decimal; then the unit.
pub fn number_text(lang: Lang, v: f64, unit: Option<&str>) -> String {
    let decimals = usize::from((v - v.round()).abs() >= 0.05);
    fmt::with_unit(fmt::decimal_grouped(lang, v, decimals), unit)
}

/// The vocabulary a track's text values are worded in: the kind's own for the headline, the
/// generic one for a named field.
fn state_ctx(kind: Kind, headline: bool) -> StateCtx {
    if !headline {
        return StateCtx::Other;
    }
    match kind {
        Kind::Vacuum => StateCtx::Vacuum,
        Kind::Cover => StateCtx::Cover,
        Kind::Climate | Kind::WaterHeater => StateCtx::Hvac,
        Kind::AlarmPanel => StateCtx::Alarm,
        Kind::MediaPlayer => StateCtx::Media,
        _ => StateCtx::Choice,
    }
}

/// A steps track's value in words; "—" for an unknown one.
pub fn scalar_text(
    lang: Lang,
    kind: Kind,
    field: Option<&str>,
    unit: Option<&str>,
    value: Option<&Scalar>,
) -> String {
    match value {
        None => DASH.to_owned(),
        Some(Scalar::Bool(on)) => {
            on_off(lang, field.is_none() && kind == Kind::Light, *on).to_owned()
        }
        Some(Scalar::Number(n)) => number_text(lang, *n, unit),
        Some(Scalar::Text(t)) => state_word(lang, state_ctx(kind, field.is_none()), t),
    }
}

/// The row label: a field's name, or for the headline "Activity" on a vacuum and "State" else.
pub fn track_label(lang: Lang, kind: Kind, field: Option<&str>) -> String {
    match field {
        Some(name) => field_label(lang, name),
        None if kind == Kind::Vacuum => field_label(lang, "action"),
        None => history_text(lang, HistoryText::State).to_owned(),
    }
}

/// The hover text of a band: "Cleaning · 08:10 – 08:42".
pub fn band_title(lang: Lang, range: HistoryRange, now: i64, word: &str, band: &Band) -> String {
    format!(
        "{word} · {} – {}",
        tooltip_time(lang, range, band.start, now),
        tooltip_time(lang, range, band.end, now)
    )
}

/// Whether the reply asks the main chart to step aside: it carries the headline value itself.
pub fn has_headline(tracks: &[FieldTrack]) -> bool {
    tracks.iter().any(|t| t.field.is_none())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> Option<Scalar> {
        Some(Scalar::Text(s.into()))
    }

    #[test]
    fn x_frac_spans_the_window_and_clamps() {
        assert_eq!(x_frac(100, 100, 200), 0.0);
        assert_eq!(x_frac(150, 100, 200), 0.5);
        assert_eq!(x_frac(200, 100, 200), 1.0);
        assert_eq!(x_frac(50, 100, 200), 0.0);
        assert_eq!(x_frac(300, 100, 200), 1.0);
        assert_eq!(x_frac(5, 10, 10), 0.0);
    }

    #[test]
    fn bands_run_to_the_next_change_and_the_last_to_the_end() {
        let b = bands(
            &[
                (0, text("docked")),
                (50, text("cleaning")),
                (80, text("docked")),
            ],
            0,
            100,
        );
        assert_eq!(b.len(), 3);
        assert_eq!((b[0].x0, b[0].x1), (0.0, 0.5));
        assert_eq!((b[1].x0, b[1].x1), (0.5, 0.8));
        assert_eq!((b[2].x0, b[2].x1), (0.8, 1.0));
        assert_eq!((b[1].start, b[1].end), (50, 80));
    }

    #[test]
    fn a_value_keeps_its_colour_and_new_ones_take_the_next() {
        let b = bands(
            &[
                (0, text("docked")),
                (50, text("cleaning")),
                (80, text("docked")),
            ],
            0,
            100,
        );
        assert_eq!(b[0].color, Some(0));
        assert_eq!(b[1].color, Some(1));
        assert_eq!(b[2].color, Some(0));
    }

    #[test]
    fn colours_wrap_after_the_palette() {
        let changes: Vec<_> = (0..7)
            .map(|i| (i * 10, Some(Scalar::Number(i as f64))))
            .collect();
        let b = bands(&changes, 0, 70);
        assert_eq!(b[5].color, Some(0));
        assert_eq!(b[6].color, Some(1));
    }

    #[test]
    fn unknown_is_a_gap_without_colour_and_takes_no_palette_slot() {
        let b = bands(&[(0, None), (50, text("on"))], 0, 100);
        assert_eq!(b[0].color, None);
        assert_eq!(b[1].color, Some(0));
    }

    #[test]
    fn a_change_before_the_window_starts_at_its_edge() {
        let b = bands(&[(-500, text("a")), (40, text("b"))], 0, 100);
        assert_eq!((b[0].start, b[0].x0, b[0].x1), (0, 0.0, 0.4));
    }

    #[test]
    fn changes_at_or_after_the_end_and_empty_stretches_are_dropped() {
        let b = bands(&[(0, text("a")), (0, text("b")), (100, text("c"))], 0, 100);
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].value, text("b"));
        assert!(bands(&[], 0, 100).is_empty());
    }

    #[test]
    fn narrow_bands_carry_no_text() {
        let b = bands(&[(0, text("a")), (5, text("b"))], 0, 100);
        assert!(!b[0].fits_label());
        assert!(b[1].fits_label());
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
    fn numbers_read_whole_or_with_one_decimal() {
        assert_eq!(number_text(Lang::En, 52.0, Some("%")), "52\u{A0}%");
        assert_eq!(number_text(Lang::Fr, 21.5, Some("°C")), "21,5\u{A0}°C");
        assert_eq!(number_text(Lang::En, 3.0, None), "3");
    }

    #[test]
    fn values_are_worded_by_kind_for_the_headline_only() {
        let v = text("cleaning");
        assert_eq!(
            scalar_text(Lang::Fr, Kind::Vacuum, None, None, v.as_ref()),
            "Nettoyage"
        );
        assert_eq!(
            scalar_text(Lang::En, Kind::Vacuum, None, None, v.as_ref()),
            "Cleaning"
        );
        assert_eq!(
            scalar_text(
                Lang::En,
                Kind::Vacuum,
                Some("fan_speed"),
                None,
                text("turbo").as_ref()
            ),
            "Turbo"
        );
        assert_eq!(scalar_text(Lang::Fr, Kind::Vacuum, None, None, None), DASH);
    }

    #[test]
    fn booleans_use_the_on_off_words() {
        let on = Some(Scalar::Bool(true));
        assert_eq!(
            scalar_text(Lang::Fr, Kind::Light, None, None, on.as_ref()),
            on_off(Lang::Fr, true, true)
        );
        assert_eq!(
            scalar_text(Lang::Fr, Kind::Switch, None, None, on.as_ref()),
            on_off(Lang::Fr, false, true)
        );
    }

    #[test]
    fn labels_reuse_the_field_names_and_word_the_headline() {
        assert_eq!(
            track_label(Lang::Fr, Kind::Vacuum, Some("battery")),
            "Batterie"
        );
        assert_eq!(
            track_label(Lang::En, Kind::Vacuum, Some("fan_speed")),
            "Fan speed"
        );
        assert_eq!(track_label(Lang::Fr, Kind::Vacuum, None), "Activité");
        assert_eq!(track_label(Lang::En, Kind::Vacuum, None), "Activity");
        assert_eq!(track_label(Lang::Fr, Kind::Climate, None), "État");
        assert_eq!(
            track_label(Lang::En, Kind::Generic, Some("water_level")),
            "Water level"
        );
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
