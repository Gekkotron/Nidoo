//! The toast queue: short messages in a glass pill at the bottom of the screen.

use std::sync::atomic::{AtomicU64, Ordering};

use leptos::prelude::*;

use crate::i18n::errors::{ErrorText, error_text};
use crate::i18n::use_lang;

/// How long a toast stays on screen.
#[cfg(target_arch = "wasm32")]
const TOAST_MS: u32 = 4_000;

static NEXT_TOAST_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, PartialEq)]
pub struct Toast {
    pub id: u64,
    pub text: String,
}

/// The queue, provided as a context by the shell.
#[derive(Clone, Copy)]
pub struct Toasts(pub RwSignal<Vec<Toast>>);

impl Toasts {
    pub fn new() -> Self {
        Self(RwSignal::new(Vec::new()))
    }

    /// Shows `text` for a few seconds. A message already on screen is not queued twice (a
    /// refused command is reported by both the REST reply and the WebSocket).
    pub fn push(&self, text: impl Into<String>) {
        let text = text.into();
        if self.0.with_untracked(|v| v.iter().any(|t| t.text == text)) {
            return;
        }
        let id = NEXT_TOAST_ID.fetch_add(1, Ordering::Relaxed);
        self.0.update(|v| v.push(Toast { id, text }));
        #[cfg(target_arch = "wasm32")]
        {
            let toasts = *self;
            gloo_timers::callback::Timeout::new(TOAST_MS, move || toasts.dismiss(id)).forget();
        }
    }

    pub fn dismiss(&self, id: u64) {
        self.0.update(|v| v.retain(|t| t.id != id));
    }
}

impl Default for Toasts {
    fn default() -> Self {
        Self::new()
    }
}

#[component]
pub fn ToastQueue() -> impl IntoView {
    let toasts = expect_context::<Toasts>();
    let lang = use_lang();
    view! {
        <div class="toasts" role="status" aria-live="polite">
            <For
                each=move || toasts.0.get()
                key=|t| t.id
                children=move |t| {
                    let id = t.id;
                    view! {
                        <button
                            type="button"
                            class="toast"
                            title=move || error_text(lang(), ErrorText::Dismiss)
                            on:click=move |_| toasts.dismiss(id)
                        >
                            {crate::format::wrap_id(&t.text)}
                        </button>
                    }
                }
            />
        </div>
    }
}
