//! The per-entity override retained on `oikos/ui/<unique_id>`, field for field as the app's `UiOverride`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Valid `energyRole` values (the app's `EnergyRole` serial names).
pub const ENERGY_ROLES: &[&str] = &[
    "solar",
    "grid",
    "grid_export",
    "battery",
    "house",
    "consumer",
    "water",
];

/// Valid `lightOnPreset` values (the app's `LightOnPreset` keys).
pub const LIGHT_ON_PRESETS: &[&str] = &["white", "warm", "red", "green", "blue"];

/// Valid `showAs` values: what a switch entity drives. Absent means a plain switch.
pub const SHOW_AS: &[&str] = &["light", "door", "siren", "chime", "plug"];

/// The retired flag a light-typed switch used to carry; read on decode, written next to
/// `showAs: "light"` until the app reads `showAs` (device sheet spec §10, step 4).
const LEGACY_AS_LIGHT: &str = "asLight";

/// Retained JSON on `oikos/ui/<unique_id>`.
///
/// Decoding is lenient (unknown keys are kept in [`UiOverride::extra`], nulls read as absent,
/// enum-like values stay strings so a role this hub does not know survives a read-modify-write);
/// encoding omits every field at its default, as the app does.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase", default)]
pub struct UiOverride {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Area id (or a raw area name, resolved by the registry).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub area: Option<String>,
    #[serde(skip_serializing_if = "is_false")]
    pub hidden: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub card_style: Option<String>,
    #[serde(skip_serializing_if = "is_false")]
    pub favorite: bool,
    /// One of [`ENERGY_ROLES`] when written by this hub; kept as a string so unknown roles round-trip.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub energy_role: Option<String>,
    #[serde(skip_serializing_if = "is_false")]
    pub energy_invert: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub security_hidden: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub energy_icon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub energy_meter: Option<String>,
    #[serde(skip_serializing_if = "is_false")]
    pub home_pinned: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub security_action: bool,
    /// Defaults to `true`, so it is only encoded when `false`.
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub security_ask: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub ungrouped: bool,
    /// One of [`SHOW_AS`] when written by this hub; only read on a switch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_as: Option<String>,
    /// One of [`LIGHT_ON_PRESETS`] when written by this hub.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub light_on_preset: Option<String>,
    /// 1..=255 when written by this hub.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub light_on_brightness: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub climate_presets: Option<String>,
    /// A generic entity's main control: one of its announced controls, or `"none"` to hide it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub main_control: Option<String>,
    /// A switch wired the other way round (relay on = device off): the hub swaps on and off
    /// in the state it decodes and the commands it sends. Only read on a switch.
    #[serde(skip_serializing_if = "is_false")]
    pub invert: bool,
    /// Fields written by a newer app, preserved verbatim.
    #[serde(flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, Value>,
}

impl Default for UiOverride {
    fn default() -> Self {
        Self {
            name: None,
            area: None,
            hidden: false,
            order: None,
            card_style: None,
            favorite: false,
            energy_role: None,
            energy_invert: false,
            security_hidden: false,
            energy_icon: None,
            energy_meter: None,
            home_pinned: false,
            security_action: false,
            security_ask: true,
            ungrouped: false,
            show_as: None,
            light_on_preset: None,
            light_on_brightness: None,
            climate_presets: None,
            main_control: None,
            invert: false,
            extra: BTreeMap::new(),
        }
    }
}

/// Why a patch could not be applied to a [`UiOverride`].
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PatchError {
    /// The key is not one of [`UiOverride::FIELDS`].
    #[error("unknown override field `{0}`")]
    UnknownField(String),
    /// The value has the wrong type or is out of range; `expected` names what is accepted.
    #[error("invalid value for `{field}`: expected {expected}")]
    InvalidValue { field: String, expected: String },
}

/// What a field's patch value must be.
#[derive(Clone, Copy)]
enum FieldType {
    Text,
    Flag,
    Int,
    Brightness,
    EnergyRole,
    LightOnPreset,
    ShowAs,
}

impl FieldType {
    fn expected(self) -> String {
        match self {
            FieldType::Text => "a string".to_owned(),
            FieldType::Flag => "a boolean".to_owned(),
            FieldType::Int => "an integer".to_owned(),
            FieldType::Brightness => "an integer between 1 and 255".to_owned(),
            FieldType::EnergyRole => format!("one of {}", ENERGY_ROLES.join(", ")),
            FieldType::LightOnPreset => format!("one of {}", LIGHT_ON_PRESETS.join(", ")),
            FieldType::ShowAs => format!("one of {}", SHOW_AS.join(", ")),
        }
    }

    fn accepts(self, value: &Value) -> bool {
        match self {
            FieldType::Text => value.is_string(),
            FieldType::Flag => value.is_boolean(),
            FieldType::Int => value.as_i64().is_some_and(|n| i32::try_from(n).is_ok()),
            FieldType::Brightness => value.as_i64().is_some_and(|n| (1..=255).contains(&n)),
            FieldType::EnergyRole => value.as_str().is_some_and(|s| ENERGY_ROLES.contains(&s)),
            FieldType::LightOnPreset => value
                .as_str()
                .is_some_and(|s| LIGHT_ON_PRESETS.contains(&s)),
            FieldType::ShowAs => value.as_str().is_some_and(|s| SHOW_AS.contains(&s)),
        }
    }
}

