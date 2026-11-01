use std::collections::HashMap;

use leptos::prelude::*;
use nidoo_core::dashboard::{Cell, GRID_COLUMNS, Widget, layout};

use super::edit::EditState;
use super::widget::{WidgetView, shown};
use crate::i18n::{DashText, dash_text, use_lang};
use crate::icons::GRIP;

/// Pointer travel in CSS pixels above which a press starts a drag; below, a press on the tile
/// stays a click that opens its settings.
const DRAG_THRESHOLD: f64 = 6.0;

/// A press on a tile or its grip, until it is released.
#[derive(Clone)]
struct Press {
    id: String,
    pointer: i32,
    x: f64,
    y: f64,
    /// The pointer's offset from the tile's top-left corner, so the tile is aimed by its corner
    /// wherever it was grabbed.
    grab: (f64, f64),
    /// Set once the pointer travelled past [`DRAG_THRESHOLD`].
    dragging: bool,
}

/// Where a tile goes when it is released.
#[derive(Clone, Debug, PartialEq)]
enum Drop {
    /// A cell of the desktop grid, for the tile's top-left corner; it may be empty.
    Cell(Cell),
    /// The cell of another tile: on a narrower grid the saved cells are not on screen.
    Onto(String),
}

/// Drag-to-place state shared by every tile of a grid.
#[derive(Clone, Copy)]
struct Drag {
    grid: NodeRef<leptos::html::Div>,
    press: StoredValue<Option<Press>>,
    /// The id of the tile being dragged, for its lifted look.
    dragging: RwSignal<Option<String>>,
    /// How far the dragged tile has followed the pointer.
    shift: RwSignal<(f64, f64)>,
    target: RwSignal<Option<Drop>>,
    /// Set when a drag ends, so the click that follows the release does not open the settings
    /// of the tile under the pointer.
    swallow_click: StoredValue<bool>,
}

impl Drag {
    fn new(grid: NodeRef<leptos::html::Div>) -> Self {
        Self {
            grid,
            press: StoredValue::new(None),
            dragging: RwSignal::new(None),
            shift: RwSignal::new((0.0, 0.0)),
            target: RwSignal::new(None),
            swallow_click: StoredValue::new(false),
        }
    }

    fn down(&self, ev: &web_sys::PointerEvent, id: &str) {
        if ev.pointer_type() == "mouse" && ev.button() != 0 {
            return;
        }
        let (x, y) = (f64::from(ev.client_x()), f64::from(ev.client_y()));
        let grab = document()
            .query_selector(&format!("[data-widget-id=\"{id}\"]"))
            .ok()
            .flatten()
            .map(|tile| {
                let r = tile.get_bounding_client_rect();
                (x - r.left(), y - r.top())
            })
            .unwrap_or((0.0, 0.0));
        self.swallow_click.set_value(false);
        self.press.set_value(Some(Press {
            id: id.to_owned(),
            pointer: ev.pointer_id(),
            x,
            y,
            grab,
            dragging: false,
        }));
    }

    /// Follows the pointer on the window rather than on the tile, which stops taking pointer
    /// events while it is dragged so the tile under it can be found.
    fn moved(&self, ev: &web_sys::PointerEvent, widgets: Signal<Vec<Widget>>) {
        let Some(mut press) = self.press.get_value() else {
            return;
        };
        if press.pointer != ev.pointer_id() {
            return;
        }
        let (x, y) = (f64::from(ev.client_x()), f64::from(ev.client_y()));
        if !press.dragging {
            if (x - press.x).hypot(y - press.y) < DRAG_THRESHOLD {
                return;
            }
            press.dragging = true;
            self.dragging.set(Some(press.id.clone()));
        }
        ev.prevent_default();
        self.shift.set((x - press.x, y - press.y));
        let target = self.target_at(&press, x, y, widgets);
        if self.target.with_untracked(|t| *t != target) {
            self.target.set(target);
        }
        self.press.set_value(Some(press));
    }

