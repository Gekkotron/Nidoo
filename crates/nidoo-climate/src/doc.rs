//! Retained climate documents.
//!
//! Timestamps are Unix seconds. Local wall-clock times (`HhMm`) are interpreted in the house zone
//! by the hub.

use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

/// Heating or cooling.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub enum Mode {
    Heat,
    Cool,
}

/// Which mode the house runs in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub enum Season {
    #[default]
    Auto,
    Heat,
    Cool,
    Off,
}

/// Lifecycle of a zone: waiting for its first approved schedule, or running.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub enum ZoneState {
    Off,
    #[default]
    Pending,
    Active,
}

/// A day of the week.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub enum Day {
    Mon,
    Tue,
    Wed,
    Thu,
    Fri,
    Sat,
    Sun,
}

impl Day {
    /// Every day, Monday first.
    pub const ALL: [Day; 7] = [
        Day::Mon,
        Day::Tue,
        Day::Wed,
        Day::Thu,
        Day::Fri,
        Day::Sat,
        Day::Sun,
    ];

    pub fn from_chrono(d: chrono::Weekday) -> Self {
        use chrono::Weekday as W;
        match d {
            W::Mon => Day::Mon,
            W::Tue => Day::Tue,
            W::Wed => Day::Wed,
            W::Thu => Day::Thu,
            W::Fri => Day::Fri,
            W::Sat => Day::Sat,
            W::Sun => Day::Sun,
        }
    }
}

/// A target temperature in degrees Celsius, or `Off`. JSON: a number or the string `"off"`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Setpoint {
    Temp(f64),
    Off,
}

impl Serialize for Setpoint {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Setpoint::Temp(t) => s.serialize_f64(*t),
            Setpoint::Off => s.serialize_str("off"),
        }
    }
}

impl<'de> Deserialize<'de> for Setpoint {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Repr {
            Num(f64),
            Str(String),
        }
        match Repr::deserialize(d)? {
            Repr::Num(t) => Ok(Setpoint::Temp(t)),
            Repr::Str(s) if s == "off" => Ok(Setpoint::Off),
            Repr::Str(s) => Err(de::Error::custom(format!(
                "expected a number or \"off\", got {s:?}"
            ))),
        }
    }
}

#[cfg(feature = "schemars")]
impl schemars::JsonSchema for Setpoint {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Setpoint".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({ "oneOf": [{ "type": "number" }, { "const": "off" }] })
    }
}

/// A local time of day as minutes since midnight, `0..=1440`. JSON: `"HH:MM"`, `"24:00"` allowed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schemars", schemars(with = "String"))]
pub struct HhMm(pub u16);

impl HhMm {
    /// Parse `"HH:MM"` (two digits each, up to `"24:00"`).
    pub fn parse(s: &str) -> Option<Self> {
        let b = s.as_bytes();
        if b.len() != 5 || b[2] != b':' || ![0, 1, 3, 4].iter().all(|&i| b[i].is_ascii_digit()) {
            return None;
        }
        let h: u16 = s[0..2].parse().ok()?;
        let m: u16 = s[3..5].parse().ok()?;
        (m < 60 && h * 60 + m <= 1440).then_some(HhMm(h * 60 + m))
    }
}

impl Serialize for HhMm {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&format!("{:02}:{:02}", self.0 / 60, self.0 % 60))
    }
}

impl<'de> Deserialize<'de> for HhMm {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        HhMm::parse(&s)
            .ok_or_else(|| de::Error::custom(format!("expected \"HH:MM\" up to 24:00, got {s:?}")))
    }
}

/// How many of the observed days agreed with a learned block.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct Evidence {
    pub agree: u32,
    pub of: u32,
}

/// A schedule block: on `days`, from `from` (inclusive) to `to` (exclusive), hold `setpoint`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct Block {
    pub days: Vec<Day>,
    pub from: HhMm,
    pub to: HhMm,
    pub setpoint: Setpoint,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<Evidence>,
}

/// The schedule of one mode: a base setpoint and the blocks that override it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct ModeRules {
    pub base: Setpoint,
    #[serde(default)]
    pub blocks: Vec<Block>,
}

