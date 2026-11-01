//! The Orbite view of the Energy screen: two concentric rings round the house. The inner ring is
//! where the power comes from, the outer ring where it goes; `energy_orbite` lays the arcs out.
//!
//! Every arc and row is a keyed child that reads its own data, so a new reading moves the existing
//! elements and CSS glides the arcs.

use leptos::prelude::*;
use nidoo_core::energy::EnergyUi;

use crate::energy::{aria_sentence, power_text};
use crate::energy_orbite::{Arc, Ring, Rings, colour_var, rings};
use crate::energy_sankey::{Sankey, SankeyRole, build};
use crate::i18n::energy::{EnergyText, energy_text as word, solar_share_line};
use crate::i18n::{Lang, use_lang};
use crate::icons::HOUSE;
use crate::screens::energy_flux::{fold_at, node_name};

/// The svg's square side, and the ring centre.
const SIZE: f64 = 340.0;
const CENTRE: f64 = SIZE / 2.0;
const INNER_R: f64 = 104.0;
const INNER_W: f64 = 14.0;
const OUTER_R: f64 = 136.0;
const OUTER_W: f64 = 22.0;

fn geometry(ring: Ring) -> (f64, f64) {
    match ring {
        Ring::Inner => (INNER_R, INNER_W),
        Ring::Outer => (OUTER_R, OUTER_W),
    }
}

/// What the dot loop needs of each arc: the arc, and the radius and stroke width it is drawn with.
#[cfg(target_arch = "wasm32")]
type DotArcs = Vec<(Arc, f64, f64)>;

/// Dots are this share of their arc's stroke width in radius.
#[cfg(target_arch = "wasm32")]
const DOT_R: f64 = 0.17;

#[cfg(target_arch = "wasm32")]
fn dot_arcs(r: &Rings) -> DotArcs {
    r.inner
        .iter()
        .chain(&r.outer)
        .map(|a| {
            let (radius, width) = geometry(a.ring);
            (a.clone(), radius, width)
        })
        .collect()
}

/// Starts the dot loop on `layer`: one `requestAnimationFrame` chain that lays each arc's dots out
/// with `arc_dots` and moves the layer's circles to match. Dots stop (and the loop ends) once the
/// returned flag is set to `false`.
#[cfg(target_arch = "wasm32")]
fn run_dots(
    layer: web_sys::Element,
    arcs: StoredValue<DotArcs>,
) -> std::rc::Rc<std::cell::Cell<bool>> {
    use std::cell::{Cell, RefCell};
    use std::collections::HashMap;
    use std::rc::Rc;

    use leptos::prelude::request_animation_frame;

    use crate::energy_orbite::{advance_phase, arc_dots, point};

    /// Where each arc's dots are, in turns, and when they were last moved.
    #[derive(Default)]
    struct Phases {
        by_id: HashMap<String, f64>,
        last_ms: Option<f64>,
    }

    const SVG_NS: &str = "http://www.w3.org/2000/svg";

    fn frame(
        layer: web_sys::Element,
        arcs: StoredValue<DotArcs>,
        alive: Rc<Cell<bool>>,
        phases: Rc<RefCell<Phases>>,
    ) {
        if !alive.get() {
            return;
        }
        let hidden = layer.owner_document().is_none_or(|d| d.hidden());
        if !hidden {
            // The store is gone once the view is: end the chain rather than touch it.
            let Some(dots) = arcs.try_with_value(|list| {
                let now = js_sys::Date::now();
                let mut ph = phases.borrow_mut();
                let dt = ph.last_ms.map_or(0.0, |l| (now - l) / 1000.0);
                ph.last_ms = Some(now);
                ph.by_id
                    .retain(|id, _| list.iter().any(|(a, _, _)| a.id == *id));
                let mut dots = Vec::new();
                for (arc, radius, width) in list {
                    let phase = ph.by_id.entry(arc.id.clone()).or_insert(0.0);
                    *phase = advance_phase(*phase, dt, arc);
                    for (deg, opacity) in arc_dots(arc, *phase) {
                        let (x, y) = point(CENTRE, CENTRE, *radius, deg);
                        dots.push((x, y, opacity, width * DOT_R));
                    }
                }
                dots
            }) else {
                return;
            };
            draw(&layer, &dots);
        }
        request_animation_frame(move || frame(layer, arcs, alive, phases));
    }

    fn draw(layer: &web_sys::Element, dots: &[(f64, f64, f64, f64)]) {
        let Some(doc) = layer.owner_document() else {
            return;
        };
        let mut have = layer.child_element_count() as usize;
        while have < dots.len() {
            let Ok(c) = doc.create_element_ns(Some(SVG_NS), "circle") else {
                break;
            };
            let _ = layer.append_child(&c);
            have += 1;
        }
        let mut node = layer.first_element_child();
        for (x, y, opacity, r) in dots {
            let Some(c) = node else { break };
            node = c.next_element_sibling();
            let _ = c.set_attribute("cx", &format!("{x:.2}"));
            let _ = c.set_attribute("cy", &format!("{y:.2}"));
            let _ = c.set_attribute("opacity", &format!("{opacity:.2}"));
            let _ = c.set_attribute("r", &format!("{r:.2}"));
        }
        while let Some(c) = node {
            node = c.next_element_sibling();
            c.remove();
        }
    }

    let alive = Rc::new(Cell::new(true));
    frame(layer, arcs, alive.clone(), Rc::default());
    alive
}

