//! The `HistoryChart` Leptos component: four states (loading, empty, unavailable, error) and
//! the renderers for the two flavors (smooth line with hover, and daily bars with hover).

use std::cell::Cell;
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};

use leptos::html;
use leptos::prelude::*;
use nidoo_api::{EnergyKind, HistoryDaily, HistoryRange, HistorySeries};

use super::format::{state_axis_labels, state_value_label, tick_labels, tooltip_day, tooltip_time};
use super::geom::{
    Bar, NiceScale, SmoothPath, StepPath, YScale, daily_bars, nearest_index, nice_scale,
    plot_points, slot_index, smooth_path, step_decimals, step_path, tooltip_flips, value_decimals,
    y_scale,
};
use crate::api::history::{HistoryApiError, fetch_daily, fetch_series};
use crate::components::now::Now;
use crate::house::House;
use crate::i18n::format as fmt;
use crate::i18n::history::{
    HistoryText, counter_legend, history_text, kwh, load_error, min_max, range_name, total_line,
};
use crate::i18n::{Lang, use_lang};

/// Which shape of chart the entity needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flavor {
    Series,
    Counter,
}

/// What the chart is currently drawing.
#[derive(Clone, Debug)]
enum State {
    Loading,
    Series(HistorySeries),
    Bars(HistoryDaily),
    NotRecorded,
    Unavailable,
    Error(String),
}

/// A late response's `request_id` must match the latest issued one, or the response is dropped.
pub fn is_stale(my_req: u32, latest: u32) -> bool {
    my_req != latest
}

/// Days to ask `/daily` for, given a non-24h range.
fn daily_days_for(range: HistoryRange) -> u16 {
    match range {
        HistoryRange::Week => 7,
        HistoryRange::Month => 30,
        HistoryRange::Year => 365,
        HistoryRange::Hour | HistoryRange::SixHours | HistoryRange::Day => 1,
    }
}

/// What a fetch was made for: entity, range, flavor, power pair and the Retry count.
type FetchInputs = (String, HistoryRange, Flavor, Option<String>, u32);

/// Seconds between two refetches of a chart left open: a minute for the day ranges, whose
/// latest points move every minute, ten for the longer ones.
pub fn refresh_period_s(range: HistoryRange) -> u64 {
    match range {
        HistoryRange::Hour | HistoryRange::SixHours | HistoryRange::Day => 60,
        HistoryRange::Week | HistoryRange::Month | HistoryRange::Year => 600,
    }
}

