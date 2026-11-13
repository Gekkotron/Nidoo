//! The solar page the header's solar card links to: today, the day curve, the figures that
//! compare forecast and production, the next days (each one opens its own curve) and the source.

use std::collections::BTreeMap;

use chrono::{DateTime, Datelike, NaiveDate, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;
use leptos::prelude::*;
use leptos::task::spawn_local;
use nidoo_api::{EnergyKind, SolarView};

use crate::api::history::fetch_daily;
use crate::components::now::{Now, unix_now};
use crate::energy::energy_entities;
use crate::home_header::mood::sun_today;
use crate::house::House;
use crate::i18n::{self, Lang, SolarText as T, WeatherText, solar_text, weather_text};
use crate::nav::BackLink;
use crate::screens::flow_canvas::sun::house_place;
use crate::solar::card::{curve_points, today_is_current, today_total};
use crate::solar::curve::{Curve, day_curve, hour_ticks, producing_span};
use crate::solar::measured::{Generation, SolarMeasured};
use crate::solar::model::{
    SolarSources, Yesterday, accuracy, best_hours_floor, daily_by_date, day_bounds,
    forecast_so_far, gap_percent, local_date, measured_power_kw, month_so_far, periods_in,
    solar_sources, sum_daily, tomorrow, yesterday,
};

const W: f64 = 600.0;
const H: f64 = 180.0;
/// Days of history read for yesterday, the accuracy and the month: the hub's daily window is
/// today−(n−1)…today and accepts only 7, 30 or 365, so on the 31st the 1st needs the long one.
fn history_days(today: NaiveDate) -> u16 {
    if today.day() > 30 { 365 } else { 30 }
}

/// What the page shows besides the curve; recomputed when the forecast, the clock or the
/// measures change, and only notifies when a figure actually changes.
#[derive(Clone, Debug, PartialEq)]
struct PageData {
    measured: Option<f64>,
    so_far: f64,
    total: Option<f64>,
    range: Option<(f64, f64)>,
    remaining: Option<f64>,
    tomorrow: Option<(f64, f64, f64)>,
    measured_kw: Option<f64>,
    forecast_kw: Option<f64>,
    peak: Option<(f64, NaiveTime)>,
    best: Option<(NaiveTime, NaiveTime, Option<f64>)>,
    gap: Option<i32>,
    sun: Option<(NaiveTime, NaiveTime)>,
    yesterday: Option<(Yesterday, Option<NaiveTime>)>,
    accuracy: Option<u32>,
    month: Option<f64>,
    roofs: Vec<(u32, Option<f64>)>,
    days: Vec<(NaiveDate, f64, f64, f64)>,
    updated: Option<NaiveTime>,
    next: Option<NaiveTime>,
    calls: u32,
    limit: u32,
    stale_at: Option<NaiveTime>,
}

/// The curve, whether its measured line is the forecast's own estimates, and the hour ticks.
type PageCurve = (Curve, bool, Vec<(f64, u32)>);

fn local_time(zone: Tz, t: u64) -> Option<NaiveTime> {
    DateTime::<Utc>::from_timestamp(t as i64, 0).map(|t| t.with_timezone(&zone).time())
}

/// Unix seconds of the sunrise on house-local `date`.
fn sunrise_utc(house: House, zone: Tz, date: NaiveDate) -> Option<u64> {
    let noon = zone
        .from_local_datetime(&date.and_hms_opt(12, 0, 0)?)
        .earliest()?
        .with_timezone(&Utc);
    let (sunrise, ..) = house.flows_config.with(|c| sun_today(c.as_ref(), noon));
    let at = zone
        .from_local_datetime(&date.and_time(sunrise?))
        .earliest()?;
    Some(at.timestamp().max(0) as u64)
}

fn page_data(
    house: House,
    now: u64,
    measured: Option<SolarMeasured>,
    daily: Option<&BTreeMap<NaiveDate, f64>>,
) -> Option<PageData> {
    let zone = house.flows_config.with(|c| house_place(c.as_ref()).1);
    let measured_kw = house
        .entities
        .with(|m| measured_power_kw(&energy_entities(m)));
    let (sunrise, sunset, _, fallback) = house.flows_config.with(|c| {
        sun_today(
            c.as_ref(),
            DateTime::<Utc>::from_timestamp(now as i64, 0).unwrap_or_default(),
        )
    });
    house.solar.with(|s| {
        let s = s.as_ref()?;
        let today = local_date(zone, now);
        let (from, to) = day_bounds(zone, today);
        let day = periods_in(&s.periods, from, to);
        let today_kwh = measured.and_then(|m| m.today_kwh.get());
        let so_far = forecast_so_far(&day, now);
        let current = today_is_current(s, today);
        // Only the days before today count as measured history; today comes from `today_kwh`.
        let past: BTreeMap<NaiveDate, f64> = daily
            .map(|d| d.range(..today).map(|(k, v)| (*k, *v)).collect())
            .unwrap_or_default();
        let yesterday_row = today.pred_opt().and_then(|date| {
            yesterday(
                &past,
                &s.history.forecasts,
                date,
                sunrise_utc(house, zone, date),
            )
        });
        let late = yesterday_row
            .as_ref()
            .and_then(|y| y.made_late_at)
            .and_then(|t| local_time(zone, t));
        Some(PageData {
            measured: today_kwh,
            so_far,
            total: today_total(s, &day, today),
            range: current.then(|| s.today.low.zip(s.today.high)).flatten(),
            remaining: s.now.remaining_today,
            tomorrow: tomorrow(s, today).map(|d| (d.estimate, d.low, d.high)),
            measured_kw,
            forecast_kw: s.now.power,
            peak: s
                .today
                .peak
                .as_ref()
                .filter(|_| current)
                .and_then(|p| Some((p.power, local_time(zone, p.at)?))),
            best: s
                .today
                .best_hours
                .as_ref()
                .filter(|_| current)
                .and_then(|b| {
                    Some((
                        local_time(zone, b.from)?,
                        local_time(zone, b.to)?,
                        s.today.peak.as_ref().map(|p| best_hours_floor(p.power)),
                    ))
                }),
            gap: gap_percent(today_kwh, so_far),
            sun: (!fallback).then_some(sunrise.zip(sunset)).flatten(),
            yesterday: yesterday_row.map(|y| (y, late)),
            accuracy: accuracy(&past, &s.history.forecasts, |d| sunrise_utc(house, zone, d)),
            month: daily.and_then(|_| month_so_far(&past, today, today_kwh)),
            roofs: if s.sites.len() >= 2 {
                s.sites.iter().map(|r| (r.index + 1, r.today)).collect()
            } else {
                Vec::new()
            },
            days: s
                .days
                .iter()
                .filter_map(|d| {
                    let date = NaiveDate::parse_from_str(&d.date, "%Y-%m-%d").ok()?;
                    Some((date, d.estimate, d.low, d.high))
                })
                .collect(),
            updated: local_time(zone, s.fetched_at),
            next: s.next_refresh_at.and_then(|t| local_time(zone, t)),
            calls: s.calls_today,
            limit: s.daily_limit,
            stale_at: s.stale.then(|| local_time(zone, s.fetched_at)).flatten(),
        })
    })
}

/// The day the curve shows: the picked one while it is still ahead, else today (`None`).
fn shown_day(picked: Option<NaiveDate>, today: NaiveDate) -> Option<NaiveDate> {
    picked.filter(|d| *d > today)
}

/// The curve of the shown day, whether its measured line is the forecast's own estimates, and
/// the `(x, local hour)` ticks of its span.
fn page_curve(
    house: House,
    now: u64,
    measured: Option<SolarMeasured>,
    picked: Option<NaiveDate>,
) -> Option<PageCurve> {
    let zone = house.flows_config.with(|c| house_place(c.as_ref()).1);
    house.solar.with(|s| {
        let s: &SolarView = s.as_ref()?;
        let today = local_date(zone, now);
        match shown_day(picked, today) {
            None => {
                let (from, to) = day_bounds(zone, today);
                let day = periods_in(&s.periods, from, to);
                let real = measured.map(|m| m.periods.get()).unwrap_or_default();
                let (points, is_forecast) = curve_points(real, &day, now);
                let span = producing_span(&day)?;
                Some((
                    day_curve(&day, &points, span, now, W, H),
                    is_forecast,
                    hour_ticks(span, zone, W),
                ))
            }
            Some(date) => {
                let (from, to) = day_bounds(zone, date);
                let day = periods_in(&s.periods, from, to);
                let span = producing_span(&day)?;
                Some((
                    day_curve(&day, &[], span, 0, W, H),
                    false,
                    hour_ticks(span, zone, W),
                ))
            }
        }
    })
}

/// kWh per house-local date over the last 30 days, summed over the solar sources: the counters
/// when there are any, else the power sensors. `None` when a source fails or none has data.
async fn load_daily(zone: Tz, days: u16, src: SolarSources) -> Option<BTreeMap<NaiveDate, f64>> {
    let mut maps = Vec::new();
    if src.counters.is_empty() {
        for p in &src.powers {
            let d = fetch_daily(&p.id, days, EnergyKind::Power).await.ok()?;
            maps.push(daily_by_date(zone, &d.days, p.sign));
        }
    } else {
        for id in &src.counters {
            let d = fetch_daily(id, days, EnergyKind::Counter).await.ok()?;
            maps.push(daily_by_date(zone, &d.days, 1.0));
        }
    }
    let sum = sum_daily(&maps);
    (!sum.is_empty()).then_some(sum)
}

fn tile(label: String, value: String, sub: String) -> impl IntoView {
    view! {
        <div class="solar-tile">
            <div class="solar-tile__label">{label}</div>
            <div class="solar-tile__value">{value}</div>
            <div class="solar-tile__sub">{sub}</div>
        </div>
    }
}

/// One day of the next days: a button that shows its curve, or today's again when it is shown.
fn day_column(
    lang: Lang,
    d: (NaiveDate, f64, f64, f64),
    max: f64,
    picked: RwSignal<Option<NaiveDate>>,
) -> impl IntoView {
    let (date, estimate, low, high) = d;
    let pct = if max > 0.0 {
        estimate / max * 100.0
    } else {
        0.0
    };
    view! {
        <button
            type="button"
            class="solar-day"
            aria-pressed=move || (picked.get() == Some(date)).to_string()
            on:click=move |_| picked.update(|p| *p = (*p != Some(date)).then_some(date))
        >
            <div class="solar-day__name">{i18n::short_weekday(lang, date)}</div>
            <div class="solar-day__track">
                <div class="solar-day__bar" style=format!("height: {pct:.0}%")></div>
            </div>
            <div class="solar-day__value">
                <b>{i18n::solar_kwh(lang, estimate)}</b>
                " "
                <span>
                    {format!(
                        "{}–{}",
                        i18n::home_number(lang, low.round() + 0.0, 0),
                        i18n::home_number(lang, high.round() + 0.0, 0),
                    )}
                </span>
            </div>
        </button>
    }
}

/// The figures under the curve; each one is left out when its input is missing.
fn tiles(d: &PageData, lang: Lang) -> Vec<AnyView> {
    let kwh = |v: f64| format!("{} {}", i18n::solar_kwh(lang, v), solar_text(lang, T::Kwh));
    let mut out = Vec::new();
    let mut add = |label: &'static str, value: String, sub: String| {
        out.push(tile(label.to_owned(), value, sub).into_any());
    };
    if let Some((kw, at)) = d.peak {
        add(
            solar_text(lang, T::Peak),
            i18n::solar_kw(lang, kw),
            i18n::home_time(lang, at),
        );
    }
    if let Some((from, to, floor)) = d.best {
        add(
            solar_text(lang, T::BestHours),
            format!(
                "{}–{}",
                i18n::home_time(lang, from),
                i18n::home_time(lang, to)
            ),
            floor
                .map(|f| i18n::solar_above(lang, f))
                .unwrap_or_default(),
        );
    }
    if let Some(g) = d.gap {
        add(
            solar_text(lang, T::Gap),
            i18n::solar_percent(lang, g),
            solar_text(lang, T::GapSub).to_owned(),
        );
    }
    if let Some((rise, set)) = d.sun {
        add(
            weather_text(lang, WeatherText::Sun),
            format!(
                "{} · {}",
                i18n::home_time(lang, rise),
                i18n::home_time(lang, set)
            ),
            String::new(),
        );
    }
    if let Some((y, late)) = &d.yesterday {
        add(
            solar_text(lang, T::Yesterday),
            kwh(y.measured + 0.0),
            y.forecast
                .map(|f| i18n::solar_yesterday_sub(lang, f, y.gap, *late))
                .unwrap_or_default(),
        );
    }
    if let Some(a) = d.accuracy {
        add(
            solar_text(lang, T::Accuracy),
            format!("± {} %", i18n::home_number(lang, f64::from(a), 0)),
            String::new(),
        );
    }
    if !d.roofs.is_empty() {
        add(
            solar_text(lang, T::Roofs),
            i18n::solar_roofs(lang, &d.roofs),
            String::new(),
        );
    }
    if let Some(m) = d.month {
        add(solar_text(lang, T::Month), kwh(m + 0.0), String::new());
    }
    out
}

