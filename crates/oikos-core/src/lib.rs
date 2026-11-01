#![forbid(unsafe_code)]
//! Pure domain of Nidoo: the model shared by the hub, the API and the web UI.
//! No I/O, no async; compiles to native and `wasm32-unknown-unknown`.

pub mod energy;
pub mod flow;
pub mod history;
pub mod model;

mod config;

pub mod command;
pub mod dashboard;
pub mod discovery;
pub mod registry;
pub mod state;
pub mod template;
pub mod types;
pub mod vocabulary;