/// A setpoint is off or within 0..=40 °C.
pub fn check_setpoint(sp: &Setpoint) -> Result<(), String> {
    match sp {
        Setpoint::Temp(t) if !(0.0..=40.0).contains(t) => {
            Err(format!("setpoint {t} is outside 0..=40"))
        }
        _ => Ok(()),
    }
}

impl ModeRules {
    fn validate(&self, label: &str) -> Result<(), String> {
        check_setpoint(&self.base).map_err(|e| format!("{label} base: {e}"))?;
        for (i, b) in self.blocks.iter().enumerate() {
            let at = format!("{label} block {i}");
            if b.days.is_empty() {
                return Err(format!("{at}: no days"));
            }
            if b.from >= b.to {
                return Err(format!("{at}: from must be before to"));
            }
            // `HhMm::parse` stops at 24:00, a block built in code may not.
            if b.to.0 > 1440 {
                return Err(format!("{at}: to must be at most 24:00"));
            }
            check_setpoint(&b.setpoint).map_err(|e| format!("{at}: {e}"))?;
            for (j, o) in self.blocks.iter().enumerate().take(i) {
                let shared = b.days.iter().any(|d| o.days.contains(d));
                if shared && b.from < o.to && o.from < b.to {
                    return Err(format!("{at}: overlaps block {j}"));
                }
            }
        }
        Ok(())
    }
}

/// The schedules of a zone, per mode.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct Rules {
    pub heat: Option<ModeRules>,
    pub cool: Option<ModeRules>,
}

impl Rules {
    pub fn get(&self, mode: Mode) -> Option<&ModeRules> {
        match mode {
            Mode::Heat => self.heat.as_ref(),
            Mode::Cool => self.cool.as_ref(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.heat.is_none() && self.cool.is_none()
    }

    pub fn validate(&self) -> Result<(), String> {
        if let Some(r) = &self.heat {
            r.validate("heat")?;
        }
        if let Some(r) = &self.cool {
            r.validate("cool")?;
        }
        Ok(())
    }
}

/// House-wide settings (`<prefix>/house`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct HouseSettings {
    pub season: Season,
    pub outdoor_entity: Option<String>,
    pub heat_below: f64,
    pub cool_above: f64,
    pub presence: Vec<String>,
    pub away_grace_min: u32,
    pub holiday_entity: Option<String>,
    /// What eco holds: a room set to eco until a date, or every room while nobody is home.
    pub eco: EcoSettings,
    pub frost: f64,
    pub overheat: f64,
    pub overcool: f64,
    pub stale_after_min: u32,
    pub learn_days: u32,
    pub learn_min_days: u32,
    pub learn_support: f64,
    pub learn_at: HhMm,
}

impl Default for HouseSettings {
    fn default() -> Self {
        Self {
            season: Season::Auto,
            outdoor_entity: None,
            heat_below: 16.0,
            cool_above: 24.0,
            presence: Vec::new(),
            away_grace_min: 15,
            holiday_entity: None,
            eco: EcoSettings::default(),
            frost: 7.0,
            overheat: 30.0,
            overcool: 16.0,
            stale_after_min: 60,
            learn_days: 28,
            learn_min_days: 10,
            learn_support: 0.6,
            learn_at: HhMm(210),
        }
    }
}

impl HouseSettings {
    pub fn validate(&self) -> Result<(), String> {
        if self.frost >= self.overheat {
            return Err("frost must be below overheat".into());
        }
        if !(self.learn_support > 0.0 && self.learn_support <= 1.0) {
            return Err("learn_support must be in (0, 1]".into());
        }
        if self.learn_days == 0 {
            return Err("learn_days must be at least 1".into());
        }
        if self.learn_min_days == 0 {
            return Err("learn_min_days must be at least 1".into());
        }
        if self.learn_min_days > self.learn_days {
            return Err("learn_min_days must not exceed learn_days".into());
        }
        if self.stale_after_min == 0 {
            return Err("stale_after_min must be above 0".into());
        }
        self.eco.validate(self)
    }
}

/// One extra setting eco gives a room's climate units, as `{ "fan_mode": "<mode>" }`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub enum EcoAction {
    /// A fan mode, sent only to units that list it in their `fan_modes`.
    FanMode(String),
}