    fn target_at(
        &self,
        press: &Press,
        x: f64,
        y: f64,
        widgets: Signal<Vec<Widget>>,
    ) -> Option<Drop> {
        let grid: web_sys::Element = self.grid.get_untracked()?.into();
        let tracks = read_tracks(&grid)?;
        if tracks.cols.len() == usize::from(GRID_COLUMNS) {
            let width = widgets.with_untracked(|ws| {
                ws.iter()
                    .find(|w| w.id == press.id)
                    .map_or(1, |w| w.span.size().0)
            });
            let col = nearest(&tracks.cols, 0.0, x - press.grab.0) as u16;
            let row = nearest(&tracks.rows, tracks.row_step, y - press.grab.1);
            return Some(Drop::Cell(Cell {
                row: u16::try_from(row).unwrap_or(u16::MAX),
                col: col.min(GRID_COLUMNS.saturating_sub(width)),
            }));
        }
        let hovered = document()
            .element_from_point(x as f32, y as f32)
            .and_then(|el| el.closest("[data-widget-id]").ok().flatten())
            .and_then(|el| el.get_attribute("data-widget-id"))?;
        (hovered != press.id).then_some(Drop::Onto(hovered))
    }

    /// Ends a press; a drag that was aimed somewhere places its tile there.
    fn up(&self, ev: &web_sys::PointerEvent, widgets: Signal<Vec<Widget>>, state: EditState) {
        let Some(press) = self.press.get_value() else {
            return;
        };
        if press.pointer != ev.pointer_id() {
            return;
        }
        self.swallow_click.set_value(press.dragging);
        let target = self.target.get_untracked();
        self.reset();
        let Some(target) = target.filter(|_| press.dragging) else {
            return;
        };
        let placed = widgets.with_untracked(|ws| {
            let at = |id: &str| ws.iter().position(|w| w.id == id);
            let to = match target {
                Drop::Cell(cell) => cell,
                Drop::Onto(other) => layout(ws)[at(&other)?],
            };
            Some((at(&press.id)?, to))
        });
        if let Some((index, to)) = placed {
            state.move_to(index, to);
        }
    }

    /// Drops a press without moving anything (pointer cancelled, Escape).
    fn cancel(&self) {
        if self
            .press
            .with_value(|p| p.as_ref().is_some_and(|p| p.dragging))
        {
            self.swallow_click.set_value(true);
        }
        self.reset();
    }

    fn reset(&self) {
        self.press.set_value(None);
        self.dragging.set(None);
        self.target.set(None);
        self.shift.set((0.0, 0.0));
    }
}

/// The grid's tracks as laid out, in viewport pixels.
struct Tracks {
    /// Left edge of each column.
    cols: Vec<f64>,
    /// Top edge of each row.
    rows: Vec<f64>,
    /// Row pitch assumed below the last row, so a tile can be dropped on a new one.
    row_step: f64,
}

/// Reads the column and row tracks the browser laid the grid out with.
fn read_tracks(grid: &web_sys::Element) -> Option<Tracks> {
    let style = window().get_computed_style(grid).ok()??;
    let prop = |name: &str| style.get_property_value(name).unwrap_or_default();
    let px = |name: &str| parse_px_list(&prop(name)).first().copied().unwrap_or(0.0);
    let rect = grid.get_bounding_client_rect();
    let left = rect.left() + px("border-left-width") + px("padding-left");
    let top = rect.top() + px("border-top-width") + px("padding-top");
    let row_gap = px("row-gap");
    let rows = parse_px_list(&prop("grid-template-rows"));
    let row_step = if rows.is_empty() {
        120.0 + row_gap
    } else {
        rows.iter().sum::<f64>() / rows.len() as f64 + row_gap
    };
    Some(Tracks {
        cols: track_starts(
            left,
            &parse_px_list(&prop("grid-template-columns")),
            px("column-gap"),
        ),
        rows: track_starts(top, &rows, row_gap),
        row_step,
    })
}

/// The pixel lengths of a resolved track list such as `"120px 98.5px"`; `none` and other
/// keywords give nothing.
fn parse_px_list(value: &str) -> Vec<f64> {
    value
        .split_whitespace()
        .filter_map(|t| t.strip_suffix("px")?.parse().ok())
        .collect()
}

