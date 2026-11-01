//! The Classic view of the Energy screen, drawn as glowing circles: gradient discs with halos,
//! curved gradient wires between each node and the house, a mix ring on the house and dots
//! travelling along the active wires.
//!
//! Geometry and values come from `energy_circles` (pure); the dots from `energy_dots`, whose
//! `requestAnimationFrame` loop reads a non-reactive store, so a new reading never restarts them.
//! Every node and wire is a keyed child that reads its own state, so an update changes the
//! existing elements in place.

use std::collections::BTreeMap;

use leptos::prelude::*;
use nidoo_core::energy::{EnergyNode, EnergyUi, IDLE_W, NodeKind, NodeStatus};

use crate::energy::{aria_sentence, circle_texts, node_label, node_value_text, power_text};
use crate::energy_circles::{
    Mix, Placed, Wire, consumer_slot, for_width, halo, house_mix, place, ring_dashes, sanitise,
    tone, wires,
};
use crate::energy_sankey::{FOLD_WIDE, build};
use crate::i18n::Lang;
use crate::i18n::energy::solar_share_line;
use crate::i18n::use_lang;
use crate::screens::energy::{glyph, kind_class, short};

/// Every tone a node or a wire end can take; each gets its disc and halo gradients.
const TONES: [&str; 10] = [
    "solar", "grid", "export", "battery", "house", "water", "more", "c0", "c1", "c2",
];
/// The mix ring's stroke width and the gap between its arcs, in px.
const RING_W: f64 = 7.0;
const RING_GAP: f64 = 4.0;
/// A node's icon, in px.
const GLYPH: f64 = 20.0;
/// The mix ring's arcs, in `ring_dashes` order.
const RING_PARTS: [&str; 3] = ["solar", "battery", "grid"];

/// Whether a node is drawn dimmed: unavailable, unknown, or (but water) under `IDLE_W`.
fn dimmed(n: &EnergyNode) -> bool {
    n.status == NodeStatus::Unavailable
        || match n.value {
            None => true,
            Some(_) if n.kind == NodeKind::Water => false,
            Some(w) => w < IDLE_W,
        }
}

/// What a small circle shows: "More" carries the power it folds, others their reading.
fn small_value(lang: Lang, n: &EnergyNode) -> String {
    if n.kind == NodeKind::More {
        power_text(lang, n.value)
    } else {
        node_value_text(lang, n)
    }
}

/// `More` toggles the list; a node with exactly one sensor opens it.
fn clickable(n: &EnergyNode) -> bool {
    n.kind == NodeKind::More || n.sensor_ids.len() == 1
}

/// The house's solar share in whole percent, when solar feeds part of it.
fn share(mix: Option<Mix>) -> Option<u8> {
    // The mix holds fractions: 0.29 * 100 is 28.999…, so a hair of slack before the floor.
    let pct = (mix?.solar * 100.0 + 1e-9).floor();
    (pct > 0.0).then_some(pct.min(100.0) as u8)
}

#[component]
fn ToneGradients() -> impl IntoView {
    TONES
        .iter()
        .map(|t| {
            view! {
                <radialGradient id=format!("energy-disc-{t}") cx="50%" cy="35%" r="75%">
                    <stop
                        offset="0"
                        style=format!(
                            "stop-color: color-mix(in srgb, var(--tone-{t}) 34%, var(--bg))",
                        )
                    />
                    <stop
                        offset="1"
                        style=format!(
                            "stop-color: color-mix(in srgb, var(--tone-{t}) 8%, var(--bg))",
                        )
                    />
                </radialGradient>
                <radialGradient id=format!("energy-halo-{t}")>
                    <stop offset="0.5" style=format!("stop-color: var(--tone-{t})") />
                    <stop
                        offset="1"
                        style=format!("stop-color: var(--tone-{t}); stop-opacity: 0")
                    />
                </radialGradient>
            }
        })
        .collect_view()
}

/// A wire's stroke: from its source colour to its destination's, along its own chord.
#[component]
fn WireGradient(id: String, wires: Memo<Vec<Wire>>) -> impl IntoView {
    let wire_id = id.clone();
    let wire = Memo::new(move |_| wires.with(|ws| ws.iter().find(|w| w.id == wire_id).cloned()));
    let end = move |pick: fn(&Wire) -> f64| move || wire.with(|w| w.as_ref().map(pick));
    let stop = move |pick: fn(&Wire) -> &'static str| {
        move || {
            wire.with(|w| {
                format!(
                    "stop-color: var(--tone-{})",
                    w.as_ref().map_or("house", pick)
                )
            })
        }
    };
    view! {
        <linearGradient
            id=format!("energy-wire-{}", sanitise(&id))
            gradientUnits="userSpaceOnUse"
            x1=end(|w| w.curve.from.0)
            y1=end(|w| w.curve.from.1)
            x2=end(|w| w.curve.to.0)
            y2=end(|w| w.curve.to.1)
        >
            <stop offset="0" style=stop(|w| w.from_tone) />
            <stop offset="1" style=stop(|w| w.to_tone) />
        </linearGradient>
    }
}

