//! The cover card: open, stop and close buttons, a position slider and a "Moving" indicator.

use leptos::prelude::*;
use nidoo_api::EntityView;
use nidoo_core::model::{Command, CoverState, EntityState, StateValue};

use super::Sender;
use crate::components::card_frame::{CardFrame, open_entity};
use crate::house::House;
use crate::i18n::devices::{DeviceText, use_device_text};
use crate::icons::{CHEVRON_DOWN, CHEVRON_UP, STOP};

/// True when a folded child named "moving" says the cover is moving: an on/off state that is
/// on, or a text state of "moving", "opening" or "closing".
pub fn is_moving<'a>(children: impl IntoIterator<Item = &'a EntityView>) -> bool {
    children
        .into_iter()
        .filter(|c| c.name.to_lowercase().contains("moving"))
        .any(|c| match &c.state.value {
            StateValue::OnOff(on) => *on,
            StateValue::Text(s) => ["moving", "opening", "closing"]
                .iter()
                .any(|w| s.eq_ignore_ascii_case(w)),
            _ => false,
        })
}

/// `state` with the cover at `position`, the guess shown while a slider move is in flight.
fn with_position(state: &EntityState, position: u8) -> Option<EntityState> {
    let StateValue::Cover(cover) = &state.value else {
        return None;
    };
    Some(EntityState {
        value: StateValue::Cover(CoverState {
            position: Some(position),
            ..cover.clone()
        }),
        ..state.clone()
    })
}

#[component]
pub fn CoverCard(entity: EntityView) -> impl IntoView {
    let sender = Sender::from_context();
    let t = use_device_text();
    let label = super::live_value(&entity);
    // The folded "moving" child changes on its own, without rebuilding this card, so the badge
    // follows it through a memo (which only notifies when the answer flips).
    let house = expect_context::<House>();
    let moving = {
        let id = entity.id.clone();
        Memo::new(move |_| {
            house.entities.with(|m| {
                is_moving(
                    m.values()
                        .filter(|e| e.part_of.as_deref() == Some(id.as_str())),
                )
            })
        })
    };
    let position = match &entity.state.value {
        StateValue::Cover(c) => c.position.unwrap_or(0),
        _ => 0,
    };

    let button = |text: DeviceText, icon: &'static str, cmd: Command| {
        let id = entity.id.clone();
        view! {
            <button
                type="button"
                class="btn btn--icon"
                aria-label=move || t(text)
                title=move || t(text)
                inner_html=icon
                on:click=move |_| sender.send(&id, cmd.clone(), None)
            ></button>
        }
    };
    let buttons = view! {
        <div class="card__controls">
            {button(DeviceText::Open, CHEVRON_UP, Command::Open)}
            {button(DeviceText::Stop, STOP, Command::Stop)}
            {button(DeviceText::Close, CHEVRON_DOWN, Command::Close)}
        </div>
    };

    let slider = entity.capabilities.position.then(|| {
        let id = entity.id.clone();
        let state = entity.state.clone();
        let on_change = move |ev: leptos::ev::Event| {
            let Ok(value) = event_target_value(&ev).parse::<u8>() else {
                return;
            };
            sender.send(
                &id,
                Command::SetPosition { position: value },
                with_position(&state, value),
            );
        };
        view! {
            <div class="card__controls">
                <input
                    class="slider"
                    type="range"
                    min="0"
                    max="100"
                    step="1"
                    aria-label=move || t(DeviceText::Position)
                    prop:value=position.to_string()
                    on:change=on_change
                />
            </div>
        }
    });

    let pending = sender.pending(&entity.id);
    let on_click = open_entity(&entity.id);
    view! {
        <CardFrame entity=entity on_click=on_click pending=pending>
            <span class="card__value">{label}</span>
            {move || moving.get().then(|| view! { <span class="badge">{move || t(DeviceText::Moving)}</span> })}
            {slider}
            {buttons}
        </CardFrame>
    }
}

#[cfg(test)]
mod tests {
    use super::{is_moving, with_position};
    use nidoo_api::EntityView;
    use nidoo_core::model::{
        Capabilities, CoverState, EntityCategory, EntityState, Kind, StateValue, UiOverride,
    };

    fn child(name: &str, value: StateValue) -> EntityView {
        EntityView {
            id: "x".into(),
            device_id: None,
            name: name.into(),
            device_name: None,
            label: None,
            kind: Kind::BinarySensor,
            component: "binary_sensor".to_string(),
            protocol_kind: Kind::BinarySensor,
            device_class: None,
            unit: None,
            icon: None,
            category: EntityCategory::Primary,
            enabled_by_default: true,
            area: "salon".into(),
            part_of: Some("cover.hall".into()),
            capabilities: Capabilities::default(),
            ui: UiOverride::default(),
            state: EntityState {
                value,
                ..Default::default()
            },
        }
    }

    #[test]
    fn moving_binary_sensor_on() {
        assert!(is_moving(&[child("Hall Moving", StateValue::OnOff(true))]));
        assert!(!is_moving(&[child(
            "Hall Moving",
            StateValue::OnOff(false)
        )]));
        assert!(!is_moving(&[child("Hall Tilt", StateValue::OnOff(true))]));
    }

    #[test]
    fn moving_text_words() {
        assert!(is_moving(&[child(
            "Moving",
            StateValue::Text("Opening".into())
        )]));
        assert!(!is_moving(&[child(
            "Moving",
            StateValue::Text("idle".into())
        )]));
        assert!(!is_moving(&[]));
    }

    #[test]
    fn position_guess_keeps_the_state_word() {
        let s = EntityState {
            value: StateValue::Cover(CoverState {
                state: Some("open".into()),
                position: Some(100),
            }),
            ..Default::default()
        };
        let guess = with_position(&s, 30).expect("cover");
        assert_eq!(
            guess.value,
            StateValue::Cover(CoverState {
                state: Some("open".into()),
                position: Some(30)
            })
        );
    }
}
