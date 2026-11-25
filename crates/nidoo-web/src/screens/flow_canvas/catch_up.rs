//! The "Catch up if missed" field of the schedule, sun and wake-up trigger sheets: a toggle and a
//! window in minutes. Off writes no window at all, so the flow keeps today's behaviour.

use leptos::prelude::*;

use crate::i18n::flows::{FlowText, use_flow_text};
use crate::i18n::format::parse_decimal;

/// The longest catch-up window, in minutes (the hub rejects more).
pub const MAX_CATCH_UP_MINUTES: u32 = 720;
/// The window a freshly switched-on field starts at.
pub const DEFAULT_CATCH_UP_MINUTES: u32 = 60;

/// What the trigger stores: no window when off or at zero, else the minutes within the limit.
pub fn catch_up_value(on: bool, minutes: u32) -> Option<u32> {
    (on && minutes > 0).then(|| minutes.min(MAX_CATCH_UP_MINUTES))
}

/// The window a typed text means: whole minutes from 0 to the limit, `None` for text that is not
/// a number.
fn typed_minutes(text: &str) -> Option<u32> {
    let m = parse_decimal(text)?;
    Some(m.clamp(0.0, f64::from(MAX_CATCH_UP_MINUTES)) as u32)
}

/// The toggle and the window of a trigger, starting from its stored window; each change hands
/// the new window (or `None`) to `write`.
#[component]
pub fn CatchUpField(initial: Option<u32>, write: Callback<Option<u32>>) -> impl IntoView {
    let t = use_flow_text();
    let stored = initial.unwrap_or(0);
    let on = RwSignal::new(stored > 0);
    let minutes = RwSignal::new(if stored > 0 {
        stored
    } else {
        DEFAULT_CATCH_UP_MINUTES
    });
    let on_toggle = move |ev: web_sys::Event| {
        let now_on = event_target_checked(&ev);
        on.set(now_on);
        if now_on && minutes.get_untracked() == 0 {
            minutes.set(DEFAULT_CATCH_UP_MINUTES);
        }
        write.run(catch_up_value(now_on, minutes.get_untracked()));
    };
    let on_minutes = move |ev: web_sys::Event| {
        let Some(m) = typed_minutes(&event_target_value(&ev)) else {
            return;
        };
        minutes.set(m);
        write.run(catch_up_value(on.get_untracked(), m));
    };
    view! {
        <div class="node-editor__field">
            <label class="node-editor__check">
                <input type="checkbox" prop:checked=move || on.get() on:change=on_toggle />
                <span>{move || t(FlowText::CatchUp)}</span>
            </label>
            <Show when=move || on.get()>
                <label class="node-editor__field">
                    <span class="node-editor__label">{move || t(FlowText::CatchUpMinutes)}</span>
                    <input
                        type="number"
                        class="field"
                        min="1"
                        max=MAX_CATCH_UP_MINUTES.to_string()
                        prop:value=move || minutes.get().to_string()
                        on:change=on_minutes
                    />
                </label>
            </Show>
            <p class="node-editor__hint">{move || t(FlowText::CatchUpHelp)}</p>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_or_zero_stores_no_window() {
        assert_eq!(catch_up_value(false, 60), None);
        assert_eq!(catch_up_value(true, 0), None);
    }

    #[test]
    fn on_stores_the_window_within_the_limit() {
        assert_eq!(catch_up_value(true, 60), Some(60));
        assert_eq!(catch_up_value(true, 5000), Some(MAX_CATCH_UP_MINUTES));
    }

    #[test]
    fn typed_minutes_clamp_and_reject_text() {
        assert_eq!(typed_minutes("90"), Some(90));
        assert_eq!(typed_minutes("-5"), Some(0));
        assert_eq!(typed_minutes("9999"), Some(MAX_CATCH_UP_MINUTES));
        assert_eq!(typed_minutes("abc"), None);
    }
}
