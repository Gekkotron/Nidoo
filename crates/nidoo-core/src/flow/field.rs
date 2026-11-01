//! The values inside an entity's state a flow can read besides its headline value (a vacuum's
//! battery, a light's brightness …). Names are the fields of [`crate::model::StateValue`]'s
//! states; a generic entity's are its type's fields.

use crate::flow::Scalar;
use crate::model::{Capabilities, GenericValue, Kind, StateValue};
use crate::types::type_def;
use crate::vocabulary::{Reports, field_reports};

/// One value inside an entity's state.
#[derive(Clone, Debug, PartialEq)]
pub struct FieldInfo {
    pub name: String,
    pub reports: Reports,
    /// The entity's recorded history number, so a flow can average it.
    pub recorded: bool,
}

fn number(unit: Option<&str>, min: Option<f64>, max: Option<f64>) -> Reports {
    Reports::Number {
        unit: unit.map(str::to_owned),
        min,
        max,
    }
}

fn percent() -> Reports {
    number(Some("%"), Some(0.0), Some(100.0))
}

fn words(list: &[String]) -> Reports {
    Reports::Values {
        values: list.iter().cloned().map(Scalar::Text).collect(),
    }
}

fn info(name: &str, reports: Reports, recorded: bool) -> FieldInfo {
    FieldInfo {
        name: name.to_owned(),
        reports,
        recorded,
    }
}

/// The fields an entity of `kind` (discovery `component`) reports besides its headline value.
pub fn fields(
    kind: Kind,
    component: &str,
    caps: &Capabilities,
    unit: Option<&str>,
) -> Vec<FieldInfo> {
    match kind {
        Kind::Vacuum => vec![
            info("battery", number(Some("%"), None, None), false),
            info("fan_speed", words(&caps.fan_speeds), false),
        ],
        Kind::Light => vec![
            info("brightness", percent(), false),
            info("color_temp", number(None, None, None), false),
            info("effect", words(&caps.effects), false),
        ],
        Kind::Cover => vec![info("position", percent(), true)],
        Kind::Climate => vec![
            info("current", number(unit, None, None), true),
            info("target", number(unit, None, None), false),
            info("fan_mode", words(&caps.fan_modes), false),
            info("action", Reports::Text, false),
        ],
        Kind::WaterHeater => vec![
            info("current", number(unit, None, None), true),
            info("target", number(unit, None, None), false),
        ],
        Kind::Generic => {
            let Some(def) = type_def(component) else {
                return Vec::new();
            };
            def.fields
                .keys()
                .filter(|name| def.primary.as_deref() != Some(name.as_str()))
                .map(|name| info(name, field_reports(component, caps, unit, name), false))
                .filter(|field| field.reports != Reports::Nothing)
                .collect()
        }
        _ => Vec::new(),
    }
}

/// The headline value of an entity of discovery `component`: its type's primary field for a
/// type-table entity, else [`Scalar::of_state`].
pub fn headline_scalar(component: &str, value: &StateValue) -> Option<Scalar> {
    match value {
        StateValue::Generic(state) => Scalar::of_generic(type_def(component)?, state),
        other => Scalar::of_state(other),
    }
}

/// Field `field` of a state; `None` for a field the state does not have or has not reported.
pub fn field_scalar(value: &StateValue, field: &str) -> Option<Scalar> {
    let number = |n: Option<f64>| n.map(Scalar::Number);
    let text = |t: &Option<String>| t.clone().map(Scalar::Text);
    match (value, field) {
        (StateValue::Vacuum(v), "battery") => number(v.battery.map(f64::from)),
        (StateValue::Vacuum(v), "fan_speed") => text(&v.fan_speed),
        (StateValue::Light(l), "brightness") => number(
            l.brightness
                .map(|level| f64::from(brightness_percent(level))),
        ),
        (StateValue::Light(l), "color_temp") => number(l.color_temp.map(f64::from)),
        (StateValue::Light(l), "effect") => text(&l.effect),
        (StateValue::Cover(c), "position") => number(c.position.map(f64::from)),
        (StateValue::Climate(c), "current") => number(c.current),
        (StateValue::Climate(c), "target") => number(c.target),
        (StateValue::Climate(c), "fan_mode") => text(&c.fan_mode),
        (StateValue::Climate(c), "action") => text(&c.action),
        (StateValue::WaterHeater(w), "current") => number(w.current),
        (StateValue::WaterHeater(w), "target") => number(w.target),
        (StateValue::Generic(g), name) => Some(match g.fields.get(name)? {
            GenericValue::Bool(b) => Scalar::Bool(*b),
            GenericValue::Number(n) => Scalar::Number(*n),
            GenericValue::Text(t) => Scalar::Text(t.clone()),
        }),
        _ => None,
    }
}