#[component]
fn CircleWire(id: String, wires: Memo<Vec<Wire>>) -> impl IntoView {
    let wire_id = id.clone();
    let wire = Memo::new(move |_| wires.with(|ws| ws.iter().find(|w| w.id == wire_id).cloned()));
    let Some(first) = wire.get_untracked() else {
        return ().into_any();
    };
    let (kind, first_tone) = (first.kind, first.tone);
    // The grid turns from import to export in place: its tone follows.
    let class = move || {
        wire.with(|w| {
            let (tone, active) = w
                .as_ref()
                .map_or((first_tone, false), |w| (w.tone, w.active));
            format!(
                "energy-edge energy-edge--{} energy-tone--{tone}{}",
                kind_class(kind),
                if active { "" } else { " energy-edge--idle" }
            )
        })
    };
    view! {
        <path
            class=class
            d=move || wire.with(|w| w.as_ref().map(|w| w.curve.path()).unwrap_or_default())
            stroke=format!("url(#energy-wire-{})", sanitise(&id))
            data-from=move || wire.with(|w| w.as_ref().map(|w| w.from.clone()))
            data-to=move || wire.with(|w| w.as_ref().map(|w| w.to.clone()))
            style=move || {
                wire.with(|w| {
                    w.as_ref().map(|w| format!("stroke-width:{}px", w.width)).unwrap_or_default()
                })
            }
        />
    }
    .into_any()
}

/// The house's mix ring: a track and one arc per source, rotated to start at the top.
#[component]
fn MixRing(r: f64, mix: Memo<Option<Mix>>) -> impl IntoView {
    let circ = std::f64::consts::TAU * r;
    RING_PARTS
        .iter()
        .enumerate()
        .map(|(i, part)| {
            let style = move || {
                let (len, offset) = mix
                    .get()
                    .map_or((0.0, 0.0), |m| ring_dashes(&m, r, RING_GAP)[i]);
                format!(
                    "stroke: var(--tone-{part}); stroke-dasharray: {len} {circ}; \
                     stroke-dashoffset: {offset}; opacity: {}",
                    if len > 0.0 { 1 } else { 0 }
                )
            };
            view! {
                <circle
                    class=format!("energy-node__arc energy-node__arc--{part}")
                    r=r
                    fill="none"
                    stroke-width=RING_W
                    style=style
                />
            }
        })
        .collect_view()
}