/// Keeps the loop's arc list in step with the rings and runs the loop once per mount, on the dots
/// layer; nothing at all when the system asks for reduced motion.
#[cfg(target_arch = "wasm32")]
fn animate_dots(layer: NodeRef<leptos::svg::G>, rings: Memo<Rings>) {
    use std::cell::Cell;
    use std::rc::Rc;

    if crate::energy_dots::reduced_motion() {
        return;
    }
    let arcs = StoredValue::new(rings.with_untracked(dot_arcs));
    Effect::new(move |_| arcs.set_value(rings.with(dot_arcs)));
    let alive = StoredValue::new_local(None::<Rc<Cell<bool>>>);
    Effect::new(move |_| {
        let Some(el) = layer.get() else {
            return;
        };
        if alive.with_value(Option::is_none) {
            alive.set_value(Some(run_dots(el.into(), arcs)));
        }
    });
    on_cleanup(move || {
        if let Some(Some(flag)) = alive.try_get_value() {
            flag.set(false);
        }
    });
}

fn find(r: &Rings, id: &str) -> Option<Arc> {
    r.inner.iter().chain(&r.outer).find(|a| a.id == id).cloned()
}

/// What an arc does besides carrying its name: "production", "↘ import", "↗ export"...
fn note(lang: Lang, role: SankeyRole) -> String {
    let (arrow, key) = match role {
        SankeyRole::Solar => ("", EnergyText::CircleProduction),
        SankeyRole::Grid => ("↘ ", EnergyText::CircleImport),
        SankeyRole::Export => ("↗ ", EnergyText::CircleExport),
        SankeyRole::Battery => ("↘ ", EnergyText::CircleDischarging),
        SankeyRole::BatteryCharge => ("↗ ", EnergyText::CircleCharging),
        _ => return String::new(),
    };
    format!("{arrow}{}", word(lang, key))
}

/// An Others arc opens the full list, any other arc its single sensor, when it has one.
fn activate(arc: &Arc, open: Callback<String>, expanded: RwSignal<bool>) {
    if arc.role == SankeyRole::Others {
        expanded.update(|e| *e = !*e);
    } else if let Some(s) = arc.sensor.clone() {
        open.run(s);
    }
}

fn clickable(arc: &Arc) -> bool {
    arc.sensor.is_some() || arc.role == SankeyRole::Others
}

