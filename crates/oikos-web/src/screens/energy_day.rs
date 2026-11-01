//! The Energy tab's grid day card: what crossed the grid meter on one day, in half-hour bars,
//! bought above the zero line (off-peak under peak) and sold below it.

use leptos::prelude::*;
#[cfg(target_arch = "wasm32")]
use leptos::task::spawn_local;

use crate::api::history::HistoryApiError;
use crate::components::now::Now;
use crate::energy_day::{
    GridDay, aria_summary, day_label, has_data, scale, shift_day, shown_day, tick_hours, tooltip,
    totals_text, x_of,
};
use crate::i18n::energy::{EnergyText, energy_text as word, history_error};
use crate::i18n::format::time;
use crate::i18n::{Key, Lang, text, use_lang};

/// Drawing width on a wide screen, and on a phone, where a narrower drawing keeps the chart
/// tall and its labels legible.
const VIEW_W_WIDE: f64 = 720.0;
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))] // only the browser measures the window
const VIEW_W_NARROW: f64 = 400.0;
const VIEW_H: f64 = 240.0;
const LEFT: f64 = 30.0;
const RIGHT: f64 = 6.0;
const TOP: f64 = 8.0;
const BOTTOM: f64 = 26.0;

/// The drawing width for the window, read once when the card is built.
fn view_width() -> f64 {
    #[cfg(target_arch = "wasm32")]
    {
        let width = web_sys::window()
            .and_then(|w| w.inner_width().ok())
            .and_then(|w| w.as_f64())
            .unwrap_or(1024.0);
        if width < 600.0 {
            return VIEW_W_NARROW;
        }
    }
    VIEW_W_WIDE
}

type Loaded = Option<Result<GridDay, HistoryApiError>>;
/// The hub's today and first day with data, from the last good reply.
type Known = Option<(String, Option<String>)>;

#[cfg(target_arch = "wasm32")]
fn page_hidden() -> bool {
    web_sys::window()
        .and_then(|w| w.document())
        .is_some_and(|d| d.hidden())
}

/// Fetches the requested day; a reply that arrives after the user moved on is dropped.
#[cfg(target_arch = "wasm32")]
fn load(day: RwSignal<Option<String>>, data: RwSignal<Loaded>, known: RwSignal<Known>) {
    let requested = day.get_untracked();
    spawn_local(async move {
        let result = crate::api::history::fetch_grid_day(requested.as_deref()).await;
        if day.try_get_untracked() != Some(requested.clone()) {
            return;
        }
        if let Ok(d) = &result {
            if !crate::energy_day::accepts(requested.as_deref(), d) {
                return;
            }
            known.try_set(Some((d.today.clone(), d.first_date.clone())));
        }
        data.try_set(Some(result));
    });
}

#[cfg(not(target_arch = "wasm32"))]
fn load(_day: RwSignal<Option<String>>, _data: RwSignal<Loaded>, _known: RwSignal<Known>) {}

