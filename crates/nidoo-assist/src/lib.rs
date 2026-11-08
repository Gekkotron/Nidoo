//! French sentences about the house turned into plans: what to do, what to answer, what to ask.
//!
//! Pure data: no I/O, builds for wasm32.

#[cfg(any(test, feature = "fixture"))]
pub mod fixture;
pub mod house;
pub mod intent;
pub mod lexicon;
pub mod pending;
pub mod plan;
pub mod reply;
pub mod room;
pub mod text;
pub mod view;

pub use house::{HeatRange, House};
pub use pending::{Pending, PendingAnswer, answer_text};
pub use plan::{Action, Done, Plan, Request, plan, plan_request};
pub use reply::ActionResult;
pub use room::{RoomMatch, resolve_room};
