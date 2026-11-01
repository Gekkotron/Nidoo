//! The Flux view of the Energy screen: the live power flow as a Sankey, drawn from
//! `energy_sankey` (what flows where) and `sankey_layout` (where it sits).
//!
//! Every bar, band and label is a keyed child that reads its own geometry and sets it through
//! `style`, so a new reading moves the existing elements and CSS glides them.

use leptos::prelude::*;
use nidoo_core::energy::EnergyUi;

use crate::energy::{aria_sentence, power_text};
use crate::energy_circles::sanitise;
use crate::energy_sankey::{FOLD_NARROW, FOLD_WIDE, Sankey, SankeyRole, build};
use crate::i18n::energy::{EnergyText, energy_text as word, others};
use crate::i18n::{Key, Lang, text, use_lang};
use crate::sankey_layout::{Anchor, LINE, NARROW, Placed, WIDE, place};
use crate::screens::energy::short;

pub(crate) fn fold_at(narrow: bool) -> usize {
    if narrow { FOLD_NARROW } else { FOLD_WIDE }
}

/// Whether the diagram folds some consumers into "+N others" (so the screen offers the list).
pub fn flux_folded(ui: &EnergyUi, narrow: bool) -> bool {
    build(ui, fold_at(narrow)).folded
}

/// The theme variable a node is drawn in.
fn color_var(role: SankeyRole, slot: usize) -> &'static str {
    match role {
        SankeyRole::Solar => "--energy-solar",
        SankeyRole::Grid => "--energy-grid",
        SankeyRole::Battery | SankeyRole::Export | SankeyRole::BatteryCharge => "--energy-battery",
        SankeyRole::House => "--ink",
        SankeyRole::Unitemised | SankeyRole::Others => "--energy-muted",
        // Consumer colours are defined on `.energy` and never reuse a source colour.
        SankeyRole::Consumer => [
            "--energy-c0",
            "--energy-c1",
            "--energy-c2",
            "--energy-c3",
            "--energy-c4",
        ][slot % 5],
    }
}

/// A band's colours at its source and target ends. The house bar is painted in what flows into
/// it, so a band touching it takes the colour of its other end along its whole length.
fn band_colors(from: (SankeyRole, usize), to: (SankeyRole, usize)) -> (&'static str, &'static str) {
    let (from, to) = if to.0 == SankeyRole::House {
        (from, from)
    } else if from.0 == SankeyRole::House {
        (to, to)
    } else {
        (from, to)
    };
    (color_var(from.0, from.1), color_var(to.0, to.1))
}

fn role_class(role: SankeyRole, slot: usize) -> String {
    let name = match role {
        SankeyRole::Solar => "solar",
        SankeyRole::Grid => "grid",
        SankeyRole::Battery => "battery",
        SankeyRole::House => "house",
        SankeyRole::Export => "export",
        SankeyRole::BatteryCharge => "charge",
        SankeyRole::Unitemised => "unitemised",
        SankeyRole::Others => "others",
        SankeyRole::Consumer => return format!("c{}", slot % 5),
    };
    name.to_string()
}

/// The words of a node: its name, from the UI words for everything but a consumer; cut short on
/// phones (Classic's `LABEL_CHARS`).
pub(crate) fn node_name(lang: Lang, model: &Sankey, id: &str, narrow: bool) -> String {
    let Some(n) = model.nodes.iter().find(|n| n.id == id) else {
        return String::new();
    };
    let name = match n.role {
        SankeyRole::Solar => text(lang, Key::Solar).to_string(),
        SankeyRole::Grid => word(lang, EnergyText::NodeGrid).to_string(),
        SankeyRole::Battery => text(lang, Key::Battery).to_string(),
        SankeyRole::House => text(lang, Key::House).to_string(),
        SankeyRole::Export => word(lang, EnergyText::Export).to_string(),
        SankeyRole::BatteryCharge => word(lang, EnergyText::BatteryCharging).to_string(),
        SankeyRole::Unitemised => word(lang, EnergyText::Unitemised).to_string(),
        SankeyRole::Others => others(lang, n.count),
        SankeyRole::Consumer => n.name.clone().unwrap_or_default(),
    };
    // Only a phone is short of room.
    if narrow { short(name) } else { name }
}