#[component]
pub fn HistoryChart(
    #[prop(into)] entity_id: Signal<String>,
    #[prop(into)] range: Signal<HistoryRange>,
    #[prop(into)] flavor: Signal<Flavor>,
    #[prop(into)] kind: Signal<String>,
    #[prop(into)] unit: Signal<Option<String>>,
    #[prop(into)] power_pair: Signal<Option<String>>,
) -> impl IntoView {
    let state = RwSignal::new(State::Loading);
    let latest: Rc<Cell<u32>> = Rc::new(Cell::new(0));
    let retry_bump = RwSignal::new(0u32);

    // The shared clock, cut to this range's refresh period: an open chart refetches each time
    // it moves on. Off the app (no clock) the chart never refreshes.
    let now = use_context::<Now>();
    let tick = Memo::new(move |_| now.map(|n| n.0.get() / refresh_period_s(range.get())));

    // Fetch effect — refires on any input change (or Retry), and on each tick. A tick alone is
    // a refresh: the chart stays up while it loads.
    {
        let latest = latest.clone();
        Effect::new(move |prev: Option<FetchInputs>| {
            let inputs = (
                entity_id.get(),
                range.get(),
                flavor.get(),
                power_pair.get(),
                retry_bump.get(),
            );
            let _ = tick.get();
            let refresh = prev.as_ref() == Some(&inputs);
            let my_req = latest.get() + 1;
            latest.set(my_req);
            if !refresh {
                state.set(State::Loading);
            }

            let (id, r, f, pp, _) = inputs.clone();
            let latest_check = latest.clone();
            leptos::task::spawn_local(async move {
                let outcome = fetch_for(&id, r, f, pp.as_deref()).await;
                if is_stale(my_req, latest_check.get()) {
                    return;
                }
                match outcome {
                    FetchResult::Series(Ok(s)) => state.set(State::Series(s)),
                    FetchResult::Daily(Ok(d)) => state.set(State::Bars(d)),
                    FetchResult::Series(Err(e)) | FetchResult::Daily(Err(e)) => {
                        // A failed refresh keeps what is drawn; one that superseded the first
                        // fetch still has to end the shimmer.
                        if !refresh || state.with_untracked(|s| matches!(s, State::Loading)) {
                            state.set(map_error(e));
                        }
                    }
                }
            });
            inputs
        });
    }

    // A counter's day ranges draw its paired power sensor, so the labels take that one's unit.
    let house = use_context::<House>();
    let drawn_unit = Memo::new(move |_| {
        let id = entity_id.get();
        let plan = plan_fetch(&id, range.get(), flavor.get(), power_pair.get().as_deref());
        drawn_unit(&plan, &id, unit.get(), |other| {
            house.and_then(|h| {
                h.entities
                    .with(|m| m.get(other).and_then(|e| e.unit.clone()))
            })
        })
    });

    // Each render reads `state` and composes the right view.
    let kind_rx = kind;
    let unit_rx = drawn_unit;
    let range_rx = range;
    let lang = use_lang();
    view! {
        <div class="history-chart">
            {move || {
                // Read here, so a language flip redraws the labels without a refetch.
                let lang = lang();
                match state.get() {
                State::Loading => view! { <div class="history-chart__shimmer" aria-busy="true"></div> }.into_any(),
                State::NotRecorded => view! {
                    <p class="history-chart__empty">{history_text(lang, HistoryText::NotRecorded)}</p>
                }.into_any(),
                State::Unavailable => view! {
                    <p class="history-chart__err">{history_text(lang, HistoryText::Unavailable)}</p>
                }.into_any(),
                State::Error(msg) => {
                    let msg = msg.clone();
                    view! {
                        <div class="history-chart__err">
                            <p>{load_error(lang, &msg)}</p>
                            <button type="button" class="chip" on:click=move |_| retry_bump.update(|v| *v += 1)>
                                {history_text(lang, HistoryText::Retry)}
                            </button>
                        </div>
                    }.into_any()
                }
                State::Series(s) => render_series(lang, s, range_rx.get(), kind_rx.get(), unit_rx.get()).into_any(),
                State::Bars(d) => render_bars(lang, d, range_rx.get()).into_any(),
                }
            }}
        </div>
    }
}

enum FetchResult {
    Series(Result<HistorySeries, HistoryApiError>),
    Daily(Result<HistoryDaily, HistoryApiError>),
}

/// Which endpoint to call, with its arguments; pure routing decision for the current chart
/// state. Extracted from `fetch_for` so it is natively testable — the Review Focus items
/// (text-entity 404, counter without power pair on 24h) are branches here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchPlan {
    /// Call `/history/series?entity={0}&range={range}`.
    Series(String),
    /// Call `/history/daily?entity={0}&days={1}&kind=counter`.
    Daily(String, u16),
}

/// Decide which endpoint to fetch for the current `(flavor, range, power_pair)` triple.
pub fn plan_fetch(
    entity: &str,
    range: HistoryRange,
    flavor: Flavor,
    power_pair: Option<&str>,
) -> FetchPlan {
    match (flavor, range) {
        (Flavor::Series, _) => FetchPlan::Series(entity.to_owned()),
        (Flavor::Counter, HistoryRange::Hour | HistoryRange::SixHours | HistoryRange::Day) => {
            match power_pair {
                Some(pid) => FetchPlan::Series(pid.to_owned()),
                None => FetchPlan::Daily(entity.to_owned(), 1),
            }
        }
        (Flavor::Counter, _) => FetchPlan::Daily(entity.to_owned(), daily_days_for(range)),
    }
}

