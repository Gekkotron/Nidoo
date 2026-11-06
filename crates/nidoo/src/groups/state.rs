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
mod tests {
    use super::*;
    use nidoo_core::model::{
        Capabilities, ClimateState, CoverState, EntityCategory, EntityState, LightState,
        UiOverride, VacuumState,
    };
    use serde_json::{Value, json};

    fn member(kind: Kind, value: StateValue, caps: Capabilities) -> EntityView {
        EntityView {
            id: "m".to_owned(),
            device_id: None,
            name: "m".to_owned(),
            device_name: None,
            label: None,
            kind,
            component: kind.component().to_string(),
            protocol_kind: kind,
            device_class: None,
            unit: None,
            icon: None,
            category: EntityCategory::Primary,
            enabled_by_default: true,
            area: String::new(),
            part_of: None,
            capabilities: caps,
            ui: UiOverride::default(),
            state: EntityState {
                value,
                ..Default::default()
            },
        }
    }

    fn encoded(kind: Kind, members: &[EntityView]) -> Value {
        let state = combine(kind, members).expect("a state");
        serde_json::from_str(&encode(&state)).unwrap()
    }

    fn switch(on: bool) -> EntityView {
        member(Kind::Switch, StateValue::OnOff(on), Capabilities::default())
    }

    fn cover(state: &str) -> EntityView {
        member(
            Kind::Cover,
            StateValue::Cover(CoverState {
                state: Some(state.to_owned()),
                position: None,
            }),
            Capabilities::default(),
        )
    }

    fn climate(mode: &str, target: f64) -> EntityView {
        member(
            Kind::Climate,
            StateValue::Climate(ClimateState {
                mode: Some(mode.to_owned()),
                target: Some(target),
                ..Default::default()
            }),
            Capabilities::default(),
        )
    }

    #[test]
    fn switch_is_on_when_any_member_is_on() {
        assert_eq!(
            encoded(Kind::Switch, &[switch(true), switch(false)]),
            json!({"state":"ON"})
        );
        assert_eq!(
            encoded(Kind::Switch, &[switch(false), switch(false)]),
            json!({"state":"OFF"})
        );
    }

    #[test]
    fn nothing_is_published_without_a_reported_member() {
        assert!(combine(Kind::Switch, &[]).is_none());
        let unknown = member(Kind::Switch, StateValue::Unknown, Capabilities::default());
        assert!(combine(Kind::Switch, &[unknown]).is_none());
        assert!(combine(Kind::Button, &[switch(true)]).is_none());
    }

    #[test]
    fn light_means_cover_the_members_that_are_on() {
        let caps = Capabilities {
            brightness: true,
            ..Default::default()
        };
        let light = |on, brightness| {
            member(
                Kind::Light,
                StateValue::Light(LightState {
                    on,
                    brightness: Some(brightness),
                    ..Default::default()
                }),
                caps.clone(),
            )
        };
        let state = encoded(
            Kind::Light,
            &[light(true, 100), light(true, 201), light(false, 0)],
        );
        assert_eq!(state, json!({"state":"ON","brightness":151}));
        let off = encoded(Kind::Light, &[light(false, 0)]);
        assert_eq!(off, json!({"state":"OFF"}));
    }

    #[test]
    fn cover_opening_wins_and_has_text() {
        let state = encoded(
            Kind::Cover,
            &[cover("open"), cover("opening"), cover("closing")],
        );
        assert_eq!(state["state"], "opening");
        assert_eq!(state["text"], "Opening…");
        let state = encoded(Kind::Cover, &[cover("closed"), cover("closing")]);
        assert_eq!(
            (state["state"].as_str(), state["text"].as_str()),
            (Some("closing"), Some("Closing…"))
        );
    }

