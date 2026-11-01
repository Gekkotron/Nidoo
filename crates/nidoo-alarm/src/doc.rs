//! The alarm settings document, retained at `<prefix>/settings`.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

/// What the alarm is armed as.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub enum ArmMode {
    Away,
    Home,
    Night,
}

/// What a watched sensor detects.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub enum SensorKind {
    Opening,
    Motion,
}

/// Whether an opening trips the alarm at once or after the entry delay.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub enum Entry {
    #[default]
    Instant,
    Delayed,
}

/// One watched binary sensor.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SensorRule {
    pub entity: String,
    pub kind: SensorKind,
    pub modes: BTreeSet<ArmMode>,
    #[serde(default)]
    pub entry: Entry,
}

/// A siren driven by raw MQTT payloads.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct MqttSiren {
    pub topic: String,
    pub on: String,
    pub off: String,
    #[serde(default)]
    pub chirp: Option<String>,
}

/// A siren: a hub entity, or raw MQTT payloads.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub enum Siren {
    Entity {
        entity: String,
        #[serde(default)]
        chirp_ms: Option<u32>,
    },
    Mqtt {
        mqtt: MqttSiren,
    },
}

/// What a remote signal does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub enum RemoteAction {
    Away,
    Home,
    Night,
    Disarm,
    Sos,
}

/// A remote: its signal names mapped to actions.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct Remote {
    pub entity: String,
    pub signals: BTreeMap<String, RemoteAction>,
}

/// How many motion hits within how long trip the alarm.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct MotionRule {
    pub count: u8,
    pub window_s: u32,
}

impl Default for MotionRule {
    fn default() -> Self {
        Self {
            count: 2,
            window_s: 120,
        }
    }
}

/// Auto arm and disarm from phone presence.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct PresenceRule {
    pub trackers: Vec<String>,
    pub arm: bool,
    pub disarm: bool,
    pub arm_after_s: u32,
}

impl Default for PresenceRule {
    fn default() -> Self {
        Self {
            trackers: Vec::new(),
            arm: true,
            disarm: true,
            arm_after_s: 120,
        }
    }
}

/// The alarm settings document.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct AlarmSettings {
    pub enabled: bool,
    pub entry_delay_s: u32,
    pub exit_delay_s: u32,
    pub siren_s: u32,
    pub motion: MotionRule,
    pub sensors: Vec<SensorRule>,
    pub sirens: Vec<Siren>,
    pub remotes: Vec<Remote>,
    pub presence: PresenceRule,
}

impl Default for AlarmSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            entry_delay_s: 30,
            exit_delay_s: 60,
            siren_s: 180,
            motion: MotionRule::default(),
            sensors: Vec::new(),
            sirens: Vec::new(),
            remotes: Vec::new(),
            presence: PresenceRule::default(),
        }
    }
}

fn first_duplicate<'a>(ids: impl Iterator<Item = &'a str>) -> Option<&'a str> {
    let mut seen = BTreeSet::new();
    ids.into_iter().find(|id| !seen.insert(*id))
}

impl AlarmSettings {
    /// Checks ranges, duplicates and MQTT siren shapes. Entity existence is checked by the hub.
    pub fn validate(&self) -> Result<(), String> {
        if self.entry_delay_s > 300 {
            return Err("entry_delay_s must be 0–300".into());
        }
        if self.exit_delay_s > 300 {
            return Err("exit_delay_s must be 0–300".into());
        }
        if !(10..=900).contains(&self.siren_s) {
            return Err("siren_s must be 10–900".into());
        }
        if !(1..=5).contains(&self.motion.count) {
            return Err("motion.count must be 1–5".into());
        }
        if !(10..=900).contains(&self.motion.window_s) {
            return Err("motion.window_s must be 10–900".into());
        }
        if self.presence.arm_after_s > 3600 {
            return Err("presence.arm_after_s must be 0–3600".into());
        }
        if let Some(id) = first_duplicate(self.sensors.iter().map(|s| s.entity.as_str())) {
            return Err(format!("sensors lists {id} twice"));
        }
        if let Some(id) = first_duplicate(self.remotes.iter().map(|r| r.entity.as_str())) {
            return Err(format!("remotes lists {id} twice"));
        }
        if let Some(id) = first_duplicate(self.presence.trackers.iter().map(String::as_str)) {
            return Err(format!("presence.trackers lists {id} twice"));
        }
        for siren in &self.sirens {
            if let Siren::Mqtt { mqtt } = siren {
                if mqtt.topic.is_empty() {
                    return Err("sirens: mqtt topic must not be empty".into());
                }
                if mqtt.topic.contains(['+', '#']) {
                    return Err("sirens: mqtt topic must not contain + or #".into());
                }
                if mqtt.on.is_empty() {
                    return Err("sirens: mqtt on payload must not be empty".into());
                }
                if mqtt.off.is_empty() {
                    return Err("sirens: mqtt off payload must not be empty".into());
                }
            }
        }
        Ok(())
    }

    /// The rule for a watched sensor.
    pub fn sensor(&self, entity: &str) -> Option<&SensorRule> {
        self.sensors.iter().find(|s| s.entity == entity)
    }

    /// Whether `entity` is watched while armed as `mode`.
    pub fn watches(&self, entity: &str, mode: ArmMode) -> bool {
        self.sensor(entity).is_some_and(|s| s.modes.contains(&mode))
    }
}