/// The curve card: the shown day's name over its curve, with a way back to today.
#[component]
fn CurveCard(
    lang: RwSignal<Lang>,
    curve: Memo<Option<PageCurve>>,
    shown: Memo<Option<NaiveDate>>,
    picked: RwSignal<Option<NaiveDate>>,
) -> impl IntoView {
    let is_today = move || shown.get().is_none();
    let field = move |pick: fn(&Curve) -> String| {
        move || curve.get().map(|c| pick(&c.0)).unwrap_or_default()
    };
    // Production is a line over the band; while nothing is measured, the hours already past are
    // a faint fill instead.
    let measured_area = move || {
        curve
            .get()
            .filter(|c| c.1)
            .map(|c| c.0.measured_area)
            .unwrap_or_default()
    };
    let measured_line = move || {
        curve
            .get()
            .filter(|c| !c.1)
            .map(|c| c.0.measured_line)
            .unwrap_or_default()
    };
    let now_line = move || {
        curve.get().and_then(|c| {
            c.0.now_x.map(|x| {
                view! { <line class="solar-curve__now" x1=x x2=x y1=c.0.now_top y2=H /> }
            })
        })
    };
    let hours = move || {
        let l = lang.get();
        curve
            .get()
            .map(|c| c.2)
            .unwrap_or_default()
            .into_iter()
            .map(|(x, hour)| {
                view! {
                    <line class="solar-curve__tick" x1=x x2=x y1=H y2=H + 4.0 />
                    <text class="solar-curve__label solar-curve__hour" x=x y=H + 14.0>
                        {i18n::solar_hour(l, hour)}
                    </text>
                }
            })
            .collect_view()
    };
    let grid = move || {
        let l = lang.get();
        curve
            .get()
            .map(|c| c.0.grid)
            .unwrap_or_default()
            .into_iter()
            .map(|(y, kw)| {
                let label_y = (y - 3.0).max(10.0);
                view! {
                    <line class="solar-curve__grid" x1="0" x2=W y1=y y2=y />
                    <text class="solar-curve__label" x="4" y=label_y>
                        {i18n::solar_kw(l, kw)}
                    </text>
                }
            })
            .collect_view()
    };
    // One entry per series drawn: the measured line or the estimated fill on Today only, and
    // the now line while it is inside the span.
    let legend = move || {
        let l = lang.get();
        let today = is_today();
        let (estimated, has_now) = curve
            .get()
            .map(|c| (c.1, c.0.now_x.is_some()))
            .unwrap_or_default();
        [
            Some(("band", T::LegendRange)),
            Some(("forecast", T::LegendForecast)),
            (today && !estimated).then_some(("measured", T::LegendMeasured)),
            (today && estimated).then_some(("so-far", T::LegendSoFar)),
            (today && has_now).then_some(("now", T::LegendNow)),
        ]
        .into_iter()
        .flatten()
        .map(|(kind, text)| {
            view! {
                <li>
                    <span class=format!("solar-legend__swatch solar-legend__swatch--{kind}") />
                    {solar_text(l, text)}
                </li>
            }
        })
        .collect_view()
    };
    view! {
        <div class="solar-page__curve">
            <div class="solar-page__curve-head">
                <div class="solar-page__day">
                    <span class="solar-page__day-name">
                        {move || match shown.get() {
                            Some(date) => i18n::short_weekday(lang.get(), date),
                            None => solar_text(lang.get(), T::Today).to_owned(),
                        }}
                    </span>
                    <button
                        type="button"
                        class="solar-page__today"
                        hidden=is_today
                        on:click=move |_| picked.set(None)
                    >
                        {move || solar_text(lang.get(), T::BackToToday)}
                    </button>
                </div>
                <ul class="solar-legend">{legend}</ul>
            </div>
            <svg class="solar-page__svg" viewBox="0 0 600 198" role="img"
                aria-label=move || solar_text(lang.get(), T::CardLabel)>
                {grid}
                <path class="solar-curve__band" d=field(|c| c.band.clone()) />
                {move || {
                    is_today()
                        .then(|| view! { <path class="solar-curve__measured" d=measured_area /> })
                }}
                <path class="solar-curve__line solar-curve__line--dashed" d=field(|c| c.forecast_line.clone()) />
                {move || {
                    is_today()
                        .then(|| view! { <path class="solar-curve__measured-line" d=measured_line /> })
                }}
                {now_line}
                {hours}
            </svg>
        </div>
    }
}

