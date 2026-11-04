//! The card of a speaker satellite (the `nidoo-sound` service): one card per satellite, on its
//! Volume entity, which holds the Mute, Allowed and Connected entities folded into it.
//!
//! Once allowed it shows the volume with its value, a Mute toggle and a test button that says a
//! short sentence on that satellite only; while it waits for approval it shows that and an
//! Allow button instead.

use gloo_timers::future::TimeoutFuture;
use leptos::prelude::*;
use leptos::task::spawn_local;
use nidoo_api::{EntityView, SoundSpeakRequest};
use nidoo_core::model::{Availability, Command, EntityState, SOUND_DEVICE_PREFIX, StateValue};

use super::Sender;
use crate::components::card_frame::{CardFrame, open_entity};
use crate::house::House;
use crate::i18n::{SoundText, speaker_test, use_lang, use_sound_text};
use crate::icons::{SPEAKER, SPEAKER_OFF, SPEECH_BUBBLE};
use crate::screens::sound_settings::{Satellite, satellite_of};

/// How long a failed test's reason stays on the card.
const ERROR_MS: u32 = 5_000;

/// `entity` with the state `value`, for the optimistic write.
fn guess(entity: &EntityView, value: StateValue) -> EntityState {
    EntityState {
        value,
        ..entity.state.clone()
    }
}

fn on_off(on: bool) -> Command {
    if on {
        Command::TurnOn {
            brightness: None,
            color: None,
            effect: None,
        }
    } else {
        Command::TurnOff
    }
}

#[component]
pub fn SatelliteCard(entity: EntityView) -> impl IntoView {
    let house = expect_context::<House>();
    let device = entity.device_id.clone().unwrap_or_default();
    let sid = device
        .strip_prefix(SOUND_DEVICE_PREFIX)
        .unwrap_or(&device)
        .to_owned();
    let satellite = Memo::new({
        let device = device.clone();
        move |_| house.entities.with(|m| satellite_of(m, &device))
    });
    let error = RwSignal::new(None::<String>);
    // Counts the failures shown: a timer clears only the message it put up, not a newer one.
    let shown = StoredValue::new(0u32);
    let on_click = open_entity(&entity.id);
    move || {
        let Some(sat) = satellite.get() else {
            return ().into_any();
        };
        view! { <SatelliteFace entity=entity.clone() sat=sat sid=sid.clone() error=error shown=shown on_click=on_click /> }
            .into_any()
    }
}

#[component]
fn SatelliteFace(
    entity: EntityView,
    sat: Satellite,
    sid: String,
    error: RwSignal<Option<String>>,
    shown: StoredValue<u32>,
    on_click: Callback<()>,
) -> impl IntoView {
    let sender = Sender::from_context();
    let t = use_sound_text();
    let lang = use_lang();
    // The frame's dot and offline look follow the player's link, not just the service.
    let mut frame = sat.volume.clone().unwrap_or(entity);
    if !sat.is_connected() {
        frame.state.availability = Availability::Offline;
    }
    let pending = sender.pending(&frame.id);
    let name = sat.name.clone();

    let body = if sat.is_allowed() {
        let volume = sat.volume.clone().map(|volume| {
            let caps = &volume.capabilities;
            let (min, max, step) = (
                caps.min.unwrap_or(0.0),
                caps.max.unwrap_or(100.0),
                caps.step.unwrap_or(1.0),
            );
            let now = match volume.state.value {
                StateValue::Number(n) => n,
                _ => min,
            };
            let on_change = move |ev: leptos::ev::Event| {
                let Ok(value) = event_target_value(&ev).parse::<f64>() else {
                    return;
                };
                sender.send(
                    &volume.id,
                    Command::SetNumber { value },
                    Some(guess(&volume, StateValue::Number(value))),
                );
            };
            view! {
                <label class="satellite__volume">
                    <input
                        class="slider"
                        type="range"
                        min=min.to_string()
                        max=max.to_string()
                        step=step.to_string()
                        aria-label=move || t(SoundText::Volume)
                        prop:value=now.to_string()
                        on:change=on_change
                    />
                    <span class="satellite__value">{format!("{now:.0}")}</span>
                </label>
            }
        });
        let mute = sat.mute.clone().map(|mute| {
            let muted = mute.state.value == StateValue::OnOff(true);
            let icon = if muted { SPEAKER_OFF } else { SPEAKER };
            view! {
                <button
                    type="button"
                    class="btn btn--icon satellite__mute"
                    aria-pressed=muted.to_string()
                    aria-label=move || t(SoundText::Mute)
                    title=move || t(SoundText::Mute)
                    inner_html=icon
                    on:click=move |_| {
                        sender.send(&mute.id, on_off(!muted), Some(guess(&mute, StateValue::OnOff(!muted))));
                    }
                ></button>
            }
        });
        let on_speak = move |_| {
            let req = SoundSpeakRequest {
                text: speaker_test(lang(), &name),
                satellite: Some(sid.clone()),
                ..Default::default()
            };
            spawn_local(async move {
                if let Err(err) = crate::api::tts::sound_speak(&req).await {
                    let Some(mine) = shown.try_update_value(|n| {
                        *n = n.wrapping_add(1);
                        *n
                    }) else {
                        return;
                    };
                    error.try_set(Some(err.error));
                    TimeoutFuture::new(ERROR_MS).await;
                    if shown.try_get_value() == Some(mine) {
                        error.try_set(None);
                    }
                }
            });
        };
        view! {
            <div class="card__controls">
                {volume}
                {mute}
                <button
                    type="button"
                    class="btn btn--icon satellite__speak"
                    aria-label=move || t(SoundText::TestSpeaker)
                    title=move || t(SoundText::TestSpeaker)
                    inner_html=SPEECH_BUBBLE
                    on:click=on_speak
                ></button>
            </div>
        }
        .into_any()
    } else {
        let allow = sat.allowed.clone().map(|allowed| {
            view! {
                <button
                    type="button"
                    class="btn satellite__allow"
                    on:click=move |_| {
                        sender.send(&allowed.id, on_off(true), Some(guess(&allowed, StateValue::OnOff(true))));
                    }
                >
                    {move || t(SoundText::Allow)}
                </button>
            }
        });
        view! {
            <p class="card__state satellite__waiting">{move || t(SoundText::WaitingForApproval)}</p>
            <div class="card__controls">{allow}</div>
        }
        .into_any()
    };

    view! {
        <CardFrame entity=frame on_click=on_click pending=pending title=sat.name.clone()>
            {body}
            {move || error.get().map(|e| view! { <p class="satellite__error" role="alert">{e}</p> })}
        </CardFrame>
    }
}
