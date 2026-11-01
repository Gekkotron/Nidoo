#![forbid(unsafe_code)]
//! Push notifications for the hub, ported from OikosPush. This module tree is pure at the
//! bottom: `normalize` turns producer payloads into alerts and FCM data maps.

pub mod fcm;
pub mod history;
pub mod normalize;
pub mod queue;
pub mod service;
pub mod tokens;

pub use history::{History, to_history_entry};
pub use normalize::{
    Action, Alert, Priority, Rejected, android_priority, normalize as normalize_payload,
    to_fcm_data, ttl_seconds,
};
pub use queue::{Pending, RetryQueue};
pub use service::{PushService, PushSettings, PushState, PushStatus};
pub use tokens::{TokenStatus, Tokens};
