//! What a field accepts, as predicates over an entity. Each field names one and wraps it with
//! [`keep`] for the picker.

use std::sync::Arc;

use nidoo_api::EntityView;
use nidoo_core::model::Kind;

use super::Keep;

/// Wraps a predicate as a [`Keep`].
pub fn keep(f: fn(&EntityView) -> bool) -> Keep {
    Arc::new(f)
}

/// Whether `entity` reads a temperature.
pub fn is_thermometer(entity: &EntityView) -> bool {
    entity.kind == Kind::Sensor && entity.device_class.as_deref() == Some("temperature")
}

/// Whether `e` reports presence: a presence, occupancy or motion sensor, a tracker or an alarm
/// panel (the hub reads its armed state as "home").
pub fn is_presence(e: &EntityView) -> bool {
    e.kind == Kind::AlarmPanel || is_room_presence(e) || is_tracker(e)
}

/// Whether `e` detects someone in a room: a presence, occupancy or motion binary sensor.
pub fn is_room_presence(e: &EntityView) -> bool {
    e.kind == Kind::BinarySensor
        && matches!(
            e.device_class.as_deref(),
            Some("presence" | "occupancy" | "motion")
        )
}

/// Whether `e` has an on/off state: a switch or a binary sensor, or a virtual switch.
pub fn is_on_off(e: &EntityView) -> bool {
    matches!(e.kind, Kind::Switch | Kind::BinarySensor)
        || (e.id.starts_with("virtual_") && e.kind == Kind::Switch)
}

/// Whether `e` can be a siren: a switch, or a generic entity with a `power` control.
pub fn is_siren(e: &EntityView) -> bool {
    e.kind == Kind::Switch
        || (e.kind == Kind::Generic
            && e.capabilities
                .generic
                .as_ref()
                .is_some_and(|g| g.controls.iter().any(|c| c == "power")))
}

/// Whether `e` emits signals a remote can use: a device trigger or an event entity.
pub fn is_remote(e: &EntityView) -> bool {
    matches!(e.component.as_str(), "device_automation" | "event")
}

/// Whether `e` reports `home` / `not_home`.
pub fn is_tracker(e: &EntityView) -> bool {
    e.capabilities
        .generic
        .as_ref()
        .and_then(|g| g.options.get("state"))
        .is_some_and(|o| o.iter().any(|v| v == "home") && o.iter().any(|v| v == "not_home"))
}

/// Whether `e` announces signals: a device trigger, an event or a tag.
pub fn is_signal(e: &EntityView) -> bool {
    matches!(e.component.as_str(), "device_automation" | "event" | "tag")
}

/// Whether a flow can send `e` a command.
pub fn is_commandable(e: &EntityView) -> bool {
    !crate::command_form::entity_actions(e).is_empty()
}

#[cfg(test)]
mod tests {
    use nidoo_core::model::{Capabilities, EntityCategory, EntityState, UiOverride};
    use nidoo_core::types::GenericCaps;

    use super::*;

    fn ent(id: &str, kind: Kind, class: Option<&str>, component: &str) -> EntityView {
        EntityView {
            id: id.into(),
            device_id: None,
            name: id.into(),
            device_name: None,
            label: None,
            kind,
            component: component.into(),
            protocol_kind: kind,
            device_class: class.map(Into::into),
            unit: None,
            icon: None,
            category: EntityCategory::Primary,
            enabled_by_default: true,
            area: "a".into(),
            part_of: None,
            capabilities: Capabilities::default(),
            ui: UiOverride::default(),
            state: EntityState::default(),
        }
    }

    fn tracker() -> EntityView {
        let mut e = ent("t", Kind::Generic, None, "device_tracker");
        let mut caps = GenericCaps::default();
        caps.options
            .insert("state".into(), vec!["home".into(), "not_home".into()]);
        e.capabilities.generic = Some(caps);
        e
    }

    #[test]
    fn room_presence_takes_binary_sensors_only() {
        for class in ["presence", "occupancy", "motion"] {
            assert!(is_room_presence(&ent(
                "p",
                Kind::BinarySensor,
                Some(class),
                "binary_sensor"
            )));
        }
        assert!(!is_room_presence(&ent(
            "d",
            Kind::BinarySensor,
            Some("door"),
            "binary_sensor"
        )));
        assert!(!is_room_presence(&ent(
            "al",
            Kind::AlarmPanel,
            None,
            "alarm_control_panel"
        )));
        assert!(!is_room_presence(&tracker()));
    }

    #[test]
    fn presence_takes_motion_and_trackers() {
        for class in ["presence", "occupancy", "motion"] {
            assert!(is_presence(&ent(
                "p",
                Kind::BinarySensor,
                Some(class),
                "binary_sensor"
            )));
        }
        assert!(!is_presence(&ent(
            "d",
            Kind::BinarySensor,
            Some("door"),
            "binary_sensor"
        )));
        assert!(!is_presence(&ent(
            "m",
            Kind::Sensor,
            Some("motion"),
            "sensor"
        )));
        assert!(is_presence(&ent(
            "al",
            Kind::AlarmPanel,
            None,
            "alarm_control_panel"
        )));
        assert!(is_presence(&tracker()));
        assert!(is_tracker(&tracker()));
    }

    #[test]
    fn signal_components() {
        for c in ["device_automation", "event", "tag"] {
            assert!(is_signal(&ent("s", Kind::Generic, None, c)));
        }
        assert!(!is_signal(&ent("s", Kind::Generic, None, "sensor")));
        assert!(is_remote(&ent("s", Kind::Generic, None, "event")));
        assert!(!is_remote(&ent("s", Kind::Generic, None, "tag")));
    }

    #[test]
    fn on_off_and_thermometer_and_siren() {
        assert!(is_on_off(&ent("s", Kind::Switch, None, "switch")));
        assert!(is_on_off(&ent(
            "b",
            Kind::BinarySensor,
            None,
            "binary_sensor"
        )));
        assert!(!is_on_off(&ent("l", Kind::Light, None, "light")));
        assert!(is_thermometer(&ent(
            "t",
            Kind::Sensor,
            Some("temperature"),
            "sensor"
        )));
        assert!(!is_thermometer(&ent(
            "t",
            Kind::Sensor,
            Some("humidity"),
            "sensor"
        )));
        assert!(is_siren(&ent("s", Kind::Switch, None, "switch")));
        assert!(!is_siren(&ent("s", Kind::Sensor, None, "sensor")));
    }

    #[test]
    fn commandable_follows_the_flow_commands() {
        assert!(is_commandable(&ent("l", Kind::Light, None, "light")));
        assert!(!is_commandable(&ent("s", Kind::Sensor, None, "sensor")));
    }
}
