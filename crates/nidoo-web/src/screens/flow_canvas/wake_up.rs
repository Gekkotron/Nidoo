//! The `trigger.wake_up` settings sheet: minutes before the alarm, which wake-up alarms it
//! follows (none picked means every one) and when it starts next.

use chrono::DateTime;
use chrono_tz::Tz;
use leptos::prelude::*;
use nidoo_api::FlowView;
use nidoo_core::flow::wake_up::{alarm_schedules, is_managed, next_pre_ring};
use nidoo_core::flow::{Flow, Node, NodeKind, TriggerWakeUpParams};

use super::catch_up::CatchUpField;
use super::sun::{fire_text, house_place, now_utc};
use crate::house::House;
use crate::i18n::flows::{FlowText, use_flow_text};
use crate::i18n::format::parse_decimal;
use crate::i18n::use_lang;

/// The most minutes before an alarm the trigger can start (the hub rejects more).
pub const MAX_BEFORE_MINUTES: u32 = 240;

/// The definitions of the flows the hub accepts: an invalid flow is a stub the engine never
/// arms, so its alarm must not count.
fn valid_definitions(views: &[FlowView]) -> Vec<Flow> {
    views
        .iter()
        .filter(|f| f.invalid.is_none())
        .map(|f| f.definition.clone())
        .collect()
}

/// The next time the trigger starts after `now`: the alarms it follows (`only`, or every enabled
/// wake-up alarm when empty) ring `before_minutes` later. `None` when no such alarm is left.
pub fn next_start(
    flows: &[Flow],
    only: &[String],
    before_minutes: u32,
    now: &DateTime<Tz>,
) -> Option<DateTime<Tz>> {
    let schedules: Vec<_> = alarm_schedules(flows, only)
        .into_iter()
        .map(|(_, _, schedule)| schedule)
        .collect();
    next_pre_ring(&schedules, before_minutes, now)
}

/// `before` as the stepper stores it: whole minutes from 0 to the limit; `None` for text that is
/// not a number.
fn typed_before(text: &str) -> Option<u32> {
    let m = parse_decimal(text)?;
    Some(m.clamp(0.0, f64::from(MAX_BEFORE_MINUTES)) as u32)
}

/// `alarms` after the user picks or drops `id`; picking keeps the list sorted and without
/// repeats.
pub(super) fn toggled(alarms: &[String], id: &str) -> Vec<String> {
    let mut next: Vec<String> = alarms.iter().filter(|a| *a != id).cloned().collect();
    if next.len() == alarms.len() {
        next.push(id.to_owned());
        next.sort();
    }
    next
}