/// The bars, gridlines, bands and labels of one day, as SVG children.
fn chart(
    lang: Lang,
    day: &GridDay,
    hover: RwSignal<Option<usize>>,
    view_w: f64,
) -> impl IntoView + use<> {
    let s = scale(day);
    let plot_w = view_w - LEFT - RIGHT;
    let plot_h = VIEW_H - TOP - BOTTOM;
    let zero = TOP + plot_h * s.up / (s.up + s.down);
    let per_kwh = plot_h / (s.up + s.down);
    let slot_w = plot_w / day.day_slots.max(1) as f64;
    let bar_w = slot_w * 0.7;

    let bands = day
        .bands
        .iter()
        .map(|&(from, to)| {
            let x = LEFT + x_of(from, day) * plot_w;
            let w = (x_of(to, day) - x_of(from, day)) * plot_w;
            view! {
                <rect class="energy-day__band" x=x y=TOP width=w height=plot_h />
            }
        })
        .collect_view();

    let step = if s.up + s.down > 3.0 { 1.0 } else { 0.5 };
    let mut lines = Vec::new();
    let mut k = step;
    while k <= s.up + 1e-9 {
        lines.push((zero - k * per_kwh, k));
        k += step;
    }
    let mut k = step;
    while k <= s.down + 1e-9 {
        lines.push((zero + k * per_kwh, k));
        k += step;
    }
    let grid = lines
        .into_iter()
        .map(|(y, k)| {
            view! {
                <line class="energy-day__gridline" x1=LEFT x2=view_w - RIGHT y1=y y2=y />
                <text class="energy-day__axis" x=LEFT - 6.0 y=y + 3.5 text-anchor="end">
                    {format!("{k}")}
                </text>
            }
        })
        .collect_view();

    let tariffs = day.tariffs;
    let bars = day
        .slots
        .iter()
        .enumerate()
        .map(|(i, slot)| {
            let x = LEFT + i as f64 * slot_w + (slot_w - bar_w) / 2.0;
            let off_h = slot.off_peak * per_kwh;
            let peak_h = slot.peak * per_kwh;
            let exp_h = slot.export * per_kwh;
            let label = tooltip(lang, slot, tariffs);
            let title = label.clone();
            let (off_class, peak_class) = if tariffs {
                ("energy-day__bar energy-day__bar--off-peak", "energy-day__bar energy-day__bar--peak")
            } else {
                ("energy-day__bar energy-day__bar--bought", "energy-day__bar energy-day__bar--bought")
            };
            view! {
                <g
                    class="energy-day__slot"
                    class:energy-day__slot--active=move || hover.get() == Some(i)
                    tabindex="0"
                    role="img"
                    aria-label=label
                    on:mouseenter=move |_| hover.set(Some(i))
                    on:focus=move |_| hover.set(Some(i))
                    on:click=move |_| hover.set(Some(i))
                >
                    <title>{title}</title>
                    <rect class="energy-day__hit" x=LEFT + i as f64 * slot_w y=TOP width=slot_w height=plot_h />
                    <rect class=off_class x=x y=zero - off_h width=bar_w height=off_h />
                    <rect class=peak_class x=x y=zero - off_h - peak_h width=bar_w height=peak_h />
                    <rect class="energy-day__bar energy-day__bar--export" x=x y=zero width=bar_w height=exp_h />
                </g>
            }
        })
        .collect_view();

    let wide = tick_hours(day.day_slots, false);
    let narrow = tick_hours(day.day_slots, true);
    let local_at = |i: usize| {
        day.slots
            .get(i)
            .map(|s| s.local.clone())
            .unwrap_or_else(|| {
                // Past the last slot of a partial today: count on from midnight.
                let minutes = i * 30;
                format!("{:02}:{:02}", (minutes / 60) % 24, minutes % 60)
            })
    };
    let ticks = wide
        .into_iter()
        .map(|i| {
            let x = LEFT + i as f64 * slot_w;
            let text = local_at(i);
            let shown = chrono::NaiveTime::parse_from_str(&text, "%H:%M").map_or(text, time);
            let class = if narrow.contains(&i) {
                "energy-day__axis"
            } else {
                "energy-day__axis energy-day__axis--wide"
            };
            view! {
                <text class=class x=x y=VIEW_H - 6.0 text-anchor="middle">{shown}</text>
            }
        })
        .collect_view();

    view! {
        {bands}
        {grid}
        {bars}
        <line class="energy-day__zero" x1=LEFT x2=view_w - RIGHT y1=zero y2=zero />
        <text class="energy-day__axis" x=LEFT - 6.0 y=zero + 3.5 text-anchor="end">"0"</text>
        {ticks}
    }
}