/// The page body. The curve card is built once; every figure reads `data` in its own closure, so
/// a clock tick or a sensor report updates text in place and never rebuilds a node holding focus.
fn content(
    data: Memo<Option<PageData>>,
    lang: RwSignal<Lang>,
    curve: Memo<Option<PageCurve>>,
    shown: Memo<Option<NaiveDate>>,
    picked: RwSignal<Option<NaiveDate>>,
) -> impl IntoView {
    let from_data = move |f: fn(&PageData, Lang) -> String| {
        move || data.get().map(|d| f(&d, lang.get())).unwrap_or_default()
    };
    view! {
        <div class="solar-page__top">
            <div>
                <div class="solar-page__title">
                    {from_data(|d, l| i18n::solar_title(l, d.measured, d.so_far, d.total))}
                </div>
                <div class="solar-page__range">
                    {from_data(|d, l| {
                        d.range.map(|(lo, hi)| i18n::solar_range(l, lo, hi)).unwrap_or_default()
                    })}
                </div>
            </div>
            <div class="solar-page__side">
                <div>
                    {from_data(|d, l| i18n::solar_power_line(l, d.measured_kw, d.forecast_kw))}
                </div>
                <div>{from_data(|d, l| i18n::solar_bottom(l, d.remaining, d.measured, d.tomorrow))}</div>
            </div>
        </div>

        <CurveCard lang curve shown picked />

        <div class="solar-tiles">
            {move || data.get().map(|d| tiles(&d, lang.get()))}
        </div>

        <h3 class="solar-page__label">{move || solar_text(lang.get(), T::NextDays)}</h3>
        <div class="solar-days">
            {move || {
                data.get()
                    .map(|d| {
                        let l = lang.get();
                        let max = d.days.iter().map(|d| d.1).fold(0.0_f64, f64::max);
                        d.days
                            .into_iter()
                            .map(|day| day_column(l, day, max, picked))
                            .collect_view()
                    })
            }}
        </div>

        <footer class="solar-page__foot">
            <p>
                {from_data(|d, l| i18n::solar_footer(l, d.updated, d.next, d.calls, d.limit))}
            </p>
            {move || {
                data.get()
                    .and_then(|d| d.stale_at)
                    .map(|at| view! { <p>{i18n::data_from(lang.get(), at)}</p> })
            }}
            <p>{move || solar_text(lang.get(), T::Credit)}</p>
        </footer>
    }
}

