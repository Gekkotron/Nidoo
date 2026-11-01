//! Navigation state that outlives a screen: the stack every Back link reads.

use leptos::prelude::*;
use leptos_router::components::A;
use leptos_router::hooks::use_location;

use crate::i18n::errors::{ErrorText, error_text};
use crate::i18n::use_lang;

/// The pages visited so far, path and query, oldest first; the last one is the current page.
/// Back goes to the one before it, so every sheet returns to the page it was opened from, and a
/// chain of sheets (dashboard → group → member → history) unwinds one step at a time.
#[derive(Clone, Copy)]
pub struct BackStack(pub RwSignal<Vec<String>>);

/// How many pages the stack keeps; older ones fall off the bottom.
const DEPTH: usize = 32;

/// Provides [`BackStack`]. Call it once, above the router.
pub fn install_nav() {
    provide_context(BackStack(RwSignal::new(Vec::new())));
}

/// The path of a stack entry, without its query or fragment.
fn path_of(entry: &str) -> &str {
    entry.split(['?', '#']).next().unwrap_or(entry)
}

/// Gives the top entry the fragment `#id` when it is the page at `path`, so Back to it scrolls
/// to the element with that id (the overview's room a card was opened from) instead of the top.
/// The router decodes the fragment once before looking the id up, hence the extra escaping.
fn anchor_top(stack: &mut [String], path: &str, id: &str) {
    if let Some(top) = stack.last_mut().filter(|top| path_of(top) == path) {
        top.truncate(top.find('#').unwrap_or(top.len()));
        top.push('#');
        top.push_str(&crate::api::encode_id(id));
    }
}

/// A function that makes Back to the current page land on the element with the given id (see
/// [`anchor_top`]). Call it while building a component: the page is read then, because by the
/// time a click handler above a card runs, the card's `navigate` has already moved the address.
pub fn use_back_anchor() -> impl Fn(&str) + Clone + 'static {
    let stack = use_context::<BackStack>().map(|b| b.0);
    let page = web_sys::window().and_then(|w| w.location().pathname().ok());
    move |id: &str| {
        if let (Some(stack), Some(page)) = (stack, page.as_deref()) {
            stack.try_update(|s| anchor_top(s, page, id));
        }
    }
}

/// A form page: once left, Back never returns to it (saving a new group opens the group, whose
/// Back then goes where the form was opened from).
fn is_form(path: &str) -> bool {
    path == "/groups/new"
        || path == "/flows/new"
        || path == "/ambiances/new"
        || (path.starts_with("/groups/") && path.ends_with("/edit"))
}

/// Records a visit to `path` with query `search` (without its `?`).
fn visit(stack: &mut Vec<String>, path: &str, search: &str) {
    // "/" only redirects to a dashboard, which is recorded when it lands.
    if path == "/" {
        return;
    }
    let entry = if search.is_empty() {
        path.to_owned()
    } else {
        format!("{path}?{search}")
    };
    // Back to a page on the stack, or the same page with another query: unwind to it.
    if let Some(at) = stack.iter().rposition(|e| path_of(e) == path) {
        stack.truncate(at);
    } else if stack.last().is_some_and(|top| is_form(path_of(top))) {
        stack.pop();
    }
    stack.push(entry);
    if stack.len() > DEPTH {
        stack.drain(..stack.len() - DEPTH);
    }
}

/// Where Back goes from a page opened with nothing before it (a shared link, a reload).
fn parent_of(path: &str) -> String {
    let child = |prefix: &str| path.strip_prefix(prefix).filter(|rest| !rest.is_empty());
    if let Some(id) = child("/history/") {
        format!("/entity/{id}")
    } else if child("/alerts/").is_some() {
        "/alerts".to_owned()
    } else if child("/flows/").is_some() {
        "/flows".to_owned()
    } else if child("/groups/").is_some() {
        "/groups".to_owned()
    } else if child("/ambiances/").is_some() {
        "/ambiances".to_owned()
    } else if child("/settings/").is_some() {
        "/settings".to_owned()
    } else {
        "/".to_owned()
    }
}

/// Where Back goes from the top of `stack`.
fn back_target(stack: &[String]) -> String {
    match stack {
        [.., previous, _] => previous.clone(),
        [current] => parent_of(path_of(current)),
        [] => "/".to_owned(),
    }
}

/// Where Back goes from the current page.
pub fn use_back() -> Signal<String> {
    let stack = expect_context::<BackStack>().0;
    Signal::derive(move || stack.with(|s| back_target(s)))
}

/// Keeps [`BackStack`] on the current page. Render it inside the router.
#[component]
pub fn TrackBackStack() -> impl IntoView {
    let location = use_location();
    let stack = expect_context::<BackStack>().0;
    Effect::new(move |_| {
        let (path, search) = (location.pathname.get(), location.search.get());
        stack.update(|s| visit(s, &path, &search));
    });
}

/// The "← Back" link of a sheet header.
#[component]
pub fn BackLink() -> impl IntoView {
    let back = use_back();
    let lang = use_lang();
    view! {
        <A href=move || back.get() attr:class="sheet__back">
            "←"
            <span class="sheet__back-label">{move || error_text(lang(), ErrorText::Back)}</span>
        </A>
    }
}

#[cfg(test)]
mod tests {
    use super::{DEPTH, anchor_top, back_target, parent_of, visit};