fn node_value(lang: Lang, model: &Sankey, id: &str) -> String {
    let Some(n) = model.nodes.iter().find(|n| n.id == id) else {
        return String::new();
    };
    match n.role {
        SankeyRole::House => power_text(lang, model.house_w),
        _ => power_text(lang, Some(n.watts)),
    }
}

#[component]
fn FluxBar(
    id: String,
    model: Memo<Sankey>,
    placed: Memo<Placed>,
    open: Callback<String>,
    expanded: RwSignal<bool>,
) -> impl IntoView {
    let node_id = id.clone();
    let node =
        Memo::new(move |_| model.with(|m| m.nodes.iter().find(|n| n.id == node_id).cloned()));
    let bar_id = id.clone();
    let bar = Memo::new(move |_| placed.with(|p| p.bars.iter().find(|b| b.id == bar_id).cloned()));
    let Some(first) = node.get_untracked() else {
        return ().into_any();
    };
    let clickable = first.sensor.is_some() || first.role == SankeyRole::Others;
    // A reorder moves a consumer to another slot; its colour follows, as its band's does.
    let class = move || {
        let (role, slot) = node.with(|n| {
            n.as_ref()
                .map_or((first.role, first.slot), |n| (n.role, n.slot))
        });
        format!(
            "energy-flux__bar energy-flux__bar--{}{}",
            role_class(role, slot),
            if clickable {
                " energy-flux__bar--click"
            } else {
                ""
            }
        )
    };
    let activate = move || {
        let Some(n) = node.get_untracked() else {
            return;
        };
        if n.role == SankeyRole::Others {
            expanded.update(|e| *e = !*e);
        } else if let Some(s) = n.sensor {
            open.run(s);
        }
    };
    let geometry = move || {
        bar.get()
            .map(|b| {
                format!(
                    "transform: translate({}px, {}px); width: {}px; height: {}px",
                    b.x, b.y, b.w, b.h
                )
            })
            .unwrap_or_default()
    };
    view! {
        <rect
            class=class
            data-id=id
            rx="4"
            width=move || bar.get().map(|b| b.w)
            height=move || bar.get().map(|b| b.h)
            style=geometry
            on:click=move |_| activate()
        />
    }
    .into_any()
}

#[component]
fn FluxLabel(
    id: String,
    narrow: bool,
    model: Memo<Sankey>,
    placed: Memo<Placed>,
    open: Callback<String>,
    expanded: RwSignal<bool>,
) -> impl IntoView {
    let lang = use_lang();
    let node_id = id.clone();
    let node =
        Memo::new(move |_| model.with(|m| m.nodes.iter().find(|n| n.id == node_id).cloned()));
    let label_id = id.clone();
    let label =
        Memo::new(move |_| placed.with(|p| p.labels.iter().find(|l| l.id == label_id).cloned()));
    let Some(first) = node.get_untracked() else {
        return ().into_any();
    };
    let clickable = first.sensor.is_some() || first.role == SankeyRole::Others;
    let activate = move || {
        let Some(n) = node.get_untracked() else {
            return;
        };
        if n.role == SankeyRole::Others {
            expanded.update(|e| *e = !*e);
        } else if let Some(s) = n.sensor {
            open.run(s);
        }
    };
    let anchor = move || match label.get().map(|l| l.anchor) {
        Some(Anchor::Start) | None => "start",
        Some(Anchor::Middle) => "middle",
        Some(Anchor::End) => "end",
    };
    let spoken_id = id.clone();
    let spoken = move || {
        model.with(|m| {
            format!(
                "{}, {}",
                node_name(lang(), m, &spoken_id, false),
                node_value(lang(), m, &spoken_id)
            )
        })
    };
    let content = move || {
        let (name, value) = model.with(|m| {
            (
                node_name(lang(), m, &id, narrow),
                node_value(lang(), m, &id),
            )
        });
        let name = view! {
            <tspan
                class="energy-flux__name"
                x=narrow.then_some("0")
                dy=narrow.then(|| LINE.to_string())
            >
                {name}
            </tspan>
        };
        let value = view! { <tspan class="energy-flux__value">{value}</tspan> };
        if narrow {
            // Two lines: the value, then the name under it.
            view! {
                {value}
                {name}
            }
            .into_any()
        } else if label.get().map(|l| l.anchor) == Some(Anchor::End) {
            // The name sits on the outer side: first when the label ends at the bar.
            view! { {name}" "{value} }.into_any()
        } else {
            view! { {value}" "{name} }.into_any()
        }
    };
    view! {
        <text
            class="energy-flux__label"
            class:energy-flux__label--click=clickable
            text-anchor=anchor
            style=move || {
                label
                    .get()
                    .map(|l| format!("transform: translate({}px, {}px)", l.x, l.y))
                    .unwrap_or_default()
            }
            tabindex=clickable.then_some("0")
            role=clickable.then_some("button")
            aria-label=move || clickable.then(&spoken)
            on:click=move |_| activate()
            on:keydown=move |ev| {
                if ev.key() == "Enter" || ev.key() == " " {
                    // Space would also scroll the page.
                    ev.prevent_default();
                    activate();
                }
            }
        >
            {content}
        </text>
    }
    .into_any()
}

