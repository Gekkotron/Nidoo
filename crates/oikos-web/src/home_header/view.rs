//! The house overview header: date, greeting, mood sentence, detail line, chips, weather card.

use chrono::{DateTime, NaiveTime, Timelike, Utc};
use leptos::prelude::*;
use leptos_router::components::A;
use oikos_api::WeatherView;
use oikos_core::model::{Command, Kind};

use super::mood::{activity, details, greeting, sun_today, temp_band};
use super::summary::{HouseSummary, ModeChip, summarize};
use super::weather_fmt::{condition_icon, weather_card_visible};
use crate::alerts::unseen_count;
use crate::cards::Sender;
use crate::components::now::{Now, unix_now};
use crate::format::word;
use crate::house::House;
use crate::i18n::{self, Lang};
use crate::icons;
use crate::rooms::use_loaded;
use crate::screens::flow_canvas::sun::house_place;
use crate::solar::card::SolarCard;
use crate::solar::model::solar_card_visible;

/// A temperature rounded to whole degrees, never "-0".
pub(crate) fn degrees(lang: Lang, t: f64) -> String {
    format!("{}°", i18n::home_number(lang, t.round() + 0.0, 0))
}

/// A chip with a long label and a short one; CSS shows one of them by width.
fn chip(class: &'static str, icon: &'static str, long: String, short: String) -> impl IntoView {
    view! {
        <span class=class>
            <span class="home-chip__icon" inner_html=icon></span>
            <span class="home-chip__long">{long}</span>
            <span class="home-chip__short">{short}</span>
        </span>
    }
}

/// The house mode pill: a menu of the select's options; a state outside the options is kept.
#[component]
fn ModePill(mode: ModeChip, lang: RwSignal<Lang>) -> impl IntoView {
    let sender = Sender::from_context();
    let mut options = mode.options.clone();
    if !options.contains(&mode.current) {
        options.insert(0, mode.current.clone());
    }
    let items = options
        .into_iter()
        .map(|option| {
            let selected = option == mode.current;
            let text = word(&option);
            view! { <option value=option selected=selected>{text}</option> }
        })
        .collect_view();
    let house: House = expect_context();
    let current = mode.current.clone();
    let id = mode.entity_id;
    let on_change = move |ev| {
        let option = event_target_value(&ev);
        sender.send(&id, Command::SelectOption { option }, None);
    };
    view! {
        <label class="home-chip home-chip--mode">
            <select
                class="home-chip__select"
                aria-label=move || i18n::house_mode_label(lang.get())
                // Re-asserted on every failed command: the hub left the state alone, so the
                // select must not keep the option the browser showed.
                prop:value=move || {
                    house.command_failed.track();
                    current.clone()
                }
                on:change=on_change
            >
                {items}
            </select>
        </label>
    }
}

/// The weather card, one link to the weather page.
fn weather_card(
    w: &WeatherView,
    lang: Lang,
    sunset: Option<NaiveTime>,
    fetched_local: Option<NaiveTime>,
) -> AnyView {
    let daylight = w.current.daylight.unwrap_or(true);
    let condition = w.current.condition;
    let icon = condition.map(|c| condition_icon(c, daylight));
    let label = condition.map(|c| i18n::condition_label(lang, c));
    let temp = w.current.temperature.map(|t| degrees(lang, t));
    let feels = w
        .current
        .apparent_temperature
        .map(|t| i18n::feels_like(lang, &degrees(lang, t)));
    let range = match (w.today.min, w.today.max) {
        (Some(lo), Some(hi)) => Some(format!("{} / {}", degrees(lang, lo), degrees(lang, hi))),
        _ => None,
    };
    let stale = w
        .stale
        .then(|| fetched_local.map(|at| i18n::data_from(lang, at)))
        .flatten();
    view! {
        <A href="/weather" attr:class="home-header__weather" attr:aria-label=i18n::weather_link_label(lang)>
            <span class="wx__now">
                {icon.map(|i| view! { <span class="wx__icon" inner_html=i></span> })}
                {temp.map(|t| view! { <span class="wx__temp">{t}</span> })}
            </span>
            <span class="wx__lines">
                <span class="wx__line">
                    {label}
                    {feels.map(|f| view! { <span class="wx__long">" · "{f}</span> })}
                    {range.clone().map(|r| view! { <span class="wx__short">" · "{r}</span> })}
                </span>
                {range
                    .map(|r| view! { <span class="wx__line wx__long wx__muted">{i18n::today_range(lang, &r)}</span> })}
                {sunset.map(|at| {
                    view! {
                        <span class="wx__line wx__muted">
                            <span class="wx__long">{i18n::card_sunset(lang, at, false)}</span>
                            <span class="wx__short">{i18n::card_sunset(lang, at, true)}</span>
                        </span>
                    }
                })}
                {stale.map(|s| view! { <span class="wx__line wx__muted">{s}</span> })}
            </span>
        </A>
    }
    .into_any()
}

