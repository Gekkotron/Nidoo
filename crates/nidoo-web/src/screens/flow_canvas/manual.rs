//! The ▶ button on a manual card: starts the flow from that card.

use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::api;
use crate::components::toast::Toasts;

/// A round ▶ button inside a manual card. It is inert while `disabled` (edit mode, or a
/// disabled flow); a refusal from the hub goes to the toasts, like Run now. Its presses never
/// reach the card, so they neither select it nor open its settings.
#[component]
pub fn ManualRunButton(
    flow_id: Signal<String>,
    node_id: String,
    disabled: Signal<bool>,
    title: Signal<&'static str>,
) -> impl IntoView {
    let toasts = expect_context::<Toasts>();
    let run = move |ev: web_sys::MouseEvent| {
        ev.stop_propagation();
        if disabled.get_untracked() {
            return;
        }
        let id = flow_id.get_untracked();
        let node = node_id.clone();
        spawn_local(async move {
            if let Err(e) = api::run_flow_from(&id, &node).await {
                toasts.push(e.error);
            }
        });
    };
    // The card listens to pointerdown (select, drag) and the surface below pans on it.
    let swallow = |ev: web_sys::PointerEvent| ev.stop_propagation();
    view! {
        <button
            type="button"
            class="flow-node__run"
            prop:disabled=move || disabled.get()
            aria-label=move || title.get()
            title=move || title.get()
            on:pointerdown=swallow
            on:pointerup=swallow
            on:click=run
        >
            "▶"
        </button>
    }
}
