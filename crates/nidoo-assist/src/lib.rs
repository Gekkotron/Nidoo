//! French sentences about the house turned into plans: what to do, what to answer, what to ask.
//!
//! Pure data: no I/O, builds for wasm32.

#[cfg(test)]
pub mod fixture;
pub mod house;
pub mod intent;
pub mod lexicon;
pub mod room;
pub mod text;

pub use house::{HeatRange, House};
pub use room::{RoomMatch, resolve_room};
