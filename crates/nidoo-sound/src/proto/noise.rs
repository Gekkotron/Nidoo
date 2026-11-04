//! Sendspin's Noise layer: the `KKpsk2` handshake with the server as the
//! initiator, then transport encryption.
//!
//! The server uses the published Sentinel PSK (unpaired access); pairing is
//! not implemented.

use std::io::Write;
use std::path::Path;

use anyhow::{Context, bail};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};

/// `SHA-256("sendspin-sentinel-psk-v1")`, the published PSK for unpaired sessions.
pub const SENTINEL_PSK: [u8; 32] = [
    0x1b, 0x5e, 0x24, 0xdb, 0xc1, 0xae, 0xd9, 0x5f, 0xc2, 0xa5, 0xa3, 0x38, 0xa9, 0x0c, 0x05, 0xdf,
    0x44, 0xbd, 0x10, 0xf5, 0xec, 0x1f, 0x4c, 0xd6, 0x6c, 0xbf, 0x86, 0x27, 0x27, 0x67, 0xb9, 0xd3,
];
/// The `psk_category` of the Sentinel PSK.
pub const SENTINEL_CATEGORY: &str = "sn";

/// Largest Noise message (handshake or transport).
const NOISE_MAX: usize = 65_535;
/// AEAD tag added by both suites.
const TAG_LEN: usize = 16;

/// `base64url(SHA-256("sendspin-psk-id-v1" || psk))`, no padding.
pub fn psk_id(psk: &[u8; 32]) -> String {
    let mut h = Sha256::new();
    h.update(b"sendspin-psk-id-v1");
    h.update(psk);
    URL_SAFE_NO_PAD.encode(h.finalize())
}

/// The two cipher suites a server must support.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Suite {
    ChaChaPoly,
    AesGcm,
}

impl Suite {
    /// Parses the `suite` field of `client/init`.
    pub fn from_wire(s: &str) -> Option<Suite> {
        match s {
            "25519_ChaChaPoly_SHA256" => Some(Suite::ChaChaPoly),
            "25519_AESGCM_SHA256" => Some(Suite::AesGcm),
            _ => None,
        }
    }

    /// The full Noise protocol name for this suite.
    pub fn noise_params(self) -> snow::params::NoiseParams {
        let name = match self {
            Suite::ChaChaPoly => "Noise_KKpsk2_25519_ChaChaPoly_SHA256",
            Suite::AesGcm => "Noise_KKpsk2_25519_AESGCM_SHA256",
        };
        name.parse().expect("valid Noise protocol name")
    }
}

/// The server's static Curve25519 keypair; `server_id` is its public key.
pub struct ServerIdentity {
    pub server_id: String,
    private: [u8; 32],
    public: [u8; 32],
}

impl std::fmt::Debug for ServerIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerIdentity")
            .field("server_id", &self.server_id)
            .finish_non_exhaustive()
    }
}

impl ServerIdentity {
    /// Reads the 32-byte private key at `path`, or creates one (mode 0600).
    /// Keep this file: a new key makes the server a stranger to its players.
    pub fn load_or_create(path: &Path) -> anyhow::Result<Self> {
        let private: [u8; 32] = match std::fs::read(path) {
            Ok(bytes) => bytes.try_into().map_err(|b: Vec<u8>| {
                anyhow::anyhow!("{} holds {} bytes, expected 32", path.display(), b.len())
            })?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let key: [u8; 32] = rand::random();
                write_private(path, &key)?;
                key
            }
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        Ok(Self::from_private(private))
    }

    fn from_private(private: [u8; 32]) -> Self {
        let secret = x25519_dalek::StaticSecret::from(private);
        let public = x25519_dalek::PublicKey::from(&secret).to_bytes();
        Self {
            server_id: URL_SAFE_NO_PAD.encode(public),
            private,
            public,
        }
    }

    /// The raw static public key.
    pub fn public_key(&self) -> [u8; 32] {
        self.public
    }
}

fn write_private(path: &Path, key: &[u8; 32]) -> anyhow::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("creating {}", path.display()))?;
    f.write_all(key)?;
    f.sync_all()?;
    Ok(())
}

