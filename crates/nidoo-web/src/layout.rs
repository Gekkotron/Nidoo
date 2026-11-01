//! Measurements of the page chrome that floating panels sit beside.

use leptos::ev;
use leptos::prelude::*;

/// Where a floating panel starts vertically: the bottom of the content header (0 when absent).
pub(crate) fn top_offset() -> f64 {
    document()
        .query_selector(".content-header")
        .ok()
        .flatten()
        .map_or(0.0, |header| {
            header.get_bounding_client_rect().bottom().max(0.0)
        })
}

/// Where a floating panel starts horizontally: the sidebar's right edge while it sits in the page
/// flow, 0 while it is an off-canvas drawer (or absent).
pub(crate) fn left_offset() -> f64 {
    let Some(sidebar) = document().query_selector(".sidebar").ok().flatten() else {
        return 0.0;
    };
    let fixed = window()
        .get_computed_style(&sidebar)
        .ok()
        .flatten()
        .and_then(|style| style.get_property_value("position").ok())
        .is_some_and(|position| position == "fixed");
    if fixed {
        0.0
    } else {
        sidebar.get_bounding_client_rect().right()
    }
}

/// The `--flow-top` / `--flow-left` style for a fixed canvas, kept current across resizes.
/// The page scrolls back to the top first: the header is not sticky, so its position depends on
/// the scroll, and the canvas freezes the page there. Must be called while building a component.
pub(crate) fn canvas_inset_style() -> impl Fn() -> String {
    window().scroll_to_with_x_and_y(0.0, 0.0);
    let inset = RwSignal::new((0.0_f64, 0.0_f64));
    let measure = move || inset.set((top_offset(), left_offset()));
    Effect::new(move |_| measure());
    let resize = window_event_listener(ev::resize, move |_| measure());
    on_cleanup(move || resize.remove());
    move || {
        let (top, left) = inset.get();
        format!("--flow-top:{top}px;--flow-left:{left}px")
    }
}
