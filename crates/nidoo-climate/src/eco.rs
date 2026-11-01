//! Eco's extra actions (spec §7): which fan modes to set when a zone enters eco, and which to
//! put back when it leaves.
//!
//! Pure. The hub passes the memory kept in the previous decision, the new decision and the
//! devices, sends the returned commands and keeps the returned memory in the new decision.

use std::collections::BTreeMap;

use crate::decide::Device;
use crate::doc::{Decision, EcoAction, FanRestore, FanState, Source, fan_mode_of};

/// A fan mode to send to one climate entity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FanCommand {
    pub entity: String,
    pub fan_mode: String,
}

/// The fan commands for `decision` and the memory to keep with it. `previous` is the memory of
/// the zone's previous decision; `zone_off` puts every remembered fan back, once.
///
/// Within one eco period each unit is set at most once: a unit already at the eco value, or one
/// the user moves, is released and left alone until eco ends. A unit whose fan is unknown keeps
/// its memory until the fan is known again.
pub fn fan_plan(
    previous: &BTreeMap<String, FanRestore>,
    decision: &Decision,
    devices: &[Device],
    actions: &[EcoAction],
    zone_off: bool,
) -> (BTreeMap<String, FanRestore>, Vec<FanCommand>) {
    let mut memory = previous.clone();
    let mut commands = Vec::new();
    let in_eco = matches!(decision.source, Source::Eco | Source::Away);
    let interrupted = matches!(decision.source, Source::Safety | Source::Override);
    if !zone_off && interrupted {
        return (memory, commands);
    }
    let want = fan_mode_of(actions);
    for device in devices {
        let id = &device.id;
        let Some(current) = device.fan_mode.clone() else {
            continue;
        };
        let mut send = |fan_mode: &str| {
            commands.push(FanCommand {
                entity: id.clone(),
                fan_mode: fan_mode.to_owned(),
            })
        };
        let entry = memory.get(id).cloned();
        let listed = want.filter(|v| device.caps.fan_modes.iter().any(|m| m == v));
        let Some(v) = listed.filter(|_| in_eco && !zone_off) else {
            if let Some(r) = entry {
                if r.state != FanState::Released && current == r.set {
                    send(&r.was);
                }
                memory.remove(id);
            }
            continue;
        };
        let driven = decision
            .entities
            .get(id)
            .is_some_and(|t| !t.held && t.mode != "off");
        if !driven {
            continue;
        }
        let sent = |was: String, from: String| FanRestore {
            was,
            set: v.to_owned(),
            state: FanState::Sent { from },
        };
        let next = match entry {
            None if current == v => FanRestore {
                was: current,
                set: v.to_owned(),
                state: FanState::Released,
            },
            None => {
                send(v);
                sent(current.clone(), current)
            }
            Some(r) => match r.state.clone() {
                FanState::Released => r,
                // At what eco set (or seen landing): done, unless the eco value changed.
                FanState::Set | FanState::Sent { .. } if current == r.set => {
                    if r.set == v {
                        FanRestore {
                            state: FanState::Set,
                            ..r
                        }
                    } else {
                        send(v);
                        sent(r.was, r.set)
                    }
                }
                // Still where it was when sent: the command did not land, send it again.
                FanState::Sent { from } if current == from => {
                    send(v);
                    sent(r.was, from)
                }
                // Anywhere else: the user moved it, never undone.
                _ => FanRestore {
                    state: FanState::Released,
                    ..r
                },
            },
        };
        memory.insert(id.clone(), next);
    }
    (memory, commands)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decide::DeviceCaps;
    use crate::doc::DeviceTarget;

    fn device(id: &str, fan: Option<&str>, modes: &[&str]) -> Device {
        Device {
            id: id.into(),
            caps: DeviceCaps {
                hvac_modes: vec!["heat".into(), "off".into()],
                fan_modes: modes.iter().map(|m| m.to_string()).collect(),
                min_temp: None,
                max_temp: None,
                temp_step: None,
            },
            target: Some(16.0),
            mode: Some("heat".into()),
            fan_mode: fan.map(str::to_owned),
        }
    }

    fn unit(fan: &str) -> Device {
        device("a", Some(fan), &["auto", "low", "high"])
    }

    fn decision(source: Source, targets: &[(&str, &str, bool)]) -> Decision {
        Decision {
            zone: "z".into(),
            mode: None,
            setpoint: None,
            source,
            cause: None,
            reason: String::new(),
            reading: None,
            reading_age_s: None,
            outdoor_mean: None,
            home: true,
            applied: false,
            at: 0,
            entities: targets
                .iter()
                .map(|(id, mode, held)| {
                    (
                        id.to_string(),
                        DeviceTarget {
                            setpoint: Some(16.0),
                            mode: mode.to_string(),
                            held: *held,
                        },
                    )
                })
                .collect(),
            eco_restore: BTreeMap::new(),
            adjust: None,
        }
    }

    fn eco() -> Decision {
        decision(Source::Eco, &[("a", "heat", false)])
    }

    fn schedule() -> Decision {
        decision(Source::Schedule, &[("a", "heat", false)])
    }

    fn want(m: &str) -> Vec<EcoAction> {
        vec![EcoAction::FanMode(m.into())]
    }

    fn entry(was: &str, set: &str, state: FanState) -> BTreeMap<String, FanRestore> {
        BTreeMap::from([(
            "a".to_string(),
            FanRestore {
                was: was.into(),
                set: set.into(),
                state,
            },
        )])
    }

    fn set(was: &str, v: &str) -> BTreeMap<String, FanRestore> {
        entry(was, v, FanState::Set)
    }

    fn sent(was: &str, v: &str, from: &str) -> BTreeMap<String, FanRestore> {
        entry(was, v, FanState::Sent { from: from.into() })
    }

    fn released(was: &str, v: &str) -> BTreeMap<String, FanRestore> {
        entry(was, v, FanState::Released)
    }

    fn cmd(m: &str) -> Vec<FanCommand> {
        vec![FanCommand {
            entity: "a".into(),
            fan_mode: m.into(),
        }]
    }

    fn plan(
        previous: &BTreeMap<String, FanRestore>,
        d: &Decision,
        devices: &[Device],
        actions: &[EcoAction],
    ) -> (BTreeMap<String, FanRestore>, Vec<FanCommand>) {
        fan_plan(previous, d, devices, actions, false)
    }

    #[test]
    fn entering_eco_sends_the_fan_and_remembers_it_as_sent() {
        let (mem, cmds) = plan(&BTreeMap::new(), &eco(), &[unit("auto")], &want("low"));
        assert_eq!(cmds, cmd("low"));
        assert_eq!(mem, sent("auto", "low", "auto"));
    }

    #[test]
    fn away_counts_as_eco() {
        let d = decision(Source::Away, &[("a", "heat", false)]);
        let (_, cmds) = plan(&BTreeMap::new(), &d, &[unit("auto")], &want("low"));
        assert_eq!(cmds, cmd("low"));
    }

    #[test]
    fn a_sent_fan_that_landed_is_set() {
        let (mem, cmds) = plan(
            &sent("auto", "low", "auto"),
            &eco(),
            &[unit("low")],
            &want("low"),
        );
        assert!(cmds.is_empty());
        assert_eq!(mem, set("auto", "low"));
    }

    #[test]
    fn a_sent_fan_that_did_not_land_is_sent_again() {
        let (mem, cmds) = plan(
            &sent("auto", "low", "auto"),
            &eco(),
            &[unit("auto")],
            &want("low"),
        );
        assert_eq!(cmds, cmd("low"));
        assert_eq!(mem, sent("auto", "low", "auto"));
    }

    #[test]
    fn a_sent_fan_moved_elsewhere_is_released() {
        let (mem, cmds) = plan(
            &sent("auto", "low", "auto"),
            &eco(),
            &[unit("high")],
            &want("low"),
        );
        assert!(cmds.is_empty());
        assert_eq!(mem, released("auto", "low"));
    }

    #[test]
    fn already_at_eco_value_is_released_and_never_reasserted() {
        let (mem, cmds) = plan(&BTreeMap::new(), &eco(), &[unit("low")], &want("low"));
        assert!(cmds.is_empty());
        assert_eq!(mem, released("low", "low"));
        // The user then moves it: eco leaves it alone for the rest of the period.
        let (mem, cmds) = plan(&mem, &eco(), &[unit("high")], &want("low"));
        assert!(cmds.is_empty());
        let (_, cmds) = plan(&mem, &schedule(), &[unit("high")], &want("low"));
        assert!(cmds.is_empty());
    }

    #[test]
    fn a_fan_moved_during_eco_is_released_for_good() {
        let (mem, cmds) = plan(&set("auto", "low"), &eco(), &[unit("high")], &want("low"));
        assert!(cmds.is_empty());
        assert_eq!(mem, released("auto", "low"));
        let (mem, cmds) = plan(&mem, &eco(), &[unit("high")], &want("low"));
        assert!(cmds.is_empty(), "not re-applied on the next pass");
        let (mem, cmds) = plan(&mem, &schedule(), &[unit("high")], &want("low"));
        assert!(cmds.is_empty(), "nor restored over the user's choice");
        assert!(mem.is_empty());
    }

    #[test]
    fn unit_without_the_value_is_left_alone() {
        let d = device("a", Some("auto"), &["auto"]);
        let (mem, cmds) = plan(&BTreeMap::new(), &eco(), &[d], &want("low"));
        assert!(cmds.is_empty());
        assert!(mem.is_empty());
    }

    #[test]
    fn unknown_fan_state_waits() {
        let d = device("a", None, &["auto", "low"]);
        let (mem, cmds) = plan(
            &BTreeMap::new(),
            &eco(),
            std::slice::from_ref(&d),
            &want("low"),
        );
        assert!(cmds.is_empty());
        assert!(mem.is_empty());
        let (mem, cmds) = plan(
            &set("auto", "low"),
            &eco(),
            std::slice::from_ref(&d),
            &want("low"),
        );
        assert!(cmds.is_empty());
        assert_eq!(mem, set("auto", "low"), "kept in eco");
        let (mem, cmds) = plan(&set("auto", "low"), &schedule(), &[d], &want("low"));
        assert!(cmds.is_empty());
        assert_eq!(mem, set("auto", "low"), "kept until the fan is known");
    }

    #[test]
    fn held_or_off_units_get_no_fan_command() {
        for target in [("a", "heat", true), ("a", "off", false)] {
            let d = decision(Source::Eco, &[target]);
            let (mem, cmds) = plan(&BTreeMap::new(), &d, &[unit("auto")], &want("low"));
            assert!(cmds.is_empty(), "{target:?}");
            assert!(mem.is_empty());
        }
        let not_driven = decision(Source::Eco, &[]);
        let (_, cmds) = plan(&BTreeMap::new(), &not_driven, &[unit("auto")], &want("low"));
        assert!(cmds.is_empty());
    }

    #[test]
    fn a_held_unit_keeps_its_memory() {
        let d = decision(Source::Eco, &[("a", "heat", true)]);
        let (mem, cmds) = plan(&set("auto", "low"), &d, &[unit("low")], &want("low"));
        assert!(cmds.is_empty());
        assert_eq!(mem, set("auto", "low"));
    }

    #[test]
    fn leaving_eco_restores_when_unchanged() {
        for previous in [set("auto", "low"), sent("auto", "low", "auto")] {
            let (mem, cmds) = plan(&previous, &schedule(), &[unit("low")], &want("low"));
            assert_eq!(cmds, cmd("auto"));
            assert!(mem.is_empty());
        }
    }

    #[test]
    fn leaving_eco_with_a_fan_that_never_landed_sends_nothing() {
        let (mem, cmds) = plan(
            &sent("auto", "low", "auto"),
            &schedule(),
            &[unit("auto")],
            &want("low"),
        );
        assert!(cmds.is_empty());
        assert!(mem.is_empty());
    }

    #[test]
    fn leaving_eco_leaves_a_hand_changed_fan() {
        let (mem, cmds) = plan(
            &set("auto", "low"),
            &schedule(),
            &[unit("high")],
            &want("low"),
        );
        assert!(cmds.is_empty());
        assert!(mem.is_empty());
    }

    #[test]
    fn safety_or_override_keeps_memory() {
        for source in [Source::Safety, Source::Override] {
            let d = decision(source, &[("a", "heat", false)]);
            let (mem, cmds) = plan(&set("auto", "low"), &d, &[unit("low")], &want("low"));
            assert!(cmds.is_empty(), "{source:?}");
            assert_eq!(mem, set("auto", "low"));
        }
    }

    #[test]
    fn eco_to_away_sends_nothing_again() {
        let d = decision(Source::Away, &[("a", "heat", false)]);
        let (mem, cmds) = plan(&set("auto", "low"), &d, &[unit("low")], &want("low"));
        assert!(cmds.is_empty());
        assert_eq!(mem, set("auto", "low"));
    }

    #[test]
    fn changing_the_eco_value_moves_units_still_at_the_old_one() {
        let (mem, cmds) = plan(&set("auto", "low"), &eco(), &[unit("low")], &want("high"));
        assert_eq!(cmds, cmd("high"));
        assert_eq!(mem, sent("auto", "high", "low"));
        // Not landed yet: sent again, the original fan still remembered.
        let (mem, cmds) = plan(&mem, &eco(), &[unit("low")], &want("high"));
        assert_eq!(cmds, cmd("high"));
        assert_eq!(mem, sent("auto", "high", "low"));
        let (mem, _) = plan(&mem, &eco(), &[unit("high")], &want("high"));
        let (_, cmds) = plan(&mem, &schedule(), &[unit("high")], &want("high"));
        assert_eq!(cmds, cmd("auto"));
    }

    #[test]
    fn removing_the_action_restores() {
        let (mem, cmds) = plan(&set("auto", "low"), &eco(), &[unit("low")], &[]);
        assert_eq!(cmds, cmd("auto"));
        assert!(mem.is_empty());
    }

    #[test]
    fn zone_off_restores() {
        let (mem, cmds) = fan_plan(
            &set("auto", "low"),
            &eco(),
            &[unit("low")],
            &want("low"),
            true,
        );
        assert_eq!(cmds, cmd("auto"));
        assert!(mem.is_empty());
    }

    #[test]
    fn absent_device_keeps_its_memory() {
        let d = decision(Source::Schedule, &[]);
        let (mem, cmds) = plan(&set("auto", "low"), &d, &[], &want("low"));
        assert!(cmds.is_empty());
        assert_eq!(mem, set("auto", "low"));
    }
}
