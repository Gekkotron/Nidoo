//! The weather page the header's weather card links to: now or a chosen day, the next five days,
//! the source.

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use leptos::prelude::*;
use leptos_router::NavigateOptions;
use leptos_router::hooks::{use_navigate, use_query_map};
use nidoo_api::{WeatherDay, WeatherView};

use crate::components::now::{Now, unix_now};
use crate::home_header::mood::{Detail, RAIN_MIN_MM, sun_today};
use crate::home_header::view::degrees;
use crate::home_header::weather_fmt::{compass_index, condition_icon, uv_level};
use crate::house::House;
use crate::i18n::{self, Lang, WeatherText as T, format, weather_text};
use crate::icons;
use crate::nav::BackLink;
use crate::screens::flow_canvas::sun::house_place;

const DASH: &str = "–";

/// A number with `decimals` places and a unit after a no-break space, or a dash when the value is
/// unknown.
fn measure(lang: Lang, v: Option<f64>, decimals: usize, unit: Option<&str>) -> String {
    v.map_or_else(
        || DASH.to_owned(),
        |v| format::with_unit(i18n::home_number(lang, v + 0.0, decimals), unit),
    )
}

fn tile(label: &'static str, value: AnyView, sub: String) -> impl IntoView {
    view! {
        <div class="weather-tile">
            <div class="weather-tile__label">{label}</div>
            <div class="weather-tile__value">{value}</div>
            <div class="weather-tile__sub">{sub}</div>
        </div>
    }
}

/// One card of the day strip: the key is the `day` query value, `None` for today.
struct Card {
    key: Option<String>,
    title: String,
    icon: Option<&'static str>,
    min: Option<f64>,
    max: Option<f64>,
    chance: Option<f64>,
}

fn today_card(lang: Lang, w: &WeatherView) -> Card {
    Card {
        key: None,
        title: weather_text(lang, T::Today).to_owned(),
        icon: w.current.condition.map(|c| condition_icon(c, true)),
        min: w.today.min,
        max: w.today.max,
        chance: w.today.precipitation_probability,
    }
}

fn forecast_card(lang: Lang, d: &WeatherDay) -> Card {
    let title = NaiveDate::parse_from_str(&d.date, "%Y-%m-%d")
        .map(|date| i18n::short_weekday(lang, date))
        .unwrap_or_else(|_| d.date.clone());
    Card {
        key: Some(d.date.clone()),
        title,
        icon: d.condition.map(|c| condition_icon(c, true)),
        min: d.min,
        max: d.max,
        chance: d.precipitation_probability,
    }
}

/// A tappable card of the strip; `chosen` marks the day the page shows.
fn day_card(lang: Lang, card: Card, chosen: bool, pick: Callback<Option<String>>) -> impl IntoView {
    let max = card
        .max
        .map_or_else(|| DASH.to_owned(), |t| degrees(lang, t));
    let min = card
        .min
        .map_or_else(|| DASH.to_owned(), |t| degrees(lang, t));
    let chance = card
        .chance
        .map(|p| format::with_unit(i18n::home_number(lang, p, 0), Some("%")));
    let key = card.key;
    view! {
        <button
            type="button"
            class="weather-day"
            class:weather-day--on=chosen
            aria-pressed=chosen.to_string()
            on:click=move |_| pick.run(key.clone())
        >
            <div class="weather-day__name">{card.title}</div>
            <span class="weather-day__icon" inner_html=card.icon.unwrap_or_default()></span>
            <div class="weather-day__range">
                <b>{max}</b>
                " "
                <span>{min}</span>
            </div>
            <div class="weather-day__rain">
                <span class="weather-day__drop" inner_html=icons::DROP></span>
                {chance}
            </div>
        </button>
    }
}

/// The forecast day the `day` query names; `None` (today's view) when it is missing, unknown or
/// today's date.
fn pick_day<'a>(
    query: Option<&str>,
    today: NaiveDate,
    days: &'a [WeatherDay],
) -> Option<&'a WeatherDay> {
    let asked = query?;
    if NaiveDate::parse_from_str(asked, "%Y-%m-%d").ok() == Some(today) {
        return None;
    }
    days.iter().find(|d| d.date == asked)
}

/// The big block at the top of the page.
struct Top {
    icon: &'static str,
    temp: String,
    headline: String,
    side: Vec<String>,
}

fn headline(label: Option<&str>, feels: Option<String>) -> String {
    [label.map(str::to_owned), feels]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ")
}

