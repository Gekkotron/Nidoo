//! The live icons themselves: a 24-unit SVG per entity, in the hub's line style, whose moving
//! parts read their phase from the shared frame loop. Only those attributes change each frame.

use leptos::prelude::*;
use nidoo_api::EntityView;
use nidoo_core::model::{Availability, Kind, StateValue};

use super::ac::{
    AcTone, DROPS_PERIOD_MS, VANE_PERIOD_MS, ac_tone, airflow_period_ms, drop_fall,
    flow_dash_offset, is_dry, vane_y,
};
use super::clock::{animations_on, use_phase};

/// Whether `kind` gets a live icon on its card tile.
pub fn has_live_icon(kind: Kind) -> bool {
    matches!(kind, Kind::Climate)
}

/// The live icon for `entity`, or its static glyph for a kind without one.
#[component]
pub fn LiveIcon(entity: EntityView) -> impl IntoView {
    match entity.kind {
        Kind::Climate => view! { <AcIcon entity=entity /> }.into_any(),
        _ => {
            let glyph = crate::icons::entity_icon(&entity);
            view! { <span class="live-icon__glyph" inner_html=glyph></span> }.into_any()
        }
    }
}

/// Whether the entity can be reached; a stale state on an offline device must not move.
fn available(entity: &EntityView) -> bool {
    entity.state.availability != Availability::Offline
}

/// An id usable inside `url(#…)`, unique per entity, so two cards on a page do not share a mask.
fn svg_id(prefix: &str, entity_id: &str) -> String {
    let safe: String = entity_id
        .chars()
        .map(|c| match c {
            'a'..='z' | '0'..='9' | '-' | '_' => c,
            'A'..='Z' => c.to_ascii_lowercase(),
            _ => '-',
        })
        .collect();
    format!("{prefix}-{safe}")
}

/// A phase for `part` of `entity` when it moves, otherwise the still value.
fn phase(entity_id: &str, part: &str, period_ms: f64, moving: bool, still: f64) -> Signal<f64> {
    if moving && animations_on() {
        use_phase(format!("{entity_id}/{part}"), period_ms)
    } else {
        Signal::stored(still)
    }
}

/// The x of each airflow line and drop, under the unit.
const COLUMNS: [f64; 3] = [7.0, 12.0, 17.0];

/// A wall AC: the unit, its light and LED, the vane below, and air or drops in the mode's tone.
#[component]
fn AcIcon(entity: EntityView) -> impl IntoView {
    let (mode, fan) = match &entity.state.value {
        StateValue::Climate(c) => (c.mode.clone(), c.fan_mode.clone()),
        _ => (None, None),
    };
    let tone = ac_tone(mode.as_deref(), available(&entity));
    let on = tone.is_some();
    let dry = on && is_dry(mode.as_deref());
    let id = &entity.id;
    let vane = phase(id, "vane", VANE_PERIOD_MS, on, 0.0);
    let flow = phase(
        id,
        "flow",
        airflow_period_ms(fan.as_deref()),
        on && !dry,
        0.0,
    );
    let drops = phase(id, "drops", DROPS_PERIOD_MS, dry, 0.5);
    let class = ["live-icon", tone.map_or("live-icon--off", AcTone::class)].join(" ");
    let vane_class = if on {
        "live-icon__vane live-icon__tint"
    } else {
        "live-icon__vane"
    };
    let (mask, fade) = (svg_id("live-air", id), svg_id("live-fade", id));
    let air = (on && !dry).then(|| {
        let lines = COLUMNS
            .iter()
            .enumerate()
            .map(|(k, x)| {
                let shift = k as f64 / 3.0;
                view! {
                    <path
                        class="live-icon__flow live-icon__tint"
                        d=format!("M{x} 14.2c-1.1 1.6 1.1 3.2 0 4.8s1.1 3.2 0 4.8")
                        stroke-width="1.5"
                        stroke-dasharray="2.6 2.4"
                        stroke-dashoffset=move || flow_dash_offset((flow.get() + shift).rem_euclid(1.0))
                    ></path>
                }
            })
            .collect_view();
        view! {
            <defs>
                <linearGradient id=fade.clone() x1="0" y1="0" x2="0" y2="1">
                    <stop offset="0" stop-color="#fff"></stop>
                    <stop offset="1" stop-color="#000"></stop>
                </linearGradient>
                <mask id=mask.clone() maskUnits="userSpaceOnUse" x="0" y="13" width="24" height="11">
                    <rect x="0" y="13" width="24" height="11" fill=format!("url(#{fade})")></rect>
                </mask>
            </defs>
            <g mask=format!("url(#{mask})")>{lines}</g>
        }
    });
    let wet = dry.then(|| {
        COLUMNS
            .iter()
            .enumerate()
            .map(|(k, x)| {
                let shift = k as f64 / 3.0;
                let fall = move || drop_fall((drops.get() + shift).rem_euclid(1.0));
                view! {
                    <path
                        class="live-icon__drop"
                        d=format!("M{x} 14.6c.8 1.1 1.3 1.8 1.3 2.4a1.3 1.3 0 0 1-2.6 0c0-.6.5-1.3 1.3-2.4z")
                        transform=move || format!("translate(0 {})", fall().0)
                        opacity=move || fall().1
                    ></path>
                }
            })
            .collect_view()
    });
    view! {
        <svg
            class=class
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="1.75"
            stroke-linecap="round"
            stroke-linejoin="round"
            aria-hidden="true"
            focusable="false"
        >
            <rect x="2.5" y="4" width="19" height="8.4" rx="2.6" fill="currentColor" fill-opacity="0.22"></rect>
            <path d="M5.5 7.3h9" stroke-width="1" opacity="0.55"></path>
            <circle class="live-icon__led" cx="18.2" cy="7.3" r="0.85" opacity=if on { "1" } else { "0.3" }></circle>
            <line
                class=vane_class
                x1="6"
                x2="18"
                y1=move || vane_y(vane.get())
                y2=move || vane_y(vane.get())
                stroke-width="1.3"
            ></line>
            {air}
            {wet}
        </svg>
    }
}