/// The grid day card, placed under the live diagram.
#[component]
pub fn GridDayCard() -> impl IntoView {
    // The app's minute clock; a page mounted without it (a test) just never refreshes.
    let now = use_context::<Now>();
    // `None` is today, as the hub sees it.
    let day: RwSignal<Option<String>> = RwSignal::new(None);
    let data: RwSignal<Loaded> = RwSignal::new(None);
    let known: RwSignal<Known> = RwSignal::new(None);
    let hover: RwSignal<Option<usize>> = RwSignal::new(None);
    let view_w = view_width();
    let lang = use_lang();

    Effect::new(move |_| {
        day.track();
        hover.set(None);
        load(day, data, known);
    });
    // Today follows the minute clock while the page is visible.
    Effect::new(move |prev: Option<u64>| {
        let t = now.map_or(0, |n| n.0.get());
        #[cfg(target_arch = "wasm32")]
        if prev.is_some() && day.get_untracked().is_none() && !page_hidden() {
            load(day, data, known);
        }
        let _ = prev;
        t
    });

    // Navigation follows the requested day and the last known today, so a failed load never
    // strands the arrows.
    let shown_date = move || {
        known.with(|k| {
            let (today, first) = k.clone()?;
            let date = shown_day(day.get().as_deref(), Some(&today))?;
            Some((date, today, first))
        })
    };
    let label = move || {
        shown_date()
            .map(|(date, today, _)| day_label(lang(), &date, &today))
            .unwrap_or_else(|| word(lang(), EnergyText::Today).to_string())
    };
    let step = move |by: i64| {
        shown_date().and_then(|(date, today, first)| {
            shift_day(&date, by, first.as_deref(), &today).map(|next| (next == today, next))
        })
    };
    let go = move |by: i64| {
        if let Some((is_today, next)) = step(by) {
            day.set((!is_today).then_some(next));
        }
    };

    let body = move || match data.get() {
        None => view! { <div class="energy-day__skeleton" aria-busy="true"></div> }.into_any(),
        Some(Err(HistoryApiError::Unavailable)) => view! {
            <p class="energy-day__empty">
                {word(lang(), EnergyText::HistoryOff)} " "
                <a href="https://gekkotron.github.io/nidoo/docs/users/history">
                    {word(lang(), EnergyText::SetItUp)}
                </a>
            </p>
        }
        .into_any(),
        Some(Err(HistoryApiError::NoGridMeter)) => view! {
            <p class="energy-day__empty">
                {word(lang(), EnergyText::GiveGridRole)} " "
                <a href="/devices">{text(lang(), Key::Devices)}</a>
            </p>
        }
        .into_any(),
        Some(Err(err)) => {
            view! { <p class="energy-day__empty">{history_error(lang(), &err)}</p> }.into_any()
        }
        Some(Ok(d)) if !has_data(&d) => {
            view! { <p class="energy-day__empty">{word(lang(), EnergyText::NoData)}</p> }.into_any()
        }
        Some(Ok(d)) => {
            let lang_now = lang();
            let (bought, split, sold) = totals_text(lang_now, &d);
            let summary = aria_summary(lang_now, &d);
            let detail = move || {
                hover.get().and_then(|i| {
                    data.with(|r| {
                        r.as_ref()
                            .and_then(|r| r.as_ref().ok())
                            .and_then(|d| d.slots.get(i).map(|s| tooltip(lang(), s, d.tariffs)))
                    })
                })
            };
            let tariffs = d.tariffs;
            let has_export = d.has_export;
            view! {
                <div class="energy-day__totals">
                    <div>
                        <div class="energy-day__total">{bought}</div>
                        {split.map(|s| view! { <div class="energy-day__split">{s}</div> })}
                    </div>
                    {sold.map(|s| view! { <div class="energy-day__total">{s}</div> })}
                </div>
                <svg
                    class="energy-day__svg"
                    viewBox=format!("0 0 {view_w} {VIEW_H}")
                    role="group"
                    aria-label=summary
                    on:mouseleave=move |_| hover.set(None)
                >
                    {chart(lang_now, &d, hover, view_w)}
                </svg>
                <p class="energy-day__detail" aria-live="polite">
                    {move || detail().unwrap_or_default()}
                </p>
                <ul class="energy-day__legend">
                    {if tariffs {
                        view! {
                            <li><span class="energy-day__key energy-day__key--off-peak"></span>{word(lang_now, EnergyText::OffPeak)}</li>
                            <li><span class="energy-day__key energy-day__key--peak"></span>{word(lang_now, EnergyText::Peak)}</li>
                        }
                        .into_any()
                    } else {
                        view! {
                            <li><span class="energy-day__key energy-day__key--bought"></span>{word(lang_now, EnergyText::Bought)}</li>
                        }
                        .into_any()
                    }}
                    {has_export.then(|| view! {
                        <li><span class="energy-day__key energy-day__key--export"></span>{word(lang_now, EnergyText::SolarExport)}</li>
                    })}
                </ul>
            }
            .into_any()
        }
    };

    view! {
        <section
            class="card energy-day"
            aria-label=move || word(lang(), EnergyText::DayCard)
        >
            <header class="energy-day__header">
                <button
                    type="button"
                    class="btn btn--compact energy-day__nav"
                    aria-label=move || word(lang(), EnergyText::PrevDay)
                    disabled=move || step(-1).is_none()
                    on:click=move |_| go(-1)
                >
                    "‹"
                </button>
                <h2 class="energy-day__title">{label}</h2>
                <button
                    type="button"
                    class="btn btn--compact energy-day__nav"
                    aria-label=move || word(lang(), EnergyText::NextDay)
                    disabled=move || step(1).is_none()
                    on:click=move |_| go(1)
                >
                    "›"
                </button>
            </header>
            {body}
        </section>
    }
}
