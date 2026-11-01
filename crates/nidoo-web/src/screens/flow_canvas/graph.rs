//! The flow itself, in world coordinates: one pill per node (role colour, icon, sentence title,
//! sub-line, pins) and the routed wires between them, with "yes"/"no" labels and the classes a
//! run lights, dims or animates. Used in view and edit mode alike; in edit mode cards drag,
//! output pins drag out new wires, wires can be selected and cut, and a card with no wires
//! dropped onto a wire is spliced into it.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use leptos::html;
use leptos::prelude::*;
use nidoo_api::StepOutcome;
use nidoo_core::flow::{Flow, NodeKind, Op, Position};
use wasm_bindgen::JsCast;

use super::manual::ManualRunButton;
use super::sun::fallback_badge_visible;
use super::viewport::Camera;
use super::{Editor, Fresh};
use crate::components::now::Now;
use crate::flow_edit::{EdgeKey, can_connect, can_insert, connect, insert_into_edge, remove_edge};
use crate::flow_layout::{
    NODE_H, Pt, in_anchor, node_height, out_anchor, positions, route, routes_past, wire_under,
};
use crate::flow_text::{Ctx, Role, exit_label, role, subline, title};
use crate::flows::{Highlight, orphans};
use crate::house::House;
use crate::i18n::flows::{FlowText, flow_text, time_left, use_flow_text};
use crate::i18n::{Lang, use_lang};
use crate::icons::{X_MARK, node_icon};

/// Whether the graph is only looked at or edited (cards drag, settings open).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    View,
    Edit,
}

/// What is selected on the canvas: a card by id, or a wire by `(from, port, to)`.
#[derive(Clone, Debug, PartialEq)]
pub enum Selection {
    Node(String),
    Edge(EdgeKey),
}

/// `m:ss` of a number of seconds.
fn minutes_seconds(secs: u64) -> String {
    format!("{}:{:02}", secs / 60, secs % 60)
}

/// `yes` for a wire leaving an `if` by `true`, `no` for `false`, nothing for other wires.
fn tone(key: &EdgeKey) -> Option<&'static str> {
    match key.1.as_deref() {
        Some("true") => Some("yes"),
        Some("false") => Some("no"),
        _ => None,
    }
}

/// The class every card carries.
const NODE_CLASS: &str = "flow-node";

/// The word on a yes / no wire.
fn wire_word(lang: Lang, tone: &str) -> &'static str {
    flow_text(
        lang,
        if tone == "yes" {
            FlowText::WireYes
        } else {
            FlowText::WireNo
        },
    )
}

fn role_class(role: Role) -> &'static str {
    match role {
        Role::Trigger => "flow-node--trigger",
        Role::Check => "flow-node--check",
        Role::Action => "flow-node--action",
        Role::Wait => "flow-node--wait",
    }
}

/// The height, from a card's top, of the pin an output `port` leaves by.
fn port_top(kind: &NodeKind, port: Option<&str>) -> f64 {
    out_anchor(kind, Pt { x: 0.0, y: 0.0 }, port).y
}

/// The label beside an exit pin, worded against the house.
fn exit_text(kind: &NodeKind, port: &str, lang: Lang, house: House) -> String {
    house.entities.with(|entities| {
        house.areas.with(|areas| {
            let ctx = Ctx {
                entities,
                areas,
                flows: &[],
                lang,
            };
            exit_label(lang, kind, port, &ctx)
        })
    })
}

/// Unix seconds, for the countdown of a parked delay.
fn unix_now() -> u64 {
    (js_sys::Date::now() / 1000.0) as u64
}

/// Where the pointer sits in world coordinates, from a client-coord pointer event. `layer` is
/// the `.flow-graph` element, whose top-left is the world origin on screen.
fn pointer_in_canvas(
    ev: &web_sys::PointerEvent,
    layer: &web_sys::Element,
    zoom: f64,
) -> (f64, f64) {
    let rect = layer.get_bounding_client_rect();
    let z = if zoom.abs() < f64::EPSILON { 1.0 } else { zoom };
    (
        (f64::from(ev.client_x()) - rect.left()) / z,
        (f64::from(ev.client_y()) - rect.top()) / z,
    )
}

