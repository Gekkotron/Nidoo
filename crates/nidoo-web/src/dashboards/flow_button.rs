//! The flow button widget: a card with one button that runs the flow.

use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::api;
use crate::components::toast::Toasts;
use crate::i18n::{WidgetText, flow_started, use_lang, widget_text};
use crate::icons::PLAY;

/// The card's name: the widget's own title when it has one, else the flow's.
fn button_name(title: Option<String>, flow_name: &str) -> String {
    title
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| flow_name.to_owned())
}

#[component]
pub fn FlowButtonWidget(
    flow_id: String,
    flow_name: String,
    #[prop(into)] title: Signal<Option<String>>,
    /// Why the flow cannot run, when it cannot.
    invalid: Option<String>,
) -> impl IntoView {
    let toasts = expect_context::<Toasts>();
    let lang = use_lang();
    let name = Memo::new(move |_| button_name(title.get(), &flow_name));
    let running = RwSignal::new(false);
    let runnable = invalid.is_none();
    let on_run = move |_| {
        let (id, name) = (flow_id.clone(), name.get_untracked());
        running.set(true);
        spawn_local(async move {
            match api::run_flow(&id).await {
                Ok(_) => toasts.push(flow_started(lang(), &name)),
                Err(e) => toasts.push(e.error),
            }
            running.set(false);
        });
    };

    view! {
        <article class="card widget-flow" class:card--pending=move || running.get()>
            <div class="widget-flow__head">
                <span class="widget-flow__icon" inner_html=PLAY></span>
                <span class="widget-flow__name" title=name>{name}</span>
            </div>
            <button
                type="button"
                class="btn widget-flow__run"
                title=invalid.clone()
                disabled=move || !runnable || running.get()
                on:click=on_run
            >
                {move || widget_text(lang(), WidgetText::Run)}
            </button>
        </article>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_title_wins_over_the_flow_name() {
        assert_eq!(button_name(Some("Night".into()), "Flow 1"), "Night");
        assert_eq!(button_name(None, "Flow 1"), "Flow 1");
        assert_eq!(button_name(Some(String::new()), "Flow 1"), "Flow 1");
    }
}
