//! The hub's REST API, grouped by surface.

pub mod alarm;
pub mod ambiances;
pub mod climate;
mod core;
pub mod history;
pub mod home;
pub mod series_cache;
pub mod setup;

pub use core::*;