/// What drag is in progress: the id of the node under the finger, the offset of its top-left
/// from the pointer's canvas-coord position at pick-up, and whether the pointer has actually
/// moved far enough to count as a drag (a pointer-up with `moved = false` is a click).
#[derive(Clone, Debug, PartialEq)]
struct Drag {
    node: String,
    offset_x: f64,
    offset_y: f64,
    moved: bool,
}

/// Pointer travel in canvas coords above which a pointerdown counts as a drag; below, it is a
/// click that selects the node.
const DRAG_THRESHOLD: f64 = 3.0;

/// A wire being dragged out of an output pin: the node and port it leaves from and where the
/// pointer is, in world coordinates.
#[derive(Clone, Debug, PartialEq)]
struct Connecting {
    from: String,
    port: Option<String>,
    to_world: Pt,
}

/// Points the pointer capture of `ev` at the element its listener sits on.
fn capture(ev: &web_sys::PointerEvent) {
    if let Some(target) = ev
        .current_target()
        .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
    {
        let _ = target.set_pointer_capture(ev.pointer_id());
    }
}

/// The signals every card reads.
#[derive(Clone, Copy)]
struct Shared {
    flow: Signal<Flow>,
    placed: Memo<BTreeMap<String, Pt>>,
    lit: Signal<Highlight>,
    dim: Signal<bool>,
    steps: Signal<BTreeMap<String, String>>,
    parked: Signal<Option<(String, u64)>>,
    clock: RwSignal<u64>,
    orphaned: Memo<BTreeSet<String>>,
    names: Memo<Vec<(String, String)>>,
    selection: RwSignal<Option<Selection>>,
    mode: Mode,
    drag: RwSignal<Option<Drag>>,
    /// The draft as it was when the current drag started; recorded once it really moves.
    drag_before: StoredValue<Option<Flow>>,
    connecting: RwSignal<Option<Connecting>>,
    /// The wire the dragged card would be spliced into if dropped now.
    drop_into: Memo<Option<EdgeKey>>,
    /// The `(from, port)` of the wire being dragged out, without the pointer position.
    wiring: Memo<Option<(String, Option<String>)>>,
    editor: Option<Editor>,
    camera: Camera,
    layer: NodeRef<html::Div>,
}

impl Shared {
    /// The pointer of `ev` in world coordinates.
    fn world(self, ev: &web_sys::PointerEvent) -> Option<Pt> {
        let layer = self.layer.get_untracked()?;
        let (x, y) = pointer_in_canvas(ev, layer.as_ref(), self.camera.view.get_untracked().zoom);
        Some(Pt { x, y })
    }
}

