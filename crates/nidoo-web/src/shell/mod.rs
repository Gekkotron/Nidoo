//! The frame every screen sits in: the sidebar beside a content column (header, then the screen).

pub mod header;
pub mod route;
pub mod sidebar;

use leptos::ev;
use leptos::prelude::*;
use leptos_router::hooks::use_location;
use wasm_bindgen::JsCast;

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
    // The listeners go with the shell.
    on_cleanup(move || {
        escape.remove();
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
        </div>
    }
}

/// Whether a resize to `width` closes the drawer: only an open one, once the sidebar is back.
fn closes_on_resize(open: bool, width: f64) -> bool {
    open && width >= 900.0
}

#[cfg(test)]
mod tests {
    use super::closes_on_resize;

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
