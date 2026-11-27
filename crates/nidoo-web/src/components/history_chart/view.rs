//! The `HistoryChart` Leptos component: four states (loading, empty, unavailable, error) and
//! the renderers for the two flavors (smooth line with hover, and daily bars with hover).

use std::cell::Cell;
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};

use chrono::NaiveDate;
use leptos::html;
use leptos::prelude::*;
use nidoo_api::{
    DailyPer, FieldTrack, HistoryDaily, HistoryFields, HistoryRange, HistorySeries, TrackData,
};
use nidoo_core::flow::Scalar;
use nidoo_core::model::Kind;

use super::compare::{
    Axis, BarsWindow, SeriesDates, axis_for, axis_x, compare_key, compared_at, dates_ctx, now_mark,
    render_bars_window, series_dates, series_when,
};
use super::fetch::{Compared, State, apply_shown, compared_next, compared_plan, fetch_plan};
pub use super::fetch::{FetchPlan, drawn_unit, is_stale, plan_fetch};
use super::format::{state_axis_labels, state_value_label, tooltip_day, tooltip_time};
use super::geom::{
    Bar, NiceScale, SmoothPath, StepPath, YScale, daily_bars, nearest_index, nice_scale,
    plot_points, slot_index, smooth_path_held, step_decimals, step_path_held, tooltip_flips,
    value_decimals, y_scale,
};
use super::periods::{Band, band_near, legend, periods, scalar_text, value_at, word_in};
use super::plot::{FieldsState, Plot, field_series, headline_or_error, percent_scale, plot_for};
use super::selection::{chip_label, chips, parse_show, show_param, toggle};
use super::summary::PeriodSummary;
pub use super::window::ChartWindow;
use super::window::{
    compared_req, domain, has_fields, local_today, refetches, runs_today, today_or, window_per,
    window_range,
};
use crate::api::history::{HistoryApiError, WindowReq, fetch_fields, fetch_fields_window};
use crate::components::now::Now;
use crate::history_period::{Span, per_for, zoom};
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

