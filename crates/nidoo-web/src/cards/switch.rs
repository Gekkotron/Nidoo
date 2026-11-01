//! The switch card: a power toggle.

use leptos::prelude::*;
use nidoo_api::EntityView;

pub use super::light::read_on;
use super::light::{power_command, with_on};
use super::text::switch_state_line;
use super::{Sender, ToggleSwitch};
use crate::components::card_frame::{CardFrame, open_entity};
use crate::format::entity_title;
use crate::i18n::use_lang;

#[component]
pub fn SwitchCard(entity: EntityView) -> impl IntoView {
    let sender = Sender::from_context();
    let lang = use_lang();
    let known = read_on(&entity.state);
    let on = known.unwrap_or(false);
    let toggle = {
        let id = entity.id.clone();
        let state = entity.state.clone();
        Callback::new(move |()| {
            sender.send(&id, power_command(!on), with_on(&state, !on));
        })
    };
    let name = {
        let entity = entity.clone();
        Signal::derive(move || entity_title(lang(), &entity))
    };
    let trailing = view! { <ToggleSwitch on=on aria_label=name on_toggle=toggle /> }.into_any();
    let state_line = Signal::derive(move || switch_state_line(lang(), known));
    let pending = sender.pending(&entity.id);
    let on_click = open_entity(&entity.id);
    view! {
        <CardFrame
            entity=entity
            on_click=on_click
            pending=pending
            trailing=trailing
            state_line=state_line
        />
    }
}

#[cfg(test)]
mod tests {
    use super::read_on;
    use nidoo_core::model::{EntityState, LightState, StateValue};

    #[test]
    fn as_light_switch_reads_on_off() {
        let s = EntityState {
            value: StateValue::OnOff(true),
            ..Default::default()
        };
        assert_eq!(read_on(&s), Some(true));
    }

    #[test]
    fn native_light_reads_light_state() {
        let s = EntityState {
            value: StateValue::Light(LightState {
                on: true,
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(read_on(&s), Some(true));
    }

    #[test]
    fn unknown_reads_none() {
        let s = EntityState {
            value: StateValue::Unknown,
            ..Default::default()
        };
        assert_eq!(read_on(&s), None);
    }
}
