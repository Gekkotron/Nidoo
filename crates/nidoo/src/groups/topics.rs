//! Topic layout of a group, for the first discovery prefix `P` (Appendix A.1): the config is
//! `P/<component>/group/<id>/config`, the state and command topic `P/group/<id>`, and the
//! definition, availability and migration marker live beneath `P/group/`.

use nidoo_core::model::Kind;

use crate::virtual_entities;

/// The id the migration marker topic takes, so no group may use it.
const RESERVED_ID: &str = "_migrated";

/// The kinds a group can present itself as.
const GROUP_KINDS: [Kind; 9] = [
    Kind::Light,
    Kind::Switch,
    Kind::Cover,
    Kind::Climate,
    Kind::Number,
    Kind::Select,
    Kind::Button,
    Kind::Vacuum,
    Kind::WaterHeater,
];

/// `[a-z0-9_]{1,48}`, as for virtual entities, and not `_migrated`.
pub fn validate_id(id: &str) -> Result<(), String> {
    virtual_entities::validate_id(id).map_err(|_| {
        format!("invalid group id `{id}`: use 1 to 48 characters of a-z, 0-9 and _")
    })?;
    if id == RESERVED_ID {
        return Err(format!("the group id `{RESERVED_ID}` is reserved"));
    }
    Ok(())
}

/// The entity's unique id.
pub fn unique_id(id: &str) -> String {
    format!("group_{id}")
}

/// The Home Assistant component of a kind a group can present, else an error.
pub fn component_for(kind: Kind) -> Result<&'static str, String> {
    if GROUP_KINDS.contains(&kind) {
        Ok(kind.component())
    } else {
        Err(format!("group kind not supported: {}", kind.component()))
    }
}

/// The retained definition topic.
pub fn definition_topic(prefix: &str, id: &str) -> String {
    format!("{prefix}/group/{id}/definition")
}

/// The retained marker that says the room groups were migrated.
pub fn migration_marker_topic(prefix: &str) -> String {
    format!("{prefix}/group/_migrated")
}

/// The state topic, and the command topic too.
pub fn state_topic(prefix: &str, id: &str) -> String {
    format!("{prefix}/group/{id}")
}

/// The retained availability topic.
pub fn availability_topic(prefix: &str, id: &str) -> String {
    format!("{prefix}/group/{id}/avail")
}

/// The retained discovery config topic. A kind a group cannot present has no component, so
/// callers validate the kind first; the raw component name is used as is here.
pub fn config_topic(prefix: &str, kind: Kind, id: &str) -> String {
    format!("{prefix}/{}/group/{id}/config", kind.component())
}

/// Whether `topic` is the config topic of group `id`, whatever its kind.
pub fn is_config_topic(prefix: &str, id: &str, topic: &str) -> bool {
    GROUP_KINDS
        .into_iter()
        .any(|kind| config_topic(prefix, kind, id) == topic)
}
#[cfg(test)]
mod tests {
    use super::*;

    const KINDS: [Kind; 9] = [
        Kind::Light,
        Kind::Switch,
        Kind::Cover,
        Kind::Climate,
        Kind::Number,
        Kind::Select,
        Kind::Button,
        Kind::Vacuum,
        Kind::WaterHeater,
    ];

    #[test]
    fn bad_ids_are_refused() {
        for id in [
            "Force",
            "",
            &"x".repeat(49),
            "a/b",
            "a-b",
            "a b",
            "é",
            "_migrated",
        ] {
            assert!(validate_id(id).is_err(), "{id:?}");
        }
        for id in ["a", "living_lights", "x1_2", &"x".repeat(48)] {
            assert!(validate_id(id).is_ok(), "{id:?}");
        }
    }

    #[test]
    fn config_topic_follows_the_component() {
        assert_eq!(
            config_topic("nidoo", Kind::Climate, "water_tank"),
            "nidoo/climate/group/water_tank/config"
        );
    }

    #[test]
    fn state_topic_is_distinct_from_config_and_definition() {
        assert_eq!(state_topic("nidoo", "lights"), "nidoo/group/lights");
        assert_ne!(
            state_topic("nidoo", "lights"),
            config_topic("nidoo", Kind::Light, "lights")
        );
        assert_eq!(
            definition_topic("nidoo", "lights"),
            "nidoo/group/lights/definition"
        );
        assert_eq!(
            availability_topic("nidoo", "lights"),
            "nidoo/group/lights/avail"
        );
        assert_eq!(migration_marker_topic("nidoo"), "nidoo/group/_migrated");
        assert_eq!(unique_id("lights"), "group_lights");
    }

    #[test]
    fn is_config_topic_matches_every_in_scope_kind() {
        for kind in KINDS {
            assert!(is_config_topic(
                "nidoo",
                "g",
                &config_topic("nidoo", kind, "g")
            ));
        }
        assert!(!is_config_topic(
            "nidoo",
            "g",
            "nidoo/switch/virtual/g/config"
        ));
        assert!(!is_config_topic("nidoo", "g", "nidoo/ui/group_g"));
        assert!(!is_config_topic("nidoo", "g", "nidoo/light/group/h/config"));
    }

    #[test]
    fn component_for_rejects_out_of_scope_kinds() {
        for kind in KINDS {
            assert_eq!(component_for(kind), Ok(kind.component()));
        }
        for kind in [Kind::Sensor, Kind::BinarySensor, Kind::AlarmPanel] {
            assert!(component_for(kind).is_err());
        }
        assert_eq!(
            component_for(Kind::AlarmPanel),
            Err("group kind not supported: alarm_control_panel".to_owned())
        );
    }
}