/// Where each track starts, from the grid's content edge.
fn track_starts(origin: f64, sizes: &[f64], gap: f64) -> Vec<f64> {
    sizes
        .iter()
        .scan(origin, |at, size| {
            let start = *at;
            *at += size + gap;
            Some(start)
        })
        .collect()
}

/// The track whose start is nearest to `p`. Past the last track, with a `step` above zero,
/// the count goes on one track per `step`, so a tile can be aimed below the last row.
fn nearest(starts: &[f64], step: f64, p: f64) -> usize {
    let Some(&last) = starts.last() else {
        return 0;
    };
    if step > 0.0 && p > last {
        return starts.len() - 1 + ((p - last) / step).round() as usize;
    }
    starts
        .iter()
        .enumerate()
        .min_by(|a, b| (a.1 - p).abs().total_cmp(&(b.1 - p).abs()))
        .map_or(0, |(i, _)| i)
}

/// The grid of a dashboard's widgets. On the desktop grid (six columns) each widget sits on its
/// cell (see `nidoo_core::dashboard::layout`), so empty space between widgets stays; narrower
/// grids flow the widgets in reading order. Column counts per breakpoint and the placement
/// live in `styles/main.css` (`.dashboard-grid`).
///
/// In edit mode a tile is dragged from the grip in its tool pill (mouse, pen or touch) or, with
/// a mouse or pen, from anywhere on it; a touch on the tile itself still scrolls the page. The
/// tile follows the pointer and, on the desktop grid, an outline marks the cell its corner will
/// land on, empty or not; on release it moves there (see `move_widget`). On a narrower grid it
/// takes the place of the tile it is released on. Escape cancels. The arrow keys move a
/// focused grip by one cell. Pointer events rather than HTML5 drag, which has no touch support.
#[component]
pub fn Grid(
    widgets: Signal<Vec<Widget>>,
    #[prop(optional)] edit: Option<EditState>,
) -> impl IntoView {
    let active = move || edit.is_some_and(|e| e.active.get());
    let lang = use_lang();
    let grid = NodeRef::<leptos::html::Div>::new();
    let drag = Drag::new(grid);
    if let Some(state) = edit {
        let moved = window_event_listener(leptos::ev::pointermove, move |ev| {
            drag.moved(&ev, widgets);
        });
        let up = window_event_listener(leptos::ev::pointerup, move |ev| {
            drag.up(&ev, widgets, state);
        });
        let cancel = window_event_listener(leptos::ev::pointercancel, move |_| drag.cancel());
        let escape = window_event_listener(leptos::ev::keydown, move |ev| {
            if ev.key() == "Escape" && drag.dragging.with_untracked(Option::is_some) {
                drag.cancel();
            }
        });
        on_cleanup(move || {
            moved.remove();
            up.remove();
            cancel.remove();
            escape.remove();
        });
    }
    // The tile of a widget just added is brought into view once it has rendered.
    #[cfg(target_arch = "wasm32")]
    if let Some(state) = edit {
        Effect::new(move |_| {
            if let Some(id) = state.fresh.get() {
                request_animation_frame(move || {
                    let selector = format!("[data-widget-id=\"{id}\"]");
                    if let Ok(Some(tile)) = document().query_selector(&selector) {
                        let options = web_sys::ScrollIntoViewOptions::new();
                        options.set_block(web_sys::ScrollLogicalPosition::Nearest);
                        options.set_behavior(web_sys::ScrollBehavior::Smooth);
                        tile.scroll_into_view_with_scroll_into_view_options(&options);
                    }
                });
            }
        });
    }
    let cells = Memo::new(move |_| {
        widgets.with(|ws| {
            ws.iter()
                .map(|w| w.id.clone())
                .zip(layout(ws))
                .collect::<HashMap<String, Cell>>()
        })
    });
    // Reading order, which narrower grids flow the tiles in.
    let ordered = move || {
        let mut ws = widgets.get();
        cells.with(|c| ws.sort_by_key(|w| c.get(&w.id).copied()));
        ws
    };
    let ghost = move || {
        let Some(Drop::Cell(at)) = drag.target.get() else {
            return None;
        };
        let span = drag.dragging.with(|id| {
            let id = id.as_deref()?;
            widgets.with(|ws| ws.iter().find(|w| w.id == id).map(|w| w.span))
        })?;
        let (w, h) = span.size();
        Some(view! {
            <div class="dashboard-grid__ghost" style=placement(at, w, h) aria-hidden="true"></div>
        })
    };
    view! {
        <div
            class="dashboard-grid"
            class:dashboard-grid--dragging=move || drag.dragging.with(Option::is_some)
            node_ref=grid
        >
            <For
                each=ordered
                key=|w| w.id.clone()
                children=move |w: Widget| {
                    // Keyed by id, so the tile survives a span change or move; the class, cell
                    // and index must therefore be looked up reactively.
                    let id = w.id.clone();
                    let position = {
                        let id = id.clone();
                        // Callback is Copy, so the handlers can each capture it.
                        Callback::new(move |()| {
                            widgets.with(|ws| ws.iter().position(|x| x.id == id))
                        })
                    };
                    let span = {
                        let id = id.clone();
                        Memo::new(move |_| {
                            widgets
                                .with(|ws| ws.iter().find(|x| x.id == id).map(|x| x.span))
                                .unwrap_or(w.span)
                        })
                    };
                    let class = move || {
                        // `concat!` keeps the bare-text guard from reading the class list as text.
                        format!(concat!("widget", " widget-span-{}"), span.get().as_str())
                    };
                    let is_dragged = {
                        let id = id.clone();
                        move || drag.dragging.with(|d| d.as_deref() == Some(id.as_str()))
                    };
                    let is_target = {
                        let id = id.clone();
                        move || drag.target.with(|t| matches!(t, Some(Drop::Onto(o)) if *o == id))
                    };
                    let style = {
                        let id = id.clone();
                        let is_dragged = is_dragged.clone();
                        move || {
                            let at = cells
                                .with(|c| c.get(&id).copied())
                                .unwrap_or(Cell { row: 0, col: 0 });
                            let (w, h) = span.get().size();
                            let mut style = placement(at, w, h);
                            if is_dragged() {
                                let (dx, dy) = drag.shift.get();
                                style.push_str(&format!(";translate:{dx}px {dy}px"));
                            }
                            style
                        }
                    };
                    let current = {
                        let id = id.clone();
                        let initial = w.clone();
                        Signal::derive(move || {
                            widgets
                                .with(|ws| ws.iter().find(|x| x.id == id).cloned())
                                .unwrap_or_else(|| initial.clone())
                        })
                    };
                    // A new target or chart range rebuilds the card, not the tile, so a drag or
                    // the open config sheet carries on.
                    let shows = Memo::new(move |_| current.with(|w| shown(&w.config)));
                    let is_fresh = {
                        let id = id.clone();
                        move || edit.is_some_and(|e| e.fresh.with(|f| f.as_deref() == Some(id.as_str())))
                    };
                    let tools = edit.map(|state| {
                        let grip_id = id.clone();
                        let cell_id = id.clone();
                        view! {
                            <Show when=active>
                                // Stop bubbling so the tile's open-config click does not fire
                                // for the grip/resize buttons.
                                <div class="widget__tools" on:click=|ev| ev.stop_propagation()>
                                    <button
                                        type="button"
                                        class="widget__grip"
                                        aria-label=move || dash_text(lang(), DashText::Move)
                                        inner_html=GRIP
                                        on:pointerdown={
                                            let grip_id = grip_id.clone();
                                            move |ev: web_sys::PointerEvent| drag.down(&ev, &grip_id)
                                        }
                                        on:keydown={
                                            let cell_id = cell_id.clone();
                                            move |ev: web_sys::KeyboardEvent| {
                                                let Some(i) = position.run(()) else { return };
                                                let Some(at) = cells.with_untracked(|c| c.get(&cell_id).copied()) else {
                                                    return;
                                                };
                                                let to = match ev.key().as_str() {
                                                    "ArrowLeft" => at.col.checked_sub(1).map(|col| Cell { col, ..at }),
                                                    "ArrowRight" => Some(Cell { col: at.col + 1, ..at }),
                                                    "ArrowUp" => at.row.checked_sub(1).map(|row| Cell { row, ..at }),
                                                    "ArrowDown" => Some(Cell { row: at.row + 1, ..at }),
                                                    _ => return,
                                                };
                                                ev.prevent_default();
                                                if let Some(to) = to {
                                                    state.move_to(i, to);
                                                }
                                            }
                                        }
                                    ></button>
                                    <button
                                        type="button"
                                        class="widget__resize"
                                        aria-label=move || dash_text(lang(), DashText::Resize)
                                        on:click=move |_| {
                                            if let Some(i) = position.run(()) {
                                                state.cycle_span(i);
                                            }
                                        }
                                    >
                                        "⤢"
                                    </button>
                                </div>
                            </Show>
                        }
                    });
                    let overlay_id = id.clone();
                    view! {
                        <div
                            class=class
                            class:widget--fresh=is_fresh
                            class:widget--dragging=is_dragged
                            class:widget--drop-target=is_target
                            style=style
                            data-widget-id=id
                        >
                            {move || {
                                shows.get();
                                view! { <WidgetView widget=current.get_untracked() current=current /> }
                            }}
                            // An overlay (not a tile click handler) so the inner card's own
                            // controls, e.g. a light toggle, cannot fire while editing. It sits
                            // above the card and below `.widget__tools` (z-index 2).
                            {edit.map(|state| view! {
                                <Show when=active>
                                    <div
                                        class="widget__edit-overlay"
                                        on:pointerdown={
                                            let overlay_id = overlay_id.clone();
                                            move |ev: web_sys::PointerEvent| {
                                                // A drag that ended over another tile leaves the flag
                                                // set, as its click went to the grid; a new press clears it.
                                                drag.swallow_click.set_value(false);
                                                // A touch on the tile scrolls the page; the grip drags.
                                                if ev.pointer_type() != "touch" {
                                                    drag.down(&ev, &overlay_id);
                                                }
                                            }
                                        }
                                        on:click=move |_| {
                                            if drag.swallow_click.get_value() {
                                                drag.swallow_click.set_value(false);
                                                return;
                                            }
                                            if let Some(i) = position.run(()) {
                                                state.selected_widget.set(Some(i));
                                            }
                                        }
                                    />
                                </Show>
                            })}
                            {tools}
                        </div>
                    }
                }
            />
            {ghost}
        </div>
    }
}

