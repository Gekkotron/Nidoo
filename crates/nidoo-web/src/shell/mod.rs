//! The frame every screen sits in: the sidebar beside a content column (header, then the screen).

pub mod header;
pub mod route;
pub mod sidebar;

use leptos::ev;
use leptos::prelude::*;
use leptos_router::hooks::{use_location, use_navigate};
use wasm_bindgen::JsCast;

use crate::components::assist_fab::AssistFab;
use header::ContentHeader;
use sidebar::Sidebar;

/// Render it inside the router, around the routes. The four signals are the sidebar's live marks:
/// the Climate entry, the armed and solar glyph colours, and the unseen-alerts badge.
#[component]
pub fn Shell(
    #[prop(into)] climate_on: Signal<bool>,
    #[prop(into)] alarm_armed: Signal<bool>,
    #[prop(into)] solar_active: Signal<bool>,
    #[prop(into)] alerts_badge: Signal<Option<String>>,
    children: Children,
) -> impl IntoView {
    // Below 900px the sidebar is a drawer; this says whether it is out.
    let drawer_open = RwSignal::new(false);
    let location = use_location();
    Effect::new(move |_| {
        location.pathname.track();
        drawer_open.set(false);
    });
    let escape = window_event_listener(ev::keydown, move |event| {
        // `try_`: the listener can outlive an unmounted shell for a moment.
        if event.key() == "Escape" && drawer_open.try_get_untracked() == Some(true) {
            drawer_open.try_set(false);
        }
    });
    // Widening past the phone layout (rotating, resizing) leaves nothing to close by hand.
    let widen = window_event_listener(ev::resize, move |_| {
        let width = window().inner_width().ok().and_then(|w| w.as_f64());
        let open = drawer_open.try_get_untracked() == Some(true);
        if width.is_some_and(|w| closes_on_resize(open, w)) {
            drawer_open.try_set(false);
        }
    });
    // `/` opens the search from anywhere but a text field, where it is just a character.
    // Already on the search, it puts the cursor back in the field instead of starting over. Not
    // under an open dialog, nor when something else already handled the key.
    let navigate = use_navigate();
    let pathname = location.pathname;
    let slash = window_event_listener(ev::keydown, move |event| {
        let modifier = event.ctrl_key() || event.meta_key() || event.alt_key();
        if event.default_prevented()
            || dialog_open()
            || !opens_search(&event.key(), typing(&event), modifier)
        {
            return;
        }
        event.prevent_default();
        if pathname.try_get_untracked().as_deref() == Some("/search") {
            let field = document()
                .query_selector("input.search__input")
                .ok()
                .flatten()
                .and_then(|el| el.dyn_into::<web_sys::HtmlElement>().ok());
            if let Some(field) = field {
                let _ = field.focus();
            }
        } else {
            navigate("/search", Default::default());
        }
    });
    // The listeners go with the shell.
    on_cleanup(move || {
        escape.remove();
        slash.remove();
        widen.remove();
    });
    // Focus follows the drawer: into it when it opens, back to the menu button when it closes.
    Effect::new(move |was_open: Option<bool>| {
        let open = drawer_open.get();
        // The content is inert while the drawer is out; free it before focus goes back to it.
        if !open && let Some(content) = document().query_selector(".shell__content").ok().flatten()
        {
            let _ = content.remove_attribute("inert");
        }
        let selector = match (was_open, open) {
            (_, true) => Some(".sidebar a"),
            (Some(true), false) => Some(".content-header__menu"),
            _ => None,
        };
        let target = selector
            .and_then(|s| document().query_selector(s).ok().flatten())
            .and_then(|el| el.dyn_into::<web_sys::HtmlElement>().ok());
        if let Some(el) = target {
            let _ = el.focus();
        }
        open
    });

    // The wizard is a bare page: no sidebar or header.
    let bare = move || location.pathname.with(|p| p.starts_with("/setup"));
    view! {
        <div class="shell" class:shell--bare=bare>
            <Show when=move || !bare()>
                <Sidebar climate_on alarm_armed solar_active alerts_badge drawer_open />
            </Show>
            <Show when=move || drawer_open.get()>
                <div class="shell__scrim" aria-hidden="true" on:click=move |_| drawer_open.set(false)></div>
            </Show>
            <div class="shell__content" inert=move || drawer_open.get().then_some("")>
                <Show when=move || !bare()>
                    <ContentHeader drawer_open />
                </Show>
                <main class="screen">{children()}</main>
            </div>
            <AssistFab drawer_open />
        </div>
    }
}

/// Whether a resize to `width` closes the drawer: only an open one, once the sidebar is back.
fn closes_on_resize(open: bool, width: f64) -> bool {
    open && width >= 900.0
}

/// Whether a key press opens the search: a bare `/` that is not typing into a text field.
pub(crate) fn opens_search(key: &str, target_is_text: bool, modifier: bool) -> bool {
    key == "/" && !target_is_text && !modifier
}

/// Whether a key event comes from somewhere keys are typed: a form field or editable text. Keys
/// there are characters, not shortcuts.
pub(crate) fn typing(event: &web_sys::KeyboardEvent) -> bool {
    let Some(target) = event.target() else {
        return false;
    };
    if let Some(el) = target.dyn_ref::<web_sys::HtmlElement>()
        && el.is_content_editable()
    {
        return true;
    }
    target.dyn_ref::<web_sys::Element>().is_some_and(|el| {
        matches!(
            el.tag_name().to_ascii_lowercase().as_str(),
            "input" | "textarea" | "select"
        )
    })
}

/// Whether a modal dialog (a popup, the camera viewer) is open over the page.
fn dialog_open() -> bool {
    document()
        .query_selector("dialog[open]")
        .ok()
        .flatten()
        .is_some()
}

#[cfg(test)]
mod tests {
    use super::{closes_on_resize, opens_search};

    #[test]
    fn opens_search_only_outside_text_fields() {
        assert!(opens_search("/", false, false));
        assert!(!opens_search("/", true, false));
        assert!(!opens_search("/", false, true));
        assert!(!opens_search("a", false, false));
    }

    #[test]
    fn widening_closes_only_an_open_drawer() {
        assert!(closes_on_resize(true, 900.0));
        assert!(closes_on_resize(true, 1400.0));
        // A closed drawer is left alone: no write, so nothing that reads it re-runs.
        assert!(!closes_on_resize(false, 1400.0));
        assert!(!closes_on_resize(true, 899.0));
        assert!(!closes_on_resize(false, 600.0));
    }
}