/// The flow as pills and wires. `lit` is the run shown (its cards and wires light up), `dim`
/// fades everything it did not visit, `live` animates its wires, `steps` is the time label of
/// each visited card and `parked` the delay a running run waits on, with the unix second the
/// wait ends at (it shows a countdown).
#[component]
pub fn Graph(
    flow: Signal<Flow>,
    mode: Mode,
    lit: Signal<Highlight>,
    dim: Signal<bool>,
    steps: Signal<BTreeMap<String, String>>,
    parked: Signal<Option<(String, u64)>>,
    live: Signal<bool>,
    selection: RwSignal<Option<Selection>>,
) -> impl IntoView {
    let house = expect_context::<House>();
    let now = expect_context::<Now>();
    let lang = use_lang();
    let t = use_flow_text();
    let placed = Memo::new(move |_| flow.with(positions));
    let drag = RwSignal::new(None::<Drag>);
    // The card being dragged: no wire steers around it, so the one under it stays there.
    let loose =
        Memo::new(move |_| drag.with(|d| d.as_ref().filter(|d| d.moved).map(|d| d.node.clone())));
    let wires = Memo::new(move |_| {
        loose.with(|loose| flow.with(|f| placed.with(|p| routes_past(f, p, loose.as_deref()))))
    });
    let drop_into = Memo::new(move |_| {
        let id = loose.get()?;
        let at = placed.with(|p| p.get(&id).copied())?;
        let height = flow.with(|f| f.node(&id).map(|n| node_height(&n.kind)))?;
        let key = wires.with(|w| wire_under(w, at, height))?;
        flow.with(|f| can_insert(f, &key, &id).is_ok())
            .then_some(key)
    });
    let ids =
        Memo::new(move |_| flow.with(|f| f.nodes.iter().map(|n| n.id.clone()).collect::<Vec<_>>()));
    let names = Memo::new(move |_| {
        house.flows.with(|flows| {
            flows
                .iter()
                .map(|f| (f.definition.id.clone(), f.definition.name.clone()))
                .collect::<Vec<_>>()
        })
    });

    // A one-second clock for the countdown, ticking only while a delay is parked.
    let clock = RwSignal::new(now.0.get_untracked());
    let ticker = StoredValue::new(None::<IntervalHandle>);
    let parked_some = Memo::new(move |_| parked.with(Option::is_some));
    Effect::new(move |_| {
        if let Some(handle) = ticker.get_value() {
            handle.clear();
            ticker.set_value(None);
        }
        if parked_some.get() {
            clock.set(unix_now());
            let handle =
                set_interval_with_handle(move || clock.set(unix_now()), Duration::from_secs(1));
            ticker.set_value(handle.ok());
        }
    });
    on_cleanup(move || {
        if let Some(handle) = ticker.try_get_value().flatten() {
            handle.clear();
        }
    });

    let connecting = RwSignal::new(None::<Connecting>);
    let editor = use_context::<Editor>();
    let shared = Shared {
        flow,
        placed,
        lit,
        dim,
        steps,
        parked,
        clock,
        orphaned: Memo::new(move |_| flow.with(orphans)),
        names,
        selection,
        mode,
        drag,
        drag_before: StoredValue::new(None),
        connecting,
        drop_into,
        wiring: Memo::new(move |_| {
            connecting.with(|w| w.as_ref().map(|w| (w.from.clone(), w.port.clone())))
        }),
        editor,
        camera: expect_context::<Camera>(),
        layer: NodeRef::new(),
    };

    let wire_paths = move || {
        let lit = lit.get();
        let (dim, live) = (dim.get(), live.get());
        let selected = selection.get();
        let drop = drop_into.get();
        wires.with(|w| {
            w.iter()
                .map(|(key, route)| {
                    let is_lit = lit.edges.contains(key);
                    let mut class = String::from("flow-wire");
                    if let Some(t) = tone(key) {
                        class.push_str(&format!(" flow-wire--{t}"));
                    }
                    if is_lit {
                        class.push_str(" flow-wire--lit");
                        if live {
                            class.push_str(" flow-wire--live");
                        }
                    } else if dim {
                        class.push_str(" flow-wire--dim");
                    }
                    if matches!(&selected, Some(Selection::Edge(k)) if k == key) {
                        class.push_str(" flow-wire--selected");
                    }
                    if drop.as_ref() == Some(key) {
                        class.push_str(" flow-wire--drop");
                    }
                    view! { <path class=class d=route.d.clone() /> }
                })
                .collect_view()
        })
    };
    // Edit mode: a wide invisible stroke per wire takes the click that selects it.
    let hits = move || {
        (mode == Mode::Edit).then(|| {
            wires.with(|w| {
                w.iter()
                    .map(|(key, route)| {
                        let key = key.clone();
                        view! {
                            <path
                                class="flow-wire-hit"
                                d=route.d.clone()
                                on:pointerdown=|ev: web_sys::PointerEvent| ev.stop_propagation()
                                on:click=move |_| selection.set(Some(Selection::Edge(key.clone())))
                            />
                        }
                    })
                    .collect_view()
            })
        })
    };
    // The wire being dragged out, from its pin to the pointer.
    let preview = move || {
        let wire = connecting.get()?;
        let from = flow.with(|f| {
            let node = f.node(&wire.from)?;
            let at = placed.with(|p| p.get(&wire.from).copied())?;
            Some(out_anchor(&node.kind, at, wire.port.as_deref()))
        })?;
        let d = route(from, wire.to_world, &[]).d;
        Some(view! { <path class="flow-wire-preview" d=d /> })
    };
    // The ✕ halfway along the selected wire, which cuts it.
    let cut = move || {
        let editor = editor.filter(|_| mode == Mode::Edit)?;
        let Some(Selection::Edge(key)) = selection.get() else {
            return None;
        };
        let mid = wires.with(|w| w.iter().find(|(k, _)| *k == key).map(|(_, r)| r.mid))?;
        Some(view! {
            <button
                type="button"
                class="flow-wire-cut"
                aria-label=move || t(FlowText::DeleteWire)
                title=move || t(FlowText::DeleteWire)
                style=format!("left:{}px;top:{}px", mid.x, mid.y)
                on:pointerdown=|ev: web_sys::PointerEvent| ev.stop_propagation()
                on:click=move |_| {
                    editor.edit(|f| remove_edge(f, &key));
                    selection.set(None);
                }
                inner_html=X_MARK
            ></button>
        })
    };
    let labels = move || {
        let lit = lit.get();
        let dim = dim.get();
        wires.with(|w| {
            w.iter()
                .filter_map(|(key, route)| {
                    let t = tone(key)?;
                    let faded = dim && !lit.edges.contains(key);
                    Some(view! {
                        <span
                            class=format!("flow-wire-label flow-wire-label--{t}")
                            class:flow-wire-label--dim=faded
                            style=format!("left:{}px;top:{}px", route.label_at.x, route.label_at.y)
                        >
                            {move || wire_word(lang(), t)}
                        </span>
                    })
                })
                .collect_view()
        })
    };

    view! {
        <div
            class="flow-graph"
            class:flow-graph--editing=mode == Mode::Edit
            class:flow-graph--connecting=move || connecting.with(Option::is_some)
            node_ref=shared.layer
        >
            <svg class="flow-wires" width="1" height="1" aria-hidden="true">
                {wire_paths}
                {hits}
                {preview}
            </svg>
            {labels}
            {cut}
            <For
                each=move || ids.get()
                key=|id| id.clone()
                children=move |id| card(id, shared, house)
            />
        </div>
    }
}

