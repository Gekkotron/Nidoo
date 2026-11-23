//! `FieldTracks`: under an entity's history chart, one row per value it reports (a vacuum's
//! activity, battery, fan speed…) on one shared time axis, so the values read side by side.

pub mod plan;

use std::cell::Cell;
use std::rc::Rc;

use leptos::prelude::*;
use nidoo_api::{FieldTrack, HistoryFields, HistoryRange, TrackData};
use nidoo_core::model::Kind;

use self::plan::{
    NUMBER_H, VIEW_W, band_title, bands, fits_label, has_headline, number_paths, number_text,
    scalar_text, track_label, track_scale,
};
use super::history_chart::format::tick_labels;
use super::history_chart::view::{is_bucketed, is_stale, near_end, refresh_period_s};
use crate::api::history::{HistoryApiError, fetch_fields};
use crate::components::now::Now;
use crate::i18n::Lang;
use crate::i18n::history::load_error;
use crate::i18n::use_lang;

/// Height of a steps strip's viewBox.
const STEPS_H: f64 = 28.0;

/// What the stack is currently showing.
#[derive(Clone, Debug)]
enum State {
    Loading,
    Ready(HistoryFields),
    /// Nothing to add: no extra values, or the main chart already says why there is no history.
    Hidden,
    Error(String),
}

/// What a fetch was made for: entity and range.
type FetchInputs = (String, HistoryRange);

/// `headline` is set to whether the reply carries the entity's headline value, which the main
/// chart cannot draw: the page hides that chart then.
#[component]
pub fn FieldTracks(
    #[prop(into)] entity_id: Signal<String>,
    #[prop(into)] range: Signal<HistoryRange>,
    #[prop(into)] kind: Signal<Kind>,
    headline: WriteSignal<bool>,
) -> impl IntoView {
    let state = RwSignal::new(State::Loading);
    let latest: Rc<Cell<u32>> = Rc::new(Cell::new(0));
    let now = use_context::<Now>();
    let tick = Memo::new(move |_| now.map(|n| n.0.get() / refresh_period_s(range.get())));

    {
        let latest = latest.clone();
        Effect::new(move |prev: Option<FetchInputs>| {
            let inputs = (entity_id.get(), range.get());
            let _ = tick.get();
            let refresh = prev.as_ref() == Some(&inputs);
            let my_req = latest.get() + 1;
            latest.set(my_req);
            if !refresh {
                state.set(State::Loading);
            }
            let (id, r) = inputs.clone();
            let latest_check = latest.clone();
            leptos::task::spawn_local(async move {
                let outcome = fetch_fields(&id, r).await;
                if is_stale(my_req, latest_check.get()) {
                    return;
                }
                match outcome {
                    Ok(fields) if fields.tracks.is_empty() => {
                        headline.set(false);
                        state.set(State::Hidden);
                    }
                    Ok(fields) => {
                        headline.set(has_headline(&fields.tracks));
                        state.set(State::Ready(fields));
                    }
                    Err(e) => {
                        // A failed refresh keeps what is drawn.
                        if !refresh || state.with_untracked(|s| matches!(s, State::Loading)) {
                            headline.set(false);
                            state.set(match e {
                                HistoryApiError::NotRecorded
                                | HistoryApiError::NoGridMeter
                                | HistoryApiError::Unavailable => State::Hidden,
                                HistoryApiError::BadRequest(m) | HistoryApiError::Other(m) => {
                                    State::Error(m)
                                }
                            });
                        }
                    }
                }
            });
            inputs
        });
    }

    let lang = use_lang();
    view! {
        <div class="field-tracks">
            {move || {
                let lang = lang();
                match state.get() {
                    State::Loading => view! {
                        <div class="field-tracks__shimmer" aria-busy="true"></div>
                    }.into_any(),
                    State::Hidden => ().into_any(),
                    State::Error(msg) => view! {
                        <p class="field-tracks__err">{load_error(lang, &msg)}</p>
                    }.into_any(),
                    State::Ready(fields) => {
                        render_tracks(lang, fields, range.get(), kind.get()).into_any()
                    }
                }
            }}
        </div>
    }
}

/// A fraction of the strip as a CSS percentage.
fn pct(frac: f64) -> String {
    format!("{:.2}%", frac * 100.0)
}