/// The fan mode among `actions`, if any.
pub fn fan_mode_of(actions: &[EcoAction]) -> Option<&str> {
    actions
        .iter()
        .map(|a| match a {
            EcoAction::FanMode(m) => m.as_str(),
        })
        .next()
}

/// The house-wide eco: a setpoint per mode and the extra actions for the climate units.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct EcoSettings {
    pub heat: f64,
    pub cool: f64,
    pub actions: Vec<EcoAction>,
}

impl Default for EcoSettings {
    fn default() -> Self {
        Self {
            heat: 16.0,
            cool: 28.0,
            actions: Vec::new(),
        }
    }
}

impl EcoSettings {
    /// The fan mode eco sets, if any.
    pub fn fan_mode(&self) -> Option<&str> {
        fan_mode_of(&self.actions)
    }

    /// The eco setpoint of `mode`.
    pub fn setpoint(&self, mode: Mode) -> f64 {
        match mode {
            Mode::Heat => self.heat,
            Mode::Cool => self.cool,
        }
    }

    fn validate(&self, house: &HouseSettings) -> Result<(), String> {
        check_setpoint(&Setpoint::Temp(self.heat)).map_err(|e| format!("eco heat: {e}"))?;
        check_setpoint(&Setpoint::Temp(self.cool)).map_err(|e| format!("eco cool: {e}"))?;
        if self.heat < house.frost {
            return Err("eco heat must not be below frost".into());
        }
        if self.cool > house.overheat {
            return Err("eco cool must not be above overheat".into());
        }
        let fans: Vec<&String> = self
            .actions
            .iter()
            .map(|a| match a {
                EcoAction::FanMode(m) => m,
            })
            .collect();
        if fans.len() > 1 {
            return Err("eco has more than one fan_mode action".into());
        }
        if fans.iter().any(|m| m.trim().is_empty()) {
            return Err("eco fan_mode must not be empty".into());
        }
        Ok(())
    }
}

/// A zone held in eco until `until` (`<prefix>/zone/<zone>/eco`), set at `set_at` (Unix
/// seconds).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct RoomEco {
    pub until: u64,
    pub set_at: u64,
}

/// What eco did with a unit's fan in the current eco period.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub enum FanState {
    /// Eco set it and saw it land: put back `was` when eco ends, if still at `set`.
    #[default]
    Set,
    /// Sent while the unit was at `from`, not seen landing yet.
    Sent { from: String },
    /// Already at the eco value, or moved by the user: left alone until eco ends.
    Released,
}

/// A unit's fan under eco: what it had before (`was`), what eco wants (`set`), and where eco
/// stands with it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct FanRestore {
    pub was: String,
    pub set: String,
    #[serde(default)]
    pub state: FanState,
}

/// A rejected proposal, remembered until `until` (Unix seconds).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct Rejection {
    pub fingerprint: String,
    pub until: u64,
}

/// Room-thermometer correction: nudges the setpoint sent to the zone's devices when the room
/// reading stays away from the schedule target.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct FeedbackSettings {
    pub enabled: bool,
    /// Tolerated distance from the target, in degrees, before the offset moves.
    pub band: f64,
    /// Offset change per correction, in degrees.
    pub step: f64,
    /// Minimum minutes between two corrections.
    pub interval_min: u32,
    /// Minutes after the schedule target began before the slow side may step: raising the
    /// setpoint in heat, extra cooling in cool.
    pub warmup_min: u32,
    /// Largest offset below the target, in degrees (positive).
    pub max_below: f64,
    /// Largest offset above the target, in degrees (positive).
    pub max_above: f64,
    /// Minutes without a usable reading after which the offset is dropped.
    pub quiet_limit_min: u32,
}

impl Default for FeedbackSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            band: 0.5,
            step: 0.5,
            interval_min: 15,
            warmup_min: 60,
            max_below: 3.0,
            max_above: 0.5,
            quiet_limit_min: 360,
        }
    }
}

