//! The combined state of a group (Appendix A.4): what the hub publishes on the group's state
//! topic from its members' states.

use nidoo_api::EntityView;
use nidoo_core::model::{Kind, StateValue};
use serde_json::{Value, json};

/// A thermostat-like group's state, shared by climate and water heater.
#[derive(Clone, Debug, PartialEq)]
pub struct Thermostat {
    pub target: Option<f64>,
    /// `[lowest, highest]` member target, only when the targets differ.
    pub range: Option<[f64; 2]>,
    /// The common mode; `None` when the members disagree or none reports one.
    pub mode: Option<String>,
}

/// The combined state of a group, one variant per kind that has a state. A button has none.
#[derive(Clone, Debug, PartialEq)]
pub enum GroupState {
    Light {
        on: bool,
        brightness: Option<u8>,
        hs: Option<[f64; 2]>,
        color_temp: Option<f64>,
    },
    Switch {
        on: bool,
    },
    Cover {
        state: &'static str,
        position: Option<u8>,
        text: String,
    },
    Climate(Thermostat),
    WaterHeater(Thermostat),
    Number(f64),
    Select(Option<String>),
    Vacuum(String),
}

fn mean(values: &[f64]) -> Option<f64> {
    (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
}

/// The value every item has, else `None`.
fn common<T: PartialEq + Clone>(items: &[T]) -> Option<T> {
    let first = items.first()?;
    items.iter().all(|i| i == first).then(|| first.clone())
}

fn caps_all(members: &[EntityView], has: impl Fn(&EntityView) -> bool) -> bool {
    !members.is_empty() && members.iter().all(has)
}

fn largest(members: &[EntityView], get: impl Fn(&EntityView) -> Option<f64>) -> Option<f64> {
    members.iter().filter_map(get).reduce(f64::max)
}

fn light(members: &[EntityView]) -> Option<GroupState> {
    let lit: Vec<&nidoo_core::model::LightState> = members
        .iter()
        .filter_map(|m| match &m.state.value {
            StateValue::Light(l) => Some(l),
            _ => None,
        })
        .collect();
    if lit.is_empty() {
        return None;
    }
    let on: Vec<_> = lit.iter().filter(|l| l.on).collect();
    let collect = |get: &dyn Fn(&nidoo_core::model::LightState) -> Option<f64>| {
        mean(&on.iter().filter_map(|l| get(l)).collect::<Vec<_>>())
    };
    let shared_brightness = caps_all(members, |m| m.capabilities.brightness);
    let shared_colour = caps_all(members, |m| {
        ["hs", "xy", "rgb", "rgbw", "rgbww"]
            .iter()
            .any(|mode| m.capabilities.color_modes.contains(*mode))
    });
    let shared_temp = caps_all(members, |m| {
        m.capabilities.color_modes.contains("color_temp")
    });
    let hue = collect(&|l| l.hs.map(|c| c.h));
    let sat = collect(&|l| l.hs.map(|c| c.s));
    Some(GroupState::Light {
        on: !on.is_empty(),
        brightness: collect(&|l| l.brightness.map(f64::from))
            .filter(|_| shared_brightness)
            .map(|b| b.round() as u8),
        hs: hue.zip(sat).filter(|_| shared_colour).map(|(h, s)| [h, s]),
        color_temp: collect(&|l| l.color_temp.map(f64::from)).filter(|_| shared_temp),
    })
}

fn cover(members: &[EntityView]) -> Option<GroupState> {
    let covers: Vec<_> = members
        .iter()
        .filter_map(|m| match &m.state.value {
            StateValue::Cover(c) => Some(c),
            _ => None,
        })
        .collect();
    if covers.is_empty() {
        return None;
    }
    let count = |wanted: &str| {
        covers
            .iter()
            .filter(|c| c.state.as_deref() == Some(wanted))
            .count()
    };
    let (opening, closing, open, closed) = (
        count("opening"),
        count("closing"),
        count("open"),
        count("closed"),
    );
    let (state, text) = if opening > 0 {
        ("opening", "Opening…".to_owned())
    } else if closing > 0 {
        ("closing", "Closing…".to_owned())
    } else {
        let text = if open == covers.len() {
            "All open".to_owned()
        } else if closed == covers.len() {
            "All closed".to_owned()
        } else {
            format!("{open} open · {closed} closed")
        };
        (if open > 0 { "open" } else { "closed" }, text)
    };
    let positions: Vec<f64> = covers
        .iter()
        .filter_map(|c| c.position.map(f64::from))
        .collect();
    Some(GroupState::Cover {
        state,
        position: mean(&positions)
            .filter(|_| caps_all(members, |m| m.capabilities.position))
            .map(|p| p.round() as u8),
        text,
    })
}

fn thermostat(members: &[EntityView]) -> Option<Thermostat> {
    let readings: Vec<(Option<f64>, Option<String>)> = members
        .iter()
        .filter_map(|m| match &m.state.value {
            StateValue::Climate(c) => Some((c.target, c.mode.clone())),
            StateValue::WaterHeater(w) => Some((w.target, w.mode.clone())),
            _ => None,
        })
        .collect();
    if readings.is_empty() {
        return None;
    }
    let targets: Vec<f64> = readings.iter().filter_map(|r| r.0).collect();
    let modes: Vec<String> = readings.iter().filter_map(|r| r.1.clone()).collect();
    let step = largest(members, |m| m.capabilities.temp_step).filter(|s| *s > 0.0);
    let min = largest(members, |m| m.capabilities.min_temp);
    let max = members
        .iter()
        .filter_map(|m| m.capabilities.max_temp)
        .reduce(f64::min);
    let target = mean(&targets).map(|t| {
        let t = step.map_or(t, |s| (t / s).round() * s);
        let t = min.map_or(t, |lo| t.max(lo));
        max.map_or(t, |hi| t.min(hi))
    });
    let low = targets.iter().copied().reduce(f64::min);
    let high = targets.iter().copied().reduce(f64::max);
    Some(Thermostat {
        target,
        range: low
            .zip(high)
            .filter(|(lo, hi)| lo != hi)
            .map(|(lo, hi)| [lo, hi]),
        mode: common(&modes),
    })
}

/// The combined state of `members`, which are the present, matching entities of a group of
/// `kind`. `None` when no member has reported a state, and always for a button.
pub fn combine(kind: Kind, members: &[EntityView]) -> Option<GroupState> {
    match kind {
        Kind::Light => light(members),
        Kind::Switch => {
            let values: Vec<bool> = members
                .iter()
                .filter_map(|m| match m.state.value {
                    StateValue::OnOff(on) => Some(on),
                    _ => None,
                })
                .collect();
            (!values.is_empty()).then(|| GroupState::Switch {
                on: values.contains(&true),
            })
        }
        Kind::Cover => cover(members),
        Kind::Climate => thermostat(members).map(GroupState::Climate),
        Kind::WaterHeater => thermostat(members).map(GroupState::WaterHeater),
        Kind::Number => {
            let values: Vec<f64> = members
                .iter()
                .filter_map(|m| match m.state.value {
                    StateValue::Number(n) => Some(n),
                    _ => None,
                })
                .collect();
            let step = largest(members, |m| m.capabilities.step).filter(|s| *s > 0.0);
            let value = common(&values)
                .or_else(|| mean(&values).map(|v| step.map_or(v, |s| (v / s).round() * s)));
            value.map(GroupState::Number)
        }
        Kind::Select => {
            let options: Vec<String> = members
                .iter()
                .filter_map(|m| match &m.state.value {
                    StateValue::Text(t) => Some(t.clone()),
                    _ => None,
                })
                .collect();
            (!options.is_empty()).then(|| GroupState::Select(common(&options)))
        }
        Kind::Vacuum => {
            let states: Vec<String> = members
                .iter()
                .filter_map(|m| match &m.state.value {
                    StateValue::Vacuum(v) => v.state.clone(),
                    _ => None,
                })
                .collect();
            let has = |wanted: &str| states.iter().any(|s| s == wanted);
            let state = if has("cleaning") {
                "cleaning".to_owned()
            } else if has("returning") {
                "returning".to_owned()
            } else if !states.is_empty() && states.iter().all(|s| s == "docked") {
                "docked".to_owned()
            } else {
                states.first()?.clone()
            };
            Some(GroupState::Vacuum(state))
        }
        Kind::Button
        | Kind::Sensor
        | Kind::BinarySensor
        | Kind::AlarmPanel
        | Kind::MediaPlayer
        | Kind::Generic => None,
    }
}

fn thermostat_json(t: &Thermostat) -> Value {
    let mut state = json!({ "mode": t.mode });
    if let Some(target) = t.target {
        state["target"] = json!(target);
    }
    if let Some(range) = t.range {
        state["range"] = json!(range);
    }
    state
}

/// The JSON payload published on the group's state topic.
pub fn encode(state: &GroupState) -> String {
    let value = match state {
        GroupState::Light {
            on,
            brightness,
            hs,
            color_temp,
        } => {
            let mut value = json!({ "state": if *on { "ON" } else { "OFF" } });
            if *on {
                if let Some(brightness) = brightness {
                    value["brightness"] = json!(brightness);
                }
                if let Some(hs) = hs {
                    value["hs"] = json!(hs);
                }
                if let Some(color_temp) = color_temp {
                    value["color_temp"] = json!(color_temp);
                }
            }
            value
        }
        GroupState::Switch { on } => json!({ "state": if *on { "ON" } else { "OFF" } }),
        GroupState::Cover {
            state,
            position,
            text,
        } => {
            let mut value = json!({ "state": state, "text": text });
            if let Some(position) = position {
                value["position"] = json!(position);
            }
            value
        }
        GroupState::Climate(t) | GroupState::WaterHeater(t) => thermostat_json(t),
        GroupState::Number(value) => json!({ "value": value }),
        GroupState::Select(option) => json!({ "option": option }),
        GroupState::Vacuum(state) => json!({ "state": state }),
    };
    value.to_string()
}
#[cfg(test)]
mod tests;