#[component]
fn OrbiteArc(
    id: String,
    rings: Memo<Rings>,
    open: Callback<String>,
    expanded: RwSignal<bool>,
    hot: RwSignal<Option<String>>,
) -> impl IntoView {
    let arc_id = id.clone();
    let arc = Memo::new(move |_| rings.with(|r| find(r, &arc_id)));
    let Some(first) = arc.get_untracked() else {
        return ().into_any();
    };
    let can_click = clickable(&first);
    let (radius, width) = geometry(first.ring);
    let circumference = std::f64::consts::TAU * radius;
    let colour = move || {
        let (role, slot) = arc.with(|a| {
            a.as_ref()
                .map_or((first.role, first.slot), |a| (a.role, a.slot))
        });
        format!("var({})", colour_var(role, slot))
    };
    // The arc is a dash of a full circle, so a new angle glides round the ring.
    let style = move || {
        let (a0, a1) = arc.with(|a| a.as_ref().map_or((0.0, 0.0), |a| (a.a0, a.a1)));
        format!(
            "stroke-dasharray:{:.2} {circumference:.2};stroke-dashoffset:{:.2};color:{}",
            (a1 - a0) / 360.0 * circumference,
            -a0 / 360.0 * circumference,
            colour()
        )
    };
    let hot_id = id.clone();
    let enter_id = id.clone();
    view! {
        <circle
            class="energy-orbite__arc"
            class:energy-orbite__arc--click=can_click
            class:energy-orbite__arc--hot=move || hot.get().as_deref() == Some(hot_id.as_str())
            data-id=id
            aria-hidden="true"
            fill="none"
            stroke=colour
            stroke-width=width
            cx=CENTRE
            cy=CENTRE
            r=radius
            transform=format!("rotate(-90 {CENTRE} {CENTRE})")
            style=style
            on:mouseenter=move |_| hot.set(Some(enter_id.clone()))
            on:mouseleave=move |_| hot.set(None)
            on:click=move |_| {
                if let Some(a) = arc.get_untracked() {
                    activate(&a, open, expanded);
                }
            }
        />
    }
    .into_any()
}

#[component]
fn OrbiteRow(
    id: String,
    model: Memo<Sankey>,
    rings: Memo<Rings>,
    open: Callback<String>,
    expanded: RwSignal<bool>,
    hot: RwSignal<Option<String>>,
) -> impl IntoView {
    let lang = use_lang();
    let arc_id = id.clone();
    let arc = Memo::new(move |_| rings.with(|r| find(r, &arc_id)));
    let Some(first) = arc.get_untracked() else {
        return ().into_any();
    };
    let can_click = clickable(&first);
    let swatch = move || {
        let (role, slot) = arc.with(|a| {
            a.as_ref()
                .map_or((first.role, first.slot), |a| (a.role, a.slot))
        });
        format!("background: var({})", colour_var(role, slot))
    };
    let name_id = id.clone();
    let name = move || model.with(|m| node_name(lang(), m, &name_id, false));
    let note_text = move || {
        let role = arc.with(|a| a.as_ref().map_or(first.role, |a| a.role));
        note(lang(), role)
    };
    let value = move || {
        let watts = arc.with(|a| a.as_ref().map_or(0.0, |a| a.watts));
        power_text(lang(), Some(watts))
    };
    let share = move || arc.with(|a| a.as_ref().map_or(0, |a| a.share));
    let hot_id = id.clone();
    let enter_id = id.clone();
    let focus_id = id.clone();
    view! {
        <button
            type="button"
            class="energy-orbite__row"
            class:energy-orbite__row--hot=move || hot.get().as_deref() == Some(hot_id.as_str())
            data-id=id
            aria-expanded=(first.role == SankeyRole::Others)
                .then(|| expanded.get().to_string())
            class:energy-orbite__row--static=!can_click
            on:mouseenter=move |_| hot.set(Some(enter_id.clone()))
            on:mouseleave=move |_| hot.set(None)
            on:focus=move |_| hot.set(Some(focus_id.clone()))
            on:blur=move |_| hot.set(None)
            on:click=move |_| {
                if let Some(a) = arc.get_untracked() {
                    activate(&a, open, expanded);
                }
            }
        >
            <span class="energy-orbite__swatch" style=swatch></span>
            <span class="energy-orbite__who">
                <span class="energy-orbite__name">{name}</span>
                <span class="energy-orbite__note">{note_text}</span>
            </span>
            <span class="energy-orbite__nums">
                <span class="energy-orbite__val">{value}</span>
                <span class="energy-orbite__pct">{move || format!("{}\u{A0}%", share())}</span>
            </span>
        </button>
    }
    .into_any()
}