impl FeedbackSettings {
    pub fn validate(&self) -> Result<(), String> {
        if !(self.band.is_finite() && self.band > 0.0) {
            return Err("feedback.band must be above 0".into());
        }
        if !(self.step.is_finite() && self.step > 0.0) {
            return Err("feedback.step must be above 0".into());
        }
        if self.interval_min == 0 {
            return Err("feedback.interval_min must be above 0".into());
        }
        if self.quiet_limit_min == 0 {
            return Err("feedback.quiet_limit_min must be above 0".into());
        }
        if !(self.max_below.is_finite() && self.max_below >= 0.0) {
            return Err("feedback.max_below must not be negative".into());
        }
        if !(self.max_above.is_finite() && self.max_above >= 0.0) {
            return Err("feedback.max_above must not be negative".into());
        }
        Ok(())
    }
}

/// Room presence setback: lowers a block's setpoint while nobody is detected in the room.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct PresenceSettings {
    pub enabled: bool,
    /// The room's presence sensors (binary sensors); any one on means someone is there.
    pub sensors: Vec<String>,
    /// Minutes with no detection before the room counts as empty.
    pub empty_after_min: u32,
    /// Minutes of presence before an empty room counts as occupied again.
    pub back_after_min: u32,
    /// How far the block's setpoint moves while the room is empty, in degrees (positive).
    pub setback: f64,
}

impl Default for PresenceSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            sensors: Vec::new(),
            empty_after_min: 30,
            back_after_min: 5,
            setback: 2.0,
        }
    }
}

impl PresenceSettings {
    pub fn validate(&self) -> Result<(), String> {
        if self.empty_after_min == 0 {
            return Err("presence.empty_after_min must be above 0".into());
        }
        if self.back_after_min == 0 {
            return Err("presence.back_after_min must be above 0".into());
        }
        if !(self.setback.is_finite() && self.setback > 0.0) {
            return Err("presence.setback must be above 0".into());
        }
        Ok(())
    }
}

/// Settings of one zone (`<prefix>/zone/<zone>/settings`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct ZoneSettings {
    pub state: ZoneState,
    pub entities: Option<Vec<String>>,
    pub sensor: Option<String>,
    pub manual_hold_max_min: u32,
    pub rules: Rules,
    pub feedback: FeedbackSettings,
    pub presence: PresenceSettings,
    pub approved_at: Option<u64>,
    pub approved_proposal: Option<String>,
    pub rejected: Vec<Rejection>,
}

impl Default for ZoneSettings {
    fn default() -> Self {
        Self {
            state: ZoneState::Pending,
            entities: None,
            sensor: None,
            manual_hold_max_min: 240,
            rules: Rules::default(),
            feedback: FeedbackSettings::default(),
            presence: PresenceSettings::default(),
            approved_at: None,
            approved_proposal: None,
            rejected: Vec::new(),
        }
    }
}

impl ZoneSettings {
    pub fn validate(&self) -> Result<(), String> {
        if self.manual_hold_max_min == 0 {
            return Err("manual_hold_max_min must be above 0".into());
        }
        self.feedback.validate()?;
        self.presence.validate()?;
        self.rules.validate()
    }
}

/// Who asked for an override: a hand action on a device, or the user through the hub.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub enum OverrideOrigin {
    Hand,
    User,
}

/// A temporary override of the schedule, until `until` (Unix seconds). The zone document is a
/// `Vec<Override>`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct Override {
    pub entity: Option<String>,
    pub setpoint: Option<Setpoint>,
    pub mode: Option<Mode>,
    pub until: u64,
    pub origin: OverrideOrigin,
}

/// Which rule produced a decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub enum Source {
    Safety,
    Override,
    Holiday,
    /// The zone's own eco until a date.
    Eco,
    /// Nobody home: eco for the whole house.
    Away,
    /// Nobody in the room: the block is set back.
    Empty,
    Schedule,
    Idle,
}

/// Why the safety row of the decision tree acted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub enum SafetyCause {
    Frost,
    Overheat,
    Overcool,
    Stale,
}

/// What the engine wants one device to do. `mode` is the device's hvac mode string.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct DeviceTarget {
    pub setpoint: Option<f64>,
    pub mode: String,
    pub held: bool,
}

