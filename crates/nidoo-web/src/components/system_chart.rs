//! The chart behind a System tile: the server's line and the hub's, on one scale.

use leptos::prelude::*;
use nidoo_api::{HistoryRange, HistorySeries};

use crate::api::history::{HistoryApiError, fetch_series};
use crate::components::history_chart::format::tick_labels;
use crate::components::history_chart::geom::{
    NiceScale, SmoothPath, YScale, nice_scale, smooth_path, step_decimals, y_scale,
};
use crate::i18n::format as fmt;
use crate::i18n::history::{HistoryText, history_text, load_error, range_name};
use crate::i18n::use_lang;

const VIEW_W: f64 = 400.0;
const VIEW_H: f64 = 220.0;

/// What the chart is drawing: each line as `(time, mean, mean, mean)` points.
#[derive(Clone, Debug)]
enum State {
    Loading,
    Lines(Vec<(i64, f64, f64, f64)>, Option<Vec<(i64, f64, f64, f64)>>),
    NotRecorded,
    Unavailable,
    Error(String),
}

/// A series' points with the min and max folded onto the mean: the chart draws no band, so the
/// scale should not make room for one.
fn means(s: HistorySeries) -> Vec<(i64, f64, f64, f64)> {
    s.points
        .into_iter()
        .map(|(t, mean, _, _)| (t, mean, mean, mean))
        .collect()
}

/// Both lines' paths on one scale, with the y labels as `(fraction down from the top, text)` and
/// the gridlines as fractions down from the top.
struct Geometry {
    server: String,
    hub: Option<String>,
    labels: Vec<(f64, String)>,
    grid: Vec<f64>,
}

fn geometry(
    lang: crate::i18n::Lang,
    server: &[(i64, f64, f64, f64)],
    hub: Option<&[(i64, f64, f64, f64)]>,
    t_start: i64,
    now: i64,
) -> Geometry {
    let both: Vec<_> = server
        .iter()
        .chain(hub.unwrap_or_default())
        .copied()
        .collect();
    let (lo, hi) = both
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| {
            (lo.min(p.1), hi.max(p.1))
        });
    let base = y_scale(&both);
    let NiceScale { scale, ticks, step } = nice_scale(if hi > lo {
        YScale { min: lo, max: hi }
    } else {
        base
    });
    let path = |points: &[(i64, f64, f64, f64)]| {
        let SmoothPath { line, .. } = smooth_path(points, scale, t_start, now, VIEW_W, VIEW_H);
        line
    };
    let span = (scale.max - scale.min).max(f64::EPSILON);
    let at = |v: f64| 1.0 - (v - scale.min) / span;
    let decimals = step_decimals(step);
    Geometry {
        server: path(server),
        hub: hub.map(path),
        labels: ticks
            .iter()
            .map(|&v| (at(v), fmt::decimal_grouped(lang, v, decimals)))
            .collect(),
        grid: ticks.iter().map(|&v| at(v)).collect(),
    }
}

/// One series, or `None` when the hub has none to give (not an error: the line is left out).
fn settle(
    result: Result<HistorySeries, HistoryApiError>,
) -> Result<Vec<(i64, f64, f64, f64)>, HistoryApiError> {
    match result {
        Ok(s) => Ok(means(s)),
        Err(HistoryApiError::NotRecorded) => Ok(Vec::new()),
        Err(e) => Err(e),
    }
}

