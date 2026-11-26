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
    let interrupted = matches!(
        decision.source,
        Source::Safety | Source::Override | Source::Boost
    );
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
mod tests;
