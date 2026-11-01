//! The Energy screen's maths, ported rule for rule from the Android app's pure Kotlin: units,
//! meter pairing and the flow builder. No I/O; the caller maps its entities to `EnergyEntity`.

mod flow;
mod meters;
mod tariff;
mod units;

pub use flow::*;
pub use meters::*;
pub use tariff::*;
pub use units::*;

#[cfg(test)]
pub(crate) mod testkit {
    use super::EnergyEntity;

    /// A live power sensor in watts with no role; override fields with struct update syntax.
    pub fn e(id: &str) -> EnergyEntity {
        EnergyEntity {
            id: id.to_string(),
            device_id: None,
            name: id.to_string(),
            label: None,
            unit: Some("W".to_string()),
            value: None,
            available: true,
            role: None,
            invert: false,
            order: None,
            icon: None,
            meter: None,
        }
    }
}