/// One pill: reads its node, position and run state from `c` by id, so a card survives (and
/// keeps its pointer capture) while the flow around it changes.
fn card(id: String, c: Shared, house: House) -> impl IntoView {
    let lang = crate::i18n::use_lang();
    let node_id = id.clone();
    let kind = Memo::new(move |_| c.flow.with(|f| f.node(&node_id).map(|n| n.kind.clone())));
    let node_id = id.clone();
    let texts = Memo::new(move |_| {
        let names = c.names.get();
        c.flow.with(|f| {
            let node = f.node(&node_id)?;
            house.entities.with(|entities| {
                house.areas.with(|areas| {
                    let ctx = Ctx {
                        entities,
                        areas,
                        flows: &names,
                        lang: lang(),
                    };
                    Some((title(node, &ctx), subline(node, &ctx)))
                })
            })
        })
    });
    let title_text = move || texts.with(|t| t.as_ref().map(|t| t.0.clone()));
    let sub_text = move || texts.with(|t| t.as_ref().and_then(|t| t.1.clone()));

    let fresh = use_context::<Fresh>();
    let fresh_id = id.clone();
    let class = move || {
        let is_fresh = fresh.is_some_and(|f| f.0.with(|f| f.as_deref() == Some(fresh_id.as_str())));
        kind.with(|k| {
            k.as_ref().map_or_else(String::new, |k| {
                let mut class = format!("{NODE_CLASS} {}", role_class(role(k)));
                if matches!(k, NodeKind::Switch(_) | NodeKind::TimeBetween(_)) {
                    class.push_str(" flow-node--exits");
                }
                // Cards with room to spare wrap their title to two lines before cutting it.
                if matches!(k, NodeKind::TimeBetween(_)) || node_height(k) >= 74.0 {
                    class.push_str(" flow-node--wrap");
                }
                if c.mode == Mode::Edit {
                    class.push_str(" flow-node--editing");
                }
                if is_fresh {
                    class.push_str(" flow-node--fresh");
                }
                class
            })
        })
    };
    let icon = move || kind.with(|k| k.as_ref().map(node_icon).unwrap_or_default());
    // Sun cards (and `if` on darkness) read the sun against Paris when the hub has no coordinates.
    let fallback = Memo::new(move |_| {
        let reads_sun = kind.with(|k| match k {
            Some(NodeKind::TriggerSun(_)) => true,
            Some(NodeKind::If(p)) => p.op == Op::IsDark,
            _ => false,
        });
        reads_sun
            && house
                .flows_config
                .with(|c| c.as_ref().is_some_and(fallback_badge_visible))
    });

    let node_id = id.clone();
    let style = move || {
        c.placed.with(|p| {
            p.get(&node_id)
                .map(|at| {
                    let height = kind.with(|k| k.as_ref().map(node_height));
                    match height.filter(|h| *h != NODE_H) {
                        Some(h) => format!("left:{}px;top:{}px;height:{h}px", at.x, at.y),
                        None => format!("left:{}px;top:{}px", at.x, at.y),
                    }
                })
                .unwrap_or_default()
        })
    };

    let pins_id = id.clone();
    let pins = move || {
        kind.get().map(|k| {
            let input = (role(&k) != Role::Trigger).then(|| {
                let top = in_anchor(Pt { x: 0.0, y: 0.0 }, node_height(&k)).y;
                view! { <span class="flow-node__in" style=format!("top:{top}px")></span> }
            });
            // An `if`, a switch and a time card have named exits; every other card one plain pin.
            let named = k.ports();
            let ports: Vec<Option<String>> = if named.is_empty() {
                vec![None]
            } else {
                named.into_iter().map(Some).collect()
            };
            let outputs = ports
                .into_iter()
                .map(|port| {
                    let top = port_top(&k, port.as_deref());
                    let label = port
                        .as_deref()
                        .map(|p| exit_text(&k, p, lang(), house))
                        .unwrap_or_default();
                    out_pin(pins_id.clone(), port, top, label, c)
                })
                .collect_view();
            view! {
                {input}
                {outputs}
            }
        })
    };

    // A manual card carries the button that starts the flow from it; saved flows only.
    let t = use_flow_text();
    let manual_id = id.clone();
    let manual_run = move || {
        kind.with(|k| matches!(k, Some(NodeKind::TriggerManual(_))))
            .then(|| {
                let edit = c.mode == Mode::Edit;
                let disabled = Signal::derive(move || edit || !c.flow.with(|f| f.enabled));
                let label = Signal::derive(move || {
                    t(if edit {
                        FlowText::ManualSaveFirst
                    } else {
                        FlowText::ManualPress
                    })
                });
                let flow_id = Signal::derive(move || c.flow.with(|f| f.id.clone()));
                view! {
                    <ManualRunButton
                        flow_id=flow_id
                        node_id=manual_id.clone()
                        disabled=disabled
                        title=label
                    />
                }
            })
    };

    let outcome_id = id.clone();
    let outcome = Memo::new(move |_| c.lit.with(|l| l.nodes.get(&outcome_id).copied()));
    let is_lit = move || outcome.get().is_some_and(|o| o != StepOutcome::Failed);
    let failed = move || outcome.get() == Some(StepOutcome::Failed);
    let dim = move || c.dim.get() && outcome.get().is_none();
    let pending_id = id.clone();
    let pending = move || {
        c.parked
            .with(|p| p.as_ref().is_some_and(|(n, _)| *n == pending_id))
    };
    let orphan_id = id.clone();
    let orphan = move || c.orphaned.with(|o| o.contains(&orphan_id));
    let selected_id = id.clone();
    let selected = move || {
        c.selection
            .with(|s| matches!(s, Some(Selection::Node(n)) if *n == selected_id))
    };
    let target_id = id.clone();
    let target = move || {
        let splicing = c.drop_into.with(Option::is_some)
            && c.drag
                .with(|d| d.as_ref().is_some_and(|d| d.node == target_id));
        splicing
            || c.wiring.with(|w| {
                w.as_ref().is_some_and(|(from, port)| {
                    c.flow
                        .with(|f| can_connect(f, from, port.as_deref(), &target_id).is_ok())
                })
            })
    };
    let step_id = id.clone();
    let step = move || match c.parked.get() {
        Some((node, deadline)) if node == step_id => time_left(
            lang(),
            &minutes_seconds(deadline.saturating_sub(c.clock.get())),
        ),
        _ => c
            .steps
            .with(|s| s.get(&step_id).cloned().unwrap_or_default()),
    };

    // Edit mode: a press selects the card, a drag moves it (one undo step per drag).
    let drag = c.drag;
    let draft = c.editor.map(|e| e.draft);
    let id_for_down = id.clone();
    let down = move |ev: web_sys::PointerEvent| {
        if c.mode != Mode::Edit {
            return;
        }
        if ev.button() != 0 && ev.pointer_type() == "mouse" {
            return;
        }
        let (Some(at), Some(draft)) = (c.world(&ev), draft) else {
            return;
        };
        let (nx, ny) = draft.with_untracked(|f| {
            f.node(&id_for_down)
                .and_then(|n| n.position)
                .map_or((0.0, 0.0), |p| (p.x, p.y))
        });
        capture(&ev);
        ev.prevent_default();
        // The surface below would pan otherwise.
        ev.stop_propagation();
        c.drag_before.set_value(Some(draft.get_untracked()));
        drag.set(Some(Drag {
            node: id_for_down.clone(),
            offset_x: at.x - nx,
            offset_y: at.y - ny,
            moved: false,
        }));
    };
    let id_for_move = id.clone();
    let moved = move |ev: web_sys::PointerEvent| {
        let Some(d) = drag.get_untracked() else {
            return;
        };
        if d.node != id_for_move {
            return;
        }
        let (Some(at), Some(editor)) = (c.world(&ev), c.editor) else {
            return;
        };
        ev.stop_propagation();
        // The canvas is infinite: a card may go anywhere, negative coordinates included.
        let new_x = at.x - d.offset_x;
        let new_y = at.y - d.offset_y;
        if !d.moved {
            let (nx, ny) = editor.draft.with_untracked(|f| {
                f.node(&id_for_move)
                    .and_then(|n| n.position)
                    .map_or((new_x, new_y), |p| (p.x, p.y))
            });
            if (new_x - nx).hypot(new_y - ny) < DRAG_THRESHOLD {
                return;
            }
            // The press is a drag after all: the state before it is one undo step.
            if let Some(before) = c.drag_before.try_update_value(Option::take).flatten() {
                editor.history.update_value(|h| h.record(before));
            }
            drag.update(|cur| {
                if let Some(c) = cur {
                    c.moved = true;
                }
            });
        }
        let id = id_for_move.clone();
        editor.draft.update(|f| {
            if let Some(n) = f.nodes.iter_mut().find(|n| n.id == id) {
                n.position = Some(Position { x: new_x, y: new_y });
            }
        });
    };
    let id_for_up = id.clone();
    let up = std::rc::Rc::new(move |_ev: web_sys::PointerEvent| {
        let was_click = drag.with_untracked(
            |d| matches!(d, Some(Drag { node, moved: false, .. }) if node == &id_for_up),
        );
        // Dropped onto a wire: spliced into it, within the drag's undo step.
        let mine = drag.with_untracked(|d| d.as_ref().is_some_and(|d| d.node == id_for_up));
        if let (true, Some(key), Some(editor)) = (mine, c.drop_into.get_untracked(), c.editor) {
            editor.draft.update(|f| {
                let _ = insert_into_edge(f, &key, &id_for_up);
            });
        }
        drag.set(None);
        c.drag_before.set_value(None);
        if was_click {
            c.selection.set(Some(Selection::Node(id_for_up.clone())));
        }
    });
    let up_on_up = {
        let up = up.clone();
        move |ev: web_sys::PointerEvent| up(ev)
    };
    let up_on_cancel = move |ev: web_sys::PointerEvent| up(ev);
    let id_for_class = id.clone();
    let is_dragging = move || {
        drag.with(|d| matches!(d, Some(Drag { node, moved: true, .. }) if node == &id_for_class))
    };

    view! {
        <div
            class=class
            class:flow-node--lit=is_lit
            class:flow-node--failed=failed
            class:flow-node--dim=dim
            class:flow-node--pending=pending
            class:flow-node--orphan=orphan
            class:flow-node--selected=selected
            class:flow-node--dragging=is_dragging
            class:flow-node--target=target
            data-node=id.clone()
            style=style
            title=title_text
            on:pointerdown=down
            on:pointermove=moved
            on:pointerup=up_on_up
            on:pointercancel=up_on_cancel
        >
            <span class="flow-node__icon" inner_html=icon></span>
            <span class="flow-node__text">
                <span class="flow-node__title">{title_text}</span>
                {move || sub_text().map(|s| view! { <span class="flow-node__sub">{s}</span> })}
            </span>
            {manual_run}
            <span class="flow-node__step">{step}</span>
            <Show when=move || fallback.get()>
                <span class="flow-node__fallback">
                    {move || flow_text(lang(), FlowText::ParisFallback)}
                </span>
            </Show>
            {pins}
        </div>
    }
}

