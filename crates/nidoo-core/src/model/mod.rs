//! Domain types, mirroring the Android app's Kotlin model field for field.

mod area;
mod command;
mod device;
mod entity;
mod kind;
mod placement;
mod state;
mod switch_type;
pub(crate) mod ui_override;

pub use area::{Area, AreasDocument, UNASSIGNED_AREA};
pub use command::{AlarmMode, Command, Hs};
pub use device::{BridgeInfo, Device};
pub use entity::{Bindings, Capabilities, CodeFormat, Entity, EntityCategory};
pub use kind::{IGNORED_COMPONENTS, Kind};
pub use placement::{Placement, SensorGroup, SensorSection};
pub use state::{
    Availability, ClimateState, CoverState, EntityState, GenericState, GenericValue, LightState,
    StateValue, VacuumState, WaterHeaterState,
};
pub use switch_type::SwitchType;
pub use ui_override::{ENERGY_ROLES, LIGHT_ON_PRESETS, PatchError, SHOW_AS, UiOverride};