/// The engine's latest decision for a zone (`<prefix>/zone/<zone>/decision`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct Decision {
    pub zone: String,
    pub mode: Option<Mode>,
    pub setpoint: Option<Setpoint>,
    pub source: Source,
    /// Why safety acted; set on every `Source::Safety` decision.
    #[serde(default)]
    pub cause: Option<SafetyCause>,
    pub reason: String,
    pub reading: Option<f64>,
    pub reading_age_s: Option<u64>,
    pub outdoor_mean: Option<f64>,
    pub home: bool,
    pub applied: bool,
    pub at: u64,
    pub entities: BTreeMap<String, DeviceTarget>,
    /// Entity -> the fan mode eco changed and what it was, to put back when eco ends.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub eco_restore: BTreeMap<String, FanRestore>,
    /// The offset applied to the zone's unheld devices; `None` when 0 or inactive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adjust: Option<f64>,
}

impl Decision {
    /// Equal ignoring the fields that change on every tick: `at`, `reading`, `reading_age_s` and
    /// `outdoor_mean`. Used to avoid republishing an unchanged decision.
    pub fn same_outcome(&self, other: &Decision) -> bool {
        self.zone == other.zone
            && self.mode == other.mode
            && self.setpoint == other.setpoint
            && self.source == other.source
            && self.cause == other.cause
            && self.reason == other.reason
            && self.home == other.home
            && self.applied == other.applied
            && self.entities == other.entities
            && self.eco_restore == other.eco_restore
            && self.adjust == other.adjust
    }
}

/// Whether a proposal is the first schedule of a zone or a change to an approved one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub enum ProposalKind {
    Initial,
    Change,
}

/// The history window a proposal was learned from (Unix seconds, and its length in days).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct Window {
    pub from: u64,
    pub to: u64,
    pub days: u32,
}

