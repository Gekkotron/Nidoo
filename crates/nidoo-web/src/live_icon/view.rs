//! The live icons themselves: a 24-unit SVG per entity, in the hub's line style, whose moving
//! parts read their phase from the shared frame loop. Only those attributes change each frame.

use std::sync::atomic::{AtomicU32, Ordering};

use leptos::prelude::*;
use nidoo_api::EntityView;
use nidoo_core::model::{Availability, Kind, StateValue};

use super::ac::{
    AcTone, DROPS_PERIOD_MS, VANE_PERIOD_MS, ac_tone, airflow_period_ms, drop_fall,
    flow_dash_offset, is_dry, vane_y,
};
use super::clock::{animations_on, use_phase};
use super::robot::{
    BOLT_PERIOD_MS, DOCK_AT, RETURN_HEADING, RETURN_PERIOD_MS, ROWS_D, RobotPose, VacuumScene,
    bolt_opacity, pose_at, return_pose, row_period_ms, trail_dash_offset, vacuum_scene,
};

/// Whether `kind` gets a live icon on its card tile.
pub fn has_live_icon(kind: Kind) -> bool {
    matches!(kind, Kind::Climate | Kind::Vacuum)
}

/// The live icon for `entity`, or its static glyph for a kind without one.
#[component]
pub fn LiveIcon(entity: EntityView) -> impl IntoView {
    match entity.kind {
        Kind::Climate => view! { <AcIcon entity=entity /> }.into_any(),
        Kind::Vacuum => view! { <VacuumIcon entity=entity /> }.into_any(),
        _ => {
            let glyph = crate::icons::entity_icon(&entity);
            view! { <span class="live-icon__glyph" inner_html=glyph></span> }.into_any()
        }
    }
}

/// The large picture for `entity` on a card: the AC or the robot vacuum drawn without the tile's
/// margins, or nothing for a kind without a live icon.
#[component]
pub fn LivePicture(entity: EntityView) -> impl IntoView {
    match entity.kind {
        Kind::Climate => view! { <AcIcon entity=entity picture=true /> }.into_any(),
        Kind::Vacuum => view! { <VacuumIcon entity=entity picture=true /> }.into_any(),
        _ => ().into_any(),
    }
}

/// Whether the entity can be reached; a stale state on an offline device must not move.
fn available(entity: &EntityView) -> bool {
    entity.state.availability != Availability::Offline
}

/// Numbers each drawing, so its ids stay unique on the page.
static NEXT_DRAWING: AtomicU32 = AtomicU32::new(0);

