//! The pannable, zoomable surface the graph is drawn on, its +/−/fit controls and the minimap.
//! The maths lives in `flow_viewport`; this is the DOM side: pointer, wheel and pinch input and
//! the measured screen size.

use leptos::ev;
use leptos::html;
use leptos::prelude::*;
use nidoo_core::flow::Flow;
use wasm_bindgen::JsCast;

use crate::flow_layout::{NODE_W, Pt, Rect, bounds, node_height, positions};
use crate::flow_text::{Role, role};
use crate::flow_viewport::{
    Mini, View, ZOOM_STEP, centre_on, fit_beside, mini_to_world, minimap, visible_world,
};
use crate::i18n::flows::{FlowText, use_flow_text};

/// Minimap size in CSS pixels.
const MINI_W: f64 = 180.0;
const MINI_H: f64 = 120.0;
/// World spacing of the dotted grid behind the cards.
const GRID: f64 = 20.0;
/// Pixels one wheel "line" scrolls by when the browser reports lines instead of pixels.
const WHEEL_LINE: f64 = 16.0;
/// Pointer travel in CSS pixels above which a press on the surface is a pan, not a tap.
const TAP_SLOP: f64 = 3.0;

/// The canvas camera, shared by the viewport, its controls, the minimap and the graph: the pan
/// and zoom, the surface's measured size in CSS pixels (0×0 until it has been laid out) and how
/// much of its right side the open side panel covers (0 when closed); fitting leaves that free.
#[derive(Clone, Copy)]
pub struct Camera {
    pub view: RwSignal<View>,
    pub size: RwSignal<(f64, f64)>,
    pub inset: RwSignal<f64>,
}

impl Camera {
    /// A camera at the origin, unmeasured; provided as context to everything below.
    pub fn provide() -> Self {
        let camera = Self {
            view: RwSignal::new(View {
                x: 0.0,
                y: 0.0,
                zoom: 1.0,
            }),
            size: RwSignal::new((0.0, 0.0)),
            inset: RwSignal::new(0.0),
        };
        provide_context(camera);
        camera
    }

    /// The measured size, or `None` before the surface has one (the maths divides by it).
    fn measured(self) -> Option<(f64, f64)> {
        let (w, h) = self.size.get_untracked();
        (w > 0.0 && h > 0.0).then_some((w, h))
    }
}

/// The client point `(client_x, client_y)` relative to `el`'s top-left corner.
fn local(el: &web_sys::Element, client_x: i32, client_y: i32) -> Pt {
    let rect = el.get_bounding_client_rect();
    Pt {
        x: f64::from(client_x) - rect.left(),
        y: f64::from(client_y) - rect.top(),
    }
}

fn midpoint(a: Pt, b: Pt) -> Pt {
    Pt {
        x: (a.x + b.x) / 2.0,
        y: (a.y + b.y) / 2.0,
    }
}

fn distance(a: Pt, b: Pt) -> f64 {
    (a.x - b.x).hypot(a.y - b.y)
}

