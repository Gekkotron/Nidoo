//! The Done step: a summary of what the wizard set up, and the button that finishes it.

use leptos::prelude::*;
use leptos_router::NavigateOptions;
use leptos_router::hooks::use_navigate;
use nidoo_api::GeoPlace;

use super::SetupState;
use super::services::{CardState, service_states};
use super::welcome::place_label;
use crate::house::House;
use crate::i18n::setup::{SetupText, setup_text, use_setup_text};
use crate::i18n::use_lang;
use crate::screens::devices::device_rooms;

/// The services' names, for the summary rows.
fn service_title(id: &str) -> SetupText {
    match id {
        "weather" => SetupText::SvcWeather,
        "solar" => SetupText::SvcSolar,
        "energy" => SetupText::SvcEnergy,
        "cameras" => SetupText::SvcCameras,
        "push" => SetupText::SvcPush,
        "climate" => SetupText::SvcClimate,
        _ => SetupText::SvcAlarm,
    }
}

/// One summary row: a mark (set or muted), the label and the value.
#[component]
fn SummaryRow(
    id: String,
    label: Signal<String>,
    value: Signal<String>,
    ok: Signal<bool>,
) -> impl IntoView {
    view! {
        <li class="done__row" id=format!("done-{id}") class:done__row--off=move || !ok.get()>
            <span class="done__mark" aria-hidden="true">
                {move || if ok.get() { "✓" } else { "–" }}
            </span>
            <span class="done__label">{move || label.get()}</span>
            <span class="done__value">{move || value.get()}</span>
        </li>
    }
}

/// The Done step. It reads the setup state and the house from context and needs a router. Open
/// my house finishes the setup and goes to the landing page.
#[component]
pub fn DoneStep() -> impl IntoView {
    let t = use_setup_text();
    let lang = use_lang();
    let state = use_context::<SetupState>();
    let house = use_context::<House>();
    let navigate = use_navigate();
    let services = service_states();
    let failure = RwSignal::new(None::<String>);
    let advancing = RwSignal::new(false);
    let finish = move |_| {
        if advancing.get_untracked() {
            return;
        }
        advancing.set(true);
        let navigate = navigate.clone();
        leptos::task::spawn_local(async move {
            match crate::api::setup::finish().await {
                Ok(status) => {
                    if let Some(state) = state {
                        state.0.try_set(Some(status));
                    }
                    navigate(&crate::app::landing_path(), NavigateOptions::default());
                }
                Err(e) => {
                    failure.try_set(Some(e.message(lang())));
                }
            }
            advancing.try_set(false);
        });
    };
    let status = move || state.and_then(|s| s.0.get());
    let not_set = move || setup_text(lang(), SetupText::CardNotSetUp).to_owned();
    let location = move || {
        status().and_then(|s| s.location).map(|l| {
            let place = place_label(&GeoPlace {
                name: l.city,
                admin: None,
                country: l.country,
                latitude: l.latitude,
                longitude: l.longitude,
                timezone: None,
            });
            format!("{place} · {}", l.timezone)
        })
    };
    let mqtt = move || {
        status()
            .and_then(|s| s.mqtt)
            .map(|m| format!("{}:{}", m.host, m.port))
    };
    let database = move || {
        status()
            .and_then(|s| s.database)
            .map(|d| format!("{}:{} / {}", d.host, d.port, d.database))
    };
    let devices = move || {
        let house = house?;
        let n = house.devices.with(Vec::len);
        if n == 0 {
            return None;
        }
        let areas = house.areas.get();
        let in_rooms = house.entities.with(|m| {
            house.devices.with(|d| {
                d.iter()
                    .filter(|dev| !device_rooms(dev, m, &areas).is_empty())
                    .count()
            })
        });
        Some(
            setup_text(lang(), SetupText::DevicesInRooms)
                .replace("{n}", &n.to_string())
                .replace("{m}", &in_rooms.to_string()),
        )
    };
    let row = move |id: &'static str,
                    label: SetupText,
                    value: Box<dyn Fn() -> Option<String> + Send + Sync>| {
        let value = std::sync::Arc::new(value);
        let v1 = value.clone();
        view! {
            <SummaryRow
                id=id.to_owned()
                label=Signal::derive(move || t(label).to_owned())
                value=Signal::derive(move || v1().unwrap_or_else(not_set))
                ok=Signal::derive(move || value().is_some())
            />
        }
    };
    view! {
        <div class="setup__body">
            <h1 class="setup__title">{move || t(SetupText::DoneTitle)}</h1>
            <p class="setup__lead">{move || t(SetupText::DoneLead)}</p>
            <ul class="done__list">
                {row("location", SetupText::RowLocation, Box::new(location))}
                {row("mqtt", SetupText::RowMqtt, Box::new(mqtt))}
                {row("database", SetupText::RowDatabase, Box::new(database))}
                {row("devices", SetupText::RowDevices, Box::new(devices))}
                {move || {
                    services
                        .get()
                        .into_iter()
                        .map(|(id, card)| {
                            let word = match card {
                                CardState::SetUp => SetupText::CardSetUp,
                                CardState::Skipped => SetupText::CardSkipped,
                                CardState::NotSetUp => SetupText::CardNotSetUp,
                                CardState::Unknown => SetupText::CardUnknown,
                                CardState::Checking => SetupText::CardChecking,
                            };
                            view! {
                                <SummaryRow
                                    id=id.to_owned()
                                    label=Signal::derive(move || t(service_title(id)).to_owned())
                                    value=Signal::derive(move || t(word).to_owned())
                                    ok=Signal::derive(move || card == CardState::SetUp)
                                />
                            }
                        })
                        .collect_view()
                }}
            </ul>
            <p class="loc__problem" role="alert">{move || failure.get()}</p>
            <div class="loc__actions">
                <button
                    type="button"
                    class="btn btn--primary loc__save done__open"
                    prop:disabled=move || advancing.get()
                    on:click=finish
                >
                    {move || t(SetupText::OpenHouse)}
                </button>
            </div>
        </div>
    }
}
