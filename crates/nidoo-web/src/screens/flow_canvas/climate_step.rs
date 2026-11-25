//! The settings sheet of the two climate steps, `climate.hold` and `climate.eco`: which rooms,
//! the temperature (hold only, or off) and how long, as minutes or as a time of day.

use leptos::prelude::*;
use nidoo_core::flow::{ClimateSetpoint, Node, NodeKind};

use super::wake_up::toggled;
use crate::api::climate::get_climate;
use crate::i18n::flows::{FlowText, use_flow_text};
use crate::i18n::format::parse_decimal;

/// The longest and shortest hold, in minutes (the hub rejects others).
const MIN_MINUTES: u32 = 1;
const MAX_MINUTES: u32 = 1440;
/// The length a step switched to "for some minutes" starts at.
const DEFAULT_MINUTES: u32 = 60;
/// The time of day a step switched to "until a time" starts at.
const DEFAULT_UNTIL: &str = "07:00";
/// The temperature a hold switched back on starts at, and the range the stepper offers.
const DEFAULT_SETPOINT: f64 = 20.0;
const MIN_SETPOINT: f64 = 5.0;
const MAX_SETPOINT: f64 = 30.0;
const SETPOINT_STEP: f64 = 0.5;

/// How long a step lasts, as the sheet's two choices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Span {
    Minutes,
    Until,
}

/// The rooms of a climate step; empty for any other kind.
fn zones_of(kind: &NodeKind) -> Vec<String> {
    match kind {
        NodeKind::ClimateHold(p) => p.zones.clone(),
        NodeKind::ClimateEco(p) => p.zones.clone(),
        _ => Vec::new(),
    }
}

/// The `(minutes, until)` of a climate step.
fn span_of(kind: &NodeKind) -> (Option<u32>, Option<String>) {
    match kind {
        NodeKind::ClimateHold(p) => (p.minutes, p.until.clone()),
        NodeKind::ClimateEco(p) => (p.minutes, p.until.clone()),
        _ => (None, None),
    }
}

/// Which duration choice a step shows: the time of day when it has one, else minutes.
fn span_choice(kind: &NodeKind) -> Span {
    match span_of(kind) {
        (_, Some(_)) => Span::Until,
        _ => Span::Minutes,
    }
}

/// Stores the duration of a climate step; exactly one of the two is kept.
fn set_span(kind: &mut NodeKind, minutes: Option<u32>, until: Option<String>) {
    match kind {
        NodeKind::ClimateHold(p) => {
            p.minutes = minutes;
            p.until = until;
        }
        NodeKind::ClimateEco(p) => {
            p.minutes = minutes;
            p.until = until;
        }
        _ => {}
    }
}

fn set_zones(kind: &mut NodeKind, zones: Vec<String>) {
    match kind {
        NodeKind::ClimateHold(p) => p.zones = zones,
        NodeKind::ClimateEco(p) => p.zones = zones,
        _ => {}
    }
}

/// `current` moved by `delta` degrees, kept in the stepper's range.
fn stepped(current: f64, delta: f64) -> f64 {
    (current + delta).clamp(MIN_SETPOINT, MAX_SETPOINT)
}

/// A typed temperature, kept in the stepper's range, rounded to the half degree.
fn typed_setpoint(text: &str) -> Option<f64> {
    let c = parse_decimal(text)?;
    Some(((c / SETPOINT_STEP).round() * SETPOINT_STEP).clamp(MIN_SETPOINT, MAX_SETPOINT))
}

/// A typed length in minutes, kept within the limits.
fn typed_minutes(text: &str) -> Option<u32> {
    let m = parse_decimal(text)?;
    Some((m as u32).clamp(MIN_MINUTES, MAX_MINUTES))
}