/// A gradient along one band, from the source colour to the target's.
#[component]
fn FluxGradient(id: String, model: Memo<Sankey>, placed: Memo<Placed>) -> impl IntoView {
    let band_id = id.clone();
    let band =
        Memo::new(move |_| placed.with(|p| p.bands.iter().find(|b| b.id == band_id).cloned()));
    let colors = move || {
        let b = band.get()?;
        model.with(|m| {
            let of = |id: &str| {
                m.nodes
                    .iter()
                    .find(|n| n.id == id)
                    .map(|n| (n.role, n.slot))
            };
            Some(band_colors(of(&b.from)?, of(&b.to)?))
        })
    };
    let from = move || format!("stop-color: var({})", colors().map_or("--ink", |c| c.0));
    let to = move || format!("stop-color: var({})", colors().map_or("--ink", |c| c.1));
    view! {
        <linearGradient
            id=format!("flux-g-{}", sanitise(&id))
            gradientUnits="userSpaceOnUse"
            x1=move || band.get().map(|b| b.x0)
            y1=move || band.get().map(|b| b.y0)
            x2=move || band.get().map(|b| b.x1)
            y2=move || band.get().map(|b| b.y1)
        >
            <stop offset="0" style=from />
            <stop offset="1" style=to />
        </linearGradient>
    }
}

/// The house bar's fill: hard stops in the colour of each source over the stretch its band lands
/// on, and the hatch's tone over any rest no source accounts for.
#[component]
fn FluxHouseMix(narrow: bool, model: Memo<Sankey>, placed: Memo<Placed>) -> impl IntoView {
    let stops = move || {
        let mut out = Vec::new();
        let mut end = 0.0;
        placed.with(|p| {
            model.with(|m| {
                for s in &p.house_mix {
                    let Some(n) = m.nodes.iter().find(|n| n.id == s.from) else {
                        continue;
                    };
                    let colour = color_var(n.role, n.slot);
                    out.push((s.start, colour));
                    out.push((s.end, colour));
                    end = s.end;
                }
            })
        });
        if end < 1.0 {
            out.push((end, "--energy-hatch"));
            out.push((1.0, "--energy-hatch"));
        }
        out.into_iter()
            .map(|(at, colour)| {
                view! { <stop offset=at style=format!("stop-color: var({colour})") /> }
            })
            .collect::<Vec<_>>()
    };
    view! {
        <linearGradient
            id="flux-house"
            x1="0"
            y1="0"
            x2=if narrow { "1" } else { "0" }
            y2=if narrow { "0" } else { "1" }
        >
            {stops}
        </linearGradient>
    }
}