/// The `server` line and, when given, the `hub` line over `range`, each fetched from the hub's
/// history. `server_label` and `hub_label` name them in the key.
#[component]
pub fn SystemChart(
    server: String,
    hub: Option<String>,
    #[prop(into)] range: Signal<HistoryRange>,
    #[prop(into)] server_label: Signal<String>,
    #[prop(into)] hub_label: Signal<String>,
) -> impl IntoView {
    let state = RwSignal::new(State::Loading);
    // A late response must not overwrite the one for the range now on screen.
    let latest = StoredValue::new(0u32);
    Effect::new(move |_| {
        let r = range.get();
        let mine = latest.try_update_value(|n| {
            *n += 1;
            *n
        });
        state.set(State::Loading);
        let (server, hub) = (server.clone(), hub.clone());
        leptos::task::spawn_local(async move {
            let first = settle(fetch_series(&server, r).await);
            let second = match &hub {
                Some(id) => Some(settle(fetch_series(id, r).await)),
                None => None,
            };
            if latest.try_get_value() != mine {
                return;
            }
            let next = match (first, second.transpose()) {
                (Ok(s), Ok(h)) if s.is_empty() && h.as_ref().is_none_or(Vec::is_empty) => {
                    State::NotRecorded
                }
                (Ok(s), Ok(h)) => State::Lines(s, h.filter(|h| !h.is_empty())),
                (Err(e), _) | (_, Err(e)) => match e {
                    HistoryApiError::Unavailable => State::Unavailable,
                    HistoryApiError::NotRecorded | HistoryApiError::NoGridMeter => {
                        State::NotRecorded
                    }
                    HistoryApiError::BadRequest(m) | HistoryApiError::Other(m) => State::Error(m),
                },
            };
            state.try_set(next);
        });
    });
    let lang = use_lang();
    view! {
        <div class="history-chart system-chart">
            {move || {
                let lang = lang();
                match state.get() {
                    State::Loading => {
                        view! { <div class="history-chart__shimmer" aria-busy="true"></div> }
                            .into_any()
                    }
                    State::NotRecorded => {
                        view! {
                            <p class="history-chart__empty">
                                {history_text(lang, HistoryText::NotRecorded)}
                            </p>
                        }
                            .into_any()
                    }
                    State::Unavailable => {
                        view! {
                            <p class="history-chart__err">
                                {history_text(lang, HistoryText::Unavailable)}
                            </p>
                        }
                            .into_any()
                    }
                    State::Error(msg) => {
                        view! { <p class="history-chart__err">{load_error(lang, &msg)}</p> }
                            .into_any()
                    }
                    State::Lines(s, h) => {
                        let r = range.get();
                        let now = (crate::energy::now_ms() / 1000) as i64;
                        let g = geometry(lang, &s, h.as_deref(), now - r.span_secs(), now);
                        let xticks = tick_labels(lang, r, now);
                        let has_hub = g.hub.is_some();
                        let has_server = !s.is_empty();
                        view! {
                            <div class="history-chart__inset">
                                <div class="history-chart__plot">
                                    <svg
                                        class="history-chart__svg"
                                        viewBox=format!("0 0 {VIEW_W} {VIEW_H}")
                                        preserveAspectRatio="none"
                                        role="img"
                                    >
                                        {g
                                            .grid
                                            .iter()
                                            .map(|y| {
                                                view! {
                                                    <line
                                                        x1="0"
                                                        y1=(y * VIEW_H).to_string()
                                                        x2=VIEW_W.to_string()
                                                        y2=(y * VIEW_H).to_string()
                                                        class="history-chart__grid"
                                                    />
                                                }
                                            })
                                            .collect::<Vec<_>>()}
                                        {has_server
                                            .then(|| {
                                                view! {
                                                    <path
                                                        d=g.server
                                                        class="history-chart__line system-chart__server"
                                                        fill="none"
                                                    />
                                                }
                                            })}
                                        {g
                                            .hub
                                            .map(|d| {
                                                view! {
                                                    <path
                                                        d=d
                                                        class="history-chart__line system-chart__hub"
                                                        fill="none"
                                                    />
                                                }
                                            })}
                                    </svg>
                                    <div class="history-chart__axis-y">
                                        {g
                                            .labels
                                            .into_iter()
                                            .map(|(y, l)| {
                                                view! {
                                                    <span style=format!("top: {:.2}%", y * 100.0)>{l}</span>
                                                }
                                            })
                                            .collect::<Vec<_>>()}
                                    </div>
                                </div>
                                <div class="history-chart__axis-x">
                                    {xticks
                                        .into_iter()
                                        .map(|(pos, l)| {
                                            view! {
                                                <span style=format!("left: {}%", pos * 100.0)>{l}</span>
                                            }
                                        })
                                        .collect::<Vec<_>>()}
                                </div>
                            </div>
                            <div class="history-chart__legend">
                                <span>{range_name(lang, r)}</span>
                                <span class="history-chart__key">
                                    {has_server
                                        .then(|| {
                                            view! {
                                                <span class="history-chart__key-item">
                                                    <i
                                                        class="history-chart__key-line system-chart__key--server"
                                                        aria-hidden="true"
                                                    ></i>
                                                    {move || server_label.get()}
                                                </span>
                                            }
                                        })}
                                    {has_hub
                                        .then(|| {
                                            view! {
                                                <span class="history-chart__key-item">
                                                    <i
                                                        class="history-chart__key-line system-chart__key--hub"
                                                        aria-hidden="true"
                                                    ></i>
                                                    {move || hub_label.get()}
                                                </span>
                                            }
                                        })}
                                </span>
                            </div>
                        }
                            .into_any()
                    }
                }
            }}
        </div>
    }
}
