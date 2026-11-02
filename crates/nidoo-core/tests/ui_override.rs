//! `UiOverride` round-trips the app's retained `nidoo/ui/<unique_id>` JSON.

mod common;

use nidoo_core::model::{Command, PatchError, UiOverride};
use serde_json::{Map, Value, json};

fn patch(pairs: Value) -> Map<String, Value> {
    match pairs {
        Value::Object(map) => map,
        other => panic!("patch must be an object, got {other}"),
    }
}

#[test]
fn decodes_every_recorded_override() {
    let dir = common::fixture_dir().join("ui");
    let mut seen = 0;
    for entry in std::fs::read_dir(&dir).expect("ui fixture dir") {
        let path = entry.expect("dir entry").path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if !name.ends_with(".json") || name == "_areas.json" {
            continue;
        }
        let bytes = std::fs::read(&path).expect("read fixture");
        assert!(
            UiOverride::decode(&bytes).is_some(),
            "{name} must decode to Some"
        );
        seen += 1;
    }
    assert_eq!(seen, 132, "expected every recorded override to be visited");
}

#[test]
fn encode_omits_defaults() {
    assert_eq!(UiOverride::default().encode(), "{}");
    let o = UiOverride {
        security_ask: false,
        ..Default::default()
    };
    assert_eq!(o.encode(), r#"{"securityAsk":false}"#);
}

#[test]
fn unknown_field_round_trips() {
    let o = UiOverride::decode(br#"{"area":"living","futureField":{"a":1}}"#).unwrap();
    assert_eq!(o.area.as_deref(), Some("living"));
    assert_eq!(o.encode(), r#"{"area":"living","futureField":{"a":1}}"#);
}

#[test]
fn unknown_energy_role_is_kept() {
    let o = UiOverride::decode(br#"{"energyRole":"fusion"}"#).unwrap();
    assert_eq!(o.energy_role.as_deref(), Some("fusion"));
    assert_eq!(o.encode(), r#"{"energyRole":"fusion"}"#);
}

#[test]
fn decode_is_lenient() {
    assert!(UiOverride::decode(b"not json").is_none());
    assert!(UiOverride::decode(b"[1,2]").is_none());
    // Nulls read as "absent", as the app's `explicitNulls = false` does.
    let o = UiOverride::decode(br#"{"hidden":null,"name":null,"favorite":true}"#).unwrap();
    assert!(!o.hidden);
    assert_eq!(o.name, None);
    assert!(o.favorite);
    assert!(o.security_ask, "securityAsk defaults to true");
}

#[test]
fn patch_sets_clears_and_rejects() {
    let base = UiOverride {
        area: Some("living".into()),
        ..Default::default()
    };

    let set = base
        .apply_patch(&patch(json!({"showAs": "light"})))
        .unwrap();
    assert_eq!(set.show_as.as_deref(), Some("light"));
    assert_eq!(set.area.as_deref(), Some("living"), "untouched fields stay");

    let cleared = base.apply_patch(&patch(json!({"area": null}))).unwrap();
    assert_eq!(cleared.area, None);

    assert_eq!(
        base.apply_patch(&patch(json!({"nope": 1}))).unwrap_err(),
        PatchError::UnknownField("nope".into())
    );
    match base
        .apply_patch(&patch(json!({"lightOnBrightness": 300})))
        .unwrap_err()
    {
        PatchError::InvalidValue { field, .. } => assert_eq!(field, "lightOnBrightness"),
        other => panic!("expected InvalidValue, got {other:?}"),
    }
    match base
        .apply_patch(&patch(json!({"energyRole": "fusion"})))
        .unwrap_err()
    {
        PatchError::InvalidValue { field, expected } => {
            assert_eq!(field, "energyRole");
            assert!(
                expected.contains("grid_export"),
                "names the options: {expected}"
            );
        }
        other => panic!("expected InvalidValue, got {other:?}"),
    }
    match base
        .apply_patch(&patch(json!({"hidden": "yes"})))
        .unwrap_err()
    {
        PatchError::InvalidValue { field, .. } => assert_eq!(field, "hidden"),
        other => panic!("expected InvalidValue, got {other:?}"),
    }

    let valid = base
        .apply_patch(&patch(json!({
            "energyRole": "grid_export",
            "lightOnPreset": "warm",
            "lightOnBrightness": 255,
            "securityAsk": false
        })))
        .unwrap();
    assert_eq!(valid.energy_role.as_deref(), Some("grid_export"));
    assert_eq!(valid.light_on_preset.as_deref(), Some("warm"));
    assert_eq!(valid.light_on_brightness, Some(255));
    assert!(!valid.security_ask);
}

#[test]
fn is_default_after_clearing_everything() {
    let o = UiOverride::decode(
        br#"{"name":"Lamp","area":"living","hidden":true,"securityAsk":false,"lightOnBrightness":42}"#,
    )
    .unwrap();
    assert!(!o.is_default());
    let cleared = o
        .apply_patch(&patch(json!({
            "name": null, "area": null, "hidden": null, "securityAsk": null, "lightOnBrightness": null
        })))
        .unwrap();
    assert!(cleared.is_default());
    assert_eq!(cleared.encode(), "{}");
    assert!(UiOverride::default().is_default());
}

#[test]
fn fields_lists_the_twenty_one_fields() {
    assert_eq!(UiOverride::FIELDS.len(), 21);
    assert_eq!(UiOverride::FIELDS[0], "name");
    assert!(UiOverride::FIELDS.contains(&"climatePresets"));
    assert!(UiOverride::FIELDS.contains(&"mainControl"));
    assert!(UiOverride::FIELDS.contains(&"invert"));
}

#[test]
fn command_json_shape() {
    assert_eq!(
        serde_json::to_string(&Command::SetPosition { position: 40 }).unwrap(),
        r#"{"type":"set_position","position":40}"#
    );
}

#[test]
fn legacy_as_light_reads_as_light_type() {
    let ui = UiOverride::decode(br#"{"name":"Lumiere tv","asLight":true}"#).unwrap();
    assert_eq!(ui.show_as.as_deref(), Some("light"));
    assert!(ui.extra.is_empty());
    let off = UiOverride::decode(br#"{"asLight":false}"#).unwrap();
    assert_eq!(off.show_as, None);
    assert!(off.is_default());
    let both = UiOverride::decode(br#"{"asLight":true,"showAs":"siren"}"#).unwrap();
    assert_eq!(both.show_as.as_deref(), Some("siren"));
}

#[test]
fn encode_writes_as_light_only_next_to_light() {
    let light = UiOverride {
        show_as: Some("light".into()),
        ..UiOverride::default()
    };
    let wire: Value = serde_json::from_str(&light.encode()).unwrap();
    assert_eq!(wire, json!({"showAs": "light", "asLight": true}));
    let siren = UiOverride {
        show_as: Some("siren".into()),
        ..UiOverride::default()
    };
    assert_eq!(
        serde_json::from_str::<Value>(&siren.encode()).unwrap(),
        json!({"showAs": "siren"})
    );
    assert!(!serde_json::to_string(&light).unwrap().contains("asLight"));
}

#[test]
fn rename_keeps_legacy_light_on_the_wire() {
    let stored = UiOverride::decode(br#"{"name":"Lumiere tv","asLight":true}"#).unwrap();
    let renamed = stored
        .apply_patch(&patch(json!({"name": "TV light"})))
        .unwrap();
    let wire: Value = serde_json::from_str(&renamed.encode()).unwrap();
    assert_eq!(
        wire,
        json!({"name": "TV light", "showAs": "light", "asLight": true})
    );
}

#[test]
fn show_as_patch_is_validated() {
    let base = UiOverride::default();
    let door = base.apply_patch(&patch(json!({"showAs": "door"}))).unwrap();
    assert_eq!(door.show_as.as_deref(), Some("door"));
    assert!(matches!(
        base.apply_patch(&patch(json!({"showAs": "fan"}))),
        Err(PatchError::InvalidValue { .. })
    ));
    assert_eq!(
        base.apply_patch(&patch(json!({"asLight": true}))),
        Err(PatchError::UnknownField("asLight".into()))
    );
    let unknown = UiOverride::decode(br#"{"showAs":"fan"}"#).unwrap();
    assert_eq!(unknown.show_as.as_deref(), Some("fan"));
}

#[test]
fn main_control_round_trips_and_null_clears() {
    let base = UiOverride::default();
    let set = base
        .apply_patch(&patch(json!({"mainControl": "locked"})))
        .unwrap();
    assert_eq!(set.main_control.as_deref(), Some("locked"));
    let wire: Value = serde_json::from_str(&set.encode()).unwrap();
    assert_eq!(wire, json!({"mainControl": "locked"}));
    assert_eq!(
        UiOverride::decode(set.encode().as_bytes()),
        Some(set.clone())
    );
    let cleared = set
        .apply_patch(&patch(json!({"mainControl": null})))
        .unwrap();
    assert_eq!(cleared.main_control, None);
    assert_eq!(cleared.encode(), "{}");
    assert!(matches!(
        base.apply_patch(&patch(json!({"mainControl": 3}))),
        Err(PatchError::InvalidValue { .. })
    ));
}