/// The unit of the line `plan` draws for `entity`: `unit` for the entity's own series, else the
/// drawn entity's (`unit_of`).
pub fn drawn_unit(
    plan: &FetchPlan,
    entity: &str,
    unit: Option<String>,
    unit_of: impl Fn(&str) -> Option<String>,
) -> Option<String> {
    match plan {
        FetchPlan::Series(drawn) if drawn != entity => unit_of(drawn),
        _ => unit,
    }
}

async fn fetch_for(
    entity: &str,
    range: HistoryRange,
    flavor: Flavor,
    power_pair: Option<&str>,
) -> FetchResult {
    match plan_fetch(entity, range, flavor, power_pair) {
        FetchPlan::Series(id) => FetchResult::Series(fetch_series(&id, range).await),
        FetchPlan::Daily(id, days) => {
            FetchResult::Daily(fetch_daily(&id, days, EnergyKind::Counter).await)
        }
    }
}

fn map_error(err: HistoryApiError) -> State {
    match err {
        HistoryApiError::NotRecorded | HistoryApiError::NoGridMeter => State::NotRecorded,
        HistoryApiError::Unavailable => State::Unavailable,
        HistoryApiError::BadRequest(m) | HistoryApiError::Other(m) => State::Error(m),
    }
}

/// SVG `preserveAspectRatio` value, not text. The plot is stretched to its box, so the overlays
/// positioned in % of that box stay on the curve.
const ASPECT: &str = "none";
const VIEW_W: f64 = 400.0;
const VIEW_H: f64 = 220.0;

/// Gradient ids come from a counter, so several charts on one page never share one.
static CHART_IDS: AtomicU32 = AtomicU32::new(0);

fn next_gradient_id() -> String {
    format!("hc-fill-{}", CHART_IDS.fetch_add(1, Ordering::Relaxed))
}

/// The pointer's x across `plot`, as a fraction of its width.
fn pointer_frac(plot: &NodeRef<html::Div>, ev: &web_sys::PointerEvent) -> Option<f64> {
    let rect = plot.get()?.get_bounding_client_rect();
    (rect.width() > 0.0)
        .then(|| ((f64::from(ev.client_x()) - rect.left()) / rect.width()).clamp(0.0, 1.0))
}

/// A fraction as a CSS percentage.
fn pct(frac: f64) -> String {
    format!("{:.2}%", frac * 100.0)
}

/// What the tooltip of one series point says.
#[derive(Clone, Debug, PartialEq)]
struct SeriesTip {
    time: String,
    value: String,
    /// "min 3,0 °C · max 21,5 °C", on the ranges whose points are buckets.
    band: Option<String>,
}

/// The tooltip text of point `p` (`t`, mean, min, max).
fn series_tip(
    lang: Lang,
    range: HistoryRange,
    kind: &str,
    unit: Option<&str>,
    decimals: usize,
    now: i64,
    p: &(i64, f64, f64, f64),
) -> SeriesTip {
    let value = state_value_label(lang, kind, p.1)
        .unwrap_or_else(|| fmt::with_unit(fmt::decimal_grouped(lang, p.1, decimals), unit));
    SeriesTip {
        time: tooltip_time(lang, range, p.0, now),
        value,
        band: (is_bucketed(range) && p.2 < p.3).then(|| min_max(lang, p.2, p.3, unit)),
    }
}

/// Whether an x label at `pos` sits close enough to the "now" label at the right edge to run
/// into it on a narrow chart (the CSS hides it there).
fn near_end(pos: f64) -> bool {
    (0.7..1.0).contains(&pos)
}

