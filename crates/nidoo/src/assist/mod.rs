//! The assistant: French sentences planned by `nidoo-assist` and executed through the hub's
//! own paths (`send_command` for lights, `set_climate_override` for setpoints), with one
//! pending question per client and a log of the turns.

mod db;
mod hub;
pub(crate) mod log;

pub use hub::{AssistState, MAX_CLIENT_LEN, PENDING_TTL_S};
