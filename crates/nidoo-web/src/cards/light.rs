//! The light card: a power toggle, a brightness slider and a colour entry.

use leptos::prelude::*;
use nidoo_api::EntityView;
use nidoo_core::model::{Command, EntityState, StateValue};

use super::text::{brightness_percent, light_state_line};
use super::{Sender, ToggleSwitch};
use crate::components::card_frame::{CardFrame, open_entity};
use crate::format::entity_title;
use crate::i18n::devices::{DeviceText, use_device_text};
use crate::i18n::{Key, use_lang, use_text};

/// Whether the entity is on: from `OnOff` (a native switch, or a switch shown as a light) or
/// from `Light`; `None` while the state is unknown.
pub fn read_on(state: &EntityState) -> Option<bool> {
    match &state.value {
        StateValue::OnOff(on) => Some(*on),
        StateValue::Light(light) => Some(light.on),
        _ => None,
    }
}

/// `state` with its on flag set, the guess shown while a toggle is in flight. `None` for a
/// state that has no on flag.
pub(super) fn with_on(state: &EntityState, on: bool) -> Option<EntityState> {
    let value = match &state.value {
        StateValue::OnOff(_) => StateValue::OnOff(on),
        StateValue::Light(light) => StateValue::Light(nidoo_core::model::LightState {
            on,
            ..light.clone()
        }),
        _ => return None,
    };
    Some(EntityState {
        value,
        ..state.clone()
    })
}

/// Whether a light with these colour modes takes a colour. Zigbee2MQTT lights advertise `xy`
/// rather than `hs`; the hub sends the picked hue and saturation whatever the mode.
pub fn has_colour(color_modes: &std::collections::BTreeSet<String>) -> bool {
    ["hs", "xy", "rgb", "rgbw", "rgbww"]
        .iter()
        .any(|mode| color_modes.contains(*mode))
}

/// The turn-on or turn-off command for a toggle to `on`.
pub(crate) fn power_command(on: bool) -> Command {
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
pub fn LightCard(entity: EntityView) -> impl IntoView {
    let sender = Sender::from_context();
    let lang = use_lang();
    let t = use_text();
    let td = use_device_text();
    let known = read_on(&entity.state);
    let on = known.unwrap_or(false);
    let light = match &entity.state.value {
        StateValue::Light(light) => Some(light.clone()),
        _ => None,
    };
    let brightness = light.as_ref().and_then(|l| l.brightness);

    let toggle = {
        let id = entity.id.clone();
        let state = entity.state.clone();
        Callback::new(move |()| {
            sender.send(&id, power_command(!on), with_on(&state, !on));
        })
    };

    let slider = entity.capabilities.brightness.then(|| {
        let id = entity.id.clone();
        let state = entity.state.clone();
        let level = light.as_ref().and_then(|l| l.brightness).unwrap_or(255);
        let on_change = move |ev: leptos::ev::Event| {
            let Ok(value) = event_target_value(&ev).parse::<u8>() else {
                return;
            };
            let guess = match &state.value {
                StateValue::Light(l) => Some(EntityState {
                    value: StateValue::Light(nidoo_core::model::LightState {
                        on: true,
                        brightness: Some(value),
                        ..l.clone()
                    }),
                    ..state.clone()
                }),
                _ => None,
            };
            sender.send(
                &id,
                Command::TurnOn {
                    brightness: Some(value),
                    color: None,
                    effect: None,
                },
                guess,
            );
        };
        let percent = if on {
            brightness.map_or(0, brightness_percent)
        } else {
            0
        };
        view! {
            <div class="level">
                <span class="level__label">{move || t(Key::Brightness)}</span>
                <span class="level__value">{format!("{percent} %")}</span>
            </div>
            <input
                class="slider"
                type="range"
                min="1"
                max="255"
                step="1"
                aria-label=move || t(Key::Brightness)
                prop:value=level.to_string()
                on:change=on_change
            />
        }
    });

    let open = open_entity(&entity.id);
    let swatch = has_colour(&entity.capabilities.color_modes).then(|| {
        let color = light.as_ref().and_then(|l| l.hs).map_or_else(
            || "var(--accent-violet)".to_owned(),
            |hs| format!("hsl({} {}% 55%)", hs.h.round(), hs.s.round()),
        );
        view! {
            <button
                type="button"
                class="swatch"
                title=move || td(DeviceText::Colour)
                aria-label=move || td(DeviceText::ColourOpen)
                style:background=color
                on:click=move |_| open.run(())
            ></button>
        }
    });

    let name = {
        let entity = entity.clone();
        Signal::derive(move || entity_title(lang(), &entity))
    };
    let trailing = view! { <ToggleSwitch on=on aria_label=name on_toggle=toggle /> }.into_any();
    let state_line = Signal::derive(move || light_state_line(lang(), known, brightness));
    let pending = sender.pending(&entity.id);
    let on_click = open_entity(&entity.id);
    view! {
        <CardFrame
            entity=entity
            on_click=on_click
            pending=pending
            trailing=trailing
            state_line=state_line
        >
            <div class="light__level">{slider}{swatch}</div>
        </CardFrame>
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::has_colour;

    fn modes(list: &[&str]) -> BTreeSet<String> {
        list.iter().map(|m| (*m).to_owned()).collect()
    }

    #[test]
    fn zigbee_xy_lights_take_a_colour() {
        assert!(has_colour(&modes(&["xy"])));
        assert!(has_colour(&modes(&["color_temp", "xy"])));
        assert!(has_colour(&modes(&["hs"])));
        assert!(has_colour(&modes(&["rgb"])));
    }

    #[test]
    fn white_only_lights_do_not() {
        assert!(!has_colour(&modes(&[])));
        assert!(!has_colour(&modes(&["brightness"])));
        assert!(!has_colour(&modes(&["color_temp"])));
        assert!(!has_colour(&modes(&["onoff"])));
    }
}