/// A learned schedule awaiting approval (`<prefix>/zone/<zone>/proposal`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct Proposal {
    pub id: String,
    pub zone: String,
    pub kind: ProposalKind,
    pub created: u64,
    pub window: Window,
    pub rules: Rules,
    pub changes: Vec<String>,
    pub fingerprint: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setpoint_serde() {
        assert_eq!(serde_json::to_string(&Setpoint::Off).unwrap(), "\"off\"");
        assert_eq!(
            serde_json::to_string(&Setpoint::Temp(20.5)).unwrap(),
            "20.5"
        );
        assert_eq!(
            serde_json::from_str::<Setpoint>("20.5").unwrap(),
            Setpoint::Temp(20.5)
        );
        assert_eq!(
            serde_json::from_str::<Setpoint>("\"off\"").unwrap(),
            Setpoint::Off
        );
        assert!(serde_json::from_str::<Setpoint>("\"on\"").is_err());
    }

    #[test]
    fn hhmm_serde_and_bounds() {
        assert_eq!(
            serde_json::from_str::<HhMm>("\"07:30\"").unwrap(),
            HhMm(450)
        );
        assert_eq!(
            serde_json::from_str::<HhMm>("\"24:00\"").unwrap(),
            HhMm(1440)
        );
        assert_eq!(serde_json::to_string(&HhMm(450)).unwrap(), "\"07:30\"");
        assert!(serde_json::from_str::<HhMm>("\"24:01\"").is_err());
        assert!(serde_json::from_str::<HhMm>("\"7:5\"").is_err());
        assert!(serde_json::from_str::<HhMm>("\"12:60\"").is_err());
    }

    #[test]
    fn day_from_chrono() {
        assert_eq!(Day::from_chrono(chrono::Weekday::Mon), Day::Mon);
        assert_eq!(Day::from_chrono(chrono::Weekday::Sun), Day::Sun);
    }

    #[test]
    fn house_defaults_match_spec() {
        let h: HouseSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(
            (h.frost, h.overheat, h.overcool, h.stale_after_min),
            (7.0, 30.0, 16.0, 60)
        );
        assert_eq!(
            (h.heat_below, h.cool_above, h.away_grace_min),
            (16.0, 24.0, 15)
        );
        assert_eq!(
            (h.learn_days, h.learn_min_days, h.learn_support, h.learn_at),
            (28, 10, 0.6, HhMm(210))
        );
        assert_eq!(h.season, Season::Auto);
        assert!(h.validate().is_ok());
    }

    #[test]
    fn zone_defaults_match_spec() {
        let z: ZoneSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(z.state, ZoneState::Pending);
        assert_eq!(z.manual_hold_max_min, 240);
        assert!(z.rules.is_empty());
        assert!(z.validate().is_ok());
    }

    fn block(days: &[Day], from: u16, to: u16, sp: f64) -> Block {
        Block {
            days: days.to_vec(),
            from: HhMm(from),
            to: HhMm(to),
            setpoint: Setpoint::Temp(sp),
            evidence: None,
        }
    }

    fn heat(blocks: Vec<Block>) -> Rules {
        Rules {
            heat: Some(ModeRules {
                base: Setpoint::Temp(17.0),
                blocks,
            }),
            cool: None,
        }
    }

    #[test]
    fn rules_validate_rejects_bad_blocks() {
        let bad = [
            heat(vec![block(&[Day::Mon], 600, 600, 20.0)]),
            heat(vec![block(&[Day::Mon], 700, 600, 20.0)]),
            heat(vec![block(&[], 600, 700, 20.0)]),
            heat(vec![
                block(&[Day::Mon, Day::Tue], 600, 700, 20.0),
                block(&[Day::Tue], 650, 800, 21.0),
            ]),
            heat(vec![block(&[Day::Mon], 600, 700, 41.0)]),
            heat(vec![block(&[Day::Mon], 600, 700, -1.0)]),
            heat(vec![block(&[Day::Mon], 1400, 1441, 20.0)]),
            Rules {
                heat: None,
                cool: Some(ModeRules {
                    base: Setpoint::Temp(50.0),
                    blocks: vec![],
                }),
            },
        ];
        for r in bad {
            assert!(r.validate().is_err(), "{r:?}");
        }
    }

    #[test]
    fn rules_validate_accepts_block_to_24_00() {
        let r = heat(vec![
            block(&[Day::Mon], 1320, 1440, 19.0),
            block(&[Day::Mon], 360, 1320, 21.0),
        ]);
        assert!(r.validate().is_ok());
        let touching_other_days = heat(vec![
            block(&[Day::Mon], 600, 700, 20.0),
            block(&[Day::Tue], 600, 700, 20.0),
        ]);
        assert!(touching_other_days.validate().is_ok());
        assert!(heat(vec![]).validate().is_ok());
    }

    #[test]
    fn house_validate_rejects() {
        let d = HouseSettings::default;
        let bad = [
            HouseSettings { frost: 30.0, ..d() },
            HouseSettings {
                learn_support: 0.0,
                ..d()
            },
            HouseSettings {
                learn_support: 1.1,
                ..d()
            },
            HouseSettings {
                learn_min_days: 29,
                ..d()
            },
            HouseSettings {
                stale_after_min: 0,
                ..d()
            },
            HouseSettings {
                learn_days: 0,
                learn_min_days: 0,
                ..d()
            },
            HouseSettings {
                learn_min_days: 0,
                ..d()
            },
        ];
        for h in bad {
            assert!(h.validate().is_err(), "{h:?}");
        }
        // overcool >= cool_above is fine, learn_support == 1 is fine
        let h = HouseSettings {
            overcool: 25.0,
            learn_support: 1.0,
            ..d()
        };
        assert!(h.validate().is_ok());
        let h = HouseSettings {
            learn_days: 1,
            learn_min_days: 1,
            ..d()
        };
        assert!(h.validate().is_ok());
    }

    #[test]
    fn zone_validate_rejects() {
        let z = ZoneSettings {
            manual_hold_max_min: 0,
            ..Default::default()
        };
        assert!(z.validate().is_err());
        let z = ZoneSettings {
            rules: heat(vec![block(&[Day::Mon], 700, 600, 20.0)]),
            ..Default::default()
        };
        assert!(z.validate().is_err());
    }

    fn decision(at: u64) -> Decision {
        Decision {
            zone: "z".into(),
            mode: Some(Mode::Heat),
            setpoint: Some(Setpoint::Temp(20.0)),
            source: Source::Schedule,
            cause: None,
            reason: "r".into(),
            reading: Some(19.0),
            reading_age_s: Some(5),
            outdoor_mean: Some(8.0),
            home: true,
            applied: true,
            at,
            entities: BTreeMap::new(),
            eco_restore: BTreeMap::new(),
            adjust: None,
        }
    }

    #[test]
    fn feedback_defaults_and_old_documents() {
        let z: ZoneSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(z.feedback, FeedbackSettings::default());
        assert!(!z.feedback.enabled);
        assert_eq!(z.feedback.band, 0.5);
        assert_eq!(z.feedback.quiet_limit_min, 360);
    }

    #[test]
    fn feedback_validation() {
        assert!(ZoneSettings::default().validate().is_ok());
        let bad = |f: fn(&mut FeedbackSettings)| {
            let mut z = ZoneSettings::default();
            f(&mut z.feedback);
            assert!(z.validate().is_err());
        };
        bad(|f| f.step = 0.0);
        bad(|f| f.band = 0.0);
        bad(|f| f.interval_min = 0);
        bad(|f| f.quiet_limit_min = 0);
        bad(|f| f.max_below = -1.0);
        bad(|f| f.max_above = -0.5);
    }

    #[test]
    fn presence_defaults_and_old_documents() {
        let z: ZoneSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(z.presence, PresenceSettings::default());
        assert!(!z.presence.enabled);
        assert!(z.presence.sensors.is_empty());
        assert_eq!(z.presence.empty_after_min, 30);
        assert_eq!(z.presence.back_after_min, 5);
        assert_eq!(z.presence.setback, 2.0);
    }

    #[test]
    fn presence_validation() {
        assert!(ZoneSettings::default().validate().is_ok());
        let bad = |f: fn(&mut PresenceSettings)| {
            let mut z = ZoneSettings::default();
            f(&mut z.presence);
            assert!(z.validate().is_err());
        };
        bad(|p| p.empty_after_min = 0);
        bad(|p| p.back_after_min = 0);
        bad(|p| p.setback = 0.0);
        bad(|p| p.setback = f64::NAN);
    }

    #[test]
    fn empty_source_round_trips() {
        assert_eq!(serde_json::to_string(&Source::Empty).unwrap(), "\"empty\"");
        assert_eq!(
            serde_json::from_str::<Source>("\"empty\"").unwrap(),
            Source::Empty
        );
    }

    #[test]
    fn decision_adjust_is_optional() {
        let mut d = decision(1);
        let json = serde_json::to_string(&d).unwrap();
        assert!(!json.contains("adjust"));
        assert_eq!(
            serde_json::from_str::<Decision>(&json).unwrap().adjust,
            None
        );
        d.adjust = Some(0.5);
        let json = serde_json::to_string(&d).unwrap();
        assert_eq!(
            serde_json::from_str::<Decision>(&json).unwrap().adjust,
            Some(0.5)
        );
        assert!(!decision(1).same_outcome(&d));
    }

    #[test]
    fn same_outcome_ignores_volatile_fields() {
        let a = decision(1);
        let mut b = decision(2);
        b.reading = Some(19.5);
        b.reading_age_s = None;
        b.outdoor_mean = None;
        assert!(a.same_outcome(&b));
        b.reason = "other".into();
        assert!(!a.same_outcome(&b));
        let mut c = decision(1);
        c.entities.insert(
            "e".into(),
            DeviceTarget {
                setpoint: Some(20.0),
                mode: "heat".into(),
                held: false,
            },
        );
        assert!(!a.same_outcome(&c));
    }

    #[test]
    fn documents_roundtrip() {
        let p = Proposal {
            id: "p1".into(),
            zone: "z".into(),
            kind: ProposalKind::Initial,
            created: 10,
            window: Window {
                from: 1,
                to: 2,
                days: 28,
            },
            rules: heat(vec![Block {
                evidence: Some(Evidence { agree: 7, of: 8 }),
                ..block(&[Day::Mon], 360, 1320, 21.0)
            }]),
            changes: vec!["c".into()],
            fingerprint: "f".into(),
        };
        let s = serde_json::to_string(&p).unwrap();
        assert_eq!(serde_json::from_str::<Proposal>(&s).unwrap(), p);
        let o = vec![Override {
            entity: None,
            setpoint: Some(Setpoint::Off),
            mode: Some(Mode::Cool),
            until: 5,
            origin: OverrideOrigin::Hand,
        }];
        let s = serde_json::to_string(&o).unwrap();
        assert_eq!(serde_json::from_str::<Vec<Override>>(&s).unwrap(), o);
        assert!(
            !serde_json::to_string(&block(&[Day::Mon], 1, 2, 3.0))
                .unwrap()
                .contains("evidence")
        );
    }

    #[test]
    fn eco_defaults_and_shape() {
        let h: HouseSettings = serde_json::from_str("{}").unwrap();
        assert_eq!((h.eco.heat, h.eco.cool), (16.0, 28.0));
        assert!(h.eco.actions.is_empty());
        let e: EcoSettings =
            serde_json::from_str(r#"{"heat":15,"actions":[{"fan_mode":"low"}]}"#).unwrap();
        assert_eq!(e.fan_mode(), Some("low"));
        assert_eq!(e.setpoint(Mode::Heat), 15.0);
        assert_eq!(e.setpoint(Mode::Cool), 28.0);
        assert_eq!(
            serde_json::to_value(&e.actions).unwrap(),
            serde_json::json!([{ "fan_mode": "low" }])
        );
        assert!(
            serde_json::from_str::<EcoSettings>(r#"{"actions":[{"swing_mode":"off"}]}"#).is_err()
        );
    }

    #[test]
    fn eco_validation() {
        let mut h = HouseSettings::default();
        h.eco.heat = 6.0;
        assert!(h.validate().unwrap_err().contains("eco"));
        let mut h = HouseSettings::default();
        h.eco.cool = 31.0;
        assert!(h.validate().unwrap_err().contains("eco"));
        // Inside frost..overheat is not enough: a setpoint stays within 0..=40.
        let mut h = HouseSettings {
            overheat: 60.0,
            ..HouseSettings::default()
        };
        h.eco.heat = 45.0;
        assert!(h.validate().unwrap_err().contains("eco heat"));
        let mut h = HouseSettings::default();
        h.eco.cool = f64::NAN;
        assert!(h.validate().unwrap_err().contains("eco cool"));
        let mut h = HouseSettings::default();
        h.eco.actions = vec![
            EcoAction::FanMode("low".into()),
            EcoAction::FanMode("high".into()),
        ];
        assert!(h.validate().is_err());
        let mut h = HouseSettings::default();
        h.eco.actions = vec![EcoAction::FanMode(String::new())];
        assert!(h.validate().is_err());
        let mut h = HouseSettings::default();
        h.eco.actions = vec![EcoAction::FanMode("low".into())];
        assert!(h.validate().is_ok());
    }

    #[test]
    fn old_zone_settings_with_away_setpoint_still_load() {
        let z: ZoneSettings =
            serde_json::from_str(r#"{"state":"active","away_setpoint":{"heat":16,"cool":28}}"#)
                .unwrap();
        assert_eq!(z.state, ZoneState::Active);
    }

    #[test]
    fn decision_eco_restore_is_optional() {
        let d = Decision {
            zone: "z".into(),
            mode: Some(Mode::Heat),
            setpoint: None,
            source: Source::Eco,
            cause: None,
            reason: "r".into(),
            reading: None,
            reading_age_s: None,
            outdoor_mean: None,
            home: true,
            applied: false,
            at: 1,
            entities: BTreeMap::new(),
            eco_restore: BTreeMap::new(),
            adjust: None,
        };
        let s = serde_json::to_string(&d).unwrap();
        assert!(!s.contains("eco_restore"), "{s}");
        assert!(s.contains(r#""source":"eco""#), "{s}");
        assert_eq!(serde_json::from_str::<Decision>(&s).unwrap(), d);
        let mut with = d.clone();
        with.eco_restore.insert(
            "a".into(),
            FanRestore {
                was: "auto".into(),
                set: "low".into(),
                state: FanState::Set,
            },
        );
        assert!(!with.same_outcome(&d));
        let s = serde_json::to_string(&with).unwrap();
        assert_eq!(serde_json::from_str::<Decision>(&s).unwrap(), with);
    }

    #[test]
    fn room_eco_round_trips() {
        let e = RoomEco {
            until: 10,
            set_at: 5,
        };
        let s = serde_json::to_string(&e).unwrap();
        assert_eq!(serde_json::from_str::<RoomEco>(&s).unwrap(), e);
    }
}