/// Whether each point of `range` is a bucket (mean, min, max) rather than a raw sample.
fn is_bucketed(range: HistoryRange) -> bool {
    matches!(
        range,
        HistoryRange::Week | HistoryRange::Month | HistoryRange::Year
    )
}

/// The `<linearGradient>` of the fill under the line, as markup: Leptos wraps a `<stop>` built
/// in `view!` in a `<g>`, and a gradient ignores stops that are not its direct children.
fn gradient_markup(id: &str) -> String {
    format!(
        r#"<linearGradient id="{id}" x1="0" y1="0" x2="0" y2="1"><stop offset="0" class="history-chart__stop-top"/><stop offset="1" class="history-chart__stop-bottom"/></linearGradient>"#
    )
}

fn render_series(
    lang: Lang,
    s: HistorySeries,
    range: HistoryRange,
    kind: String,
    unit: Option<String>,
) -> impl IntoView {
    let discrete = state_axis_labels(lang, None, &kind).is_some();
    // The x-axis is the whole range ending now, so short history sits at the right edge.
    let now = (crate::energy::now_ms() / 1000) as i64;
    let t_start = now - range.span_secs();
    let base = y_scale(&s.points);
    let (min_v, max_v) = series_min_max(&s.points);
    // Round ticks around the data itself; a flat line keeps the padded extent.
    let nice = nice_scale(if max_v > min_v {
        YScale {
            min: min_v,
            max: max_v,
        }
    } else {
        base
    });
    let scale = if discrete { base } else { nice.scale };
    let (line, area, band) = if discrete {
        let StepPath { d, .. } = step_path(&s.points, scale, t_start, now, VIEW_W, VIEW_H);
        (d, String::new(), String::new())
    } else {
        let SmoothPath { line, area, band } =
            smooth_path(&s.points, scale, t_start, now, VIEW_W, VIEW_H);
        // Short ranges are raw samples: their min/max spread is noise, not a band.
        (
            line,
            area,
            if is_bucketed(range) {
                band
            } else {
                String::new()
            },
        )
    };
    let keyed = shows_band(range, discrete, &s.points);
    let (ylabels, hlines) = y_axis(lang, &nice, scale, &kind);
    let xticks = tick_labels(lang, range, now);
    let fracs = plot_points(&s.points, scale, t_start, now);
    let xs = StoredValue::new(fracs.iter().map(|p| p.0).collect::<Vec<_>>());
    let ys = StoredValue::new(fracs.iter().map(|p| p.1).collect::<Vec<_>>());
    let decimals = value_decimals(nice.step);
    let unit_tip = unit.clone();
    let points = StoredValue::new(s.points);

    let plot = NodeRef::<html::Div>::new();
    let hover = RwSignal::new(None::<usize>);
    let track = move |ev: web_sys::PointerEvent| {
        if let Some(f) = pointer_frac(&plot, &ev) {
            hover.set(xs.with_value(|xs| nearest_index(xs, f)));
        }
    };
    let x_at = move || {
        hover
            .get()
            .and_then(|i| xs.with_value(|xs| xs.get(i).copied()))
    };
    let gid = next_gradient_id();
    let fill = format!("url(#{gid})");

    view! {
        <div class="history-chart__inset">
            <div class="history-chart__plot" node_ref=plot
                on:pointermove=track on:pointerdown=track
                on:pointerleave=move |_| hover.set(None)
                on:pointercancel=move |_| hover.set(None)>
                <svg class="history-chart__svg" viewBox=format!("0 0 {VIEW_W} {VIEW_H}") preserveAspectRatio=ASPECT role="img">
                    <defs inner_html=gradient_markup(&gid)></defs>
                    {hlines.into_iter().map(|y| view! {
                        <line x1="0" y1=format!("{}", y * VIEW_H) x2=format!("{VIEW_W}") y2=format!("{}", y * VIEW_H) class="history-chart__grid" />
                    }).collect::<Vec<_>>()}
                    {xticks.iter().filter(|(pos, _)| *pos < 1.0).map(|(pos, _)| view! {
                        <line x1=format!("{}", pos * VIEW_W) y1="0" x2=format!("{}", pos * VIEW_W) y2=format!("{VIEW_H}") class="history-chart__grid" />
                    }).collect::<Vec<_>>()}
                    <path d=band class="history-chart__band" />
                    <path d=area fill=fill class="history-chart__area" />
                    <path d=line class="history-chart__line" class:history-chart__line--step=discrete fill="none" />
                </svg>
                <div class="history-chart__axis-y">
                    {ylabels.into_iter().map(|(y, l)| view! {
                        <span style=format!("top: {}", pct(y))>{l}</span>
                    }).collect::<Vec<_>>()}
                </div>
                <div class="history-chart__cursor" hidden=move || hover.get().is_none()
                    style=move || x_at().map(|x| format!("left: {}", pct(x))).unwrap_or_default()></div>
                <div class="history-chart__dot" hidden=move || hover.get().is_none()
                    style=move || hover.get().and_then(|i| {
                        let (x, y) = (xs.with_value(|v| v.get(i).copied())?, ys.with_value(|v| v.get(i).copied())?);
                        Some(format!("left: {}; top: {}", pct(x), pct(y)))
                    }).unwrap_or_default()></div>
                <div class="history-chart__tip" hidden=move || hover.get().is_none()
                    class:history-chart__tip--left=move || x_at().is_some_and(tooltip_flips)
                    style=move || x_at().map(|x| format!("left: {}", pct(x))).unwrap_or_default()>
                    {move || hover.get().and_then(|i| points.with_value(|p| p.get(i).cloned())).map(|p| {
                        let tip = series_tip(lang, range, &kind, unit_tip.as_deref(), decimals, now, &p);
                        view! {
                            <span class="history-chart__tip-time">{tip.time}</span>
                            <strong class="history-chart__tip-value">{tip.value}</strong>
                            {tip.band.map(|b| view! { <span class="history-chart__tip-band">{b}</span> })}
                        }
                    })}
                </div>
            </div>
            <div class="history-chart__axis-x">
                {xticks.into_iter().map(|(pos, l)| view! {
                    <span class:history-chart__tick--near-end=near_end(pos) style=format!("left: {}%", pos * 100.0)>{l}</span>
                }).collect::<Vec<_>>()}
            </div>
        </div>
        <div class="history-chart__legend">
            <span>{range_name(lang, range)}</span>
            {keyed.then(|| view! {
                <span class="history-chart__key" title=history_text(lang, HistoryText::SpreadHint)>
                    <span class="history-chart__key-item">
                        <i class="history-chart__key-line" aria-hidden="true"></i>
                        {history_text(lang, HistoryText::Average)}
                    </span>
                    <span class="history-chart__key-item">
                        <i class="history-chart__key-band" aria-hidden="true"></i>
                        {history_text(lang, HistoryText::Spread)}
                    </span>
                </span>
            })}
            <span>{min_max(lang, min_v, max_v, unit.as_deref())}</span>
        </div>
    }
}