fn render_tracks(
    lang: Lang,
    fields: HistoryFields,
    range: HistoryRange,
    kind: Kind,
) -> impl IntoView {
    let HistoryFields { from, to, tracks } = fields;
    let ticks = tick_labels(lang, range, to);
    let grid: Vec<f64> = ticks
        .iter()
        .map(|(pos, _)| *pos)
        .filter(|pos| *pos < 1.0)
        .collect();
    let rows = tracks
        .into_iter()
        .map(|t| render_row(lang, t, range, kind, from, to, grid.clone()))
        .collect_view();
    view! {
        <div class="field-tracks__stack">
            {rows}
            <div class="field-tracks__row">
                <span></span>
                <div class="field-tracks__axis">
                    {ticks.into_iter().map(|(pos, l)| view! {
                        <span class:field-tracks__tick--near-end=near_end(pos)
                            style=format!("left: {}", pct(pos))>{l}</span>
                    }).collect_view()}
                </div>
                <span></span>
            </div>
        </div>
    }
}

/// Faint vertical lines at the time-axis ticks, in strip viewBox coordinates.
fn grid_lines(grid: &[f64], h: f64) -> impl IntoView {
    grid.iter()
        .map(|pos| {
            let x = format!("{}", pos * VIEW_W);
            view! { <line x1=x.clone() y1="0" x2=x y2=format!("{h}") class="field-tracks__grid" /> }
        })
        .collect_view()
}

fn render_row(
    lang: Lang,
    track: FieldTrack,
    range: HistoryRange,
    kind: Kind,
    from: i64,
    to: i64,
    grid: Vec<f64>,
) -> impl IntoView {
    let FieldTrack { field, unit, data } = track;
    let label = track_label(lang, kind, field.as_deref());
    let (strip, value, tall) = match data {
        TrackData::Number { window, points } => {
            let scale = track_scale(unit.as_deref(), &points);
            // Short ranges are raw samples: their min/max spread is noise, not a band.
            let paths = number_paths(&points, window, from, to, scale, is_bucketed(range));
            let latest = points
                .last()
                .map_or_else(String::new, |p| number_text(lang, p.1, unit.as_deref()));
            let strip = view! {
                <svg class="field-tracks__svg" viewBox=format!("0 0 {VIEW_W} {NUMBER_H}")
                    preserveAspectRatio="none" role="img" aria-label=label.clone()>
                    {grid_lines(&grid, NUMBER_H)}
                    <path d=paths.band class="field-tracks__spread" />
                    <path d=paths.line class="field-tracks__line" />
                </svg>
            }
            .into_any();
            (strip, latest, true)
        }
        TrackData::Steps { changes } => {
            let spans = bands(&changes, from, to);
            let words: Vec<String> = spans
                .iter()
                .map(|b| {
                    scalar_text(
                        lang,
                        kind,
                        field.as_deref(),
                        unit.as_deref(),
                        b.value.as_ref(),
                    )
                })
                .collect();
            let rects = spans
                .iter()
                .zip(&words)
                .filter_map(|(b, word)| {
                    let color = b.color?;
                    let title = band_title(lang, range, to, word, b);
                    Some(view! {
                        <rect x=format!("{}", b.x0 * VIEW_W) y="0"
                            width=format!("{}", (b.x1 - b.x0) * VIEW_W) height=format!("{STEPS_H}")
                            class=format!("field-tracks__band field-tracks__band--{color}")>
                            <title>{title}</title>
                        </rect>
                    })
                })
                .collect_view();
            let texts = spans
                .iter()
                .zip(words)
                .filter(|(b, _)| b.color.is_some() && fits_label(b))
                .map(|(b, word)| {
                    view! {
                        <span class="field-tracks__word"
                            style=format!("left: {}; width: {}", pct(b.x0), pct(b.x1 - b.x0))>{word}</span>
                    }
                })
                .collect_view();
            let strip = view! {
                <svg class="field-tracks__svg" viewBox=format!("0 0 {VIEW_W} {STEPS_H}")
                    preserveAspectRatio="none" role="img" aria-label=label.clone()>
                    {grid_lines(&grid, STEPS_H)}
                    {rects}
                </svg>
                <div class="field-tracks__words">{texts}</div>
            }
            .into_any();
            (strip, String::new(), false)
        }
    };
    view! {
        <div class="field-tracks__row">
            <span class="field-tracks__label">{label}</span>
            <div class="field-tracks__strip" class:field-tracks__strip--tall=tall>{strip}</div>
            <span class="field-tracks__value">{value}</span>
        </div>
    }
}