/// The solar page; with no forecast it says how to turn it on.
#[component]
pub fn SolarScreen() -> impl IntoView {
    let house: House = expect_context();
    let lang = use_context::<RwSignal<Lang>>().unwrap_or_else(|| RwSignal::new(Lang::En));
    let now = use_context::<Now>().map_or_else(|| RwSignal::new(unix_now()), |n| n.0);
    let measured = use_context::<SolarMeasured>();
    // The next day picked in the list; `None` shows today.
    let picked: RwSignal<Option<NaiveDate>> = RwSignal::new(None);
    let daily: RwSignal<Option<BTreeMap<NaiveDate, f64>>> = RwSignal::new(None);

    // The house-local date: a new day makes yesterday a different day, so the history is re-read.
    let date = Memo::new(move |_| {
        let zone = house.flows_config.with(|c| house_place(c.as_ref()).1);
        local_date(zone, now.get())
    });
    // The last 30 days, read once the solar entities are known and again on a new local date.
    let sources = Memo::new(move |_| house.entities.with(|m| solar_sources(&energy_entities(m))));
    let generation = Generation::default();
    Effect::new(move |_| {
        let src = sources.get();
        date.track();
        if src.counters.is_empty() && src.powers.is_empty() {
            return;
        }
        let zone = house
            .flows_config
            .with_untracked(|c| house_place(c.as_ref()).1);
        let g = generation.next();
        let generation = generation.clone();
        let days = history_days(local_date(zone, unix_now()));
        spawn_local(async move {
            let loaded = load_daily(zone, days, src).await;
            if generation.is_current(g) {
                daily.set(loaded);
            }
        });
    });

    let data =
        Memo::new(move |_| daily.with(|d| page_data(house, now.get(), measured, d.as_ref())));
    let shown = Memo::new(move |_| shown_day(picked.get(), date.get()));
    let curve = Memo::new(move |_| page_curve(house, now.get(), measured, picked.get()));
    let has_solar = Memo::new(move |_| house.solar.with(Option::is_some));
    view! {
        <section class="solar-page">
            <BackLink />
            {move || {
                if has_solar.get() {
                    content(data, lang, curve, shown, picked).into_any()
                } else if !house.solar_loaded.get() {
                    // Nothing until the first fetch has answered, then the hint.
                    ().into_any()
                } else {
                    let l = lang.get();
                    view! {
                        <div class="solar-page__empty">
                            <p>{solar_text(l, T::Unavailable)}</p>
                            <p class="solar-page__hint">{solar_text(l, T::EnableHint)}</p>
                        </div>
                    }
                        .into_any()
                }
            }}
        </section>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, d).unwrap()
    }

    #[test]
    fn history_window_covers_the_first_of_the_month() {
        assert_eq!(history_days(date(1)), 30);
        assert_eq!(history_days(date(30)), 30);
        assert_eq!(history_days(date(31)), 365);
    }
}
