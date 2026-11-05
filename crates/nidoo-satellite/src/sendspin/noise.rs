//! Sendspin's Noise layer from the player's side: the `KKpsk2` responder with the
//! Sentinel PSK, then transport encryption. `nidoo-sound` is the initiator.

use anyhow::{Context, bail};
use nidoo_sound::proto::noise::{SENTINEL_PSK, Suite, decode_client_id, psk_id};

/// Largest Noise message (handshake or transport).
const NOISE_MAX: usize = 65_535;
/// AEAD tag added by the suite.
const TAG_LEN: usize = 16;

/// The responder between `server/init` and our message 2.
pub struct NoiseClient {
    hs: snow::HandshakeState,
}

impl NoiseClient {
    /// `server_id` is the server's base64url static public key from `server/init`.
    pub fn new(private: &[u8; 32], server_id: &str, prologue: &[u8]) -> anyhow::Result<Self> {
        let server_pub = decode_client_id(server_id).context("server_id")?;
        let hs = snow::Builder::new(Suite::ChaChaPoly.noise_params())
            .local_private_key(private)?
            .remote_public_key(&server_pub)?
            .psk(2, &SENTINEL_PSK)?
            .prologue(prologue)?
            .build_responder()?;
        Ok(Self { hs })
    }

    /// Reads message 1 (its payload must name the Sentinel PSK) and writes message 2
    /// (payload `{}`).
    pub fn respond(mut self, msg1: &[u8]) -> anyhow::Result<(Vec<u8>, NoiseTransport)> {
        let mut payload = vec![0u8; NOISE_MAX];
        let n = self.hs.read_message(msg1, &mut payload)?;
        let named: serde_json::Value =
            serde_json::from_slice(&payload[..n]).context("noise message 1 payload")?;
        if named.get("psk_id").and_then(|v| v.as_str()) != Some(psk_id(&SENTINEL_PSK).as_str()) {
            bail!("the server offers a PSK other than the Sentinel one");
        }
        let mut msg2 = vec![0u8; NOISE_MAX];
        let n = self.hs.write_message(b"{}", &mut msg2)?;
        msg2.truncate(n);
        let t = self.hs.into_transport_mode()?;
        Ok((msg2, NoiseTransport { t }))
    }
}

/// Encrypts and decrypts binary messages after the handshake.
pub struct NoiseTransport {
    t: snow::TransportState,
}

impl NoiseTransport {
    pub fn encrypt(&mut self, plain: &[u8]) -> anyhow::Result<Vec<u8>> {
        if plain.len() + TAG_LEN > NOISE_MAX {
            bail!("message of {} bytes needs fragmentation", plain.len());
        }
        let mut out = vec![0u8; plain.len() + TAG_LEN];
        let n = self.t.write_message(plain, &mut out)?;
        out.truncate(n);
        Ok(out)
    }

    pub fn decrypt(&mut self, cipher: &[u8]) -> anyhow::Result<Vec<u8>> {
        let mut out = vec![0u8; cipher.len()];
        let n = self.t.read_message(cipher, &mut out)?;
        out.truncate(n);
        Ok(out)
    }
}
