//! Closing an overlay with the browser's Back: while it is open the overlay owns one history
//! entry (no URL change, so the router stays out), and a `popstate` closes it.

use std::sync::atomic::{AtomicUsize, Ordering};

use leptos::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

/// The history state key that marks an overlay's entry.
const STATE_KEY: &str = "overlay";

fn next_marker() -> String {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    format!("overlay{}", NEXT.fetch_add(1, Ordering::Relaxed))
}

/// Whether the current history entry is the one the overlay marked `marker` pushed.
fn is_top(marker: &str) -> bool {
    web_sys::window()
        .and_then(|w| w.history().ok())
        .and_then(|h| h.state().ok())
        .and_then(|s| js_sys::Reflect::get(&s, &JsValue::from_str(STATE_KEY)).ok())
        .and_then(|m| m.as_string())
        .is_some_and(|m| m == marker)
}

/// What an open overlay keeps: the `popstate` listener and whether Back already popped the
/// entry it pushed.
#[derive(Clone, Copy)]
pub struct BackClose {
    listener: StoredValue<Option<Closure<dyn FnMut()>>, LocalStorage>,
    attached: StoredValue<bool, LocalStorage>,
    popped: StoredValue<bool, LocalStorage>,
    /// The state value this instance pushed.
    marker: StoredValue<String, LocalStorage>,
}

impl Default for BackClose {
    fn default() -> Self {
        Self::new()
    }
}

impl BackClose {
    pub fn new() -> Self {
        Self {
            listener: StoredValue::new_local(None),
            attached: StoredValue::new_local(false),
            popped: StoredValue::new_local(false),
            marker: StoredValue::new_local(next_marker()),
        }
    }

    pub fn is_attached(self) -> bool {
        self.attached.get_value()
    }

    /// Pushes the overlay's entry and runs `on_back` when Back pops it.
    pub fn attach(self, on_back: Callback<()>) {
        let Some(window) = web_sys::window() else {
            return;
        };
        if let Ok(history) = window.history() {
            let state = js_sys::Object::new();
            let _ = js_sys::Reflect::set(
                &state,
                &JsValue::from_str(STATE_KEY),
                &JsValue::from_str(&self.marker.get_value()),
            );
            let _ = history.push_state(&state, "");
        }
        let popped = self.popped;
        let marker = self.marker;
        let cb = Closure::<dyn FnMut()>::new(move || {
            // Back landed on this overlay's own entry: an overlay opened over it dropped its
            // entry, and this one stays open.
            if marker.with_value(|m| is_top(m)) {
                return;
            }
            popped.set_value(true);
            on_back.run(());
        });
        let _ = window.add_event_listener_with_callback("popstate", cb.as_ref().unchecked_ref());
        self.listener.set_value(Some(cb));
        self.popped.set_value(false);
        self.attached.set_value(true);
    }

    /// Removes the listener first, then drops the pushed entry unless Back already did.
    pub fn detach(self) {
        if !self.attached.get_value() {
            return;
        }
        self.attached.set_value(false);
        let Some(window) = web_sys::window() else {
            return;
        };
        self.listener.with_value(|cb| {
            if let Some(cb) = cb {
                let _ = window
                    .remove_event_listener_with_callback("popstate", cb.as_ref().unchecked_ref());
            }
        });
        // Only drop the entry while it is still the top one: if something else navigated, the
        // overlay's entry is not ours to pop.
        if !self.popped.get_value()
            && self.marker.with_value(|m| is_top(m))
            && let Ok(history) = window.history()
        {
            let _ = history.back();
        }
        self.popped.set_value(false);
    }
}