#[component]
fn FluxBand(id: String, model: Memo<Sankey>, placed: Memo<Placed>) -> impl IntoView {
    // What no consumer accounts for is hatched, its band as its bar.
    let to = placed.with_untracked(|p| p.bands.iter().find(|b| b.id == id).map(|b| b.to.clone()));
    let unitemised = model.with_untracked(|m| {
        m.nodes
            .iter()
            .any(|n| n.role == SankeyRole::Unitemised && Some(&n.id) == to.as_ref())
    });
    let fill = if unitemised {
        "url(#flux-hatch)".to_string()
    } else {
        format!("url(#flux-g-{})", sanitise(&id))
    };
    let band_id = id.clone();
    let path = Memo::new(move |_| {
        placed.with(|p| {
            p.bands
                .iter()
                .find(|b| b.id == band_id)
                .map(|b| b.path.clone())
                .unwrap_or_default()
        })
    });
    let style = move || format!("d: path(\"{}\")", path.get());
    view! {
        <path
            class="energy-flux__band"
            data-id=id.clone()
            fill=fill
            d=move || path.get()
            style=style
        />
    }
}

#[component]
fn FluxSweep(id: String, placed: Memo<Placed>) -> impl IntoView {
    let path = Memo::new(move |_| {
        placed.with(|p| {
            p.bands
                .iter()
                .find(|b| b.id == id)
                .map(|b| b.path.clone())
                .unwrap_or_default()
        })
    });
    let style = move || format!("d: path(\"{}\")", path.get());
    view! { <path class="energy-flux__sweep" fill="url(#flux-sweep)" d=move || path.get() style=style /> }
}

#[component]
fn Chip(value: String, label: &'static str) -> impl IntoView {
    view! {
        <div class="energy-flux__chip">
            <b>{value}</b>
            <span>{label}</span>
        </div>
    }
}