/// What a fetch was made for: entity, range, flavor, power pair and the Retry count.
type FetchInputs = (String, ChartWindow, Flavor, Option<String>, u32);

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
    #[prop(into)] window: Signal<ChartWindow>,
    #[prop(into)] flavor: Signal<Flavor>,
    #[prop(into)] kind: Signal<String>,
    #[prop(into)] unit: Signal<Option<String>>,
    #[prop(into)] power_pair: Signal<Option<String>>,
    /// The raw `show` query value: which value chips are on.
    #[prop(optional, into)]
    show: Signal<Option<String>>,
    /// Called with the new `show` value when a chip is tapped. Without it the chart is a plain
    /// series chart: no fields fetch, no chips (dashboard widgets).
    #[prop(optional)]
    on_show: Option<Callback<String>>,
    /// The entity's device class, which names the headline number's chip.
    #[prop(optional, into)]
    device_class: Signal<Option<String>>,
    /// The entity's kind, which words the chips and the periods.
    #[prop(optional)]
    field_kind: Option<Signal<Kind>>,
    /// Called with the local date of a clicked bucket of a date window, when its unit zooms
    /// into a finer one (a year into its months, a week or month into its days).
    #[prop(optional)]
    on_zoom: Option<Callback<NaiveDate>>,
    /// The hub's today, once known: it decides whether a date window is still running. Without
    /// it the browser's clock does.
    #[prop(optional, into)]
    today: Signal<Option<NaiveDate>>,
) -> impl IntoView {
    let wants_fields = on_show.is_some();
    let state = RwSignal::new(State::Loading);
    // The compared window: off, still loading, its reply (`State::Series` or `State::Bars`), or
    // empty (the reply failed or had no data).
    let compared = RwSignal::new(Compared::Off);
    let fields = RwSignal::new(FieldsState::Loading);
    let latest: Rc<Cell<u32>> = Rc::new(Cell::new(0));
    let retry_bump = RwSignal::new(0u32);

    // The shared clock, cut to this range's refresh period: an open chart refetches each time
    // it moves on. Off the app (no clock) the chart never refreshes.
    let now = use_context::<Now>();
    // A past date window never changes, so its tick stays put.
    let tick = Memo::new(move |_| {
        let w = window.get();
        now.map(|n| {
            let secs = n.0.get();
            if refetches(&w, today_or(today.get(), secs as i64)) {
                secs / refresh_period_s(window_range(&w))
            } else {
                0
            }
        })
    });

    // Whether the shown date window holds today. A memo, so the hub's `today` arriving refetches
    // only when it changes the answer.
    let running = Memo::new(move |_| match window.get() {
        ChartWindow::Dates { shown, .. } => runs_today(
            &shown,
            today_or(today.get(), (crate::energy::now_ms() / 1000) as i64),
        ),
        ChartWindow::Preset(_) => false,
    });

    // Fetch effect — refires on any input change (or Retry), and on each tick. A tick alone is
    // a refresh: the chart stays up while it loads.
    {
        let latest = latest.clone();
        Effect::new(move |prev: Option<FetchInputs>| {
            let inputs = (
                entity_id.get(),
                window.get(),
                flavor.get(),
                power_pair.get(),
                retry_bump.get(),
            );
            let _ = tick.get();
            let refresh = prev.as_ref() == Some(&inputs);
            let my_req = latest.get() + 1;
            latest.set(my_req);
            let with_fields = wants_fields && inputs.2 == Flavor::Series && has_fields(&inputs.1);
            if !refresh {
                state.set(State::Loading);
                fields.set(if with_fields {
                    FieldsState::Loading
                } else {
                    FieldsState::Ready(no_fields())
                });
                compared.set(Compared::starting(matches!(
                    &inputs.1,
                    ChartWindow::Dates {
                        compare: Some(_),
                        ..
                    }
                )));
            }

            let (id, w, f, pp, _) = inputs.clone();
            if with_fields {
                let (id, latest_check, w) = (id.clone(), latest.clone(), w.clone());
                leptos::task::spawn_local(async move {
                    let outcome = match &w {
                        ChartWindow::Preset(r) => fetch_fields(&id, *r).await,
                        ChartWindow::Dates { shown, .. } => fetch_fields_window(&id, shown).await,
                    };
                    if is_stale(my_req, latest_check.get()) {
                        return;
                    }
                    match outcome {
                        Ok(f) => fields.set(FieldsState::Ready(f)),
                        Err(e) => {
                            // A failed refresh keeps what is drawn.
                            if !refresh
                                || fields.with_untracked(|s| matches!(s, FieldsState::Loading))
                            {
                                fields.set(match e {
                                    HistoryApiError::NotRecorded
                                    | HistoryApiError::NoGridMeter
                                    | HistoryApiError::Unavailable => {
                                        FieldsState::Ready(no_fields())
                                    }
                                    HistoryApiError::BadRequest(m) | HistoryApiError::Other(m) => {
                                        FieldsState::Failed(m)
                                    }
                                });
                            }
                        }
                    }
                });
            }
            let range = window_range(&w);
            let plan = plan_fetch(&id, &w, f, pp.as_deref());
            let plan_shown = plan.clone();
            // Set against an earlier period: what to fetch it as, and whether the shown period
            // is still running (then the shown reply says how far in to cut it).
            let comparison = match &w {
                ChartWindow::Dates {
                    compare: Some(cmp), ..
                } => Some((*cmp, running.get())),
                _ => None,
            };
            let latest_check = latest.clone();
            let fetch_compared = move |req: WindowReq| {
                let latest_check = latest_check.clone();
                let plan = compared_plan(&plan_shown, req);
                async move {
                    let outcome = fetch_plan(plan, range).await;
                    if !is_stale(my_req, latest_check.get())
                        && let Some(next) = compared_next(outcome, refresh)
                    {
                        let _ = compared.try_set(next);
                    }
                }
            };
            let latest_check = latest.clone();
            leptos::task::spawn_local(async move {
                match comparison {
                    // Running: the compared window waits for the shown reply's span, and is
                    // not fetched at all (uncut) when the shown fetch failed.
                    Some((cmp, true)) => {
                        let outcome = fetch_plan(plan, range).await;
                        if is_stale(my_req, latest_check.get()) {
                            return;
                        }
                        let req = compared_req(outcome.span(), true, cmp);
                        apply_shown(state, outcome, refresh);
                        match req {
                            Some(req) => fetch_compared(req).await,
                            None => {
                                let _ = compared.try_update(|c| *c = c.clone().unrequested());
                            }
                        }
                    }
                    // Not running: both at once.
                    Some((cmp, false)) => {
                        leptos::task::spawn_local(fetch_compared(cmp));
                        let outcome = fetch_plan(plan, range).await;
                        if !is_stale(my_req, latest_check.get()) {
                            apply_shown(state, outcome, refresh);
                        }
                    }
                    None => {
                        let outcome = fetch_plan(plan, range).await;
                        if !is_stale(my_req, latest_check.get()) {
                            apply_shown(state, outcome, refresh);
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
        let plan = plan_fetch(
            &id,
            &window.get(),
            flavor.get(),
            power_pair.get().as_deref(),
        );
        drawn_unit(&plan, &id, unit.get(), |other| {
            house.and_then(|h| {
                h.entities
                    .with(|m| m.get(other).and_then(|e| e.unit.clone()))
            })
        })
    });

    // Each render reads `state` and `fields` and composes the right view.
    let kind_rx = kind;
    let unit_rx = drawn_unit;
    let lang = use_lang();
    view! {
        <div class="history-chart">
            {move || {
                // Read here, so a language flip redraws the labels without a refetch.
                let lang = lang();
                let win = window.get();
                let range = window_range(&win);
                let now_s = (crate::energy::now_ms() / 1000) as i64;
                let fields_now = fields.get();
                let entity_kind = field_kind.map_or(Kind::Generic, |k| k.get());
                let tracks = match &fields_now {
                    FieldsState::Ready(f) => f.tracks.clone(),
                    _ => Vec::new(),
                };
                let chip_list = chips(&tracks);
                let sel = parse_show(show.get().as_deref(), &chip_list);
                // The x domain of a field plot: the fields reply's own axis.
                let fspan = match &fields_now {
                    FieldsState::Ready(f) if f.to > f.from => Some((f.from, f.to)),
                    _ => None,
                };
                let fdomain = domain(&win, fspan, now_s);
                let dctx = dates_ctx(lang, &win, now_s, today.get());
                let name = dctx
                    .as_ref()
                    .map_or_else(|| range_name(lang, range).to_owned(), |c| c.name.clone());
                let cmp_all = compared.get();
                let cmp_state = cmp_all.ready().cloned();
                let plot = plot_for(flavor.get(), &fields_now, &sel, &chip_list);
                let entity_plot = matches!(plot, Plot::Bars | Plot::Headline { .. });
                let row = match on_show {
                    Some(on_show)
                        if chip_list.len() > 1 && matches!(fields_now, FieldsState::Ready(_)) =>
                    {
                        let dc = device_class.get();
                        let buttons = chip_list
                            .iter()
                            .map(|chip| {
                                let label = chip_label(lang, entity_kind, dc.as_deref(), &tracks, chip);
                                let on = sel.number.as_deref() == Some(chip.key.as_str())
                                    || sel.word.as_deref() == Some(chip.key.as_str());
                                let (chip, sel) = (chip.clone(), sel.clone());
                                view! {
                                    <button type="button" class="chip"
                                        class:chip--selected=on
                                        aria-pressed=on.to_string()
                                        on:click={move |_| on_show.run(show_param(&toggle(&sel, &chip)))}>
                                        {label}
                                    </button>
                                }
                            })
                            .collect_view();
                        Some(view! {
                            <div class="chips history-chart__chips" role="group"
                                aria-label=history_text(lang, HistoryText::ValuesShown)>
                                {buttons}
                            </div>
                        })
                    }
                    _ => None,
                };
                let body = match plot {
                    Plot::Loading => {
                        view! { <div class="history-chart__shimmer" aria-busy="true"></div> }.into_any()
                    }
                    Plot::Field { track, overlay } => {
                        let overlay = overlay_of(lang, entity_kind, &tracks, overlay, fdomain);
                        match tracks.get(track).and_then(field_series) {
                            Some((s, unit)) => {
                                let fixed = percent_scale(unit.as_deref());
                                let axis = axis_for(lang, dctx.as_ref(), range, fspan, fdomain.1, false);
                                render_series(SeriesView {
                                    lang, s, range, domain: fdomain, kind: field_series_kind(),
                                    unit, overlay, fixed, axis, name, dates: None, zoom: None,
                                }).into_any()
                            }
                            None => {
                                let axis = axis_for(lang, dctx.as_ref(), range, fspan, fdomain.1, false);
                                render_words(lang, range, fdomain, overlay, axis, name).into_any()
                            }
                        }
                    }
                    Plot::Words { track } => {
                        let overlay = overlay_of(lang, entity_kind, &tracks, Some(track), fdomain);
                        let axis = axis_for(lang, dctx.as_ref(), range, fspan, fdomain.1, false);
                        render_words(lang, range, fdomain, overlay, axis, name).into_any()
                    }
                    Plot::Bars | Plot::Headline { .. } => {
                        let overlay = match plot {
                            Plot::Headline { overlay } => {
                                overlay_of(lang, entity_kind, &tracks, overlay, fdomain)
                            }
                            _ => None,
                        };
                        match state.get() {
                        State::Loading => {
                            view! { <div class="history-chart__shimmer" aria-busy="true"></div> }.into_any()
                        }
                        State::NotRecorded => view! {
                            <p class="history-chart__empty">{history_text(lang, HistoryText::NotRecorded)}</p>
                        }.into_any(),
                        State::Unavailable => view! {
                            <p class="history-chart__err">{history_text(lang, HistoryText::Unavailable)}</p>
                        }.into_any(),
                        State::Error(msg) => error_block(lang, &msg, retry_bump).into_any(),
                        State::Series(s) => match headline_or_error(&fields_now, &s) {
                            Some(msg) => error_block(lang, &msg, retry_bump).into_any(),
                            None => {
                                let dom = domain(&win, s.span, now_s);
                                let cmp = match &cmp_state {
                                    Some(State::Series(c)) => Some(c),
                                    _ => None,
                                };
                                let dates = dctx.as_ref().map(|c| series_dates(c, &s, cmp));
                                // A point zooms like a bar, by the buckets the window would have.
                                let zoom_to = dctx.as_ref().and_then(|c| {
                                    let span = Span { from: c.req.from, to: c.req.to };
                                    on_zoom.filter(|_| zoom(c.unit, per_for(c.unit, span)).is_some())
                                });
                                let axis = axis_for(lang, dctx.as_ref(), range, s.span, dom.1, false);
                                render_series(SeriesView {
                                    lang, s, range, domain: dom, kind: kind_rx.get(), unit: unit_rx.get(),
                                    overlay, fixed: None, axis, name, dates, zoom: zoom_to,
                                }).into_any()
                            }
                        },
                        State::Bars(d) => {
                            let per = window_per(&win, flavor.get()).unwrap_or(DailyPer::Day);
                            match &dctx {
                                Some(c) => {
                                    let compared = match &cmp_state {
                                        Some(State::Bars(c)) => Some(c.clone()),
                                        _ => None,
                                    };
                                    render_bars_window(BarsWindow {
                                        lang,
                                        axis: axis_for(lang, Some(c), range, d.span, fdomain.1, per == DailyPer::Month),
                                        shown: d,
                                        compared,
                                        per,
                                        ctx: c.clone(),
                                        zoom: on_zoom.filter(|_| zoom(c.unit, per).is_some()),
                                    }).into_any()
                                }
                                None => render_bars(lang, d, range, per).into_any(),
                            }
                        }
                        }
                    }
                };
                let summary = (dctx.is_some() && entity_plot).then(|| {
                    view! {
                        <PeriodSummary lang=lang window=win.clone() shown=state.get()
                            compared=cmp_all.clone() kind=kind_rx.get() unit=unit_rx.get()
                            today=today.get() />
                    }
                });
                view! { {row} {body} {summary} }
            }}
        </div>
    }
}

/// The kind a field's series is worded with: a plain continuous number, whatever the entity. A
/// field is never the entity's own state, so it takes none of the entity kind's wording.
fn field_series_kind() -> String {
    format!("{:?}", Kind::Sensor)
}

/// A fields reply with nothing in it.
fn no_fields() -> HistoryFields {
    HistoryFields {
        from: 0,
        to: 0,
        tracks: Vec::new(),
    }
}

/// The load error with its Retry button, which refetches the series and the fields.
fn error_block(lang: Lang, msg: &str, retry_bump: RwSignal<u32>) -> impl IntoView {
    view! {
        <div class="history-chart__err">
            <p>{load_error(lang, msg)}</p>
            <button type="button" class="chip" on:click=move |_| retry_bump.update(|v| *v += 1)>
                {history_text(lang, HistoryText::Retry)}
            </button>
        </div>
    }
}

/// A word value's periods, drawn over the plot with a legend and named in the tooltip.
#[derive(Clone, Debug)]
pub(crate) struct Overlay {
    pub changes: Vec<(i64, Option<Scalar>)>,
    pub periods: Vec<Band>,
    /// The word of each `legend` entry.
    pub words: Vec<String>,
    pub legend: Vec<(Scalar, Option<usize>)>,
    /// The window the periods span, unix seconds.
    from: i64,
    to: i64,
    lang: Lang,
    kind: Kind,
    field: Option<String>,
    unit: Option<String>,
}

impl Overlay {
    /// The overlay of a steps track over the window `from..to`; `None` for a number track.
    fn new(lang: Lang, kind: Kind, track: &FieldTrack, from: i64, to: i64) -> Option<Self> {
        let TrackData::Steps { changes } = &track.data else {
            return None;
        };
        let periods = periods(changes, from, to);
        let legend = legend(&periods);
        let (field, unit) = (track.field.clone(), track.unit.clone());
        let words = legend
            .iter()
            .map(|(v, _)| scalar_text(lang, kind, field.as_deref(), unit.as_deref(), Some(v)))
            .collect();
        Some(Self {
            changes: changes.clone(),
            periods,
            words,
            legend,
            from,
            to,
            lang,
            kind,
            field,
            unit,
        })
    }

    fn text(&self, value: Option<&Scalar>) -> String {
        scalar_text(
            self.lang,
            self.kind,
            self.field.as_deref(),
            self.unit.as_deref(),
            value,
        )
    }

    /// The word in force at unix second `t`.
    fn word_at(&self, t: i64) -> String {
        self.text(value_at(&self.changes, t).as_ref())
    }

    /// The word naming the span `start..end`: the coloured value held longest in it.
    fn word_over(&self, start: i64, end: i64) -> String {
        self.text(word_in(&self.changes, &self.periods, start, end).as_ref())
    }

    /// The period under `frac` of the window, with `tol` of slack either side.
    fn band_near(&self, frac: f64, tol: f64) -> Option<&Band> {
        band_near(&self.periods, frac, tol)
    }
}

/// The overlay of track `idx` of the fields reply, over the x domain `from..to`.
fn overlay_of(
    lang: Lang,
    kind: Kind,
    tracks: &[FieldTrack],
    idx: Option<usize>,
    (from, to): (i64, i64),
) -> Option<Overlay> {
    Overlay::new(lang, kind, tracks.get(idx?)?, from, to)
}

/// The shaded periods behind the plot: only the coloured ones; the baseline stays plain.
fn periods_layer(overlay: &Overlay, behind: bool) -> impl IntoView + use<> {
    let coloured: Vec<(f64, f64, usize)> = overlay
        .periods
        .iter()
        .filter_map(|b| Some((b.x0, b.x1, b.color?)))
        .collect();
    view! {
        <div class="history-chart__periods" class:history-chart__periods--behind=behind>
            {coloured.into_iter().map(|(x0, x1, n)| {
                view! {
                    <i class=format!("history-chart__period history-chart__period--{n}")
                        style=period_style(x0, x1)></i>
                }
            }).collect::<Vec<_>>()}
        </div>
    }
}

/// The inline style of a period: at least 3 px wide, and never pushed out of the box when it
/// starts at the right edge.
fn period_style(x0: f64, x1: f64) -> String {
    format!(
        "left: min({}, calc(100% - 3px)); width: max({}, 3px)",
        pct(x0),
        pct(x1 - x0)
    )
}

/// What each value of the periods is, under the chart.
fn period_key(overlay: &Overlay) -> impl IntoView + use<> {
    let items = overlay
        .legend
        .iter()
        .zip(overlay.words.iter().cloned())
        .map(|((_, color), word)| {
            let swatch = match color {
                Some(n) => format!("history-chart__swatch history-chart__swatch--{n}"),
                None => "history-chart__swatch history-chart__swatch--base".to_owned(),
            };
            view! {
                <span class="history-chart__key-item">
                    <i class=swatch aria-hidden="true"></i>
                    {word}
                </span>
            }
        })
        .collect::<Vec<_>>();
    view! { <span class="history-chart__key">{items}</span> }
}

/// The plot box of a word value shown alone: its periods, the x axis and the legend.
fn render_words(
    lang: Lang,
    range: HistoryRange,
    _domain: (i64, i64),
    overlay: Option<Overlay>,
    axis: Axis,
    name: String,
) -> impl IntoView {
    let layer = overlay.as_ref().map(|o| periods_layer(o, false));
    let key = overlay.as_ref().map(period_key);
    let overlay = StoredValue::new(overlay);
    let plot = NodeRef::<html::Div>::new();
    let hover = RwSignal::new(None::<f64>);
    // The hover slack, as a fraction of the plot width at the time of the pointer.
    let tol = StoredValue::new(0.0_f64);
    let track = move |ev: web_sys::PointerEvent| {
        if let Some((f, width)) = pointer_frac_width(&plot, &ev) {
            tol.set_value(HOVER_SLACK_PX / width);
            hover.set(Some(f));
        }
    };
    view! {
        <div class="history-chart__inset">
            <div class="history-chart__plot" node_ref=plot
                on:pointermove=track on:pointerdown=track
                on:pointerleave=move |_| hover.set(None)
                on:pointercancel=move |_| hover.set(None)>
                {layer}
                <div class="history-chart__cursor" hidden=move || hover.get().is_none()
                    style=move || hover.get().map(|x| format!("left: {}", pct(x))).unwrap_or_default()></div>
                <div class="history-chart__tip" hidden=move || hover.get().is_none()
                    class:history-chart__tip--left=move || hover.get().is_some_and(tooltip_flips)
                    style=move || hover.get().map(|x| format!("left: {}", pct(x))).unwrap_or_default()>
                    {move || hover.get().and_then(|f| overlay.with_value(|o| o.as_ref().map(|o| {
                        let t = o.from + (f * (o.to - o.from) as f64) as i64;
                        let band = o.band_near(f, tol.get_value());
                        let span = band.map(|b| format!(
                            "{} – {}",
                            tooltip_time(lang, range, b.start, o.to),
                            tooltip_time(lang, range, b.end, o.to)
                        ));
                        let word = match band {
                            Some(b) => o.text(b.value.as_ref()),
                            None => o.word_at(t),
                        };
                        (tooltip_time(lang, range, t, o.to), word, span)
                    }))).map(|(time, word, span)| view! {
                        <span class="history-chart__tip-time">{time}</span>
                        <strong class="history-chart__tip-value history-chart__tip-word">{word}</strong>
                        {span.map(|s| view! { <span class="history-chart__tip-band">{s}</span> })}
                    })}
                </div>
            </div>
            {axis_x(axis)}
        </div>
        <div class="history-chart__legend">
            <span>{name}</span>
            {key}
        </div>
    }
}

/// What a bar's tooltip names: its hour, its day or its month.
fn bar_label(lang: Lang, per: DailyPer, t: i64) -> String {
    match per {
        DailyPer::Hour => tooltip_time(lang, HistoryRange::Day, t, t),
        DailyPer::Day => tooltip_day(lang, t),
        DailyPer::Month => tooltip_time(lang, HistoryRange::Year, t, t),
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

/// The pointer's x across `plot`, as a fraction of its width, and that width in pixels.
fn pointer_frac_width(plot: &NodeRef<html::Div>, ev: &web_sys::PointerEvent) -> Option<(f64, f64)> {
    let rect = plot.get()?.get_bounding_client_rect();
    (rect.width() > 0.0).then(|| {
        (
            ((f64::from(ev.client_x()) - rect.left()) / rect.width()).clamp(0.0, 1.0),
            rect.width(),
        )
    })
}

/// The pointer's x across `plot`, as a fraction of its width.
pub(super) fn pointer_frac(plot: &NodeRef<html::Div>, ev: &web_sys::PointerEvent) -> Option<f64> {
    pointer_frac_width(plot, ev).map(|(f, _)| f)
}

/// How far from a period, in pixels, the pointer still counts as on it: periods are drawn at
/// least 3 px wide, however briefly they lasted.
const HOVER_SLACK_PX: f64 = 1.5;

/// A fraction as a CSS percentage.
pub(super) fn pct(frac: f64) -> String {
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
pub(crate) fn near_end(pos: f64) -> bool {
    (0.7..1.0).contains(&pos)
}

/// Where the bucket of point `i` ends: at the next point, else as long as the one before it.
fn bucket_end(points: &[(i64, f64, f64, f64)], i: usize) -> i64 {
    let t = points[i].0;
    match (
        points.get(i + 1),
        i.checked_sub(1).and_then(|j| points.get(j)),
    ) {
        (Some(next), _) => next.0,
        (None, Some(prev)) => t + (t - prev.0),
        (None, None) => t + 1,
    }
}

/// Whether each point of `range` is a bucket (mean, min, max) rather than a raw sample.
pub(crate) fn is_bucketed(range: HistoryRange) -> bool {
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

/// Everything `render_series` draws: the series, its axes and, for a date window, the compared
/// period and the running mark.
struct SeriesView {
    lang: Lang,
    s: HistorySeries,
    range: HistoryRange,
    /// The x domain, unix seconds `(start, end)`.
    domain: (i64, i64),
    kind: String,
    unit: Option<String>,
    overlay: Option<Overlay>,
    fixed: Option<YScale>,
    axis: Axis,
    /// The legend's name of what is shown.
    name: String,
    dates: Option<SeriesDates>,
    /// Called with the local date of a clicked point, when the unit zooms.
    zoom: Option<Callback<NaiveDate>>,
}

fn render_series(v: SeriesView) -> impl IntoView {
    let SeriesView {
        lang,
        s,
        range,
        domain: (t_start, now),
        kind,
        unit,
        overlay,
        fixed,
        axis,
        name,
        dates,
        zoom,
    } = v;
    let discrete = state_axis_labels(lang, None, &kind).is_some();
    let cmp_pts: Vec<(i64, f64, f64, f64)> = dates
        .as_ref()
        .map(|d| {
            d.compared
                .iter()
                .map(|c| (c.t, c.v.0, c.v.1, c.v.2))
                .collect()
        })
        .unwrap_or_default();
    // The compared series shares the axis, so it must fit the scale too.
    let scaled: Vec<(i64, f64, f64, f64)> = s.points.iter().chain(&cmp_pts).copied().collect();
    // The x-axis is the whole domain: a preset ends now, so short history sits at the right
    // edge; a date window is the whole period.
    let base = y_scale(&scaled);
    let (min_v, max_v) = series_min_max(&s.points);
    let (scale_lo, scale_hi) = series_min_max(&scaled);
    // Round ticks around the data itself; a flat line keeps the padded extent.
    // A number field in % is the one series with a fixed extent.
    let nice = nice_scale(nice_input(fixed, scale_lo, scale_hi, base));
    let scale = if discrete { base } else { nice.scale };
    // A running period's lines stop at its now line; anything else runs on to the edge.
    let x_frac = |t: i64| ((t - t_start) as f64 / (now - t_start).max(1) as f64).clamp(0.0, 1.0);
    let hold_px = dates
        .as_ref()
        .and_then(|d| d.running_to)
        .map_or(VIEW_W, |t| x_frac(t) * VIEW_W);
    let (line, area, band) = if discrete {
        let StepPath { d, .. } =
            step_path_held(&s.points, scale, t_start, now, VIEW_W, hold_px, VIEW_H);
        (d, String::new(), String::new())
    } else {
        let SmoothPath { line, area, band } =
            smooth_path_held(&s.points, scale, t_start, now, VIEW_W, hold_px, VIEW_H);
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
    let cmp_hold = cmp_pts.last().map_or(VIEW_W, |p| x_frac(p.0) * VIEW_W);
    let cmp_line = if discrete {
        step_path_held(&cmp_pts, scale, t_start, now, VIEW_W, cmp_hold, VIEW_H).d
    } else {
        smooth_path_held(&cmp_pts, scale, t_start, now, VIEW_W, cmp_hold, VIEW_H).line
    };
    let keyed = shows_band(range, discrete, &s.points);
    let (ylabels, hlines) = y_axis(lang, &nice, scale, &kind);
    let grid_x: Vec<f64> = axis
        .ticks
        .iter()
        .map(|(p, _)| *p)
        .filter(|p| *p < 1.0)
        .collect();
    let fracs = plot_points(&s.points, scale, t_start, now);
    let xs = StoredValue::new(fracs.iter().map(|p| p.0).collect::<Vec<_>>());
    let ys = StoredValue::new(fracs.iter().map(|p| p.1).collect::<Vec<_>>());
    let decimals = value_decimals(nice.step);
    let unit_tip = unit.clone();
    let points = StoredValue::new(s.points);
    let now_line = now_mark(dates.as_ref().and_then(|d| d.now_frac));
    let cmp_name = dates.as_ref().and_then(|d| d.cmp_name.clone());
    let dates = StoredValue::new(dates);

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
    let click = move |_| {
        let t = hover
            .get_untracked()
            .and_then(|i| points.with_value(|p| p.get(i).map(|p| p.0)));
        if let (Some(zoom), Some(t)) = (zoom, t) {
            zoom.run(local_today(t));
        }
    };
    let gid = next_gradient_id();
    let fill = format!("url(#{gid})");
    let layer = overlay.as_ref().map(|o| periods_layer(o, true));
    let key = overlay.as_ref().map(period_key);
    let overlay = StoredValue::new(overlay);

    view! {
        <div class="history-chart__inset">
            <div class="history-chart__plot" node_ref=plot
                class:history-chart__plot--zoom=zoom.is_some()
                on:pointermove=track on:pointerdown=track on:click=click
                on:pointerleave=move |_| hover.set(None)
                on:pointercancel=move |_| hover.set(None)>
                {layer}
                <svg class="history-chart__svg" viewBox=format!("0 0 {VIEW_W} {VIEW_H}") preserveAspectRatio=ASPECT role="img">
                    <defs inner_html=gradient_markup(&gid)></defs>
                    {hlines.into_iter().map(|y| view! {
                        <line x1="0" y1=format!("{}", y * VIEW_H) x2=format!("{VIEW_W}") y2=format!("{}", y * VIEW_H) class="history-chart__grid" />
                    }).collect::<Vec<_>>()}
                    {grid_x.into_iter().map(|pos| view! {
                        <line x1=format!("{}", pos * VIEW_W) y1="0" x2=format!("{}", pos * VIEW_W) y2=format!("{VIEW_H}") class="history-chart__grid" />
                    }).collect::<Vec<_>>()}
                    <path d=band class="history-chart__band" />
                    <path d=area fill=fill class="history-chart__area" />
                    {(!cmp_line.is_empty()).then(|| view! {
                        <path d=cmp_line class="history-chart__line history-chart__line--compare" fill="none" />
                    })}
                    <path d=line class="history-chart__line" class:history-chart__line--step=discrete fill="none" />
                </svg>
                {now_line}
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
                        let mut tip = series_tip(lang, range, &kind, unit_tip.as_deref(), decimals, now, &p);
                        // Against another period, each value says which date it is.
                        let versus = dates.with_value(|d| d.as_ref().and_then(|d| {
                            let c = compared_at(&d.compared, p.0, d.start, d.bucketing)?;
                            let mut other = series_tip(
                                lang, range, &kind, unit_tip.as_deref(), decimals, now,
                                &(c.orig, c.v.0, c.v.1, c.v.2),
                            );
                            other.time = series_when(lang, range, c.orig);
                            Some(other)
                        }));
                        if versus.is_some() {
                            tip.time = series_when(lang, range, p.0);
                        }
                        let word = overlay.with_value(|o| o.as_ref().map(|o| {
                            if is_bucketed(range) {
                                let end = hover.get_untracked().map_or(p.0 + 1, |i| {
                                    points.with_value(|pts| bucket_end(pts, i))
                                });
                                o.word_over(p.0, end)
                            } else {
                                o.word_at(p.0)
                            }
                        }));
                        view! {
                            <span class="history-chart__tip-time">{tip.time}</span>
                            <strong class="history-chart__tip-value">{tip.value}</strong>
                            {tip.band.map(|b| view! { <span class="history-chart__tip-band">{b}</span> })}
                            {versus.map(|o| view! {
                                <span class="history-chart__tip-time history-chart__tip-time--compare">{o.time}</span>
                                <strong class="history-chart__tip-value history-chart__tip-value--compare">{o.value}</strong>
                                {o.band.map(|b| view! { <span class="history-chart__tip-band">{b}</span> })}
                            })}
                            {word.map(|w| view! { <span class="history-chart__tip-word">{w}</span> })}
                        }
                    })}
                </div>
            </div>
            {axis_x(axis)}
        </div>
        <div class="history-chart__legend">
            <span>{name}</span>
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
            {compare_key(cmp_name)}
            {key}
            <span>{min_max(lang, min_v, max_v, unit.as_deref())}</span>
        </div>
    }
}

/// The extent the y ticks are rounded around: the fixed one of a percentage field, else the data
/// itself; a flat line keeps the padded extent.
fn nice_input(fixed: Option<YScale>, min_v: f64, max_v: f64, padded: YScale) -> YScale {
    fixed.unwrap_or(if max_v > min_v {
        YScale {
            min: min_v,
            max: max_v,
        }
    } else {
        padded
    })
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

fn render_bars(lang: Lang, d: HistoryDaily, range: HistoryRange, per: DailyPer) -> impl IntoView {
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
                    <span class="history-chart__tip-time">{bar_label(lang, per, t)}</span>
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
mod tests;