    #[test]
    fn cover_texts_for_settled_members() {
        let all_open = encoded(Kind::Cover, &[cover("open"), cover("open")]);
        assert_eq!(
            (all_open["state"].as_str(), all_open["text"].as_str()),
            (Some("open"), Some("All open"))
        );
        let all_closed = encoded(Kind::Cover, &[cover("closed")]);
        assert_eq!(
            (all_closed["state"].as_str(), all_closed["text"].as_str()),
            (Some("closed"), Some("All closed"))
        );
        let split = encoded(
            Kind::Cover,
            &[cover("open"), cover("closed"), cover("closed")],
        );
        assert_eq!(
            (split["state"].as_str(), split["text"].as_str()),
            (Some("open"), Some("1 open · 2 closed"))
        );
    }

    #[test]
    fn cover_position_is_the_rounded_mean_when_shared() {
        let at = |position| {
            member(
                Kind::Cover,
                StateValue::Cover(CoverState {
                    state: Some("open".to_owned()),
                    position: Some(position),
                }),
                Capabilities {
                    position: true,
                    ..Default::default()
                },
            )
        };
        assert_eq!(encoded(Kind::Cover, &[at(10), at(25)])["position"], 18);
    }

    #[test]
    fn climate_common_mode_and_range() {
        let state = encoded(
            Kind::Climate,
            &[climate("heat", 19.0), climate("heat", 21.0)],
        );
        assert_eq!(state["mode"], "heat");
        assert_eq!(state["target"], 20.0);
        assert_eq!(state["range"], json!([19.0, 21.0]));
        let same = encoded(
            Kind::Climate,
            &[climate("heat", 20.0), climate("heat", 20.0)],
        );
        assert!(same.get("range").is_none());
    }

    #[test]
    fn climate_mixed_modes_encode_null() {
        let state = encoded(
            Kind::Climate,
            &[climate("heat", 20.0), climate("off", 20.0)],
        );
        assert_eq!(state["mode"], Value::Null);
    }

    #[test]
    fn climate_target_is_rounded_to_the_step_and_clamped() {
        let caps = Capabilities {
            min_temp: Some(5.0),
            max_temp: Some(21.0),
            temp_step: Some(0.5),
            ..Default::default()
        };
        let with = |mut view: EntityView| {
            view.capabilities = caps.clone();
            view
        };
        let state = encoded(
            Kind::Climate,
            &[with(climate("heat", 20.0)), with(climate("heat", 23.0))],
        );
        assert_eq!(state["target"], 21.0);
    }

    #[test]
    fn number_and_select_share_a_common_value() {
        let number = |v| member(Kind::Number, StateValue::Number(v), Capabilities::default());
        assert_eq!(
            encoded(Kind::Number, &[number(3.0), number(3.0)]),
            json!({"value":3.0})
        );
        assert_eq!(
            encoded(Kind::Number, &[number(1.0), number(2.0)]),
            json!({"value":1.5})
        );
        let select = |o: &str| {
            member(
                Kind::Select,
                StateValue::Text(o.to_owned()),
                Capabilities::default(),
            )
        };
        assert_eq!(
            encoded(Kind::Select, &[select("a"), select("a")]),
            json!({"option":"a"})
        );
        assert_eq!(
            encoded(Kind::Select, &[select("a"), select("b")]),
            json!({"option":null})
        );
    }

    #[test]
    fn vacuum_precedence() {
        let vacuum = |s: &str| {
            member(
                Kind::Vacuum,
                StateValue::Vacuum(VacuumState {
                    state: Some(s.to_owned()),
                    ..Default::default()
                }),
                Capabilities::default(),
            )
        };
        assert_eq!(
            encoded(Kind::Vacuum, &[vacuum("docked"), vacuum("cleaning")])["state"],
            "cleaning"
        );
        assert_eq!(
            encoded(Kind::Vacuum, &[vacuum("docked"), vacuum("returning")])["state"],
            "returning"
        );
        assert_eq!(
            encoded(Kind::Vacuum, &[vacuum("docked"), vacuum("docked")])["state"],
            "docked"
        );
        assert_eq!(
            encoded(Kind::Vacuum, &[vacuum("paused"), vacuum("docked")])["state"],
            "paused"
        );
    }
}
