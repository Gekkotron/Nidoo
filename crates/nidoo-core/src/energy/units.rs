//! Energy roles, consumer icons and the units the Energy screen understands.

/// What an entity is on the Energy screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EnergyRole {
    Solar,
    Grid,
    GridExport,
    Battery,
    House,
    Consumer,
    Water,
}

impl EnergyRole {
    /// The stored role name; `water_heater` is `consumer`'s name from before the energy screen.
    pub fn parse(s: &str) -> Option<EnergyRole> {
        match s {
            "solar" => Some(EnergyRole::Solar),
            "grid" => Some(EnergyRole::Grid),
            "grid_export" => Some(EnergyRole::GridExport),
            "battery" => Some(EnergyRole::Battery),
            "house" => Some(EnergyRole::House),
            "consumer" | "water_heater" => Some(EnergyRole::Consumer),
            "water" => Some(EnergyRole::Water),
            _ => None,
        }
    }

    /// Its sign means a direction and can be flipped by `energy_invert`.
    pub fn signed(self) -> bool {
        matches!(self, EnergyRole::Grid | EnergyRole::Battery)
    }

    /// Its value must be a power ([`PowerUnits`]).
    pub fn power(self) -> bool {
        self != EnergyRole::Water
    }

    pub fn name(self) -> &'static str {
        match self {
            EnergyRole::Solar => "Solar",
            EnergyRole::Grid => "Grid",
            EnergyRole::GridExport => "Grid export",
            EnergyRole::Battery => "Battery",
            EnergyRole::House => "House",
            EnergyRole::Consumer => "Consumer",
            EnergyRole::Water => "Water",
        }
    }
}

/// The glyphs a consumer can wear on the Energy screen, by their stored key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConsumerIcon {
    WaterHeater,
    HeatPump,
    Oven,
    Fridge,
    Washer,
    Dryer,
    Dishwasher,
}

impl ConsumerIcon {
    pub fn parse(s: &str) -> Option<ConsumerIcon> {
        match s {
            "water_heater" => Some(ConsumerIcon::WaterHeater),
            "heat_pump" => Some(ConsumerIcon::HeatPump),
            "oven" => Some(ConsumerIcon::Oven),
            "fridge" => Some(ConsumerIcon::Fridge),
            "washer" => Some(ConsumerIcon::Washer),
            "dryer" => Some(ConsumerIcon::Dryer),
            "dishwasher" => Some(ConsumerIcon::Dishwasher),
            _ => None,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            ConsumerIcon::WaterHeater => "water_heater",
            ConsumerIcon::HeatPump => "heat_pump",
            ConsumerIcon::Oven => "oven",
            ConsumerIcon::Fridge => "fridge",
            ConsumerIcon::Washer => "washer",
            ConsumerIcon::Dryer => "dryer",
            ConsumerIcon::Dishwasher => "dishwasher",
        }
    }
}

/// The power units the energy screen understands.
pub struct PowerUnits;

impl PowerUnits {
    /// Watts per one `unit`, or `None` when it is not a power unit. W and kW ignore case; `mW`
    /// (milliwatt) and `MW` (megawatt) differ only by case, so they match exactly and `mw`
    /// matches neither.
    pub fn factor(unit: Option<&str>) -> Option<f64> {
        let u = unit?.trim();
        if u == "mW" {
            Some(0.001)
        } else if u == "MW" {
            Some(1_000_000.0)
        } else if u.eq_ignore_ascii_case("w") {
            Some(1.0)
        } else if u.eq_ignore_ascii_case("kw") {
            Some(1_000.0)
        } else {
            None
        }
    }
}

/// The energy-counter units the energy screen understands.
pub struct EnergyUnits;

impl EnergyUnits {
    /// kWh per one `unit`, or `None` when it is not an energy unit. Wh and kWh ignore case;
    /// `MWh` matches exactly, like `MW` for power.
    pub fn factor(unit: Option<&str>) -> Option<f64> {
        let u = unit?.trim();
        if u == "MWh" {
            Some(1_000.0)
        } else if u.eq_ignore_ascii_case("wh") {
            Some(0.001)
        } else if u.eq_ignore_ascii_case("kwh") {
            Some(1.0)
        } else {
            None
        }
    }
}

/// The water volume units Home Assistant reports.
pub struct VolumeUnits;