#[component]
fn CircleNode(
    id: String,
    narrow: bool,
    ui: Memo<EnergyUi>,
    placed: Memo<BTreeMap<String, Placed>>,
    mix: Memo<Option<Mix>>,
    open: Callback<String>,
    expanded: RwSignal<bool>,
) -> impl IntoView {
    let lang = use_lang();
    let node_id = id.clone();
    let node = Memo::new(move |_| ui.with(|u| u.diagram.iter().find(|n| n.id == node_id).cloned()));
    let spot_id = id.clone();
    let spot = Memo::new(move |_| placed.with(|p| p.get(&spot_id).copied()));
    let slot_id = id;
    let slot = Memo::new(move |_| ui.with(|u| consumer_slot(&u.diagram, &slot_id)));
    let (Some(first), Some(at)) = (node.get_untracked(), spot.get_untracked()) else {
        return ().into_any();
    };
    let kind = first.kind;
    // A node's radius follows its kind, which never changes under its id.
    let r = at.r;
    let first_tone = tone(&first, None);

    let tone_now = move || node.with(|n| n.as_ref().map_or(first_tone, |n| tone(n, slot.get())));
    let can_click = move || node.with(|n| n.as_ref().is_some_and(clickable));
    let class = move || {
        let (off, click) = node.with(|n| {
            n.as_ref()
                .map_or((true, false), |n| (dimmed(n), clickable(n)))
        });
        format!(
            "energy-node energy-node--{} energy-tone--{}{}{}",
            kind_class(kind),
            tone_now(),
            if off { " energy-node--off" } else { "" },
            if click { " energy-node--click" } else { "" }
        )
    };
    let activate = move || {
        let Some(n) = node.get_untracked() else {
            return;
        };
        if n.kind == NodeKind::More {
            expanded.update(|e| *e = !*e);
        } else if let [only] = n.sensor_ids.as_slice() {
            open.run(only.clone());
        }
    };
    let spoken = move || {
        node.with(|n| {
            n.as_ref()
                .filter(|n| clickable(n))
                .map(|n| format!("{} {}", node_label(lang(), n), small_value(lang(), n)))
        })
    };
    let glow = move || {
        node.with(|n| match n {
            Some(n) if n.kind != NodeKind::Water => halo(n.value, n.status == NodeStatus::Live),
            _ => (0.0, 0.0),
        })
    };
    let label = move || node.with(|n| n.as_ref().map(|n| short(node_label(lang(), n))));
    let glyph_html = move || node.with(|n| n.as_ref().map(glyph).unwrap_or_default());

    let big = matches!(kind, NodeKind::Solar | NodeKind::Grid | NodeKind::Battery);
    let house = kind == NodeKind::House;
    // The mockups' type grid: wide, then phone.
    let (icon, icon_top, value_y, unit_y, name_y) = if narrow {
        (20.0, -r + 8.0, 8.0, 22.0, r + 25.0)
    } else {
        (22.0, -r + 9.0, 10.0, 26.0, r + 30.0)
    };
    let (shared_value_y, shared_unit_y, share_y) = if narrow {
        (6.0, 19.0, 32.0)
    } else {
        (8.0, 22.0, 36.0)
    };
    let glyph_at = if big || house {
        (-icon / 2.0, icon_top)
    } else {
        (-icon / 2.0, -icon / 2.0)
    };

    let texts = if big || house {
        let lines = move || {
            node.with(|n| {
                n.as_ref()
                    .map(|n| circle_texts(lang(), n))
                    .unwrap_or_default()
            })
        };
        let pct = move || if house { share(mix.get()) } else { None };
        let y_value = move || {
            if pct().is_some() {
                shared_value_y
            } else {
                value_y
            }
        };
        let y_unit = move || {
            if pct().is_some() {
                shared_unit_y
            } else {
                unit_y
            }
        };
        view! {
            <text class="energy-node__big" y=y_value text-anchor="middle">
                {move || lines().0}
            </text>
            <text class="energy-node__small" y=y_unit text-anchor="middle">
                {move || lines().1}
            </text>
            {move || {
                pct()
                    .map(|p| {
                        view! {
                            <text class="energy-node__share" y=share_y text-anchor="middle">
                                {solar_share_line(lang(), p)}
                            </text>
                        }
                    })
            }}
            <text class="energy-node__label" y=name_y text-anchor="middle">
                {label}
            </text>
        }
        .into_any()
    } else {
        // Wide: right of the circle; phone: under it.
        let (x, anchor, value_at, name_at) = if narrow {
            (0.0, "middle", r + 21.0, r + 35.0)
        } else {
            (r + 20.0, "start", -1.0, 16.0)
        };
        view! {
            <text class="energy-node__value" x=x y=value_at text-anchor=anchor>
                {move || node.with(|n| n.as_ref().map(|n| small_value(lang(), n)))}
            </text>
            <text class="energy-node__label" x=x y=name_at text-anchor=anchor>
                {label}
            </text>
        }
        .into_any()
    };

    view! {
        <g
            class=class
            style=move || {
                // A CSS transform, so the node glides to its new place.
                spot.get()
                    .map(|p| format!("transform: translate({}px, {}px)", p.at.0, p.at.1))
                    .unwrap_or_default()
            }
            tabindex=move || can_click().then_some("0")
            role=move || can_click().then_some("button")
            aria-label=spoken
            on:click=move |_| activate()
            on:keydown=move |ev| {
                if ev.key() == "Enter" || ev.key() == " " {
                    // Space would also scroll the page.
                    ev.prevent_default();
                    activate();
                }
            }
        >
            <circle
                class="energy-node__halo"
                r=move || r + glow().0
                fill=move || format!("url(#energy-halo-{})", tone_now())
                style=move || format!("opacity: calc({} * var(--energy-glow))", glow().1)
            />
            <circle
                class="energy-node__disc"
                r=r
                fill=move || format!("url(#energy-disc-{})", tone_now())
            />
            {(!house)
                .then(|| {
                    view! {
                        <circle
                            class="energy-node__rim"
                            r=r
                            fill="none"
                            stroke-dasharray=(kind == NodeKind::Water).then_some("3 3")
                        />
                    }
                })}
            {house
                .then(|| {
                    view! {
                        <g class="energy-node__mix" transform="rotate(-90)">
                            <circle
                                class="energy-node__track"
                                r=r
                                fill="none"
                                stroke-width=RING_W
                            />
                            <MixRing r=r mix=mix />
                        </g>
                    }
                })}
            // A nested viewport, not a foreignObject: Safari misplaces those under a CSS transform.
            <svg
                class="energy-node__glyph"
                x=glyph_at.0
                y=glyph_at.1
                width=GLYPH
                height=GLYPH
                inner_html=glyph_html
            ></svg>
            {texts}
        </g>
    }
    .into_any()
}