/// Whether everything that sizes the header has answered: the state snapshot (the chips) and the
/// weather and solar fetches (the side cards). A screen that holds its content until then paints
/// the header once at its full height, instead of letting it grow and push the page down.
pub fn use_header_ready() -> Memo<bool> {
    let house: House = expect_context();
    let loaded = use_loaded();
    Memo::new(move |_| loaded.get() && house.weather_loaded.get() && house.solar_loaded.get())
}

/// The header of the house overview.
#[component]
pub fn HomeHeader() -> impl IntoView {
    let house: House = expect_context();
    let lang = use_context::<RwSignal<Lang>>().unwrap_or_else(|| RwSignal::new(Lang::En));
    let now = use_context::<Now>().map_or_else(|| RwSignal::new(unix_now()), |n| n.0);

    let summary = Memo::new(move |_| -> HouseSummary {
        let unseen = house
            .alerts
            .with(|a| unseen_count(a, house.last_alerts_seen.get()));
        house.entities.with(|e| {
            house.areas.with(|areas| {
                house
                    .home_settings
                    .with(|s| summarize(lang.get(), e, areas, s, unseen))
            })
        })
    });
    let utc = move || DateTime::<Utc>::from_timestamp(now.get() as i64, 0).unwrap_or_default();
    let zone = move || house.flows_config.with(|c| house_place(c.as_ref()).1);
    let local = move || utc().with_timezone(&zone());
    let sun = Memo::new(move |_| house.flows_config.with(|c| sun_today(c.as_ref(), utc())));
    let card_visible = move || {
        house
            .weather
            .with(|w| weather_card_visible(w.as_ref(), now.get()))
    };

    let date = move || i18n::home_date(lang.get(), local().date_naive());
    let hello = move || {
        let hour = local().hour();
        i18n::greeting_text(lang.get(), greeting(hour))
    };
    let mood = move || {
        let (_, _, dark, _) = sun.get();
        let hour = local().hour();
        let s = summary.get();
        let band = s
            .indoor
            .map(|t| house.home_settings.with(|hs| temp_band(t, hs)));
        i18n::mood_sentence(lang.get(), &activity(&s, dark, hour), band)
    };
    let detail_line = move || {
        let (_, sunset, _, fallback) = sun.get();
        let visible = card_visible();
        let list = house.weather.with(|w| {
            details(
                w.as_ref().filter(|_| visible),
                sunset,
                fallback,
                local().time(),
                visible,
            )
        });
        let l = lang.get();
        list.iter()
            .map(|d| i18n::detail_text(l, d))
            .collect::<Vec<_>>()
            .join(" · ")
    };

    let lights = move || {
        let n = summary.with(|s| s.lights_on);
        (n > 0).then(|| {
            let l = lang.get();
            chip(
                "home-chip home-chip--lit",
                icons::kind_icon(Kind::Light, None),
                i18n::lights_chip(l, n, false),
                i18n::lights_chip(l, n, true),
            )
        })
    };
    let indoor = move || {
        summary.with(|s| s.indoor).map(|t| {
            let l = lang.get();
            chip(
                "home-chip",
                icons::THERMOMETER,
                i18n::indoor_chip(l, t, false),
                i18n::indoor_chip(l, t, true),
            )
        })
    };
    let heating = move || {
        summary.with(|s| s.heating).map(|h| {
            let l = lang.get();
            chip(
                "home-chip",
                icons::HEAT_PUMP,
                i18n::heating_chip_text(l, h, false).to_owned(),
                i18n::heating_chip_text(l, h, true).to_owned(),
            )
        })
    };
    let mode = move || {
        summary
            .with(|s| s.mode.clone())
            .map(|m| view! { <ModePill mode=m lang=lang /> })
    };

    // A memo, so the card subtree is rebuilt only when visibility flips, not on every clock tick.
    let solar_visible = Memo::new(move |_| {
        house
            .solar
            .with(|s| solar_card_visible(s.as_ref(), now.get()))
    });
    let card = move || {
        if !card_visible() {
            return None;
        }
        let l = lang.get();
        let (_, sunset, _, fallback) = sun.get();
        let fetched_local = house.weather.with(|w| {
            w.as_ref().and_then(|w| {
                DateTime::<Utc>::from_timestamp(w.fetched_at as i64, 0)
                    .map(|t| t.with_timezone(&zone()).time())
            })
        });
        house.weather.with(|w| {
            w.as_ref()
                .map(|w| weather_card(w, l, sunset.filter(|_| !fallback), fetched_local))
        })
    };

    view! {
        <section class="home-header" class:home-header--solar=solar_visible>
            <div class="home-header__main">
                <p class="home-header__date">{date}</p>
                <h2 class="home-header__greeting">{hello}</h2>
                <p class="home-header__mood">{mood}</p>
                {move || {
                    let line = detail_line();
                    (!line.is_empty()).then(|| view! { <p class="home-header__details">{line}</p> })
                }}
                <div class="home-header__chips">{lights} {indoor} {heating} {mode}</div>
            </div>
            {move || {
                if solar_visible.get() {
                    view! { <div class="home-header__cards">{card} <SolarCard /></div> }.into_any()
                } else {
                    view! { {card} }.into_any()
                }
            }}
        </section>
    }
}
