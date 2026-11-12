//! The snapshot of the house a plan is made against, borrowed from the hub's views.

use nidoo_api::{AreaView, EntityView};
use nidoo_climate::view::ZoneView;
use nidoo_core::flow::Lang;

/// The setpoints the assistant applies without asking, in °C.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeatRange {
    pub min: f64,
    pub max: f64,
}

impl Default for HeatRange {
    fn default() -> Self {
        Self {
            min: 16.0,
            max: 23.0,
        }
    }
}

/// What a plan reads: rooms, entities with their state, climate zones and a few settings.
#[derive(Clone, Copy, Debug)]
pub struct House<'a> {
    pub areas: &'a [AreaView],
    pub entities: &'a [EntityView],
    pub zones: &'a [ZoneView],
    /// The climate engine's outdoor sensor, when set.
    pub outdoor_entity: Option<&'a str>,
    /// A reading older than this is said with its age.
    pub stale_after_min: u32,
    pub range: HeatRange,
    /// Unix seconds.
    pub now: u64,
    /// The language sentences are read in and replies are written in.
    pub lang: Lang,
}

impl House<'_> {
    pub fn area(&self, id: &str) -> Option<&AreaView> {
        self.areas.iter().find(|a| a.id == id)
    }

    pub fn area_name<'b>(&'b self, id: &'b str) -> &'b str {
        self.area(id).map_or(id, |a| a.name.as_str())
    }

    pub fn entity(&self, id: &str) -> Option<&EntityView> {
        self.entities.iter().find(|e| e.id == id)
    }

    pub fn zone(&self, id: &str) -> Option<&ZoneView> {
        self.zones.iter().find(|z| z.zone == id)
    }
}
