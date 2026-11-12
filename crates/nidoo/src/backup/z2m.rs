//! Zigbee2MQTT's own backup (network key, paired devices), fetched over MQTT.
//!
//! Z2M answers a `<base>/bridge/request/backup` request on `<base>/bridge/response/backup` with
//! a base64 zip. The zip holds the Zigbee network key: it is never logged and never put in an
//! error or `Debug` output.

use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use base64::Engine as _;
use bytes::Bytes;
use futures_util::future::BoxFuture;
use rumqttc::{
    AsyncClient, ConnectReturnCode, ConnectionError, Event, MqttOptions, Packet, QoS, StateError,
};

use crate::config::MqttConfig;

/// How long the hub waits for Zigbee2MQTT's answer.
pub const Z2M_WAIT: Duration = Duration::from_secs(60);
/// The largest incoming MQTT packet the dedicated connection accepts.
pub const Z2M_MAX_PACKET: usize = 32 * 1024 * 1024;
/// The largest decoded zip taken into a backup.
pub const Z2M_MAX_ZIP: usize = 24_000_000;

const KEEP_ALIVE: Duration = Duration::from_secs(30);

/// Why a backup goes without Zigbee2MQTT data. Holds no payload.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Z2mSkip {
    #[error("Zigbee2MQTT is offline")]
    Offline,
    #[error("Zigbee2MQTT didn't answer within 60 s")]
    NoAnswer,
    #[error("Zigbee2MQTT refused: {0}")]
    Refused(String),
    #[error("Zigbee2MQTT's backup is too big (over 24 MB)")]
    TooBig,
    #[error("Zigbee2MQTT's answer could not be read")]
    Invalid,
    #[error("broker: {0}")]
    Broker(String),
}

/// Where Zigbee2MQTT's backup zip comes from.
pub trait Z2mSource: Send + Sync {
    fn fetch(&self) -> BoxFuture<'_, Result<Bytes, Z2mSkip>>;
}

/// A fixed answer, for tests.
pub struct MemZ2m(pub Mutex<Result<Bytes, Z2mSkip>>);

impl Z2mSource for MemZ2m {
    fn fetch(&self) -> BoxFuture<'_, Result<Bytes, Z2mSkip>> {
        let out = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        Box::pin(async move { out })
    }
}

/// The request body Zigbee2MQTT echoes the transaction of.
pub fn request_payload(transaction: &str) -> Vec<u8> {
    serde_json::json!({ "transaction": transaction })
        .to_string()
        .into_bytes()
}

/// Reads one response: `None` when it answers another request.
pub fn parse_response(payload: &[u8], transaction: &str) -> Option<Result<Bytes, Z2mSkip>> {
    let Ok(doc) = serde_json::from_slice::<serde_json::Value>(payload) else {
        // Not JSON: it cannot be matched to a request, but a backup response that is garbled
        // must not hang the wait either.
        return Some(Err(Z2mSkip::Invalid));
    };
    if doc.get("transaction").and_then(|t| t.as_str()) != Some(transaction) {
        return None;
    }
    match doc.get("status").and_then(|s| s.as_str()) {
        Some("ok") => {
            let Some(text) = doc.pointer("/data/zip").and_then(|z| z.as_str()) else {
                return Some(Err(Z2mSkip::Invalid));
            };
            match base64::engine::general_purpose::STANDARD.decode(text) {
                Ok(zip) if zip.len() > Z2M_MAX_ZIP => Some(Err(Z2mSkip::TooBig)),
                Ok(zip) => Some(Ok(Bytes::from(zip))),
                Err(_) => Some(Err(Z2mSkip::Invalid)),
            }
        }
        Some("error") => {
            let error = doc
                .get("error")
                .and_then(|e| e.as_str())
                .unwrap_or("unknown error");
            Some(Err(Z2mSkip::Refused(error.to_owned())))
        }
        _ => Some(Err(Z2mSkip::Invalid)),
    }
}

/// Zigbee2MQTT over the broker the hub is configured with, on its own connection.
pub struct MqttZ2m {
    cfg: MqttConfig,
    base: String,
    wait: Duration,
}