fn big_temp(lang: Lang, t: Option<f64>) -> String {
    t.map_or_else(
        || DASH.to_owned(),
        |t| format!("{}°", i18n::home_number(lang, t + 0.0, 1)),
    )
}

fn range_line(lang: Lang, name: &str, min: Option<f64>, max: Option<f64>) -> Option<String> {
    match (min, max) {
        (Some(lo), Some(hi)) => Some(format!(
            "{name} {} / {}",
            degrees(lang, lo),
            degrees(lang, hi)
        )),
        _ => None,
    }
}

fn now_top(w: &WeatherView, lang: Lang) -> Top {
    let cur = &w.current;
    let icon = cur
        .condition
        .map(|c| condition_icon(c, cur.daylight.unwrap_or(true)))
        .unwrap_or_default();
    let label = cur.condition.map(|c| i18n::condition_label(lang, c));
    let feels = cur
        .apparent_temperature
        .map(|t| i18n::feels_like(lang, &degrees(lang, t)));
    let tonight = w
        .tonight_min
        .map(|t| format!("{} {}", weather_text(lang, T::TonightMin), degrees(lang, t)));
    let rain_soon = match w.rain_next_hour {
        Some(mm) if mm > RAIN_MIN_MM => i18n::rain_next_hour(lang, &i18n::home_number(lang, mm, 1)),
        Some(_) => weather_text(lang, T::NoRainSoon).to_owned(),
        None => String::new(),
    };
    let side = [
        range_line(lang, weather_text(lang, T::Today), w.today.min, w.today.max),
        tonight,
        Some(rain_soon),
    ];
    Top {
        icon,
        temp: big_temp(lang, cur.temperature),
        headline: headline(label, feels),
        side: side.into_iter().flatten().collect(),
    }
}

fn day_top(d: &WeatherDay, lang: Lang, name: &str) -> Top {
    let label = d.condition.map(|c| i18n::condition_label(lang, c));
    let feels = d
        .apparent_max
        .map(|t| i18n::feels_like(lang, &degrees(lang, t)));
    let chance = d
        .precipitation_probability
        .map(|p| i18n::chance_sub(lang, Some(&i18n::home_number(lang, p, 0))));
    Top {
        icon: d
            .condition
            .map(|c| condition_icon(c, true))
            .unwrap_or_default(),
        temp: big_temp(lang, d.max),
        headline: headline(label, feels),
        side: [range_line(lang, name, d.min, d.max), chance]
            .into_iter()
            .flatten()
            .collect(),
    }
}

/// The numbers of the tiles, for now or for one forecast day.
struct Figures {
    wind_speed: Option<f64>,
    wind_direction: Option<f64>,
    wind_gusts: Option<f64>,
    humidity: Option<f64>,
    pressure: Option<f64>,
    cloud_cover: Option<f64>,
    rain_mm: Option<f64>,
    rain_sub: String,
    max_gusts: Option<f64>,
    max_gusts_sub: String,
    uv: Option<f64>,
    /// Any instant of the day the sun tile is about.
    midday: DateTime<Utc>,
}

fn chance_text(lang: Lang, p: Option<f64>) -> Option<String> {
    p.map(|p| i18n::home_number(lang, p, 0))
}

fn now_figures(w: &WeatherView, lang: Lang, midday: DateTime<Utc>) -> Figures {
    let cur = &w.current;
    let chance = chance_text(lang, w.today.precipitation_probability);
    Figures {
        wind_speed: cur.wind_speed,
        wind_direction: cur.wind_direction,
        wind_gusts: cur.wind_gusts,
        humidity: cur.humidity,
        pressure: cur.pressure,
        cloud_cover: cur.cloud_cover,
        rain_mm: w.today.precipitation,
        rain_sub: i18n::rain_sub(lang, chance.as_deref()),
        max_gusts: w.today.wind_gusts_max,
        max_gusts_sub: weather_text(lang, T::Today).to_lowercase(),
        uv: cur.uv_index,
        midday,
    }
}

fn day_figures(d: &WeatherDay, lang: Lang, name: &str, midday: DateTime<Utc>) -> Figures {
    let chance = chance_text(lang, d.precipitation_probability);
    Figures {
        wind_speed: d.wind_speed_max,
        wind_direction: d.wind_direction,
        wind_gusts: d.wind_gusts_max,
        humidity: d.humidity,
        pressure: d.pressure,
        cloud_cover: d.cloud_cover,
        rain_mm: d.precipitation,
        rain_sub: i18n::chance_sub(lang, chance.as_deref()),
        max_gusts: d.wind_gusts_max,
        max_gusts_sub: name.to_lowercase(),
        uv: d.uv_index_max,
        midday,
    }
}