/// The 20 app fields, in the app's order, with what a patch may set them to.
const FIELD_TYPES: &[(&str, FieldType)] = &[
    ("name", FieldType::Text),
    ("area", FieldType::Text),
    ("hidden", FieldType::Flag),
    ("order", FieldType::Int),
    ("cardStyle", FieldType::Text),
    ("favorite", FieldType::Flag),
    ("energyRole", FieldType::EnergyRole),
    ("energyInvert", FieldType::Flag),
    ("securityHidden", FieldType::Flag),
    ("energyIcon", FieldType::Text),
    ("energyMeter", FieldType::Text),
    ("homePinned", FieldType::Flag),
    ("securityAction", FieldType::Flag),
    ("securityAsk", FieldType::Flag),
    ("ungrouped", FieldType::Flag),
    ("showAs", FieldType::ShowAs),
    ("lightOnPreset", FieldType::LightOnPreset),
    ("lightOnBrightness", FieldType::Brightness),
    ("climatePresets", FieldType::Text),
    ("mainControl", FieldType::Text),
    ("invert", FieldType::Flag),
];

impl UiOverride {
    /// The 21 camelCase field names, in the app's order; `invert` is hub-only so far.
    pub const FIELDS: &[&str] = &[
        "name",
        "area",
        "hidden",
        "order",
        "cardStyle",
        "favorite",
        "energyRole",
        "energyInvert",
        "securityHidden",
        "energyIcon",
        "energyMeter",
        "homePinned",
        "securityAction",
        "securityAsk",
        "ungrouped",
        "showAs",
        "lightOnPreset",
        "lightOnBrightness",
        "climatePresets",
        "mainControl",
        "invert",
    ];

    /// Decodes a retained payload leniently; `None` when it is not a JSON object or a known
    /// field has the wrong type.
    pub fn decode(payload: &[u8]) -> Option<UiOverride> {
        let mut value: Value = serde_json::from_slice(payload).ok()?;
        strip_nulls(value.as_object_mut()?);
        let mut ui: UiOverride = serde_json::from_value(value).ok()?;
        ui.absorb_legacy();
        Some(ui)
    }

    /// Encodes with every default omitted; `{}` for a default override.
    pub fn encode(&self) -> String {
        let Ok(Value::Object(mut map)) = serde_json::to_value(self) else {
            return "{}".to_owned();
        };
        // Compatibility write for apps that only know `asLight`; dropped at step 4 of the
        // device sheet spec's §10, once the app reads `showAs`.
        if self.show_as.as_deref() == Some("light") {
            map.insert(LEGACY_AS_LIGHT.to_owned(), Value::Bool(true));
        }
        Value::Object(map).to_string()
    }

    /// Turns a stored `asLight` (caught by `extra`) into `showAs: "light"` unless `showAs` is set.
    fn absorb_legacy(&mut self) {
        if self.extra.remove(LEGACY_AS_LIGHT) == Some(Value::Bool(true)) && self.show_as.is_none() {
            self.show_as = Some("light".to_owned());
        }
    }

    /// True when nothing differs from the default (and no extra field is carried).
    pub fn is_default(&self) -> bool {
        *self == UiOverride::default()
    }

    /// Applies a partial update: a `null` clears the field, other values replace it; fields
    /// absent from the patch are untouched. Unknown keys and wrong or out-of-range values are
    /// rejected and nothing is changed.
    pub fn apply_patch(&self, patch: &Map<String, Value>) -> Result<UiOverride, PatchError> {
        for (key, value) in patch {
            let Some((_, field_type)) = FIELD_TYPES.iter().find(|(name, _)| name == key) else {
                return Err(PatchError::UnknownField(key.clone()));
            };
            if !value.is_null() && !field_type.accepts(value) {
                return Err(PatchError::InvalidValue {
                    field: key.clone(),
                    expected: field_type.expected(),
                });
            }
        }

        let mut merged = match serde_json::to_value(self) {
            Ok(Value::Object(map)) => map,
            _ => Map::new(),
        };
        for (key, value) in patch {
            if value.is_null() {
                merged.remove(key);
            } else {
                merged.insert(key.clone(), value.clone());
            }
        }
        let mut ui: UiOverride = serde_json::from_value(Value::Object(merged)).map_err(|e| {
            PatchError::InvalidValue {
                field: "patch".to_owned(),
                expected: format!("a valid override ({e})"),
            }
        })?;
        ui.absorb_legacy();
        Ok(ui)
    }
}

/// Drops top-level nulls so a `"field": null` reads as absent (the app's `explicitNulls = false`).
pub(crate) fn strip_nulls(map: &mut Map<String, Value>) {
    map.retain(|_, value| !value.is_null());
}

fn default_true() -> bool {
    true
}

fn is_true(value: &bool) -> bool {
    *value
}

fn is_false(value: &bool) -> bool {
    !*value
}