    fn walk(pages: &[&str]) -> Vec<String> {
        let mut stack = Vec::new();
        for page in pages {
            let (path, search) = page.split_once('?').unwrap_or((page, ""));
            visit(&mut stack, path, search);
        }
        stack
    }

    #[test]
    fn group_back_returns_to_its_dashboard() {
        // The bug: Back on a group went to "/", which redirects to the first dashboard.
        let stack = walk(&["/", "/dashboards/first", "/dashboards/mine", "/group/salon"]);
        assert_eq!(back_target(&stack), "/dashboards/mine");
    }

    #[test]
    fn sheets_unwind_one_step_at_a_time() {
        let mut stack = walk(&[
            "/dashboards/mine",
            "/group/salon",
            "/entity/lamp",
            "/history/lamp?range=24h",
        ]);
        assert_eq!(back_target(&stack), "/entity/lamp");
        for (page, back) in [
            ("/entity/lamp", "/group/salon"),
            ("/group/salon", "/dashboards/mine"),
        ] {
            visit(&mut stack, page, "");
            assert_eq!(back_target(&stack), back);
        }
    }

    #[test]
    fn a_link_back_to_an_open_page_does_not_loop() {
        // Group → member → the member's own link to the group: Back leaves the group.
        let stack = walk(&["/devices", "/group/salon", "/entity/lamp", "/group/salon"]);
        assert_eq!(stack, ["/devices", "/group/salon"]);
        assert_eq!(back_target(&stack), "/devices");
    }

    #[test]
    fn a_query_change_stays_on_the_page() {
        let stack = walk(&[
            "/devices?q=lamp",
            "/entity/lamp",
            "/history/lamp?range=24h",
            "/history/lamp?range=7d",
        ]);
        assert_eq!(
            stack,
            ["/devices?q=lamp", "/entity/lamp", "/history/lamp?range=7d"]
        );
        assert_eq!(back_target(&stack[..2]), "/devices?q=lamp");
    }

    #[test]
    fn a_saved_form_is_not_a_back_target() {
        let new = walk(&["/groups", "/groups/new", "/group/salon"]);
        assert_eq!(back_target(&new), "/groups");
        let edited = walk(&[
            "/dashboards/mine",
            "/group/salon",
            "/groups/salon/edit",
            "/group/salon",
        ]);
        assert_eq!(back_target(&edited), "/dashboards/mine");
        let flow = walk(&["/flows", "/flows/new", "/flows/porch"]);
        assert_eq!(back_target(&flow), "/flows");
        let mood = walk(&["/ambiances", "/ambiances/new", "/ambiances/night"]);
        assert_eq!(back_target(&mood), "/ambiances");
    }

    #[test]
    fn a_page_opened_first_goes_to_its_parent() {
        assert_eq!(back_target(&walk(&["/history/lamp"])), "/entity/lamp");
        assert_eq!(back_target(&walk(&["/alerts/a1"])), "/alerts");
        assert_eq!(back_target(&walk(&["/flows/porch"])), "/flows");
        assert_eq!(back_target(&walk(&["/groups/salon/edit"])), "/groups");
        for page in [
            "/ambiances/night",
            "/ambiances/night/report",
            "/ambiances/about",
            "/ambiances/new",
        ] {
            assert_eq!(back_target(&walk(&[page])), "/ambiances", "{page}");
        }
        assert_eq!(back_target(&walk(&["/settings/zigbee2mqtt"])), "/settings");
        assert_eq!(back_target(&walk(&["/group/salon"])), "/");
        assert_eq!(back_target(&walk(&["/entity/lamp"])), "/");
        assert_eq!(back_target(&[]), "/");
        assert_eq!(parent_of("/alerts"), "/");
    }

    #[test]
    fn the_stack_is_bounded() {
        let pages: Vec<String> = (0..40).map(|i| format!("/entity/e{i}")).collect();
        let pages: Vec<&str> = pages.iter().map(String::as_str).collect();
        let stack = walk(&pages);
        assert_eq!(stack.len(), DEPTH);
        assert_eq!(stack.last().map(String::as_str), Some("/entity/e39"));
    }

    #[test]
    fn back_returns_to_the_room_a_card_was_opened_from() {
        let mut stack = walk(&["/dashboards/mine", "/dashboards/house"]);
        anchor_top(&mut stack, "/dashboards/house", "room-Cuisine");
        // A later click in another room moves the anchor.
        anchor_top(&mut stack, "/dashboards/house", "room-Salle%20de%20bain");
        assert_eq!(
            stack.last().map(String::as_str),
            Some("/dashboards/house#room-Salle%2520de%2520bain")
        );
        visit(&mut stack, "/entity/lamp", "");
        assert_eq!(
            back_target(&stack),
            "/dashboards/house#room-Salle%2520de%2520bain"
        );
        // Landing back on the overview unwinds to it, without the anchor.
        visit(&mut stack, "/dashboards/house", "");
        assert_eq!(stack, ["/dashboards/mine", "/dashboards/house"]);
        // Only the page on top takes an anchor, and a query stays.
        anchor_top(&mut stack, "/rooms/a", "room-a");
        assert_eq!(stack.last().map(String::as_str), Some("/dashboards/house"));
        let mut edit = walk(&["/dashboards/d1?edit=1"]);
        anchor_top(&mut edit, "/dashboards/d1", "room-a");
        assert_eq!(edit, ["/dashboards/d1?edit=1#room-a"]);
    }
}
