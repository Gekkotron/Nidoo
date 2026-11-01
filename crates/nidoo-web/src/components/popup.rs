//! A modal popup: a titled `<dialog>` over the page, for settings and short forms. It renders
//! nothing while closed and builds its content each time it opens. The ✕, Escape (the dialog's
//! `cancel`), a click on the backdrop and Back (a history pop, or `cancel` on Android) close it.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use leptos::prelude::*;

use crate::components::back_close::BackClose;
use crate::i18n::{DashText, dash_text, use_lang};

/// How much room a popup takes: a sheet fills a phone's screen, a small one never does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PopupSize {
    #[default]
    Sheet,
    Small,
}

#[component]
pub fn Popup(
    /// Open while true; every close path sets it false.
    open: RwSignal<bool>,
    #[prop(into)] title: Signal<String>,
    #[prop(optional)] size: PopupSize,
    children: ChildrenFn,
) -> impl IntoView {
    // Setting `open` to true again must not rebuild an open popup.
    let shown = Memo::new(move |_| open.get());
    move || {
        shown.get().then(|| {
            view! { <PopupDialog open=open title=title size=size children=children.clone() /> }
        })
    }
}

#[component]
fn PopupDialog(
    open: RwSignal<bool>,
    title: Signal<String>,
    size: PopupSize,
    children: ChildrenFn,
) -> impl IntoView {
    let lang = use_lang();
    let dialog = NodeRef::<leptos::html::Dialog>::new();
    // ✕, Back, `cancel` and `close` can all arrive for one closing; only the first counts.
    let closed = Arc::new(AtomicBool::new(false));
    let close = Callback::new(move |()| {
        if closed.swap(true, Ordering::SeqCst) {
            return;
        }
        open.try_set(false);
    });
    let hook = BackClose::new();
    Effect::new(move |_| {
        let Some(d) = dialog.get() else { return };
        if !d.open() {
            let _ = d.show_modal();
        }
        if !hook.is_attached() {
            hook.attach(close);
        }
    });
    on_cleanup(move || hook.detach());

    let class = match size {
        PopupSize::Sheet => "popup",
        PopupSize::Small => "popup popup--small",
    };
    view! {
        // The dialog must not close itself behind the app's back: `cancel` (Escape, Android's
        // Back) closes it through `close`, and a close the browser forces anyway ends it too.
        <dialog
            class=class
            node_ref=dialog
            on:cancel=move |ev: web_sys::Event| {
                ev.prevent_default();
                close.run(());
            }
            on:close=move |_| close.run(())
            // The dialog has no padding, so a click whose target is the dialog itself landed
            // on the backdrop.
            on:click=move |ev: web_sys::MouseEvent| {
                if ev.target() == ev.current_target() {
                    close.run(());
                }
            }
        >
            <div class="popup__bar">
                <h2 class="popup__title">{move || title.get()}</h2>
                <button
                    type="button"
                    class="popup__close"
                    aria-label=move || dash_text(lang(), DashText::Close)
                    on:click=move |_| close.run(())
                >
                    "✕"
                </button>
            </div>
            <div class="popup__body">{children()}</div>
        </dialog>
    }
}