impl MqttZ2m {
    pub fn new(cfg: MqttConfig, base: String, wait: Duration) -> Self {
        Self { cfg, base, wait }
    }

    async fn run(&self) -> Result<Bytes, Z2mSkip> {
        let mut options = MqttOptions::new(
            format!("{}-z2m-backup", self.cfg.client_id),
            self.cfg.host.clone(),
            self.cfg.port,
        );
        if let Some(user) = &self.cfg.user {
            options.set_credentials(user, self.cfg.password.as_deref().unwrap_or(""));
        }
        options
            .set_keep_alive(KEEP_ALIVE)
            .set_clean_session(true)
            .set_max_packet_size(Z2M_MAX_PACKET, Z2M_MAX_PACKET);
        let (client, mut eventloop) = AsyncClient::new(options, 10);

        let base = self.base.trim_end_matches('/');
        let response_topic = format!("{base}/bridge/response/backup");
        let request_topic = format!("{base}/bridge/request/backup");
        let transaction = new_transaction();
        let mut subscribed = false;
        let mut requested = false;
        let result = loop {
            let event = match eventloop.poll().await {
                Ok(event) => event,
                Err(err) => break Err(map_connection_error(&err)),
            };
            match event {
                Event::Incoming(Packet::ConnAck(ack)) => {
                    if ack.code != ConnectReturnCode::Success {
                        break Err(Z2mSkip::Broker("the broker refused the connection".into()));
                    }
                    if let Err(e) = client
                        .subscribe(response_topic.clone(), QoS::AtLeastOnce)
                        .await
                    {
                        break Err(Z2mSkip::Broker(format!("could not subscribe: {e}")));
                    }
                }
                Event::Incoming(Packet::SubAck(_)) if !subscribed => {
                    subscribed = true;
                    // Subscribed first, so the answer cannot be missed.
                    if let Err(e) = client
                        .publish(
                            request_topic.clone(),
                            QoS::AtLeastOnce,
                            false,
                            request_payload(&transaction),
                        )
                        .await
                    {
                        break Err(Z2mSkip::Broker(format!("could not publish: {e}")));
                    }
                    requested = true;
                }
                Event::Incoming(Packet::Publish(p)) if requested && p.topic == response_topic => {
                    if let Some(answer) = parse_response(&p.payload, &transaction) {
                        break answer;
                    }
                }
                _ => {}
            }
        };
        let _ = client.try_disconnect();
        result
    }
}

impl Z2mSource for MqttZ2m {
    fn fetch(&self) -> BoxFuture<'_, Result<Bytes, Z2mSkip>> {
        Box::pin(async move {
            match tokio::time::timeout(self.wait, self.run()).await {
                Ok(out) => out,
                Err(_) => Err(Z2mSkip::NoAnswer),
            }
        })
    }
}

/// `nidoo-<unix millis>-<random u32>`.
fn new_transaction() -> String {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    let mut raw = [0u8; 4];
    let _ = getrandom::getrandom(&mut raw);
    format!("nidoo-{millis}-{}", u32::from_be_bytes(raw))
}