fn wind_value(lang: Lang, f: &Figures) -> AnyView {
    match f.wind_speed {
        Some(speed) => {
            let arrow = f.wind_direction.map(|deg| {
                view! {
                    <span
                        class="weather-wind"
                        style=format!("transform: rotate({deg}deg)")
                        inner_html=icons::ARROW_UP
                    ></span>
                }
            });
            view! { {arrow} {measure(lang, Some(speed), 0, Some("km/h"))} }.into_any()
        }
        None => DASH.into_any(),
    }
}

/// The sun tile's value and sub-line for the day of `midday`.
fn sun_figures(lang: Lang, midday: DateTime<Utc>, house: House) -> (String, String) {
    let (sunrise, sunset, _, fallback) = house.flows_config.with(|c| sun_today(c.as_ref(), midday));
    match (fallback, sunset) {
        (false, Some(set)) => (
            i18n::home_time(lang, set),
            sunrise
                .map(|r| i18n::sunrise_sub(lang, r))
                .unwrap_or_default(),
        ),
        (true, _) => (
            DASH.to_owned(),
            i18n::detail_text(lang, &Detail::SunsetFallback),
        ),
        _ => (DASH.to_owned(), String::new()),
    }
}

fn tiles(lang: Lang, f: &Figures, house: House) -> AnyView {
    let compass = f
        .wind_direction
        .map(|d| i18n::compass_word(lang, compass_index(d)));
    let gusts = f.wind_gusts.map(|g| i18n::home_number(lang, g, 0));
    let uv_sub =
        f.uv.map(|uv| i18n::uv_word(lang, uv_level(uv)).to_owned())
            .unwrap_or_default();
    let (sun_value, sun_sub) = sun_figures(lang, f.midday, house);
    view! {
        <div class="weather-tiles">
            {tile(weather_text(lang, T::Wind), wind_value(lang, f), i18n::wind_sub(lang, compass, gusts.as_deref()))}
            {tile(weather_text(lang, T::Humidity), measure(lang, f.humidity, 0, Some("%")).into_any(), String::new())}
            {tile(weather_text(lang, T::Pressure), measure(lang, f.pressure, 0, Some("hPa")).into_any(), String::new())}
            {tile(weather_text(lang, T::Clouds), measure(lang, f.cloud_cover, 0, Some("%")).into_any(), String::new())}
            {tile(weather_text(lang, T::Rain), measure(lang, f.rain_mm, 1, Some("mm")).into_any(), f.rain_sub.clone())}
            {tile(weather_text(lang, T::MaxGusts), measure(lang, f.max_gusts, 0, Some("km/h")).into_any(), f.max_gusts_sub.clone())}
            {tile(weather_text(lang, T::UvIndex), measure(lang, f.uv, 0, None).into_any(), uv_sub)}
            {tile(weather_text(lang, T::Sun), sun_value.into_any(), sun_sub)}
        </div>
    }
    .into_any()
}

/// The noon of the house-local `date`.
fn local_noon(zone: chrono_tz::Tz, date: NaiveDate) -> Option<DateTime<Utc>> {
    date.and_hms_opt(12, 0, 0)
        .and_then(|noon| zone.from_local_datetime(&noon).earliest())
        .map(|t| t.with_timezone(&Utc))
}