impl VolumeUnits {
    /// Any water volume unit: L, mL, m³, ft³, gal, CCF. Case and the ASCII "3" for "³" don't matter.
    pub fn is_volume(unit: Option<&str>) -> bool {
        match key(unit) {
            Some(k) => METER_SCALE.contains(&k.as_str()) || VOLUME_ONLY.contains(&k.as_str()),
            None => false,
        }
    }

    /// A unit a meter index is kept in (m³, ft³, CCF), rather than a day's usage.
    pub fn is_meter_scale(unit: Option<&str>) -> bool {
        key(unit).is_some_and(|k| METER_SCALE.contains(&k.as_str()))
    }
}

const METER_SCALE: [&str; 3] = ["m³", "ft³", "ccf"];
const VOLUME_ONLY: [&str; 3] = ["l", "ml", "gal"];

fn key(unit: Option<&str>) -> Option<String> {
    Some(unit?.trim().to_lowercase().replace('3', "³"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_roles_with_alias() {
        assert_eq!(EnergyRole::parse("solar"), Some(EnergyRole::Solar));
        assert_eq!(
            EnergyRole::parse("grid_export"),
            Some(EnergyRole::GridExport)
        );
        assert_eq!(
            EnergyRole::parse("water_heater"),
            Some(EnergyRole::Consumer)
        );
        assert_eq!(EnergyRole::parse("consumer"), Some(EnergyRole::Consumer));
        assert_eq!(EnergyRole::parse("Solar"), None);
        assert_eq!(EnergyRole::parse(""), None);
        assert!(EnergyRole::Grid.signed() && EnergyRole::Battery.signed());
        assert!(!EnergyRole::Solar.signed());
        assert!(!EnergyRole::Water.power() && EnergyRole::House.power());
        assert_eq!(EnergyRole::GridExport.name(), "Grid export");
        assert_eq!(
            ConsumerIcon::parse("heat_pump"),
            Some(ConsumerIcon::HeatPump)
        );
        assert_eq!(ConsumerIcon::parse("toaster"), None);
        assert_eq!(ConsumerIcon::Dishwasher.key(), "dishwasher");
    }

    #[test]
    fn power_factors() {
        assert_eq!(PowerUnits::factor(Some("W")), Some(1.0));
        assert_eq!(PowerUnits::factor(Some("w")), Some(1.0));
        assert_eq!(PowerUnits::factor(Some("kw")), Some(1000.0));
        assert_eq!(PowerUnits::factor(Some(" kW ")), Some(1000.0));
        assert_eq!(PowerUnits::factor(Some("mW")), Some(0.001));
        assert_eq!(PowerUnits::factor(Some("MW")), Some(1e6));
        assert_eq!(PowerUnits::factor(Some("mw")), None);
        assert_eq!(PowerUnits::factor(Some("kWh")), None);
        assert_eq!(PowerUnits::factor(None), None);
    }

    #[test]
    fn energy_factors() {
        assert_eq!(EnergyUnits::factor(Some("Wh")), Some(0.001));
        assert_eq!(EnergyUnits::factor(Some("wh")), Some(0.001));
        assert_eq!(EnergyUnits::factor(Some("kWh")), Some(1.0));
        assert_eq!(EnergyUnits::factor(Some("KWH")), Some(1.0));
        assert_eq!(EnergyUnits::factor(Some("MWh")), Some(1000.0));
        assert_eq!(EnergyUnits::factor(Some("mwh")), None);
        assert_eq!(EnergyUnits::factor(Some("W")), None);
        assert_eq!(EnergyUnits::factor(None), None);
    }

    #[test]
    fn volume_units() {
        for u in ["m³", "m3", "ft³", "ft3", "CCF", "ccf"] {
            assert!(VolumeUnits::is_meter_scale(Some(u)), "{u}");
            assert!(VolumeUnits::is_volume(Some(u)), "{u}");
        }
        for u in ["L", "l", "mL", "gal"] {
            assert!(VolumeUnits::is_volume(Some(u)), "{u}");
            assert!(!VolumeUnits::is_meter_scale(Some(u)), "{u}");
        }
        assert!(!VolumeUnits::is_volume(Some("kWh")));
        assert!(!VolumeUnits::is_volume(None));
        assert!(!VolumeUnits::is_meter_scale(None));
    }
}