/// Signal names pre-filled when a remote is added (Heiman / IAS names).
pub fn default_signals() -> BTreeMap<String, RemoteAction> {
    [
        ("arm_all_zones", RemoteAction::Away),
        ("arm_day_zones", RemoteAction::Home),
        ("arm_night_zones", RemoteAction::Night),
        ("disarm", RemoteAction::Disarm),
        ("emergency", RemoteAction::Sos),
        ("panic", RemoteAction::Sos),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect()
}

/// The rule a sensor gets when it is added: an opening watches away and night, motion only away.
pub fn default_sensor(entity: &str, kind: SensorKind) -> SensorRule {
    let modes = match kind {
        SensorKind::Opening => BTreeSet::from([ArmMode::Away, ArmMode::Night]),
        SensorKind::Motion => BTreeSet::from([ArmMode::Away]),
    };
    SensorRule {
        entity: entity.to_string(),
        kind,
        modes,
        entry: Entry::Instant,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn err_of(f: impl FnOnce(&mut AlarmSettings), field: &str) {
        let mut s = AlarmSettings::default();
        f(&mut s);
        let e = s.validate().expect_err(field);
        assert!(e.contains(field), "{e} should name {field}");
    }

    #[test]
    fn defaults_match_spec() {
        let d = AlarmSettings::default();
        assert!(!d.enabled);
        assert_eq!((d.entry_delay_s, d.exit_delay_s, d.siren_s), (30, 60, 180));
        assert_eq!(
            d.motion,
            MotionRule {
                count: 2,
                window_s: 120
            }
        );
        assert_eq!(
            d.presence,
            PresenceRule {
                trackers: vec![],
                arm: true,
                disarm: true,
                arm_after_s: 120
            }
        );
        assert_eq!(serde_json::from_str::<AlarmSettings>("{}").unwrap(), d);
        assert!(d.validate().is_ok());
    }

    #[test]
    fn validate_rejects_out_of_range() {
        err_of(|s| s.entry_delay_s = 301, "entry_delay_s");
        err_of(|s| s.exit_delay_s = 301, "exit_delay_s");
        err_of(|s| s.siren_s = 9, "siren_s");
        err_of(|s| s.motion.count = 0, "motion.count");
        err_of(|s| s.motion.count = 6, "motion.count");
        err_of(|s| s.motion.window_s = 901, "motion.window_s");
        err_of(|s| s.presence.arm_after_s = 3601, "presence.arm_after_s");
    }

    #[test]
    fn validate_rejects_duplicates() {
        err_of(
            |s| {
                s.sensors = vec![
                    default_sensor("binary_sensor.a", SensorKind::Opening),
                    default_sensor("binary_sensor.a", SensorKind::Motion),
                ]
            },
            "sensors",
        );
        err_of(
            |s| {
                let r = Remote {
                    entity: "device_automation.r".into(),
                    signals: default_signals(),
                };
                s.remotes = vec![r.clone(), r];
            },
            "remotes",
        );
        err_of(
            |s| s.presence.trackers = vec!["device_tracker.p".into(), "device_tracker.p".into()],
            "presence.trackers",
        );
    }

    #[test]
    fn validate_rejects_bad_mqtt_siren() {
        let mk = |topic: &str, on: &str, off: &str| {
            let s = AlarmSettings {
                sirens: vec![Siren::Mqtt {
                    mqtt: MqttSiren {
                        topic: topic.into(),
                        on: on.into(),
                        off: off.into(),
                        chirp: None,
                    },
                }],
                ..Default::default()
            };
            s.validate()
        };
        assert!(mk("a/b/set", "1", "0").is_ok());
        assert!(mk("", "1", "0").is_err());
        assert!(mk("a/+/set", "1", "0").is_err());
        assert!(mk("a/#", "1", "0").is_err());
        assert!(mk("a/b/set", "", "0").is_err());
        assert!(mk("a/b/set", "1", "").is_err());
    }

    #[test]
    fn siren_untagged_roundtrip() {
        let e: Siren = serde_json::from_str(r#"{"entity":"switch.x","chirp_ms":null}"#).unwrap();
        assert_eq!(
            e,
            Siren::Entity {
                entity: "switch.x".into(),
                chirp_ms: None
            }
        );
        let m: Siren =
            serde_json::from_str(r#"{"mqtt":{"topic":"a/b/set","on":"1","off":"0","chirp":"c"}}"#)
                .unwrap();
        assert_eq!(
            m,
            Siren::Mqtt {
                mqtt: MqttSiren {
                    topic: "a/b/set".into(),
                    on: "1".into(),
                    off: "0".into(),
                    chirp: Some("c".into())
                }
            }
        );
        let back: Siren = serde_json::from_str(&serde_json::to_string(&m).unwrap()).unwrap();
        assert_eq!(back, m);
    }

    #[test]
    fn opening_defaults_to_away_night_instant() {
        let r = default_sensor("binary_sensor.door", SensorKind::Opening);
        assert_eq!(r.modes, BTreeSet::from([ArmMode::Away, ArmMode::Night]));
        assert_eq!(r.entry, Entry::Instant);
    }

    #[test]
    fn motion_defaults_to_away() {
        let r = default_sensor("binary_sensor.pir", SensorKind::Motion);
        assert_eq!(r.modes, BTreeSet::from([ArmMode::Away]));
    }

    #[test]
    fn watches_follows_modes() {
        let s = AlarmSettings {
            sensors: vec![default_sensor("binary_sensor.pir", SensorKind::Motion)],
            ..Default::default()
        };
        assert!(s.watches("binary_sensor.pir", ArmMode::Away));
        assert!(!s.watches("binary_sensor.pir", ArmMode::Home));
        assert!(!s.watches("binary_sensor.other", ArmMode::Away));
    }

    #[test]
    fn default_signals_cover_heiman_names() {
        let d = default_signals();
        assert_eq!(d["emergency"], RemoteAction::Sos);
        assert_eq!(d["arm_day_zones"], RemoteAction::Home);
        assert_eq!(d.len(), 6);
    }
}
