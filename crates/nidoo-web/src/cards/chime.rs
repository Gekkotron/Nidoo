//! The chime card: one "Play" button that turns the relay on.

use leptos::prelude::*;
use nidoo_api::EntityView;

use super::Sender;
use super::door::turn_on;
use crate::components::card_frame::{CardFrame, open_entity};
use crate::i18n::devices::{DeviceText, use_device_text};

#[component]
pub fn ChimeCard(entity: EntityView) -> impl IntoView {
    let sender = Sender::from_context();
    let t = use_device_text();
    let on_play = {
        let id = entity.id.clone();
        move |_| sender.send(&id, turn_on(), None)
    };
    let pending = sender.pending(&entity.id);
    let on_click = open_entity(&entity.id);
    view! {
        <CardFrame entity=entity on_click=on_click pending=pending>
            <div class="card__controls">
                <button type="button" class="btn" on:click=on_play>
                    {move || t(DeviceText::Play)}
                </button>
            </div>
        </CardFrame>
    }
}
