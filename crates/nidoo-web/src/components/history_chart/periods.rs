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
mod tests;