/// Whether the chart draws the min/max band, and so keys it in the legend: a continuous value
/// on a bucketed range with at least one bucket that spread.
fn shows_band(range: HistoryRange, discrete: bool, points: &[(i64, f64, f64, f64)]) -> bool {
    !discrete && is_bucketed(range) && points.iter().any(|p| p.2 < p.3)
}

/// The y labels as `(fraction down from the top, text)` and the gridlines as fractions down
/// from the top. A discrete entity (`kind`) keeps its two state words at the edges.
fn y_axis(
    lang: Lang,
    nice: &NiceScale,
    scale: YScale,
    kind: &str,
) -> (Vec<(f64, String)>, Vec<f64>) {
    if let Some((lo, hi)) = state_axis_labels(lang, None, kind) {
        return (vec![(0.0, hi), (1.0, lo)], vec![0.0, 0.5, 1.0]);
    }
    let span = (scale.max - scale.min).max(f64::EPSILON);
    let decimals = step_decimals(nice.step);
    let at = |v: f64| 1.0 - (v - scale.min) / span;
    let labels = nice
        .ticks
        .iter()
        .map(|&v| (at(v), fmt::decimal_grouped(lang, v, decimals)))
        .collect();
    (labels, nice.ticks.iter().map(|&v| at(v)).collect())
}

