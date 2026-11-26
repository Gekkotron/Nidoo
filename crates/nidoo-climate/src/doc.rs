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
    /// Heat blocks only: the heaters and units that heat the room in this block. `None` follows
    /// the room's `heated_by`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heated_by: Option<Vec<String>>,
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
            if label == "cool" && b.heated_by.is_some() {
                return Err(format!("{at}: heated_by only applies to heat blocks"));
            }
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
    /// Minimum minutes between two steps. The saving side's first move (lowering in heat, raising
    /// in cool) jumps by the gap at once instead.
    pub interval_min: u32,
    /// Longest wait after the schedule target began before the slow side may step (raising the
    /// setpoint in heat, extra cooling in cool). The wait ends earlier once the room reaches the
    /// band.
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

/// How the engine drives an on/off heater.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub enum HeaterDrive {
    /// Switch on below the target minus the band, off at the target.
    #[default]
    Thermostat,
    /// Hold the heater on while the room is in a heating block.
    Hold,
}

/// An on/off switch heater driven by the engine.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct Heater {
    pub entity: String,
    pub drive: HeaterDrive,
    pub enabled: bool,
    /// Minimum minutes off before switching on again (0..=240).
    pub min_off_min: u32,
    /// Minimum minutes on before switching off again (0..=120).
    pub min_on_min: u32,
}

impl Default for Heater {
    fn default() -> Self {
        Self {
            entity: String::new(),
            drive: HeaterDrive::default(),
            enabled: true,
            min_off_min: 0,
            min_on_min: 0,
        }
    }
}

/// What the engine remembers of one heater between passes (Unix seconds).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct HeaterRun {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub off_since: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_since: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait_until: Option<u64>,
}

/// A thermostat heater switches on at `target - THERMOSTAT_BAND` and off at `target`.
pub const THERMOSTAT_BAND: f64 = 0.3;

fn default_true() -> bool {
    true
}

fn is_true(b: &bool) -> bool {
    *b
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
    /// On/off switch heaters of the zone.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub heaters: Vec<Heater>,
    /// The heaters and units that heat the room by default; `None` means every unit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heated_by: Option<Vec<String>>,
    /// The climate units heat, at the same target, while a chosen on/off heater cannot run
    /// (cooling down, unreachable).
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub fill_in: bool,
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
            heaters: Vec::new(),
            heated_by: None,
            fill_in: true,
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
        for (i, h) in self.heaters.iter().enumerate() {
            if h.entity.starts_with("group_") {
                return Err(format!("heater {}: a group cannot be a heater", h.entity));
            }
            if self.heaters[..i].iter().any(|o| o.entity == h.entity) {
                return Err(format!("heater {} listed twice", h.entity));
            }
            if h.min_off_min > 240 {
                return Err(format!(
                    "heater {}: min_off_min must be at most 240",
                    h.entity
                ));
            }
            if h.min_on_min > 120 {
                return Err(format!(
                    "heater {}: min_on_min must be at most 120",
                    h.entity
                ));
            }
        }
        self.rules.validate()
    }

    /// The heater with this entity id, enabled or not.
    pub fn heater(&self, id: &str) -> Option<&Heater> {
        self.heaters.iter().find(|h| h.entity == id)
    }

    /// Checks the room's `heated_by` and every heat block's against this zone's heaters and the
    /// given unit entity ids.
    pub fn validate_heated_by(&self, units: &[String]) -> Result<(), String> {
        let known = |id: &String| self.heater(id).is_some() || units.contains(id);
        let check = |at: &str, ids: &[String]| -> Result<(), String> {
            if ids.is_empty() {
                return Err(format!("{at}: heated_by is empty"));
            }
            match ids.iter().find(|id| !known(id)) {
                Some(id) => Err(format!("{at}: unknown heater {id}")),
                None => Ok(()),
            }
        };
        if let Some(ids) = &self.heated_by {
            check("room", ids)?;
        }
        if let Some(heat) = &self.rules.heat {
            for (i, b) in heat.blocks.iter().enumerate() {
                if let Some(ids) = &b.heated_by {
                    check(&format!("heat block {i}"), ids)?;
                }
            }
        }
        Ok(())
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
    /// A whole-zone override with a temperature: a room target the correction drives.
    Boost,
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
    /// The house was on holiday; with `home`, how the next pass tells the house was just left.
    #[serde(default)]
    pub holiday: bool,
    pub applied: bool,
    pub at: u64,
    pub entities: BTreeMap<String, DeviceTarget>,
    /// Entity -> the fan mode eco changed and what it was, to put back when eco ends.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub eco_restore: BTreeMap<String, FanRestore>,
    /// The offset applied to the zone's unheld devices; `None` when 0 or inactive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adjust: Option<f64>,
    /// Heater entity -> what the engine remembers of it.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub heaters: BTreeMap<String, HeaterRun>,
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
            && self.holiday == other.holiday
            && self.applied == other.applied
            && self.entities == other.entities
            && self.eco_restore == other.eco_restore
            && self.adjust == other.adjust
            && self.heaters == other.heaters
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
mod tests;
