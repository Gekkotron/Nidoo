//! The round button in the bottom-right corner of every page, and the small chat panel it opens
//! above itself: the Assistant's conversation without leaving the page.

use leptos::ev;
use leptos::prelude::*;
use leptos_router::hooks::use_location;
use wasm_bindgen::JsCast;

use crate::i18n::{AssistantText, assistant_text, use_lang};
use crate::icons::{SPEECH, X_MARK};
use crate::screens::assistant::AssistChat;
use crate::shell::dialog_open;

/// Whether the button shows on `path`: not on the Assistant itself, nor in the setup wizard.
pub fn fab_shown(path: &str) -> bool {
    !(path == "/assistant" || path.starts_with("/assistant/") || path.starts_with("/setup"))
}

/// Hidden, with its panel, while the phone drawer (`drawer_open`) is out. The panel stays open
/// from page to page, until the button, Escape or its link to the Assistant page closes it.
#[component]
pub fn AssistFab(drawer_open: RwSignal<bool>) -> impl IntoView {
    let lang = use_lang();
    let location = use_location();
    let open = RwSignal::new(false);
    let shown = move || location.pathname.with(|p| fab_shown(p)) && !drawer_open.get();
    // On the Assistant page the panel has nothing to add, and leaving it does not bring it back.
    Effect::new(move |_| {
        if !location.pathname.with(|p| fab_shown(p)) {
            open.set(false);
        }
    });
    // Escape closes the panel, unless a dialog over the page or something else took the key.
    let escape = window_event_listener(ev::keydown, move |event| {
        // `try_`: the listener can outlive an unmounted shell for a moment.
        if event.key() != "Escape"
            || event.default_prevented()
            || open.try_get_untracked() != Some(true)
            || dialog_open()
        {
            return;
        }
        open.try_set(false);
        // Focus was in the panel, which is gone: give it to the button.
        let fab = document()
            .query_selector(".assist-fab")
            .ok()
            .flatten()
            .and_then(|el| el.dyn_into::<web_sys::HtmlElement>().ok());
        if let Some(fab) = fab {
            let _ = fab.focus();
        }
    });
    on_cleanup(move || escape.remove());
    let label = move || {
        assistant_text(
            lang(),
            if open.get() {
                AssistantText::Close
            } else {
                AssistantText::Open
            },
        )
    };
    view! {
        <Show when=shown>
            <Show when=move || open.get()>
                <div
                    class="assist-panel"
                    role="dialog"
                    aria-label=move || assistant_text(lang(), AssistantText::Title)
                >
                    <AssistChat compact=true />
                </div>
            </Show>
            <button
                type="button"
                class="assist-fab"
                class:assist-fab--open=move || open.get()
                aria-expanded=move || if open.get() { "true" } else { "false" }
                aria-label=label
                title=label
                inner_html=move || if open.get() { X_MARK } else { SPEECH }
                on:click=move |_| open.update(|o| *o = !*o)
            ></button>
        </Show>
    }
}

#[cfg(test)]
mod tests {
    use super::fab_shown;

    #[test]
    fn shown_everywhere_but_the_assistant_and_setup() {
        assert!(fab_shown("/"));
        assert!(fab_shown("/rooms/salon"));
        assert!(fab_shown("/assistant-like"));
        assert!(!fab_shown("/assistant"));
        assert!(!fab_shown("/setup/broker"));
    }
}