fn render_bars(lang: Lang, d: HistoryDaily, range: HistoryRange) -> impl IntoView {
    let bars = daily_bars(&d.days, VIEW_W, VIEW_H);
    let total: f64 = d.days.iter().map(|d| d.1).sum();
    let n = bars.len();
    let centers = StoredValue::new(
        bars.iter()
            .map(|b| (b.x + b.w / 2.0) / VIEW_W)
            .collect::<Vec<_>>(),
    );
    let days = StoredValue::new(d.days);

    let plot = NodeRef::<html::Div>::new();
    let hover = RwSignal::new(None::<usize>);
    let track = move |ev: web_sys::PointerEvent| {
        if let Some(f) = pointer_frac(&plot, &ev) {
            hover.set(slot_index(n, f));
        }
    };
    let x_at = move || {
        hover
            .get()
            .and_then(|i| centers.with_value(|c| c.get(i).copied()))
    };

    view! {
        <div class="history-chart__plot" node_ref=plot
            on:pointermove=track on:pointerdown=track
            on:pointerleave=move |_| hover.set(None)
            on:pointercancel=move |_| hover.set(None)>
            {bars.into_iter().enumerate().map(|(i, Bar { x, h, w, .. })| view! {
                <div class="history-chart__bar"
                    class:history-chart__bar--on=move || hover.get() == Some(i)
                    style=format!("left: {}; width: {}; height: {}", pct(x / VIEW_W), pct(w / VIEW_W), pct(h / VIEW_H))></div>
            }).collect::<Vec<_>>()}
            <div class="history-chart__tip" hidden=move || hover.get().is_none()
                class:history-chart__tip--left=move || x_at().is_some_and(tooltip_flips)
                style=move || x_at().map(|x| format!("left: {}", pct(x))).unwrap_or_default()>
                {move || hover.get().and_then(|i| days.with_value(|d| d.get(i).copied())).map(|(t, kwh_v)| view! {
                    <span class="history-chart__tip-time">{tooltip_day(lang, t)}</span>
                    <strong class="history-chart__tip-value">{kwh(lang, kwh_v)}</strong>
                })}
            </div>
        </div>
        <div class="history-chart__legend">
            <span>{counter_legend(lang, range)}</span>
            <span>{total_line(lang, range, total)}</span>
        </div>
    }
}

