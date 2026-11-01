//! The number card: a slider over the entity's range and the current value.

use leptos::prelude::*;
use nidoo_api::EntityView;
use nidoo_core::model::{Command, EntityState, StateValue};

use super::Sender;
use crate::components::card_frame::{CardFrame, open_entity};
use crate::format::value_class;
use crate::i18n::devices::{DeviceText, use_device_text};

#[component]
pub fn NumberCard(entity: EntityView) -> impl IntoView {
    let sender = Sender::from_context();
    let t = use_device_text();
    let label = super::live_value(&entity);
    let caps = &entity.capabilities;
    let (min, max, step) = (
        caps.min.unwrap_or(0.0),
        caps.max.unwrap_or(100.0),
        caps.step.unwrap_or(1.0),
    );
    let value = match &entity.state.value {
        StateValue::Number(n) => *n,
        _ => min,
    };

    let on_change = {
        let id = entity.id.clone();
        let state = entity.state.clone();
        move |ev: leptos::ev::Event| {
            let Ok(value) = event_target_value(&ev).parse::<f64>() else {
                return;
            };
            let guess = EntityState {
                value: StateValue::Number(value),
                ..state.clone()
            };
            sender.send(&id, Command::SetNumber { value }, Some(guess));
        }
    };

    let pending = sender.pending(&entity.id);
    let on_click = open_entity(&entity.id);
    view! {
        <CardFrame entity=entity on_click=on_click pending=pending>
            <span class=move || value_class(&label.get())>{label}</span>
            <div class="card__controls">
                <input
                    class="slider"
                    type="range"
                    min=min.to_string()
                    max=max.to_string()
                    step=step.to_string()
                    aria-label=move || t(DeviceText::Value)
                    prop:value=value.to_string()
                    on:change=on_change
                />
            </div>
        </CardFrame>
    }
}
