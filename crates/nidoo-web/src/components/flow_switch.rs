//! The enable switch of a flow, shared by the list and the canvas header.

use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::api;
use crate::components::toast::Toasts;
use crate::i18n::flows::flow_switch;
use crate::i18n::use_lang;

/// A switch that enables or disables flow `id`; `invalid` flows cannot be toggled.
#[component]
pub fn FlowSwitch(id: String, name: String, enabled: bool, invalid: bool) -> impl IntoView {
    let toasts = expect_context::<Toasts>();
    let lang = use_lang();
    let label = move || flow_switch(lang(), true, &name);
    let toggle = move |_| {
        let id = id.clone();
        spawn_local(async move {
            if let Err(e) = api::set_flow_enabled(&id, !enabled).await {
                toasts.push(e.error);
            }
        });
    };
    view! {
        <button
            type="button"
            role="switch"
            class="switch flow-switch"
            class:switch--on=enabled
            aria-checked=enabled.to_string()
            aria-label=label
            disabled=invalid
            on:click=toggle
        >
            <span class="switch__knob"></span>
        </button>
    }
}
