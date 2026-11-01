//! The select card: a dropdown over the entity's options.

use leptos::prelude::*;
use nidoo_api::EntityView;
use nidoo_core::model::{Command, EntityState, StateValue};

use super::{ModeSelect, Sender};
use crate::components::card_frame::{CardFrame, open_entity};
use crate::i18n::devices::{DeviceText, use_device_text};

#[component]
pub fn SelectCard(entity: EntityView) -> impl IntoView {
    let sender = Sender::from_context();
    let t = use_device_text();
    let current = match &entity.state.value {
        StateValue::Text(s) => Some(s.clone()),
        _ => None,
    };
    let on_select = {
        let id = entity.id.clone();
        let state = entity.state.clone();
        Callback::new(move |option: String| {
            let guess = EntityState {
                value: StateValue::Text(option.clone()),
                ..state.clone()
            };
            sender.send(&id, Command::SelectOption { option }, Some(guess));
        })
    };
    let options = entity.capabilities.options.clone();
    let pending = sender.pending(&entity.id);
    let on_click = open_entity(&entity.id);
    view! {
        <CardFrame entity=entity on_click=on_click pending=pending>
            <div class="card__controls card__controls--stack">
                <ModeSelect label=Signal::derive(move || t(DeviceText::Option).to_owned()) options=options selected=current on_select=on_select />
            </div>
        </CardFrame>
    }
}
