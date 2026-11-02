//! The round button in the bottom-right corner of every page that opens the Assistant.

use leptos::prelude::*;
use leptos_router::hooks::use_location;

use crate::i18n::{AssistantText, assistant_text, use_lang};
use crate::icons::SPEECH;

/// Whether the button shows on `path`: not on the Assistant itself, nor in the setup wizard.
pub fn fab_shown(path: &str) -> bool {
    !(path == "/assistant" || path.starts_with("/assistant/") || path.starts_with("/setup"))
}

/// Hidden while the phone drawer (`drawer_open`) is out.
#[component]
pub fn AssistFab(drawer_open: RwSignal<bool>) -> impl IntoView {
    let lang = use_lang();
    let location = use_location();
    let shown = move || location.pathname.with(|p| fab_shown(p)) && !drawer_open.get();
    view! {
        <Show when=shown>
            <a
                class="assist-fab"
                href="/assistant"
                aria-label=move || assistant_text(lang(), AssistantText::Open)
                title=move || assistant_text(lang(), AssistantText::Open)
                inner_html=SPEECH
            ></a>
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