/// The surface: `overflow: hidden`, one inner layer carrying [`View::css_transform`] that the
/// children (cards and wires, in world coordinates) live in. Drag pans, Ctrl/⌘-wheel and pinch
/// zoom, plain wheel pans. It measures itself into the [`Camera`] and fits `content` once it has
/// a size; until the user moves the view, a resize fits again. A press that does not move is a
/// tap: `on_tap` gets the element it landed on.
#[component]
pub fn Viewport(
    view: RwSignal<View>,
    content: Signal<Rect>,
    #[prop(optional)] on_tap: Option<Callback<Option<web_sys::Element>>>,
    children: Children,
) -> impl IntoView {
    let camera = expect_context::<Camera>();
    let size = camera.size;
    let surface: NodeRef<html::Div> = NodeRef::new();
    // The view the last automatic fit produced; while the view still equals it, the user has
    // not moved it and a new size fits again.
    let last_fit = StoredValue::new(None::<View>);
    // Active pointers as (pointer id, surface point).
    let pointers = StoredValue::new(Vec::<(i32, Pt)>::new());
    // A press that may still be a tap: where it started and what it landed on.
    let tap = StoredValue::new_local(None::<(Pt, Option<web_sys::Element>)>);

    // Also runs from an animation frame, which can fire after the canvas is gone.
    let measure = move || {
        if let Some(Some(el)) = surface.try_get_untracked() {
            let rect = el.get_bounding_client_rect();
            size.try_set((rect.width(), rect.height()));
        }
    };
    Effect::new(move |_| {
        if surface.get().is_some() {
            measure();
            // Again once the page has settled (the top offset is measured on mount too).
            request_animation_frame(measure);
        }
    });
    let resize = window_event_listener(ev::resize, move |_| request_animation_frame(measure));
    on_cleanup(move || resize.remove());

    // Fits again when the size or the side panel changes, until the user moves the view.
    Effect::new(move |_| {
        let (w, h) = size.get();
        let inset = camera.inset.get();
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        let untouched = last_fit
            .get_value()
            .is_none_or(|fitted| fitted == view.get_untracked());
        if untouched {
            let fitted = fit_beside(content.get_untracked(), w, h, inset);
            last_fit.set_value(Some(fitted));
            view.set(fitted);
        }
    });

    let down = move |ev: web_sys::PointerEvent| {
        if ev.button() != 0 && ev.pointer_type() == "mouse" {
            return;
        }
        let Some(el) = surface.get_untracked() else {
            return;
        };
        let _ = el.set_pointer_capture(ev.pointer_id());
        let at = local(&el, ev.client_x(), ev.client_y());
        pointers.update_value(|p| {
            p.retain(|(id, _)| *id != ev.pointer_id());
            p.push((ev.pointer_id(), at));
        });
        let first = pointers.with_value(|p| p.len() == 1);
        let target = ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::Element>().ok());
        tap.set_value(first.then_some((at, target)));
    };
    let moved = move |ev: web_sys::PointerEvent| {
        let Some(el) = surface.get_untracked() else {
            return;
        };
        let at = local(&el, ev.client_x(), ev.client_y());
        let before = pointers.get_value();
        let Some(i) = before.iter().position(|(id, _)| *id == ev.pointer_id()) else {
            return;
        };
        let mut after = before.clone();
        after[i].1 = at;
        let panned =
            tap.with_value(|t| t.as_ref().is_some_and(|(s, _)| distance(*s, at) > TAP_SLOP));
        if panned {
            tap.set_value(None);
        }
        match (before.as_slice(), after.as_slice()) {
            ([(_, a)], [(_, b)]) => view.update(|v| *v = v.pan(b.x - a.x, b.y - a.y)),
            ([(_, a0), (_, a1), ..], [(_, b0), (_, b1), ..]) => {
                let (m0, m1) = (midpoint(*a0, *a1), midpoint(*b0, *b1));
                let (d0, d1) = (distance(*a0, *a1), distance(*b0, *b1));
                view.update(|v| {
                    let panned = v.pan(m1.x - m0.x, m1.y - m0.y);
                    *v = if d0 > 0.0 {
                        panned.zoom_at(m1, d1 / d0)
                    } else {
                        panned
                    };
                });
            }
            _ => {}
        }
        pointers.set_value(after);
    };
    let up = move |ev: web_sys::PointerEvent| {
        pointers.update_value(|p| p.retain(|(id, _)| *id != ev.pointer_id()));
        let tapped = tap.try_update_value(Option::take).flatten();
        if let (Some((_, target)), Some(on_tap), "pointerup") =
            (tapped, on_tap, ev.type_().as_str())
        {
            on_tap.run(target);
        }
    };
    let wheel = move |ev: web_sys::WheelEvent| {
        let Some(el) = surface.get_untracked() else {
            return;
        };
        ev.prevent_default();
        let scale = if ev.delta_mode() == web_sys::WheelEvent::DOM_DELTA_LINE {
            WHEEL_LINE
        } else {
            1.0
        };
        let (dx, dy) = (ev.delta_x() * scale, ev.delta_y() * scale);
        if ev.ctrl_key() || ev.meta_key() {
            let at = local(&el, ev.client_x(), ev.client_y());
            view.update(|v| *v = v.zoom_at(at, (-dy * 0.01).exp()));
        } else {
            view.update(|v| *v = v.pan(-dx, -dy));
        }
    };

    let grid = move || {
        let v = view.get();
        let step = GRID * v.zoom;
        format!(
            "background-size:{step}px {step}px;background-position:{}px {}px",
            v.x, v.y
        )
    };
    view! {
        <div
            class="flow-canvas"
            node_ref=surface
            style=grid
            on:pointerdown=down
            on:pointermove=moved
            on:pointerup=up
            on:pointercancel=up
            on:wheel=wheel
        >
            <div
                class="flow-world"
                style=move || {
                    let v = view.get();
                    // `--flow-zoom` lets controls drawn in the world (the wire ✕) keep their size.
                    format!("transform:{};--flow-zoom:{}", v.css_transform(), v.zoom)
                }
            >
                {children()}
            </div>
        </div>
    }
}