/// Decodes a `client_id` (base64url Curve25519 public key, 43 chars).
pub fn decode_client_id(client_id: &str) -> anyhow::Result<[u8; 32]> {
    let bytes = URL_SAFE_NO_PAD
        .decode(client_id)
        .context("client_id is not base64url")?;
    bytes
        .try_into()
        .map_err(|b: Vec<u8>| anyhow::anyhow!("client_id is {} bytes, expected 32", b.len()))
}

/// The server half of the handshake, between message 1 and message 2.
pub struct NoiseServer {
    hs: snow::HandshakeState,
}

impl NoiseServer {
    /// Builds the initiator and writes Noise message 1, whose payload names
    /// the PSK: `{"psk_id":…,"psk_category":…}`.
    pub fn start(
        id: &ServerIdentity,
        client_id: &str,
        suite: Suite,
        prologue: &[u8],
        psk: &[u8; 32],
        psk_category: &str,
    ) -> anyhow::Result<(NoiseServer, Vec<u8>)> {
        let client_pub = decode_client_id(client_id)?;
        let mut hs = snow::Builder::new(suite.noise_params())
            .local_private_key(&id.private)?
            .remote_public_key(&client_pub)?
            .psk(2, psk)?
            .prologue(prologue)?
            .build_initiator()?;
        let payload = serde_json::json!({ "psk_id": psk_id(psk), "psk_category": psk_category });
        let payload = serde_json::to_vec(&payload)?;
        let mut msg1 = vec![0u8; NOISE_MAX];
        let n = hs.write_message(&payload, &mut msg1)?;
        msg1.truncate(n);
        Ok((NoiseServer { hs }, msg1))
    }

    /// Reads Noise message 2 (payload must be exactly `{}`) and switches to transport mode.
    pub fn finish(mut self, msg2: &[u8]) -> anyhow::Result<NoiseTransport> {
        let mut payload = vec![0u8; NOISE_MAX];
        let n = self.hs.read_message(msg2, &mut payload)?;
        if &payload[..n] != b"{}" {
            bail!("noise message 2 payload is not {{}}");
        }
        Ok(NoiseTransport {
            t: self.hs.into_transport_mode()?,
        })
    }
}

