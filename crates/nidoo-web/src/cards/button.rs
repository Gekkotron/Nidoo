//! The button card: one wide press button.

use leptos::prelude::*;
use nidoo_api::EntityView;
use nidoo_core::model::Command;

use super::Sender;
use crate::components::card_frame::{CardFrame, open_entity};
use crate::i18n::devices::{DeviceText, use_device_text};

#[component]
pub fn ButtonCard(entity: EntityView) -> impl IntoView {
    let sender = Sender::from_context();
    let t = use_device_text();
    let on_press = {
        let id = entity.id.clone();
        move |_| sender.send(&id, Command::Press, None)
    };
    let pending = sender.pending(&entity.id);
    let on_click = open_entity(&entity.id);
    view! {
        <CardFrame entity=entity on_click=on_click pending=pending>
            <div class="card__controls">
                <button type="button" class="btn" on:click=on_press>
                    {move || t(DeviceText::Press)}
                </button>
            </div>
        </CardFrame>
    }
}