/// Zoom in, zoom out (about the middle of the screen) and fit view, stacked bottom-left.
#[component]
pub fn CanvasControls(view: RwSignal<View>, content: Signal<Rect>) -> impl IntoView {
    let camera = expect_context::<Camera>();
    let t = use_flow_text();
    let zoom_by = move |factor: f64| {
        if let Some((w, h)) = camera.measured() {
            let centre = Pt {
                x: w / 2.0,
                y: h / 2.0,
            };
            view.update(|v| *v = v.zoom_at(centre, factor));
        }
    };
    let fit_view = move |_| {
        if let Some((w, h)) = camera.measured() {
            view.set(fit_beside(
                content.get_untracked(),
                w,
                h,
                camera.inset.get_untracked(),
            ));
        }
    };
    view! {
        <div class="flow-controls" role="toolbar" aria-label=move || t(FlowText::Zoom)>
            <button
                type="button"
                class="flow-controls__btn"
                aria-label=move || t(FlowText::ZoomIn)
                title=move || t(FlowText::ZoomIn)
                on:click=move |_| zoom_by(ZOOM_STEP)
            >
                "+"
            </button>
            <button
                type="button"
                class="flow-controls__btn"
                aria-label=move || t(FlowText::ZoomOut)
                title=move || t(FlowText::ZoomOut)
                on:click=move |_| zoom_by(1.0 / ZOOM_STEP)
            >
                "−"
            </button>
            <button
                type="button"
                class="flow-controls__btn"
                aria-label=move || t(FlowText::FitView)
                title=move || t(FlowText::FitView)
                on:click=fit_view
            >
                "⤢"
            </button>
        </div>
    }
}

fn role_word(role: Role) -> &'static str {
    match role {
        Role::Trigger => "trigger",
        Role::Check => "check",
        Role::Action => "action",
        Role::Wait => "wait",
    }
}

/// The whole flow in 180×120: every card as a rect in its role colour and the visible part
/// outlined. Clicking or dragging in it centres the view on that point.
#[component]
pub fn Minimap(view: RwSignal<View>, flow: Signal<Flow>) -> impl IntoView {
    let camera = expect_context::<Camera>();
    let t = use_flow_text();
    let placed = Memo::new(move |_| flow.with(positions));
    let content = Memo::new(move |_| flow.with(|f| placed.with(|p| bounds(f, p))));
    let visible = Memo::new(move |_| {
        let (w, h) = camera.size.get();
        (w > 0.0 && h > 0.0).then(|| visible_world(&view.get(), w, h))
    });
    let mini = Memo::new(move |_| {
        let content = content.get();
        minimap(content, visible.get().unwrap_or(content), MINI_W, MINI_H)
    });
    // The transform a drag started with, kept so the map does not shift under the pointer.
    let dragging = StoredValue::new(None::<Mini>);
    let svg: NodeRef<leptos::svg::Svg> = NodeRef::new();

    let centre = move |ev: &web_sys::PointerEvent| {
        let (Some(m), Some(el), Some((w, h))) =
            (dragging.get_value(), svg.get_untracked(), camera.measured())
        else {
            return;
        };
        let world = mini_to_world(&m, local(&el, ev.client_x(), ev.client_y()));
        view.update(|v| *v = centre_on(v, world, w, h));
    };
    let down = move |ev: web_sys::PointerEvent| {
        if let Some(el) = svg.get_untracked() {
            let _ = el.set_pointer_capture(ev.pointer_id());
        }
        dragging.set_value(Some(mini.get_untracked()));
        centre(&ev);
    };
    let moved = move |ev: web_sys::PointerEvent| centre(&ev);
    let up = move |_: web_sys::PointerEvent| dragging.set_value(None);

    let cards = move || {
        let m = mini.get();
        flow.with(|f| {
            placed.with(|p| {
                f.nodes
                    .iter()
                    .filter_map(|n| {
                        let at = p.get(&n.id)?;
                        let height = node_height(&n.kind);
                        let class = format!(
                            "flow-minimap__node flow-minimap__node--{}",
                            role_word(role(&n.kind))
                        );
                        Some(view! {
                            <rect
                                class=class
                                x=(at.x - m.ox) * m.scale
                                y=(at.y - m.oy) * m.scale
                                width=NODE_W * m.scale
                                height=height * m.scale
                                rx="2"
                            />
                        })
                    })
                    .collect_view()
            })
        })
    };
    let outline = move || {
        let m = mini.get();
        visible.get().map(|r| {
            view! {
                <rect
                    class="flow-minimap__view"
                    x=(r.x - m.ox) * m.scale
                    y=(r.y - m.oy) * m.scale
                    width=r.w * m.scale
                    height=r.h * m.scale
                    rx="3"
                />
            }
        })
    };
    view! {
        <svg
            class="flow-minimap"
            node_ref=svg
            width=MINI_W
            height=MINI_H
            viewBox=format!("0 0 {MINI_W} {MINI_H}")
            role="img"
            aria-label=move || t(FlowText::Minimap)
            on:pointerdown=down
            on:pointermove=moved
            on:pointerup=up
            on:pointercancel=up
        >
            {cards}
            {outline}
        </svg>
    }
}