/// An incoming packet over the size limit is `TooBig`; any other failure is the broker's.
fn map_connection_error(err: &ConnectionError) -> Z2mSkip {
    if let ConnectionError::MqttState(StateError::Deserialization(
        rumqttc::Error::PayloadSizeLimitExceeded(_),
    )) = err
    {
        return Z2mSkip::TooBig;
    }
    Z2mSkip::Broker(err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok_payload(zip: &[u8], transaction: &str) -> Vec<u8> {
        let b64 = base64::engine::general_purpose::STANDARD.encode(zip);
        format!(r#"{{"data":{{"zip":"{b64}"}},"status":"ok","transaction":"{transaction}"}}"#)
            .into_bytes()
    }

    #[test]
    fn parses_an_ok_response() {
        let got = parse_response(&ok_payload(b"PK\x03\x04zip", "t1"), "t1");
        assert_eq!(got, Some(Ok(Bytes::from_static(b"PK\x03\x04zip"))));
    }

    #[test]
    fn ignores_another_transaction() {
        assert_eq!(parse_response(&ok_payload(b"PKzip", "t1"), "t2"), None);
        assert_eq!(parse_response(br#"{"data":{},"status":"ok"}"#, "t1"), None);
    }

    #[test]
    fn reports_a_refusal() {
        let p = br#"{"data":{},"status":"error","error":"busy","transaction":"t1"}"#;
        let got = parse_response(p, "t1");
        assert_eq!(got, Some(Err(Z2mSkip::Refused("busy".into()))));
        assert_eq!(
            got.unwrap().unwrap_err().to_string(),
            "Zigbee2MQTT refused: busy"
        );
        let p = br#"{"data":{},"status":"error","transaction":"t1"}"#;
        assert_eq!(
            parse_response(p, "t1"),
            Some(Err(Z2mSkip::Refused("unknown error".into())))
        );
    }

    #[test]
    fn garbled_and_oversized() {
        let bad = br#"{"data":{"zip":"!!not base64!!"},"status":"ok","transaction":"t1"}"#;
        assert_eq!(parse_response(bad, "t1"), Some(Err(Z2mSkip::Invalid)));
        let missing = br#"{"data":{},"status":"ok","transaction":"t1"}"#;
        assert_eq!(parse_response(missing, "t1"), Some(Err(Z2mSkip::Invalid)));
        assert_eq!(
            parse_response(b"not json", "t1"),
            Some(Err(Z2mSkip::Invalid))
        );
        let big = vec![0u8; Z2M_MAX_ZIP + 1];
        let got = parse_response(&ok_payload(&big, "t1"), "t1");
        assert_eq!(got, Some(Err(Z2mSkip::TooBig)));
        assert_eq!(
            got.unwrap().unwrap_err().to_string(),
            "Zigbee2MQTT's backup is too big (over 24 MB)"
        );
        let edge = vec![0u8; Z2M_MAX_ZIP];
        assert!(matches!(
            parse_response(&ok_payload(&edge, "t1"), "t1"),
            Some(Ok(_))
        ));
    }

    #[test]
    fn skip_reasons_hold_no_payload() {
        let b64 = base64::engine::general_purpose::STANDARD.encode(b"PK\x03\x04zip");
        let skips = [
            Z2mSkip::Offline,
            Z2mSkip::NoAnswer,
            Z2mSkip::TooBig,
            Z2mSkip::Invalid,
            Z2mSkip::Refused("busy".into()),
            Z2mSkip::Broker("down".into()),
        ];
        for skip in skips {
            for text in [format!("{skip:?}"), skip.to_string()] {
                assert!(!text.contains("PK"), "{text}");
                assert!(!text.contains(&b64), "{text}");
            }
        }
    }

    #[test]
    fn copy_and_request() {
        assert_eq!(Z2mSkip::Offline.to_string(), "Zigbee2MQTT is offline");
        assert_eq!(
            Z2mSkip::NoAnswer.to_string(),
            "Zigbee2MQTT didn't answer within 60 s"
        );
        assert_eq!(
            Z2mSkip::Invalid.to_string(),
            "Zigbee2MQTT's answer could not be read"
        );
        assert_eq!(Z2mSkip::Broker("x".into()).to_string(), "broker: x");
        assert_eq!(request_payload("t1"), br#"{"transaction":"t1"}"#);
    }

    #[test]
    fn oversized_packet_maps_to_too_big() {
        let big = ConnectionError::MqttState(StateError::Deserialization(
            rumqttc::Error::PayloadSizeLimitExceeded(1),
        ));
        assert_eq!(map_connection_error(&big), Z2mSkip::TooBig);
        assert!(matches!(
            map_connection_error(&ConnectionError::NetworkTimeout),
            Z2mSkip::Broker(_)
        ));
    }

    #[tokio::test]
    async fn mem_source_clones_its_result() {
        let m = MemZ2m(Mutex::new(Ok(Bytes::from_static(b"z"))));
        assert_eq!(m.fetch().await, Ok(Bytes::from_static(b"z")));
        assert_eq!(m.fetch().await, Ok(Bytes::from_static(b"z")));
    }
}