/// Feeds the dot store from the wires and runs the dot loop once per mount, on the dots layer;
/// nothing at all when the system asks for reduced motion.
#[cfg(target_arch = "wasm32")]
fn animate_dots(layer: NodeRef<leptos::svg::G>, wires: Memo<Vec<Wire>>) {
    use std::cell::Cell;
    use std::rc::Rc;

    use crate::energy_dots::{DotStore, reduced_motion, run, sync};

    if reduced_motion() {
        return;
    }
    let store: DotStore = Rc::default();
    let synced = store.clone();
    Effect::new(move |_| wires.with(|ws| sync(&synced, ws)));
    let alive = StoredValue::new_local(None::<Rc<Cell<bool>>>);
    Effect::new(move |_| {
        let Some(el) = layer.get() else {
            return;
        };
        if alive.with_value(Option::is_none) {
            alive.set_value(Some(run(el.into(), store.clone())));
        }
    });
    on_cleanup(move || {
        if let Some(Some(flag)) = alive.try_get_value() {
            flag.set(false);
        }
    });
}

#[cfg(not(target_arch = "wasm32"))]
fn animate_dots(_layer: NodeRef<leptos::svg::G>, _wires: Memo<Vec<Wire>>) {}

#[component]
pub fn CirclesDiagram(
    ui: Memo<EnergyUi>,
    narrow: bool,
    open: Callback<String>,
    expanded: RwSignal<bool>,
) -> impl IntoView {
    let lang = use_lang();
    let layout = for_width(narrow);
    let placed = Memo::new(move |_| ui.with(|u| place(&u.diagram, layout)));
    let wire_list = Memo::new(move |_| ui.with(|u| placed.with(|p| wires(&u.diagram, p, layout))));
    // Only the mix ring reads the Sankey allocation; the house links never fold.
    let mix = Memo::new(move |_| ui.with(|u| house_mix(&build(u, FOLD_WIDE))));
    let description = move || ui.with(|u| aria_sentence(lang(), u));
    let wire_ids = move || wire_list.with(|ws| ws.iter().map(|w| w.id.clone()).collect::<Vec<_>>());
    let node_ids = move || ui.with(|u| u.diagram.iter().map(|n| n.id.clone()).collect::<Vec<_>>());
    let dots = NodeRef::<leptos::svg::G>::new();
    animate_dots(dots, wire_list);

    view! {
        <div class="energy-diagram energy-circles" class:energy-circles--narrow=narrow>
            <svg
                class="energy-diagram__svg"
                viewBox=format!("0 0 {} {}", layout.view_w, layout.view_h)
                role="group"
                aria-label=description
            >
                <title>{description}</title>
                <defs>
                    <ToneGradients />
                    <For
                        each=wire_ids
                        key=|id| id.clone()
                        children=move |id| view! { <WireGradient id=id wires=wire_list /> }
                    />
                </defs>
                <g class="energy-circles__wires">
                    <For
                        each=wire_ids
                        key=|id| id.clone()
                        children=move |id| view! { <CircleWire id=id wires=wire_list /> }
                    />
                </g>
                <g class="energy-circles__dots" node_ref=dots></g>
                <g class="energy-circles__nodes">
                    <For
                        each=node_ids
                        key=|id| id.clone()
                        children=move |id| {
                            view! {
                                <CircleNode
                                    id=id
                                    narrow=narrow
                                    ui=ui
                                    placed=placed
                                    mix=mix
                                    open=open
                                    expanded=expanded
                                />
                            }
                        }
                    />
                </g>
            </svg>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn share_is_whole_percent_and_only_when_solar_feeds() {
        let mix = |solar: f64| Mix {
            solar,
            battery: 0.0,
            grid: 1.0 - solar,
        };
        assert_eq!(share(Some(mix(0.756))), Some(75));
        assert_eq!(share(Some(mix(0.004))), None);
        assert_eq!(share(Some(mix(1.0))), Some(100));
        assert_eq!(share(None), None);
    }

    #[test]
    fn share_does_not_lose_a_percent_to_rounding() {
        // 290 W of 1000 W is 0.29, and 0.29 * 100 is 28.999…
        let solar = 290.0 / 1000.0;
        let mix = Mix {
            solar,
            battery: 0.0,
            grid: 1.0 - solar,
        };
        assert_eq!(share(Some(mix)), Some(29));
    }
}