fn series_min_max(points: &[(i64, f64, f64, f64)]) -> (f64, f64) {
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for p in points {
        if p.2 < lo {
            lo = p.2;
        }
        if p.3 > hi {
            hi = p.3;
        }
    }
    if !lo.is_finite() {
        lo = 0.0;
    }
    if !hi.is_finite() {
        hi = 0.0;
    }
    (lo, hi)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_guard_rejects_old_request_ids() {
        assert!(is_stale(1, 2));
        assert!(is_stale(3, 5));
        assert!(!is_stale(5, 5));
        assert!(!is_stale(1, 1));
    }

    #[test]
    fn an_open_chart_refreshes_each_minute_on_day_ranges() {
        assert_eq!(refresh_period_s(HistoryRange::Hour), 60);
        assert_eq!(refresh_period_s(HistoryRange::SixHours), 60);
        assert_eq!(refresh_period_s(HistoryRange::Day), 60);
        assert_eq!(refresh_period_s(HistoryRange::Week), 600);
        assert_eq!(refresh_period_s(HistoryRange::Month), 600);
        assert_eq!(refresh_period_s(HistoryRange::Year), 600);
    }

    #[test]
    fn daily_days_for_matches_range_windows() {
        assert_eq!(daily_days_for(HistoryRange::Week), 7);
        assert_eq!(daily_days_for(HistoryRange::Month), 30);
        assert_eq!(daily_days_for(HistoryRange::Year), 365);
        assert_eq!(daily_days_for(HistoryRange::Hour), 1);
        assert_eq!(daily_days_for(HistoryRange::SixHours), 1);
        assert_eq!(daily_days_for(HistoryRange::Day), 1);
    }

    #[test]
    fn plan_fetch_series_flavor_uses_entity_series_for_every_range() {
        for r in [
            HistoryRange::Hour,
            HistoryRange::SixHours,
            HistoryRange::Day,
            HistoryRange::Week,
            HistoryRange::Month,
            HistoryRange::Year,
        ] {
            assert_eq!(
                plan_fetch("sensor.temp", r, Flavor::Series, None),
                FetchPlan::Series("sensor.temp".into())
            );
        }
    }

    #[test]
    fn plan_fetch_counter_24h_uses_paired_power_series_when_present() {
        for r in [
            HistoryRange::Hour,
            HistoryRange::SixHours,
            HistoryRange::Day,
        ] {
            assert_eq!(
                plan_fetch("counter.e", r, Flavor::Counter, Some("sensor.power")),
                FetchPlan::Series("sensor.power".into())
            );
        }
    }

    #[test]
    fn drawn_unit_is_the_paired_power_sensors_for_a_counter_day_chart() {
        let unit_of = |id: &str| (id == "sensor.power").then(|| "W".to_owned());
        let wh = || Some("Wh".to_owned());
        let paired = plan_fetch(
            "counter.e",
            HistoryRange::Day,
            Flavor::Counter,
            Some("sensor.power"),
        );
        assert_eq!(
            drawn_unit(&paired, "counter.e", wh(), unit_of),
            Some("W".into())
        );
        let own = plan_fetch(
            "counter.e",
            HistoryRange::Week,
            Flavor::Counter,
            Some("sensor.power"),
        );
        assert_eq!(drawn_unit(&own, "counter.e", wh(), unit_of), wh());
        let temp = plan_fetch("sensor.t", HistoryRange::Day, Flavor::Series, None);
        assert_eq!(
            drawn_unit(&temp, "sensor.t", Some("°C".into()), unit_of),
            Some("°C".into())
        );
    }

    #[test]
    fn plan_fetch_counter_24h_falls_back_to_single_day_bar_without_power() {
        // Review Focus #2.
        for r in [
            HistoryRange::Hour,
            HistoryRange::SixHours,
            HistoryRange::Day,
        ] {
            assert_eq!(
                plan_fetch("counter.e", r, Flavor::Counter, None),
                FetchPlan::Daily("counter.e".into(), 1)
            );
        }
    }

    #[test]
    fn plan_fetch_counter_long_ranges_use_daily_with_right_days() {
        assert_eq!(
            plan_fetch("counter.e", HistoryRange::Week, Flavor::Counter, None),
            FetchPlan::Daily("counter.e".into(), 7)
        );
        assert_eq!(
            plan_fetch("counter.e", HistoryRange::Month, Flavor::Counter, None),
            FetchPlan::Daily("counter.e".into(), 30)
        );
        assert_eq!(
            plan_fetch("counter.e", HistoryRange::Year, Flavor::Counter, None),
            FetchPlan::Daily("counter.e".into(), 365)
        );
    }

    #[test]
    fn map_error_routes_each_variant_to_its_state() {
        // Review Focus #1: a 404 on a text/alarm/vacuum entity renders the empty copy.
        assert!(matches!(
            map_error(HistoryApiError::NotRecorded),
            State::NotRecorded
        ));
        assert!(matches!(
            map_error(HistoryApiError::Unavailable),
            State::Unavailable
        ));
        assert!(matches!(
            map_error(HistoryApiError::BadRequest("bad".into())),
            State::Error(_)
        ));
        assert!(matches!(
            map_error(HistoryApiError::Other("boom".into())),
            State::Error(_)
        ));
    }

    #[test]
    fn series_tip_reads_value_with_unit_and_the_band_on_long_ranges() {
        let p = (1_791_288_000, 21.4, 19.0, 23.5);
        let day = series_tip(
            Lang::En,
            HistoryRange::Day,
            "Sensor",
            Some("°C"),
            1,
            1_791_300_000,
            &p,
        );
        assert_eq!(day.value, "21.4\u{a0}°C");
        assert_eq!(day.band, None);
        let week = series_tip(
            Lang::Fr,
            HistoryRange::Week,
            "Sensor",
            Some("°C"),
            1,
            1_791_300_000,
            &p,
        );
        assert_eq!(week.value, "21,4\u{a0}°C");
        assert_eq!(
            week.band.as_deref(),
            Some("min 19,0\u{a0}°C · max 23,5\u{a0}°C")
        );
        // A bucket whose min equals its max has no band line.
        let flat = (1_791_288_000, 5.0, 5.0, 5.0);
        let tip = series_tip(
            Lang::En,
            HistoryRange::Month,
            "Sensor",
            None,
            1,
            1_791_300_000,
            &flat,
        );
        assert_eq!(tip.band, None);
    }

    #[test]
    fn the_band_is_keyed_only_where_it_is_drawn() {
        let spread = [(0, 20.0, 18.0, 22.0), (3_600, 21.0, 21.0, 21.0)];
        let flat = [(0, 20.0, 20.0, 20.0)];
        assert!(shows_band(HistoryRange::Week, false, &spread));
        assert!(shows_band(HistoryRange::Year, false, &spread));
        // Raw samples, a two-state entity and buckets that never moved have no band to explain.
        assert!(!shows_band(HistoryRange::Day, false, &spread));
        assert!(!shows_band(HistoryRange::Week, true, &spread));
        assert!(!shows_band(HistoryRange::Month, false, &flat));
    }

    #[test]
    fn series_tip_words_a_discrete_state() {
        let p = (1_791_288_000, 1.0, 1.0, 1.0);
        let tip = series_tip(
            Lang::En,
            HistoryRange::Day,
            "Light",
            None,
            1,
            1_791_300_000,
            &p,
        );
        assert_eq!(tip.value, "On");
    }

    #[test]
    fn gradient_stops_are_direct_children() {
        let m = gradient_markup("hc-fill-7");
        assert!(m.starts_with(r#"<linearGradient id="hc-fill-7""#));
        assert_eq!(m.matches("<stop ").count(), 2);
        assert!(!m.contains("<g>"));
    }

    #[test]
    fn only_labels_just_before_now_are_near_the_end() {
        assert!(near_end(0.76));
        assert!(!near_end(0.6));
        assert!(!near_end(1.0));
    }

    #[test]
    fn only_long_ranges_are_bucketed() {
        assert!(!is_bucketed(HistoryRange::Day));
        assert!(!is_bucketed(HistoryRange::Hour));
        assert!(is_bucketed(HistoryRange::Week));
        assert!(is_bucketed(HistoryRange::Year));
    }

    #[test]
    fn gradient_ids_are_unique_per_chart() {
        assert_ne!(next_gradient_id(), next_gradient_id());
    }

    #[test]
    fn series_min_max_handles_empty() {
        let (lo, hi) = series_min_max(&[]);
        assert_eq!((lo, hi), (0.0, 0.0));
    }
}
