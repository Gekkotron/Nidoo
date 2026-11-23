//! The periods of a value over a window: bands that hold, merged, coloured by first appearance
//! with the longest-held value as a quiet baseline, plus the words that name them.

use nidoo_core::flow::Scalar;
use nidoo_core::model::Kind;

use crate::format::DASH;
use crate::i18n::Lang;
use crate::i18n::flows::field_label;
use crate::i18n::format as fmt;
use crate::i18n::history::{HistoryText, history_text};
use crate::i18n::states::{StateCtx, on_off, state_word};

/// How many accent colours a value legend cycles through.
pub const PALETTE: usize = 5;

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
    /// Index into the palette; `None` for an unknown stretch or the baseline.
    pub color: Option<usize>,
}

/// The key two values share when they are the same value.
pub fn value_key(v: &Scalar) -> String {
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

/// `bands`, with adjacent equal values merged and recoloured: unknown and the baseline have no
/// colour, every other value takes a palette slot by first appearance, skipping the baseline.
pub fn periods(changes: &[(i64, Option<Scalar>)], from: i64, to: i64) -> Vec<Band> {
    let mut merged: Vec<Band> = Vec::new();
    for b in bands(changes, from, to) {
        match merged.last_mut() {
            Some(last) if last.value.as_ref().map(value_key) == b.value.as_ref().map(value_key) => {
                last.x1 = b.x1;
                last.end = b.end;
            }
            _ => merged.push(b),
        }
    }
    let base = baseline(&merged).as_ref().map(value_key);
    let mut seen: Vec<String> = Vec::new();
    for b in &mut merged {
        b.color = b.value.as_ref().map(value_key).and_then(|key| {
            if Some(&key) == base.as_ref() {
                return None;
            }
            let at = seen.iter().position(|k| *k == key).unwrap_or_else(|| {
                seen.push(key);
                seen.len() - 1
            });
            Some(at % PALETTE)
        });
    }
    merged
}

/// The value held longest (unknown never counts; ties go to the first seen). `None` unless at
/// least two distinct known values appear: a lone value is the whole story, not a baseline.
pub fn baseline(bands: &[Band]) -> Option<Scalar> {
    let mut totals: Vec<(String, Scalar, i64)> = Vec::new();
    for b in bands {
        let Some(v) = &b.value else { continue };
        let key = value_key(v);
        match totals.iter_mut().find(|t| t.0 == key) {
            Some(t) => t.2 += b.end - b.start,
            None => totals.push((key, v.clone(), b.end - b.start)),
        }
    }
    if totals.len() < 2 {
        return None;
    }
    let mut best = 0;
    for (i, t) in totals.iter().enumerate() {
        if t.2 > totals[best].2 {
            best = i;
        }
    }
    Some(totals.swap_remove(best).1)
}

/// The distinct known values in first-appearance order, with their colour (`None` = baseline).
pub fn legend(periods: &[Band]) -> Vec<(Scalar, Option<usize>)> {
    let mut out: Vec<(Scalar, Option<usize>)> = Vec::new();
    for b in periods {
        let Some(v) = &b.value else { continue };
        if !out.iter().any(|(o, _)| value_key(o) == value_key(v)) {
            out.push((v.clone(), b.color));
        }
    }
    out
}

/// The value of the last change at or before `t`.
pub fn value_at(changes: &[(i64, Option<Scalar>)], t: i64) -> Option<Scalar> {
    changes
        .iter()
        .take_while(|(at, _)| *at <= t)
        .last()
        .and_then(|(_, v)| v.clone())
}

/// The value that names the span `start..end`: among the coloured values present in it, the one
/// held longest; with none, the value in force at `start`. A short run inside a long bucket
/// still names the bucket.
pub fn word_in(
    changes: &[(i64, Option<Scalar>)],
    periods: &[Band],
    start: i64,
    end: i64,
) -> Option<Scalar> {
    let mut totals: Vec<(String, Scalar, i64)> = Vec::new();
    for b in periods.iter().filter(|b| b.color.is_some()) {
        let Some(v) = &b.value else { continue };
        let held = b.end.min(end) - b.start.max(start);
        if held <= 0 {
            continue;
        }
        let key = value_key(v);
        match totals.iter_mut().find(|t| t.0 == key) {
            Some(t) => t.2 += held,
            None => totals.push((key, v.clone(), held)),
        }
    }
    let mut best: Option<&(String, Scalar, i64)> = None;
    for t in &totals {
        if best.is_none_or(|b| t.2 > b.2) {
            best = Some(t);
        }
    }
    match best {
        Some(t) => Some(t.1.clone()),
        None => value_at(changes, start),
    }
}

/// The period under `frac` of the window, widened by `tol` on each side so a sliver drawn wider
/// than it lasted can be hit: a coloured period wins over the baseline.
pub fn band_near(periods: &[Band], frac: f64, tol: f64) -> Option<&Band> {
    let hit = |b: &&Band, t: f64| frac >= b.x0 - t && frac <= b.x1 + t;
    periods
        .iter()
        .find(|b| b.color.is_some() && hit(b, tol))
        .or_else(|| periods.iter().find(|b| hit(b, 0.0)))
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
    fn adjacent_equal_values_merge() {
        let p = periods(
            &[
                (0, text("docked")),
                (10, text("docked")),
                (50, text("cleaning")),
                (60, text("docked")),
            ],
            0,
            100,
        );
        assert_eq!(p.len(), 3);
        assert_eq!((p[0].start, p[0].end), (0, 50));
    }

    #[test]
    fn the_longest_value_is_the_uncoloured_baseline() {
        // docked 0-90 (90 s) vs cleaning 90-100 (10 s)
        let p = periods(&[(0, text("docked")), (90, text("cleaning"))], 0, 100);
        assert_eq!(p[0].color, None);
        assert_eq!(
            p[1].color,
            Some(0),
            "first non-baseline value takes palette slot 0"
        );
        assert_eq!(baseline(&p), text("docked"));
    }

    #[test]
    fn a_single_value_has_no_baseline_and_is_coloured() {
        let p = periods(&[(0, text("turbo"))], 0, 100);
        assert_eq!(baseline(&p), None);
        assert_eq!(p[0].color, Some(0));
    }

    #[test]
    fn baseline_ties_go_to_the_first_seen_and_unknown_never_counts() {
        let p = periods(&[(0, None), (60, text("a")), (80, text("b"))], 0, 100);
        // null 60 s ignored; a 20 s, b 20 s -> a
        assert_eq!(baseline(&p), text("a"));
        assert_eq!(p[0].color, None);
    }

    #[test]
    fn legend_lists_each_value_once_in_order() {
        let p = periods(
            &[
                (0, text("docked")),
                (40, text("cleaning")),
                (45, text("docked")),
                (90, text("error")),
            ],
            0,
            100,
        );
        assert_eq!(
            legend(&p),
            vec![
                (Scalar::Text("docked".into()), None),
                (Scalar::Text("cleaning".into()), Some(0)),
                (Scalar::Text("error".into()), Some(1)),
            ]
        );
    }

    #[test]
    fn all_unknown_has_no_periods_or_legend() {
        let p = periods(&[(0, None)], 0, 100);
        assert!(p.iter().all(|b| b.color.is_none()));
        assert!(legend(&p).is_empty());
    }

    #[test]
    fn value_at_reads_the_change_in_force() {
        let c = [(0, text("a")), (50, text("b"))];
        assert_eq!(value_at(&c, 49), text("a"));
        assert_eq!(value_at(&c, 50), text("b"));
        assert_eq!(value_at(&c, -1), None);
    }

    #[test]
    fn a_short_run_names_a_long_bucket() {
        // Docked for a day, with a 9 minute cleaning inside the 1 h bucket 3600..7200.
        let c = [
            (0, text("docked")),
            (4000, text("cleaning")),
            (4540, text("docked")),
        ];
        let p = periods(&c, 0, 86_400);
        assert_eq!(word_in(&c, &p, 3600, 7200), text("cleaning"));
    }

    #[test]
    fn with_no_coloured_value_the_word_in_force_names_the_span() {
        let c = [(0, text("docked")), (4000, text("cleaning"))];
        let p = periods(&c, 0, 86_400);
        // Before the cleaning: only the baseline is present.
        assert_eq!(word_in(&c, &p, 0, 3600), text("docked"));
        // Nothing coloured at all.
        let one = [(0, text("docked"))];
        assert_eq!(
            word_in(&one, &periods(&one, 0, 100), 10, 20),
            text("docked")
        );
    }

    #[test]
    fn the_longest_coloured_value_wins() {
        let c = [
            (0, text("docked")),
            (100, text("cleaning")),
            (200, text("returning")),
            (500, text("docked")),
            (10_000, text("docked")),
        ];
        let p = periods(&c, 0, 20_000);
        assert_eq!(word_in(&c, &p, 0, 1000), text("returning"));
    }

    #[test]
    fn a_widened_hover_finds_a_sliver_over_the_baseline() {
        let c = [
            (0, text("docked")),
            (500, text("cleaning")),
            (503, text("docked")),
        ];
        let p = periods(&c, 0, 1000);
        let at = |f: f64, tol: f64| band_near(&p, f, tol).and_then(|b| b.value.clone());
        assert_eq!(at(0.51, 0.0), text("docked"));
        assert_eq!(at(0.51, 0.01), text("cleaning"));
        assert_eq!(at(0.2, 0.01), text("docked"));
    }
}