/// The editor of a `climate.hold` or `climate.eco` node.
pub fn climate_step_sheet(node: RwSignal<Node>) -> impl IntoView {
    let t = use_flow_text();
    let is_hold = node.with_untracked(|n| matches!(n.kind, NodeKind::ClimateHold(_)));
    let write = move |change: &dyn Fn(&mut NodeKind)| node.update(|n| change(&mut n.kind));

    // The rooms the hub has a heating zone for, by name; loaded once.
    let known = RwSignal::new(Vec::<(String, String)>::new());
    leptos::task::spawn_local(async move {
        if let Ok(climate) = get_climate().await
            && climate.enabled
        {
            known.try_set(
                climate
                    .zones
                    .iter()
                    .map(|z| (z.zone.clone(), z.name.clone()))
                    .collect(),
            );
        }
    });
    let zones = Memo::new(move |_| node.with(|n| zones_of(&n.kind)));
    // The known zones, then any stored room the hub no longer lists (shown by id, so it can be
    // dropped).
    let choices = Memo::new(move |_| {
        let mut list = known.get();
        for id in zones.get() {
            if !list.iter().any(|(z, _)| *z == id) {
                list.push((id.clone(), id));
            }
        }
        list
    });

    let span = Memo::new(move |_| node.with(|n| span_choice(&n.kind)));
    let minutes = Memo::new(move |_| node.with(|n| span_of(&n.kind).0));
    let until = Memo::new(move |_| node.with(|n| span_of(&n.kind).1));
    let set_choice = move |choice: Span| {
        write(&|k| match choice {
            Span::Minutes => set_span(k, Some(DEFAULT_MINUTES), None),
            Span::Until => set_span(k, None, Some(DEFAULT_UNTIL.to_owned())),
        });
    };
    let on_minutes = move |ev: web_sys::Event| {
        if let Some(m) = typed_minutes(&event_target_value(&ev)) {
            write(&|k| set_span(k, Some(m), None));
        }
    };
    let on_until = move |ev: web_sys::Event| {
        let time = event_target_value(&ev);
        if !time.is_empty() {
            write(&|k| set_span(k, None, Some(time.clone())));
        }
    };

    let setpoint = Memo::new(move |_| {
        node.with(|n| match &n.kind {
            NodeKind::ClimateHold(p) => p.setpoint,
            _ => ClimateSetpoint::Off,
        })
    });
    let set_setpoint = move |value: ClimateSetpoint| {
        write(&|k| {
            if let NodeKind::ClimateHold(p) = k {
                p.setpoint = value;
            }
        });
    };
    let degrees = move || match setpoint.get() {
        ClimateSetpoint::Temp(c) => c,
        ClimateSetpoint::Off => DEFAULT_SETPOINT,
    };
    let on_setpoint = move |ev: web_sys::Event| {
        if let Some(c) = typed_setpoint(&event_target_value(&ev)) {
            set_setpoint(ClimateSetpoint::Temp(c));
        }
    };
    let on_off = move |ev: web_sys::Event| {
        set_setpoint(if event_target_checked(&ev) {
            ClimateSetpoint::Off
        } else {
            ClimateSetpoint::Temp(DEFAULT_SETPOINT)
        });
    };

    let spans = [
        (Span::Minutes, FlowText::ClimateForMinutes),
        (Span::Until, FlowText::ClimateUntilTime),
    ];
    view! {
        <div class="node-editor__field">
            <span class="node-editor__label">{move || t(FlowText::ClimateZones)}</span>
            <div class="segmented" role="group" aria-label=move || t(FlowText::ClimateZones)>
                <For
                    each=move || choices.get()
                    key=|(id, name)| (id.clone(), name.clone())
                    children=move |(id, name)| {
                        let pick = id.clone();
                        let on = id.clone();
                        view! {
                            <button
                                type="button"
                                class="chip segmented__option"
                                class:chip--selected=move || zones.with(|z| z.contains(&on))
                                on:click={move |_| {
                                    write(&|k| set_zones(k, toggled(&zones_of(k), &pick)))
                                }}
                            >
                                {name}
                            </button>
                        }
                    }
                />
            </div>
            <Show when=move || choices.with(Vec::is_empty)>
                <p class="node-editor__hint">{move || t(FlowText::ClimateNoZones)}</p>
            </Show>
        </div>
        <Show when=move || is_hold>
            <div class="node-editor__field">
                <span class="node-editor__label">{move || t(FlowText::ClimateSetpoint)}</span>
                <Show when=move || setpoint.get() != ClimateSetpoint::Off>
                    <div class="segmented" role="group">
                        <button
                            type="button"
                            class="chip segmented__option"
                            aria-label="-"
                            on:click={move |_| {
                                set_setpoint(ClimateSetpoint::Temp(stepped(degrees(), -SETPOINT_STEP)))
                            }}
                        >
                            "−"
                        </button>
                        <input
                            type="number"
                            class="field"
                            step=SETPOINT_STEP.to_string()
                            min=MIN_SETPOINT.to_string()
                            max=MAX_SETPOINT.to_string()
                            prop:value=move || degrees().to_string()
                            on:change=on_setpoint
                        />
                        <button
                            type="button"
                            class="chip segmented__option"
                            aria-label="+"
                            on:click={move |_| {
                                set_setpoint(ClimateSetpoint::Temp(stepped(degrees(), SETPOINT_STEP)))
                            }}
                        >
                            "+"
                        </button>
                    </div>
                </Show>
                <label class="node-editor__check">
                    <input
                        type="checkbox"
                        prop:checked=move || setpoint.get() == ClimateSetpoint::Off
                        on:change=on_off
                    />
                    <span>{move || t(FlowText::ClimateSetpointOff)}</span>
                </label>
            </div>
        </Show>
        <div class="node-editor__field">
            <span class="node-editor__label">{move || t(FlowText::ClimateDuration)}</span>
            <div class="segmented" role="group" aria-label=move || t(FlowText::ClimateDuration)>
                {spans
                    .into_iter()
                    .map(|(choice, label)| {
                        view! {
                            <button
                                type="button"
                                class="chip segmented__option"
                                class:chip--selected=move || span.get() == choice
                                on:click={move |_| set_choice(choice)}
                            >
                                {move || t(label)}
                            </button>
                        }
                    })
                    .collect_view()}
            </div>
            <Show
                when=move || span.get() == Span::Minutes
                fallback=move || {
                    view! {
                        <input
                            type="time"
                            class="field"
                            prop:value=move || until.get().unwrap_or_default()
                            on:change=on_until
                        />
                    }
                }
            >
                <input
                    type="number"
                    class="field"
                    min=MIN_MINUTES.to_string()
                    max=MAX_MINUTES.to_string()
                    prop:value=move || minutes.get().unwrap_or(DEFAULT_MINUTES).to_string()
                    on:change=on_minutes
                />
            </Show>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use nidoo_core::flow::{ClimateEcoParams, ClimateHoldParams};

    use super::*;

    fn hold(minutes: Option<u32>, until: Option<&str>) -> NodeKind {
        NodeKind::ClimateHold(ClimateHoldParams {
            zones: vec!["living".into()],
            setpoint: ClimateSetpoint::Temp(21.0),
            mode: Some("heat".into()),
            minutes,
            until: until.map(Into::into),
            extra: Default::default(),
        })
    }

    #[test]
    fn a_step_shows_the_duration_choice_it_stores() {
        assert_eq!(span_choice(&hold(Some(90), None)), Span::Minutes);
        assert_eq!(span_choice(&hold(None, Some("10:00"))), Span::Until);
    }

    #[test]
    fn switching_the_duration_keeps_exactly_one_of_the_two() {
        let mut k = hold(Some(90), None);
        set_span(&mut k, None, Some("10:00".into()));
        assert_eq!(span_of(&k), (None, Some("10:00".to_owned())));
        let mut eco = NodeKind::ClimateEco(ClimateEcoParams::default());
        set_span(&mut eco, Some(30), None);
        assert_eq!(span_of(&eco), (Some(30), None));
    }

    #[test]
    fn editing_the_rooms_keeps_the_rest_of_the_step() {
        let mut k = hold(Some(90), None);
        set_zones(&mut k, vec!["bedroom".into()]);
        let NodeKind::ClimateHold(p) = &k else {
            unreachable!()
        };
        assert_eq!(p.zones, ["bedroom"]);
        assert_eq!(p.mode.as_deref(), Some("heat"));
        assert_eq!(p.setpoint, ClimateSetpoint::Temp(21.0));
    }

    #[test]
    fn the_setpoint_stays_in_range_and_on_half_degrees() {
        assert_eq!(stepped(29.8, 0.5), MAX_SETPOINT);
        assert_eq!(stepped(5.2, -0.5), MIN_SETPOINT);
        assert_eq!(stepped(20.0, 0.5), 20.5);
        assert_eq!(typed_setpoint("21.3"), Some(21.5));
        assert_eq!(typed_setpoint("99"), Some(MAX_SETPOINT));
        assert_eq!(typed_setpoint("warm"), None);
    }

    #[test]
    fn typed_minutes_stay_within_the_limits() {
        assert_eq!(typed_minutes("0"), Some(MIN_MINUTES));
        assert_eq!(typed_minutes("90"), Some(90));
        assert_eq!(typed_minutes("5000"), Some(MAX_MINUTES));
        assert_eq!(typed_minutes(""), None);
    }
}