/// A light's 0–255 brightness as a percentage; never 0 while the light gives any light.
pub fn brightness_percent(level: u8) -> u8 {
    let percent = (f64::from(level) * 100.0 / 255.0).round() as u8;
    if level > 0 { percent.max(1) } else { 0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{GenericState, LightState, VacuumState};

    fn names(list: &[FieldInfo]) -> Vec<&str> {
        list.iter().map(|f| f.name.as_str()).collect()
    }

    fn find<'a>(list: &'a [FieldInfo], name: &str) -> &'a FieldInfo {
        list.iter().find(|f| f.name == name).unwrap()
    }

    #[test]
    fn vacuum_lists_battery_and_fan_speed() {
        let caps = Capabilities {
            fan_speeds: vec!["Silent".into(), "Turbo".into()],
            ..Default::default()
        };
        let list = fields(Kind::Vacuum, "vacuum", &caps, None);
        assert_eq!(names(&list), ["battery", "fan_speed"]);
        assert!(matches!(
            &find(&list, "battery").reports,
            Reports::Number { unit: Some(u), .. } if u == "%"
        ));
        assert_eq!(
            find(&list, "fan_speed").reports,
            Reports::Values {
                values: vec![Scalar::Text("Silent".into()), Scalar::Text("Turbo".into())]
            }
        );
    }

    #[test]
    fn only_recorded_fields_are_recorded() {
        let caps = Capabilities::default();
        let climate = fields(Kind::Climate, "climate", &caps, Some("°C"));
        assert!(find(&climate, "current").recorded);
        assert!(!find(&climate, "target").recorded);
        assert!(find(&fields(Kind::Cover, "cover", &caps, None), "position").recorded);
        assert!(
            find(
                &fields(Kind::WaterHeater, "water_heater", &caps, None),
                "current"
            )
            .recorded
        );
        assert!(!find(&fields(Kind::Vacuum, "vacuum", &caps, None), "battery").recorded);
    }

    #[test]
    fn kinds_without_fields_list_none() {
        let caps = Capabilities::default();
        for kind in [Kind::Sensor, Kind::Switch, Kind::BinarySensor] {
            assert!(fields(kind, "x", &caps, None).is_empty(), "{kind:?}");
        }
    }

    #[test]
    fn reads_vacuum_battery() {
        let value = StateValue::Vacuum(VacuumState {
            state: Some("cleaning".into()),
            battery: Some(62),
            fan_speed: None,
        });
        assert_eq!(field_scalar(&value, "battery"), Some(Scalar::Number(62.0)));
        assert_eq!(field_scalar(&value, "fan_speed"), None);
        assert_eq!(field_scalar(&value, "nope"), None);
    }

    #[test]
    fn brightness_reads_percent_even_when_off() {
        let value = StateValue::Light(LightState {
            on: false,
            brightness: Some(128),
            ..Default::default()
        });
        assert_eq!(
            field_scalar(&value, "brightness"),
            Some(Scalar::Number(50.0))
        );
    }

    #[test]
    fn generic_field_reads_from_fields_map() {
        let value = StateValue::Generic(GenericState {
            fields: [
                ("state".to_owned(), GenericValue::Bool(true)),
                ("percentage".to_owned(), GenericValue::Number(40.0)),
            ]
            .into(),
        });
        assert_eq!(
            field_scalar(&value, "percentage"),
            Some(Scalar::Number(40.0))
        );
        assert_eq!(field_scalar(&value, "preset"), None);
    }

    #[test]
    fn generic_lists_its_fields_but_the_primary() {
        let list = fields(Kind::Generic, "fan", &Capabilities::default(), None);
        assert_eq!(
            names(&list),
            ["direction", "oscillation", "percentage", "preset"]
        );
        // `update` reports versions, which a flow cannot compare.
        assert!(fields(Kind::Generic, "update", &Capabilities::default(), None).is_empty());
    }

    #[test]
    fn headline_reads_a_generic_primary() {
        let lock = StateValue::Generic(GenericState {
            fields: [("state".to_owned(), GenericValue::Text("locked".into()))].into(),
        });
        assert_eq!(
            headline_scalar("lock", &lock),
            Some(Scalar::Text("locked".into()))
        );
        assert_eq!(headline_scalar("nope", &lock), None);
        let on = StateValue::OnOff(true);
        assert_eq!(headline_scalar("switch", &on), Some(Scalar::Bool(true)));
    }

    #[test]
    fn brightness_percent_rounds() {
        assert_eq!(brightness_percent(0), 0);
        assert_eq!(brightness_percent(1), 1);
        assert_eq!(brightness_percent(36), 14);
        assert_eq!(brightness_percent(255), 100);
    }
}