/// The CSS variables that put a tile of `w` × `h` cells on cell `at` of the desktop grid
/// (1-based lines for `grid-column` / `grid-row`).
fn placement(at: Cell, w: u16, h: u16) -> String {
    format!("--col:{};--row:{};--w:{w};--h:{h}", at.col + 1, at.row + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn px_lists_parse_and_keywords_give_nothing() {
        assert_eq!(parse_px_list("120px 98.5px"), vec![120.0, 98.5]);
        assert_eq!(parse_px_list("12px"), vec![12.0]);
        assert!(parse_px_list("none").is_empty());
        assert!(parse_px_list("").is_empty());
    }

    #[test]
    fn track_starts_add_sizes_and_gaps() {
        assert_eq!(
            track_starts(10.0, &[100.0, 100.0, 50.0], 12.0),
            vec![10.0, 122.0, 234.0]
        );
    }

    #[test]
    fn nearest_picks_the_closest_start_and_counts_on_past_the_last() {
        let starts = [0.0, 100.0, 200.0];
        assert_eq!(nearest(&starts, 0.0, -40.0), 0);
        assert_eq!(nearest(&starts, 0.0, 140.0), 1);
        assert_eq!(nearest(&starts, 0.0, 160.0), 2);
        // Columns: no step, so far right stays on the last track.
        assert_eq!(nearest(&starts, 0.0, 900.0), 2);
        // Rows: one more row per step below the last.
        assert_eq!(nearest(&starts, 100.0, 240.0), 2);
        assert_eq!(nearest(&starts, 100.0, 260.0), 3);
        assert_eq!(nearest(&starts, 100.0, 410.0), 4);
        assert_eq!(nearest(&[], 100.0, 50.0), 0);
    }

    #[test]
    fn placement_uses_one_based_lines() {
        assert_eq!(
            placement(Cell { row: 1, col: 0 }, 2, 1),
            "--col:1;--row:2;--w:2;--h:1"
        );
    }
}