/// The page body for one weather document; `midday` is any instant of the house-local day and
/// `day` the `day` query value: a forecast day to show instead of today.
fn content<'a>(
    w: &'a WeatherView,
    lang: Lang,
    midday: DateTime<Utc>,
    day: Option<&str>,
    pick: Callback<Option<String>>,
    house: House,
) -> impl IntoView + use<'a> {
    let zone = house.flows_config.with(|c| house_place(c.as_ref()).1);
    let today = midday.with_timezone(&zone).date_naive();
    let fetched = DateTime::<Utc>::from_timestamp(w.fetched_at as i64, 0)
        .map(|t| t.with_timezone(&zone).time());
    let updated = fetched.map(|at| i18n::updated_at(lang, at));

    let chosen = pick_day(day, today, &w.days);
    let (top, figures, heading) = match chosen {
        Some(d) => {
            let date = NaiveDate::parse_from_str(&d.date, "%Y-%m-%d").ok();
            let name = date.map_or_else(|| d.date.clone(), |n| i18n::day_heading(lang, n, today));
            let noon = date.and_then(|n| local_noon(zone, n)).unwrap_or(midday);
            (
                day_top(d, lang, &name),
                day_figures(d, lang, &name, noon),
                name,
            )
        }
        None => (
            now_top(w, lang),
            now_figures(w, lang, midday),
            weather_text(lang, T::Now).to_owned(),
        ),
    };

    let tiles = tiles(lang, &figures, house);
    let place = [Some(w.place.name.clone()), w.place.admin.clone()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(", ");
    let strip = std::iter::once((today_card(lang, w), chosen.is_none()))
        .chain(w.days.iter().map(|d| {
            let on = chosen.is_some_and(|c| c.date == d.date);
            (forecast_card(lang, d), on)
        }))
        .map(|(card, on)| day_card(lang, card, on, pick))
        .collect_view();
    let stale = w
        .stale
        .then(|| fetched.map(|at| i18n::data_from(lang, at)))
        .flatten();

    view! {
        <div class="weather-page__top">
            <span class="weather-page__icon" inner_html=top.icon></span>
            <div>
                <div class="weather-page__temp">{top.temp}</div>
                <div class="weather-page__headline">{top.headline}</div>
            </div>
            <div class="weather-page__side">
                {top.side.into_iter().map(|l| view! { <div>{l}</div> }).collect_view()}
            </div>
        </div>

        <h3 class="weather-page__label">{heading}</h3>
        {tiles}

        <h3 class="weather-page__label">{weather_text(lang, T::NextDays)}</h3>
        <div class="weather-days">{strip}</div>

        <footer class="weather-page__foot">
            <p>{place}{updated.map(|u| view! { " · " {u} })}</p>
            {stale.map(|s| view! { <p>{s}</p> })}
            <p>{weather_text(lang, T::Credit)}</p>
        </footer>
    }
}

/// The weather page; with no weather it says how to turn it on.
#[component]
pub fn WeatherScreen() -> impl IntoView {
    let house: House = expect_context();
    let lang = use_context::<RwSignal<Lang>>().unwrap_or_else(|| RwSignal::new(Lang::En));
    let now = use_context::<Now>().map_or_else(|| RwSignal::new(unix_now()), |n| n.0);
    // Re-renders on a new house-local day, not every minute.
    let midday = Memo::new(move |_| {
        let zone = house.flows_config.with(|c| house_place(c.as_ref()).1);
        let utc = DateTime::<Utc>::from_timestamp(now.get() as i64, 0).unwrap_or_default();
        local_noon(zone, utc.with_timezone(&zone).date_naive()).unwrap_or(utc)
    });
    let query = use_query_map();
    let navigate = use_navigate();
    // Replace, not push: tapping through the days must not pile up Back entries.
    let pick = Callback::new(move |date: Option<String>| {
        let url = date.map_or_else(|| "/weather".to_owned(), |d| format!("/weather?day={d}"));
        navigate(
            &url,
            NavigateOptions {
                replace: true,
                ..Default::default()
            },
        );
    });
    view! {
        <section class="weather-page">
            <BackLink />
            {move || {
                let l = lang.get();
                let day = query.with(|q| q.get("day"));
                match house.weather.get() {
                    Some(w) => content(&w, l, midday.get(), day.as_deref(), pick, house).into_any(),
                    // Nothing until the first fetch has answered, then the hint.
                    None if !house.weather_loaded.get() => ().into_any(),
                    None => {
                        view! {
                            <div class="weather-page__empty">
                                <p>{weather_text(l, T::Unavailable)}</p>
                                <p class="weather-page__hint">{weather_text(l, T::EnableHint)}</p>
                            </div>
                        }
                            .into_any()
                    }
                }
            }}
        </section>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(date: &str) -> WeatherDay {
        serde_json::from_value(serde_json::json!({ "date": date })).unwrap()
    }

    #[test]
    fn the_query_picks_a_forecast_day_or_falls_back_to_today() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 27).unwrap();
        let days = [day("2026-10-28"), day("2026-10-29")];
        let date = |q| pick_day(q, today, &days).map(|d| d.date.as_str());
        assert_eq!(date(Some("2026-10-29")), Some("2026-10-29"));
        assert_eq!(date(None), None);
        assert_eq!(date(Some("2026-10-27")), None);
        assert_eq!(date(Some("2026-12-01")), None);
        assert_eq!(date(Some("tomorrow")), None);
    }
}