#[component]
pub fn FluxDiagram(
    ui: Memo<EnergyUi>,
    narrow: bool,
    open: Callback<String>,
    expanded: RwSignal<bool>,
) -> impl IntoView {
    let lang = use_lang();
    let frame = if narrow { NARROW } else { WIDE };
    let model = Memo::new(move |_| ui.with(|u| build(u, fold_at(narrow))));
    let placed = Memo::new(move |_| model.with(|m| place(m, frame)));
    let description = move || ui.with(|u| aria_sentence(lang(), u));

    let bars = move || model.with(|m| m.nodes.iter().map(|n| n.id.clone()).collect::<Vec<_>>());
    let bands = move || placed.with(|p| p.bands.iter().map(|b| b.id.clone()).collect::<Vec<_>>());
    let labels = move || placed.with(|p| p.labels.iter().map(|l| l.id.clone()).collect::<Vec<_>>());

    let chips = move || {
        let l = lang();
        model.with(|m| {
            let mut out = Vec::new();
            if let Some(w) = m.house_w {
                out.push(
                    view! {
                        <Chip value=power_text(l, Some(w)) label=word(l, EnergyText::ChipConsumed) />
                    }
                    .into_any(),
                );
            }
            if let Some(w) = m.solar_w {
                out.push(
                    view! {
                        <Chip value=power_text(l, Some(w)) label=word(l, EnergyText::ChipSolar) />
                    }
                    .into_any(),
                );
            }
            if m.export_w > 0.0 {
                out.push(
                    view! {
                        <Chip
                            value=format!("↗ {}", power_text(l, Some(m.export_w)))
                            label=word(l, EnergyText::ChipToGrid)
                        />
                    }
                    .into_any(),
                );
            } else if let Some(w) = m.import_w {
                out.push(
                    view! {
                        <Chip value=power_text(l, Some(w)) label=word(l, EnergyText::ChipFromGrid) />
                    }
                    .into_any(),
                );
            }
            if let Some(p) = m.solar_share {
                out.push(
                    view! {
                        <Chip
                            value=format!("{p}\u{A0}%")
                            label=word(l, EnergyText::ChipSolarShare)
                        />
                    }
                    .into_any(),
                );
            }
            out
        })
    };

    view! {
        <div class="energy-diagram energy-flux" class:energy-flux--narrow=narrow>
            <div class="energy-flux__chips">{chips}</div>
            <Show
                when=move || !model.with(Sankey::is_empty)
                fallback=move || {
                    view! {
                        <p class="energy-flux__empty">
                            {move || word(lang(), EnergyText::NoLivePower)}
                        </p>
                    }
                }
            >
                <svg
                    class="energy-flux__svg"
                    viewBox=format!("0 0 {} {}", frame.w, frame.h)
                    role="group"
                    aria-label=description
                >
                    <title>{description}</title>
                    <defs>
                        <For
                            each=bands
                            key=|id| id.clone()
                            children=move |id| {
                                view! { <FluxGradient id=id model=model placed=placed /> }
                            }
                        />
                        <FluxHouseMix narrow=narrow model=model placed=placed />
                        <pattern
                            id="flux-hatch"
                            patternUnits="userSpaceOnUse"
                            width="6"
                            height="6"
                            patternTransform="rotate(45)"
                        >
                            <rect class="energy-flux__hatch-ground" width="6" height="6" />
                            <rect class="energy-flux__hatch-line" width="2.5" height="6" />
                        </pattern>
                        <linearGradient
                            id="flux-sweep"
                            gradientUnits="userSpaceOnUse"
                            x1="0"
                            y1="0"
                            x2=if narrow { "0" } else { "260" }
                            y2=if narrow { "260" } else { "0" }
                        >
                            <stop offset="0" style="stop-color: var(--ink); stop-opacity: 0" />
                            <stop offset=".5" style="stop-color: var(--ink); stop-opacity: .22" />
                            <stop offset="1" style="stop-color: var(--ink); stop-opacity: 0" />
                            <animateTransform
                                attributeName="gradientTransform"
                                type="translate"
                                from=if narrow { "0 -260" } else { "-260 0" }
                                to=if narrow {
                                    format!("0 {}", frame.h)
                                } else {
                                    format!("{} 0", frame.w)
                                }
                                dur="3.2s"
                                repeatCount="indefinite"
                            />
                        </linearGradient>
                    </defs>
                    <For
                        each=bands
                        key=|id| id.clone()
                        children=move |id| view! { <FluxBand id=id model=model placed=placed /> }
                    />
                    <For
                        each=bands
                        key=|id| id.clone()
                        children=move |id| view! { <FluxSweep id=id placed=placed /> }
                    />
                    <For
                        each=bars
                        key=|id| id.clone()
                        children=move |id| {
                            view! {
                                <FluxBar
                                    id=id
                                    model=model
                                    placed=placed
                                    open=open
                                    expanded=expanded
                                />
                            }
                        }
                    />
                    <For
                        each=labels
                        key=|id| id.clone()
                        children=move |id| {
                            view! {
                                <FluxLabel
                                    id=id
                                    narrow=narrow
                                    model=model
                                    placed=placed
                                    open=open
                                    expanded=expanded
                                />
                            }
                        }
                    />
                </svg>
            </Show>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_come_from_the_shared_palette() {
        assert_eq!(color_var(SankeyRole::Solar, 0), "--energy-solar");
        assert_eq!(color_var(SankeyRole::Grid, 0), "--energy-grid");
        assert_eq!(color_var(SankeyRole::Export, 0), "--energy-battery");
        assert_eq!(color_var(SankeyRole::Consumer, 6), "--energy-c1");
        assert_eq!(color_var(SankeyRole::Others, 0), "--energy-muted");
        assert_eq!(color_var(SankeyRole::House, 0), "--ink");
    }

    #[test]
    fn bands_touching_the_house_take_the_other_end_colour() {
        let house = (SankeyRole::House, 0);
        assert_eq!(
            band_colors((SankeyRole::Grid, 0), house),
            ("--energy-grid", "--energy-grid")
        );
        assert_eq!(
            band_colors(house, (SankeyRole::Consumer, 1)),
            ("--energy-c1", "--energy-c1")
        );
        assert_eq!(
            band_colors((SankeyRole::Solar, 0), (SankeyRole::Export, 0)),
            ("--energy-solar", "--energy-battery")
        );
    }
}
