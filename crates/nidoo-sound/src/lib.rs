//! nidoo-sound: a Sendspin-compatible server.
//!
//! It plays one raw PCM input (the phone, on TCP 4953) on every approved
//! satellite in sync, and exposes each satellite's volume, mute and approval
//! to Nidoo through MQTT discovery.

pub mod clock;
pub mod discover;
pub mod player;
pub mod proto;
pub mod tone;