/// The editor of a `trigger.wake_up` node.
pub fn wake_up_sheet(node: RwSignal<Node>) -> impl IntoView {
    let lang = use_lang();
    let t = use_flow_text();
    let house = expect_context::<House>();
    let params = move || {
        node.with(|n| match &n.kind {
            NodeKind::TriggerWakeUp(p) => p.clone(),
            _ => TriggerWakeUpParams::default(),
        })
    };
    let write = move |change: &dyn Fn(&mut TriggerWakeUpParams)| {
        node.update(|n| {
            if let NodeKind::TriggerWakeUp(ref mut p) = n.kind {
                change(p);
            }
        });
    };
    let before = Memo::new(move |_| params().before_minutes);
    let alarms = Memo::new(move |_| params().alarms);
    let on_before = move |ev: web_sys::Event| {
        if let Some(m) = typed_before(&event_target_value(&ev)) {
            write(&|p| p.before_minutes = m);
        }
    };
    // Every wake-up alarm, by name; a followed alarm that was deleted stays listed so it can be
    // dropped.
    let choices = Memo::new(move |_| {
        let mut list: Vec<(String, String)> = house.flows.with(|flows| {
            flows
                .iter()
                .filter(|f| is_managed(&f.definition))
                .map(|f| (f.definition.id.clone(), f.definition.name.clone()))
                .collect()
        });
        for id in alarms.get() {
            if !list.iter().any(|(known, _)| *known == id) {
                list.push((id.clone(), id));
            }
        }
        list.sort_by_key(|a| a.1.to_lowercase());
        list
    });
    let next = Memo::new(move |_| {
        let (_, zone) = house.flows_config.with(|c| house_place(c.as_ref()));
        let now = now_utc().with_timezone(&zone);
        let flows = house.flows.with(|v| valid_definitions(v));
        next_start(&flows, &alarms.get(), before.get(), &now)
    });
    let catch_up = Callback::new(move |value: Option<u32>| {
        write(&|p| p.catch_up_minutes = value);
    });
    let initial_catch_up = params().catch_up_minutes;

    view! {
        <label class="node-editor__field">
            <span class="node-editor__label">{move || t(FlowText::WakeUpMinutesBefore)}</span>
            <input
                type="number"
                class="field"
                min="0"
                max=MAX_BEFORE_MINUTES.to_string()
                prop:value=move || before.get().to_string()
                on:change=on_before
            />
        </label>
        <div class="node-editor__field">
            <span class="node-editor__label">{move || t(FlowText::WakeUpAlarms)}</span>
            <div class="segmented" role="group" aria-label=move || t(FlowText::WakeUpAlarms)>
                <button
                    type="button"
                    class="chip segmented__option"
                    class:chip--selected=move || alarms.with(Vec::is_empty)
                    on:click={move |_| write(&|p| p.alarms.clear())}
                >
                    {move || t(FlowText::WakeUpAllAlarms)}
                </button>
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
                                class:chip--selected=move || alarms.with(|a| a.contains(&on))
                                on:click={move |_| {
                                    write(&|p| p.alarms = toggled(&p.alarms, &pick))
                                }}
                            >
                                {name}
                            </button>
                        }
                    }
                />
            </div>
            <Show when=move || choices.with(Vec::is_empty)>
                <p class="node-editor__hint">{move || t(FlowText::WakeUpNoAlarms)}</p>
            </Show>
        </div>
        <div class="node-editor__field">
            <span class="node-editor__label">{move || t(FlowText::WakeUpNextStart)}</span>
            <p class="node-editor__hint">
                {move || match next.get() {
                    Some(at) => fire_text(lang(), at),
                    None => t(FlowText::WakeUpNoNextStart).to_owned(),
                }}
            </p>
        </div>
        <CatchUpField initial=initial_catch_up write=catch_up />
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, TimeZone};
    use nidoo_core::flow::wake_up::{WakeUp, to_flow};

    use super::*;

    fn alarm(id: &str, enabled: bool, hour: u8, minute: u8) -> Flow {
        to_flow(
            id,
            id,
            enabled,
            &WakeUp {
                version: 1,
                hour,
                minute,
                days: vec![0, 1, 2, 3, 4, 5, 6],
                player: "media.player".into(),
                playlist: "favorites".into(),
                shuffle: false,
                source: None,
                fade_in_s: 0,
                speakers: Vec::new(),
                muted: Vec::new(),
            },
        )
    }

    fn at(h: u32, m: u32) -> DateTime<Tz> {
        chrono_tz::Europe::Paris
            .with_ymd_and_hms(2026, 1, 5, h, m, 0)
            .unwrap()
    }

    #[test]
    fn next_start_is_the_alarm_minus_the_minutes() {
        let flows = [alarm("wake-up-1", true, 7, 0)];
        let got = next_start(&flows, &[], 45, &at(5, 0)).unwrap();
        assert_eq!(got, at(6, 15));
    }

    #[test]
    fn next_start_rolls_to_tomorrow_once_past() {
        let flows = [alarm("wake-up-1", true, 7, 0)];
        let got = next_start(&flows, &[], 45, &at(6, 30)).unwrap();
        assert_eq!(got, at(6, 15) + Duration::days(1));
    }

    #[test]
    fn next_start_follows_only_the_picked_enabled_alarms() {
        let flows = [
            alarm("wake-up-1", true, 7, 0),
            alarm("wake-up-2", true, 9, 0),
            alarm("wake-up-3", false, 6, 0),
        ];
        let only = vec!["wake-up-2".to_owned()];
        assert_eq!(next_start(&flows, &only, 0, &at(5, 0)), Some(at(9, 0)));
        // A disabled alarm is never followed, even when it is the only one picked.
        let off = vec!["wake-up-3".to_owned()];
        assert_eq!(next_start(&flows, &off, 0, &at(5, 0)), None);
        // Every enabled alarm when none is picked.
        assert_eq!(next_start(&flows, &[], 0, &at(5, 0)), Some(at(7, 0)));
    }

    #[test]
    fn invalid_flows_are_left_out_of_the_next_start() {
        let view = |definition: Flow, invalid: Option<&str>| FlowView {
            definition,
            invalid: invalid.map(str::to_owned),
            active_run: None,
            runs: Vec::new(),
        };
        let views = [
            view(alarm("wake-up-1", true, 7, 0), Some("broken")),
            view(alarm("wake-up-2", true, 9, 0), None),
        ];
        let flows = valid_definitions(&views);
        assert_eq!(flows.len(), 1);
        assert_eq!(next_start(&flows, &[], 0, &at(5, 0)), Some(at(9, 0)));
    }

    #[test]
    fn toggling_picks_and_drops_an_alarm() {
        let picked = toggled(&[], "b");
        assert_eq!(picked, ["b"]);
        assert_eq!(toggled(&picked, "a"), ["a", "b"]);
        assert_eq!(toggled(&["a".into(), "b".into()], "a"), ["b"]);
    }

    #[test]
    fn typed_minutes_clamp_to_the_limit() {
        assert_eq!(typed_before("45"), Some(45));
        assert_eq!(typed_before("500"), Some(MAX_BEFORE_MINUTES));
        assert_eq!(typed_before("x"), None);
    }
}