/// An output pin. In edit mode a drag from it draws a wire that follows the pointer, and the
/// drop connects to the card under the pointer when the editor allows it (else nothing).
fn out_pin(id: String, port: Option<String>, top: f64, label: String, c: Shared) -> impl IntoView {
    let data_port = port.clone();
    let down = move |ev: web_sys::PointerEvent| {
        if c.mode != Mode::Edit || (ev.button() != 0 && ev.pointer_type() == "mouse") {
            return;
        }
        let Some(to_world) = c.world(&ev) else {
            return;
        };
        // Neither the card (drag) nor the surface (pan) takes this press.
        ev.stop_propagation();
        ev.prevent_default();
        capture(&ev);
        c.connecting.set(Some(Connecting {
            from: id.clone(),
            port: port.clone(),
            to_world,
        }));
    };
    let moved = move |ev: web_sys::PointerEvent| {
        if c.connecting.with_untracked(Option::is_none) {
            return;
        }
        ev.stop_propagation();
        if let Some(at) = c.world(&ev) {
            c.connecting.update(|w| {
                if let Some(w) = w {
                    w.to_world = at;
                }
            });
        }
    };
    let up = move |ev: web_sys::PointerEvent| {
        let Some(wire) = c.connecting.try_update(Option::take).flatten() else {
            return;
        };
        ev.stop_propagation();
        let to = document()
            .element_from_point(ev.client_x() as f32, ev.client_y() as f32)
            .and_then(|el| el.closest("[data-node]").ok().flatten())
            .and_then(|card| card.get_attribute("data-node"));
        if let (Some(to), Some(editor)) = (to, c.editor)
            && editor
                .draft
                .with_untracked(|f| can_connect(f, &wire.from, wire.port.as_deref(), &to).is_ok())
        {
            let _ = editor.edit(|f| connect(f, &wire.from, wire.port.as_deref(), &to));
        }
    };
    let cancel = move |_: web_sys::PointerEvent| c.connecting.set(None);
    view! {
        <span
            class="flow-node__out"
            data-port=data_port
            style=format!("top:{top}px")
            on:pointerdown=down
            on:pointermove=moved
            on:pointerup=up
            on:pointercancel=cancel
        >
            {(!label.is_empty()).then(|| view! { <span class="flow-node__exit">{label}</span> })}
        </span>
    }
}