/// Encrypts and decrypts Sendspin binary messages after the handshake.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn responder(
        suite: Suite,
        client: &snow::Keypair,
        server_pub: &[u8],
        prologue: &[u8],
    ) -> snow::HandshakeState {
        snow::Builder::new(suite.noise_params())
            .local_private_key(&client.private)
            .unwrap()
            .remote_public_key(server_pub)
            .unwrap()
            .psk(2, &SENTINEL_PSK)
            .unwrap()
            .prologue(prologue)
            .unwrap()
            .build_responder()
            .unwrap()
    }

    fn client_keypair() -> snow::Keypair {
        snow::Builder::new(Suite::ChaChaPoly.noise_params())
            .generate_keypair()
            .unwrap()
    }

    fn b64(bytes: &[u8]) -> String {
        use base64::Engine;
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
    }

    #[test]
    fn sentinel_constants_match_spec() {
        assert_eq!(
            hex::encode(SENTINEL_PSK),
            "1b5e24dbc1aed95fc2a5a338a90c05df44bd10f5ec1f4cd66cbf86272767b9d3"
        );
        assert_eq!(
            psk_id(&SENTINEL_PSK),
            "GFsV9tLaSQm9HcFWpKsgYQOr7wFTvNUtkmFwuVz3zoo"
        );
    }

    #[test]
    fn suites_parse_from_wire_names() {
        assert_eq!(
            Suite::from_wire("25519_ChaChaPoly_SHA256"),
            Some(Suite::ChaChaPoly)
        );
        assert_eq!(Suite::from_wire("25519_AESGCM_SHA256"), Some(Suite::AesGcm));
        assert_eq!(Suite::from_wire("25519_ChaChaPoly_BLAKE2s"), None);
    }

    #[test]
    fn identity_persists_and_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sound").join("identity.key");
        let a = ServerIdentity::load_or_create(&path).unwrap();
        let b = ServerIdentity::load_or_create(&path).unwrap();
        assert_eq!(a.server_id, b.server_id);
        assert_eq!(a.server_id.len(), 43);
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(std::fs::read(&path).unwrap().len(), 32);
    }

    #[test]
    fn kkpsk2_round_trip_with_snow_responder() {
        for suite in [Suite::ChaChaPoly, Suite::AesGcm] {
            let dir = tempfile::tempdir().unwrap();
            let id = ServerIdentity::load_or_create(&dir.path().join("identity.key")).unwrap();
            let client = client_keypair();
            let prologue = b"client/init bytes then server/init bytes";

            let (server, msg1) = NoiseServer::start(
                &id,
                &b64(&client.public),
                suite,
                prologue,
                &SENTINEL_PSK,
                "sn",
            )
            .unwrap();
            let mut hs = responder(suite, &client, &id.public_key(), prologue);
            let mut buf = vec![0u8; 1024];
            let n = hs.read_message(&msg1, &mut buf).unwrap();
            let payload: serde_json::Value = serde_json::from_slice(&buf[..n]).unwrap();
            assert_eq!(payload["psk_id"], psk_id(&SENTINEL_PSK));
            assert_eq!(payload["psk_category"], "sn");

            let mut msg2 = vec![0u8; 1024];
            let n = hs.write_message(b"{}", &mut msg2).unwrap();
            let mut server = server.finish(&msg2[..n]).unwrap();
            let mut client_t = hs.into_transport_mode().unwrap();

            let c = server.encrypt(b" hello player").unwrap();
            let n = client_t.read_message(&c, &mut buf).unwrap();
            assert_eq!(&buf[..n], b" hello player");

            let n = client_t.write_message(b" hello server", &mut msg2).unwrap();
            assert_eq!(server.decrypt(&msg2[..n]).unwrap(), b" hello server");
        }
    }

    #[test]
    fn message_two_payload_must_be_empty_object() {
        let dir = tempfile::tempdir().unwrap();
        let id = ServerIdentity::load_or_create(&dir.path().join("identity.key")).unwrap();
        let client = client_keypair();
        let (server, msg1) = NoiseServer::start(
            &id,
            &b64(&client.public),
            Suite::ChaChaPoly,
            b"p",
            &SENTINEL_PSK,
            "sn",
        )
        .unwrap();
        let mut hs = responder(Suite::ChaChaPoly, &client, &id.public_key(), b"p");
        let mut buf = vec![0u8; 1024];
        hs.read_message(&msg1, &mut buf).unwrap();
        let n = hs.write_message(b"", &mut buf).unwrap();
        assert!(server.finish(&buf[..n]).is_err());
    }

    #[test]
    fn wrong_prologue_fails() {
        let dir = tempfile::tempdir().unwrap();
        let id = ServerIdentity::load_or_create(&dir.path().join("identity.key")).unwrap();
        let client = client_keypair();
        let (server, msg1) = NoiseServer::start(
            &id,
            &b64(&client.public),
            Suite::ChaChaPoly,
            b"the real prologue",
            &SENTINEL_PSK,
            "sn",
        )
        .unwrap();
        let mut hs = responder(
            Suite::ChaChaPoly,
            &client,
            &id.public_key(),
            b"a tampered prologue",
        );
        let mut buf = vec![0u8; 1024];
        // The responder may already reject message 1; if not, the server rejects message 2.
        if hs.read_message(&msg1, &mut buf).is_ok() {
            let n = hs.write_message(b"{}", &mut buf).unwrap();
            assert!(server.finish(&buf[..n]).is_err());
        }
    }

    #[test]
    fn bad_client_id_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let id = ServerIdentity::load_or_create(&dir.path().join("identity.key")).unwrap();
        assert!(
            NoiseServer::start(
                &id,
                "not-a-key",
                Suite::ChaChaPoly,
                b"",
                &SENTINEL_PSK,
                "sn"
            )
            .is_err()
        );
    }
}