/// An id usable inside `url(#…)`, unique per drawing: a tile, the card's picture and a sheet's
/// picture of one entity can all be on the page, and a repeated id would point at the first one.
fn svg_id(prefix: &str, entity_id: &str, drawing: u32) -> String {
    let safe: String = entity_id
        .chars()
        .map(|c| match c {
            'a'..='z' | '0'..='9' | '-' | '_' => c,
            'A'..='Z' => c.to_ascii_lowercase(),
            _ => '-',
        })
        .collect();
    format!("{prefix}-{safe}-{drawing}")
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
fn AcIcon(entity: EntityView, #[prop(optional)] picture: bool) -> impl IntoView {
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
    let mut classes = vec!["live-icon", tone.map_or("live-icon--off", AcTone::class)];
    if picture {
        classes.push("live-icon--picture");
    }
    let class = classes.join(" ");
    let vane_class = if on {
        "live-icon__vane live-icon__tint"
    } else {
        "live-icon__vane"
    };
    let drawing = NEXT_DRAWING.fetch_add(1, Ordering::Relaxed);
    let (mask, fade) = (
        svg_id("live-air", id, drawing),
        svg_id("live-fade", id, drawing),
    );
    let view_box = if picture { "1 2.5 22 22" } else { "0 0 24 24" };
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
            viewBox=view_box
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

/// Draws a part in the icon's tone: its `currentColor` becomes `--live-tone`.
const TINT: &str = "color: var(--live-tone)";

/// The robot, drawn around the origin facing +x and moved to `pose`: a disc, its front bumper and
/// its centre puck. `tinted` draws it in the icon's tone, else in the tile's colour.
fn robot(pose: Signal<RobotPose>, tinted: bool, fill_opacity: &'static str) -> impl IntoView {
    view! {
        <g
            class="live-icon__robot"
            style=tinted.then_some(TINT)
            transform=move || pose.get().transform()
            stroke-width="1.3"
        >
            <circle r="3.1" fill="currentColor" fill-opacity=fill_opacity></circle>
            <path d="M2.05 -2.05A2.9 2.9 0 0 1 2.05 2.05"></path>
            <circle r="1.05" fill="currentColor" stroke="none"></circle>
        </g>
    }
}

/// The dock in the room's bottom right corner; `tinted` while the robot charges in it.
fn dock(tinted: bool) -> impl IntoView {
    view! {
        <rect
            class="live-icon__dock"
            style=tinted.then_some(TINT)
            x="18"
            y="16.5"
            width="4"
            height="5"
            rx="1"
            fill="currentColor"
            fill-opacity="0.22"
        ></rect>
    }
}

/// A robot vacuum in its room: cleaning the rows with a trail, gliding into its dock, charging
/// there, or still at the start.
#[component]
fn VacuumIcon(entity: EntityView, #[prop(optional)] picture: bool) -> impl IntoView {
    let (state, fan) = match &entity.state.value {
        StateValue::Vacuum(v) => (v.state.clone(), v.fan_speed.clone()),
        _ => (None, None),
    };
    let scene = vacuum_scene(state.as_deref(), available(&entity));
    let id = &entity.id;
    let mut classes = vec![
        "live-icon",
        match scene {
            VacuumScene::Cleaning | VacuumScene::Returning => "live-icon--moving",
            VacuumScene::Docked => "live-icon--docked",
            VacuumScene::Idle => "live-icon--plain",
        },
    ];
    if picture {
        classes.push("live-icon--picture");
    }
    let class = classes.join(" ");
    let parts = match scene {
        VacuumScene::Cleaning => {
            let rows = phase(id, "rows", row_period_ms(fan.as_deref()), true, 0.5);
            view! {
                <path
                    class="live-icon__trail live-icon__tint"
                    d=ROWS_D
                    stroke-width="4.4"
                    opacity="0.16"
                    stroke-dasharray="69.5 69.5"
                    stroke-dashoffset=move || trail_dash_offset(rows.get())
                ></path>
                {robot(Signal::derive(move || pose_at(rows.get())), true, "0.35")}
            }
            .into_any()
        }
        VacuumScene::Returning => {
            let home = phase(id, "return", RETURN_PERIOD_MS, true, 1.0);
            view! {
                {dock(false)}
                {robot(Signal::derive(move || return_pose(home.get())), true, "0.35")}
            }
            .into_any()
        }
        VacuumScene::Docked => {
            let bolt = phase(id, "bolt", BOLT_PERIOD_MS, true, 0.75);
            let at = RobotPose {
                x: DOCK_AT.0,
                y: DOCK_AT.1,
                heading_deg: RETURN_HEADING,
            };
            view! {
                {dock(true)}
                {robot(Signal::stored(at), false, "0.22")}
                <path
                    class="live-icon__bolt"
                    d="M12.3 8 L10 12 h2.5 l-.8 3.8 L14.4 11.5 H11.9 L12.8 8z"
                    opacity=move || bolt_opacity(bolt.get())
                ></path>
            }
            .into_any()
        }
        VacuumScene::Idle => robot(Signal::stored(pose_at(0.0)), false, "0.22").into_any(),
    };
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
            <rect x="1.5" y="2" width="21" height="20.5" rx="3" stroke-width="1" opacity="0.28"></rect>
            {parts}
        </svg>
    }
}