#[component]
pub fn OrbiteDiagram(
    ui: Memo<EnergyUi>,
    narrow: bool,
    open: Callback<String>,
    expanded: RwSignal<bool>,
) -> impl IntoView {
    let lang = use_lang();
    let model = Memo::new(move |_| ui.with(|u| build(u, fold_at(narrow))));
    let ring_set = Memo::new(move |_| model.with(rings));
    let hot = RwSignal::new(None::<String>);
    let dots_layer = NodeRef::<leptos::svg::G>::new();
    #[cfg(target_arch = "wasm32")]
    animate_dots(dots_layer, ring_set);
    let description = move || ui.with(|u| aria_sentence(lang(), u));
    let empty = move || model.with(Sankey::is_empty);

    let inner = move || ring_set.with(|r| r.inner.iter().map(|a| a.id.clone()).collect::<Vec<_>>());
    let outer = move || ring_set.with(|r| r.outer.iter().map(|a| a.id.clone()).collect::<Vec<_>>());

    let house = move || {
        model.with(|m| {
            let shown = m
                .house_w
                .map_or_else(|| "—".to_string(), |w| power_text(lang(), Some(w)));
            view! { <b class="energy-orbite__house">{shown}</b> }
        })
    };
    let mix = move || {
        model.with(|m| {
            m.solar_share.filter(|n| *n > 0).map(|n| {
                view! { <span class="energy-orbite__mix">{solar_share_line(lang(), n)}</span> }
            })
        })
    };

    let row = move |id: String| {
        view! { <OrbiteRow id=id model=model rings=ring_set open=open expanded=expanded hot=hot /> }
    };
    let arc = move |id: String| {
        view! { <OrbiteArc id=id rings=ring_set open=open expanded=expanded hot=hot /> }
    };

    view! {
        <div
            class="energy-diagram energy-orbite"
            class:energy-orbite--narrow=narrow
            class:energy-orbite--empty=empty
            class:energy-orbite--dim=move || {
                hot.with(|h| {
                    h.as_ref().is_some_and(|id| ring_set.with(|r| find(r, id).is_some()))
                })
            }
        >
            <div class="energy-orbite__ring">
                <svg
                    class="energy-orbite__svg"
                    viewBox=format!("0 0 {SIZE} {SIZE}")
                    role="group"
                    aria-label=description
                >
                    <title>{description}</title>
                    <circle
                        class="energy-orbite__track"
                        cx=CENTRE
                        cy=CENTRE
                        r=INNER_R
                        stroke-width=INNER_W
                    />
                    <circle
                        class="energy-orbite__track"
                        cx=CENTRE
                        cy=CENTRE
                        r=OUTER_R
                        stroke-width=OUTER_W
                    />
                    <For each=inner key=|id| id.clone() children=arc />
                    <For each=outer key=|id| id.clone() children=arc />
                    <g class="energy-orbite__dots" node_ref=dots_layer></g>
                </svg>
                <div class="energy-orbite__centre">
                    <span class="energy-orbite__ico" inner_html=HOUSE></span>
                    {house}
                    <Show when=move || model.with(|m| m.house_w.is_some())>
                        <span class="energy-orbite__small">
                            {move || word(lang(), EnergyText::InTheHouse)}
                        </span>
                    </Show>
                    {mix}
                </div>
            </div>
            <Show
                when=move || !empty()
                fallback=move || {
                    view! {
                        <p class="energy-orbite__empty">
                            {move || word(lang(), EnergyText::NoLivePower)}
                        </p>
                    }
                }
            >
                <section class="energy-orbite__list energy-orbite__list--in">
                    <h3>{move || word(lang(), EnergyText::ComingIn)}</h3>
                    <For each=inner key=|id| id.clone() children=row />
                </section>
                <section class="energy-orbite__list energy-orbite__list--out">
                    <h3>{move || word(lang(), EnergyText::GoingOut)}</h3>
                    <For each=outer key=|id| id.clone() children=row />
                </section>
            </Show>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_say_what_each_arc_does() {
        assert_eq!(note(Lang::En, SankeyRole::Solar), "production");
        assert_eq!(note(Lang::En, SankeyRole::Grid), "↘ import");
        assert_eq!(note(Lang::En, SankeyRole::Export), "↗ export");
        assert_eq!(note(Lang::En, SankeyRole::Consumer), "");
    }

    #[test]
    fn rings_have_room_for_their_strokes() {
        let (r, w) = geometry(Ring::Outer);
        assert!(r + w / 2.0 <= CENTRE);
    }
}
