//! The Sendspin player side: we advertise `_sendspin._tcp`, `nidoo-sound` connects to
//! us, and we play the `player@v1` role (Noise responder, clock sync, audio chunks).

pub mod clock;
mod conn;
pub mod messages;
pub mod noise;

use nidoo_sound::proto::messages::AudioFormat;

pub use conn::serve;

/// The WebSocket path we advertise and accept.
pub const PATH: &str = "/sendspin";
/// The mDNS service type `nidoo-sound` browses.
pub const SERVICE_TYPE: &str = nidoo_sound::discover::SERVICE_TYPE;

/// How we present ourselves to the server.
#[derive(Debug, Clone)]
pub struct PlayerConfig {
    /// The player name Nidoo shows (the computer's name).
    pub name: String,
    /// Bytes of outstanding audio chunks (header + payload) we can hold.
    pub buffer_capacity: u32,
}

/// What the server did.
#[derive(Debug, Clone, PartialEq)]
pub enum SendspinEvent {
    /// Handshake done, `client/hello` and `client/state` sent.
    Connected,
    /// The connection ended (only after `Connected`).
    Disconnected,
    StreamStart(AudioFormat),
    /// PCM to play at `local_play_at_us` on [`clock::now_us`]'s clock.
    Chunk {
        local_play_at_us: i64,
        pcm: bytes::Bytes,
    },
    StreamEnd,
    Volume(u8),
    Mute(bool),
}

/// The volume and mute we apply; a change is reported with `client/state`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalPlayerState {
    pub volume: u8,
    pub muted: bool,
}

/// The mDNS record for our player on `port`.
pub fn service_info(port: u16, instance: &str) -> anyhow::Result<mdns_sd::ServiceInfo> {
    let host: String = instance
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let info = mdns_sd::ServiceInfo::new(
        SERVICE_TYPE,
        instance,
        &format!("{host}.local."),
        "",
        port,
        &[("path", PATH)][..],
    )?
    .enable_addr_auto();
    Ok(info)
}

/// Advertises our player on `port` until the returned daemon is dropped or shut down.
pub fn advertise(port: u16, instance: &str) -> anyhow::Result<mdns_sd::ServiceDaemon> {
    let daemon = mdns_sd::ServiceDaemon::new()?;
    daemon.register(service_info(port, instance)?)?;
    Ok(daemon)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_info_advertises_sendspin_with_path() {
        let info = service_info(4962, "Test Mac").unwrap();
        assert_eq!(info.get_type(), "_sendspin._tcp.local.");
        assert_eq!(info.get_fullname(), "Test Mac._sendspin._tcp.local.");
        assert_eq!(info.get_port(), 4962);
        assert_eq!(info.get_property_val_str("path"), Some("/sendspin"));
        assert_eq!(info.get_hostname(), "Test-Mac.local.");
    }
}
