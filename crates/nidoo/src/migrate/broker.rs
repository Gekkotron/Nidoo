//! Broker side of the switch-over: read every retained message, back it up, publish the
//! renamed copies, check them, then clear the old ones and check again.

use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use rumqttc::{
    AsyncClient, Event, EventLoop, MqttOptions, Outgoing, Packet, QoS, SubAck, SubscribeReasonCode,
};
use serde::{Deserialize, Serialize};
use tokio::time::Instant;

use super::rewrite::{Action, Retained, restorable};

/// How long the broker must stay silent (no new retained message) before a read is complete.
pub const DEFAULT_QUIET: Duration = Duration::from_secs(3);

const KEEP_ALIVE: Duration = Duration::from_secs(30);
const MAX_PACKET: usize = 4 * 1024 * 1024;
const REQUEST_CAPACITY: usize = 64;

/// Where and how to reach the broker.
#[derive(Debug, Clone)]
pub struct BrokerOpts {
    pub host: String,
    pub port: u16,
    pub user: Option<String>,
    pub password: Option<String>,
}

/// What [`apply`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyReport {
    pub published: usize,
    pub cleared: usize,
}

#[derive(Serialize, Deserialize)]
struct BackupLine {
    topic: String,
    payload_b64: String,
    qos: u8,
}

fn connect(o: &BrokerOpts) -> (AsyncClient, EventLoop) {
    let mut options = MqttOptions::new(
        format!("nidoo-migrate-{}", std::process::id()),
        o.host.clone(),
        o.port,
    );
    if let (Some(user), Some(password)) = (&o.user, &o.password) {
        options.set_credentials(user, password);
    }
    options
        .set_keep_alive(KEEP_ALIVE)
        .set_max_packet_size(MAX_PACKET, MAX_PACKET);
    AsyncClient::new(options, REQUEST_CAPACITY)
}

fn qos_from(q: u8) -> QoS {
    match q {
        0 => QoS::AtMostOnce,
        1 => QoS::AtLeastOnce,
        _ => QoS::ExactlyOnce,
    }
}

fn qos_to(q: QoS) -> u8 {
    match q {
        QoS::AtMostOnce => 0,
        QoS::AtLeastOnce => 1,
        QoS::ExactlyOnce => 2,
    }
}

/// Fails when the broker refused the `#` subscription: an empty read would look like a broker
/// with nothing to move.
fn check_suback(ack: &SubAck) -> Result<()> {
    if ack.return_codes.is_empty()
        || ack
            .return_codes
            .iter()
            .any(|c| matches!(c, SubscribeReasonCode::Failure))
    {
        bail!("the broker refused the subscription to # (check the ACL of the hub's broker login)");
    }
    Ok(())
}

/// Fails on an empty read: a house broker always holds retained messages (Zigbee2MQTT's bridge
/// topics at least), so none means the login may not read them (ACL) or this is the wrong
/// broker, and every check after it would pass for the wrong reason.
pub fn require_retained(msgs: &[Retained]) -> Result<()> {
    if msgs.is_empty() {
        bail!(
            "the broker returned no retained message at all: the hub's broker login may not be \
             allowed to read them (ACL), or hub.json points at the wrong broker"
        );
    }
    Ok(())
}

/// Subscribes to `#` and collects the retained messages, until none new arrived for `quiet`.
pub async fn read_retained(o: &BrokerOpts, quiet: Duration) -> Result<Vec<Retained>> {
    let (client, mut eventloop) = connect(o);
    client
        .subscribe("#", QoS::AtLeastOnce)
        .await
        .context("queueing the subscription")?;
    let mut found: std::collections::BTreeMap<String, Retained> = Default::default();
    let mut deadline: Option<Instant> = None;
    loop {
        let event = match deadline {
            Some(d) => match tokio::time::timeout_at(d, eventloop.poll()).await {
                Ok(ev) => ev,
                Err(_) => break,
            },
            None => eventloop.poll().await,
        };
        match event.context("reading retained messages from the broker")? {
            Event::Incoming(Packet::SubAck(ack)) => {
                check_suback(&ack)?;
                deadline = Some(Instant::now() + quiet);
            }
            Event::Incoming(Packet::Publish(p)) if p.retain && !p.payload.is_empty() => {
                found.insert(
                    p.topic.clone(),
                    Retained {
                        topic: p.topic,
                        payload: p.payload.to_vec(),
                        qos: qos_to(p.qos),
                    },
                );
                deadline = Some(Instant::now() + quiet);
            }
            _ => {}
        }
    }
    let _ = client.try_disconnect();
    Ok(found.into_values().collect())
}

/// Publishes every `(topic, payload, qos)` retained and waits until the broker acknowledged
/// them all (QoS 0 messages are flushed by the disconnect).
async fn publish_all(o: &BrokerOpts, items: Vec<(String, Vec<u8>, u8)>) -> Result<()> {
    if items.is_empty() {
        return Ok(());
    }
    let expected = items.iter().filter(|(_, _, q)| *q > 0).count();
    let (client, mut eventloop) = connect(o);
    let sender = client.clone();
    let mut sending = tokio::spawn(async move {
        for (topic, payload, qos) in items {
            sender
                .publish(topic, qos_from(qos), true, payload)
                .await
                .context("queueing a publish")?;
        }
        anyhow::Ok(())
    });
    let mut sent = false;
    let mut acked = 0usize;
    let mut disconnecting = false;
    loop {
        tokio::select! {
            done = &mut sending, if !sent => {
                done.context("publisher task")??;
                sent = true;
            }
            event = eventloop.poll() => {
                match event.context("publishing to the broker")? {
                    Event::Incoming(Packet::PubAck(_) | Packet::PubComp(_)) => acked += 1,
                    Event::Outgoing(Outgoing::Disconnect) => return Ok(()),
                    _ => {}
                }
            }
        }
        if sent && acked >= expected && !disconnecting {
            disconnecting = true;
            client.try_disconnect().context("queueing the disconnect")?;
        }
    }
}

/// Writes the messages as JSON Lines (retain is implied).
pub fn write_backup(path: &Path, msgs: &[Retained]) -> std::io::Result<()> {
    let mut out = String::new();
    for m in msgs {
        let line = BackupLine {
            topic: m.topic.clone(),
            payload_b64: STANDARD.encode(&m.payload),
            qos: m.qos,
        };
        out.push_str(&serde_json::to_string(&line).map_err(std::io::Error::other)?);
        out.push('\n');
    }
    std::fs::write(path, out)
}

pub fn read_backup(path: &Path) -> std::io::Result<Vec<Retained>> {
    let text = std::fs::read_to_string(path)?;
    let mut msgs = Vec::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let l: BackupLine = serde_json::from_str(line).map_err(std::io::Error::other)?;
        msgs.push(Retained {
            topic: l.topic,
            payload: STANDARD
                .decode(l.payload_b64)
                .map_err(std::io::Error::other)?,
            qos: l.qos,
        });
    }
    Ok(msgs)
}

/// The backup entries `--restore` republishes: only the old and new names' topics.
pub fn restore_selection(msgs: &[Retained]) -> Vec<&Retained> {
    msgs.iter().filter(|m| restorable(&m.topic)).collect()
}

/// Republishes, retained, the backup entries [`restore_selection`] keeps; returns how many.
pub async fn restore(o: &BrokerOpts, msgs: &[Retained]) -> Result<usize> {
    let items: Vec<_> = restore_selection(msgs)
        .into_iter()
        .map(|m| (m.topic.clone(), m.payload.clone(), m.qos))
        .collect();
    let n = items.len();
    publish_all(o, items).await?;
    Ok(n)
}

/// Publishes the new messages, checks them, clears the old ones, checks again.
pub async fn apply(o: &BrokerOpts, actions: &[Action], quiet: Duration) -> Result<ApplyReport> {
    let refused: Vec<&str> = actions
        .iter()
        .filter_map(|a| match a {
            Action::Refuse { topic, .. } => Some(topic.as_str()),
            _ => None,
        })
        .collect();
    if !refused.is_empty() {
        bail!("refusing to apply, unsafe topics: {}", refused.join(", "));
    }

    let moves: Vec<&Retained> = actions
        .iter()
        .filter_map(|a| match a {
            Action::Move { to, .. } => Some(to),
            _ => None,
        })
        .collect();
    let clears: Vec<&str> = actions
        .iter()
        .filter_map(|a| match a {
            Action::Move { from, to, .. } if *from != to.topic => Some(from.as_str()),
            Action::Drop { topic, .. } => Some(topic.as_str()),
            _ => None,
        })
        .collect();

    // Phase 1: publish the new messages.
    publish_all(
        o,
        moves
            .iter()
            .map(|m| (m.topic.clone(), m.payload.clone(), m.qos))
            .collect(),
    )
    .await?;

    // Phase 2: they must be on the broker, byte for byte, before anything is cleared.
    let now = read_retained(o, quiet).await?;
    let missing: Vec<&str> = moves
        .iter()
        .filter(|m| {
            !now.iter()
                .any(|r| r.topic == m.topic && r.payload == m.payload)
        })
        .map(|m| m.topic.as_str())
        .collect();
    if !missing.is_empty() {
        bail!(
            "published messages not found on the broker, nothing cleared: {}",
            missing.join(", ")
        );
    }

    // Phase 3: clear the old topics.
    publish_all(
        o,
        clears
            .iter()
            .map(|t| (t.to_string(), Vec::new(), 1))
            .collect(),
    )
    .await?;

    // Phase 4: they must be gone.
    let now = read_retained(o, quiet).await?;
    let left: Vec<&str> = clears
        .iter()
        .filter(|t| now.iter().any(|r| r.topic == **t))
        .copied()
        .collect();
    if !left.is_empty() {
        bail!("topics still retained after clearing: {}", left.join(", "));
    }

    Ok(ApplyReport {
        published: moves.len(),
        cleared: clears.len(),
    })
}

/// One line per leftover that still carries the old name, in the topic or the payload.
pub fn verify_retained(msgs: &[Retained]) -> Vec<String> {
    let mut lines = Vec::new();
    for m in msgs {
        let lower_topic = m.topic.to_ascii_lowercase();
        let why = if m.topic.starts_with("oikos/") || m.topic.starts_with("oikos-push/") {
            Some("still under the old prefix")
        } else if lower_topic.contains("oikos") {
            Some("topic contains oikos")
        } else if m
            .payload
            .to_ascii_lowercase()
            .windows(5)
            .any(|w| w == b"oikos")
        {
            Some("payload contains oikos")
        } else {
            None
        };
        if let Some(why) = why {
            lines.push(format!("{}: {why}", m.topic));
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::super::rewrite::plan;
    use super::*;

    fn msg(topic: &str, payload: &[u8]) -> Retained {
        Retained {
            topic: topic.to_string(),
            payload: payload.to_vec(),
            qos: 1,
        }
    }

    #[test]
    fn backup_round_trips_binary_and_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("backup.jsonl");
        let msgs = vec![
            msg("a/empty", b""),
            msg("a/bin", &[0xff]),
            msg("a/json", br#"{"k":"v","n":1}"#),
        ];
        write_backup(&path, &msgs).unwrap();
        assert_eq!(read_backup(&path).unwrap(), msgs);
    }

    #[test]
    fn plan_after_partial_apply_only_clears() {
        let existing = [
            msg("oikos/ui/a", br#"{"name":"x"}"#),
            msg("nidoo/ui/a", br#"{"name":"x"}"#),
        ];
        let actions = plan(&existing);
        let moves: Vec<_> = actions
            .iter()
            .filter(|a| matches!(a, Action::Move { .. }))
            .collect();
        assert_eq!(moves.len(), 1);
        assert!(!actions.iter().any(|a| matches!(a, Action::Refuse { .. })));
        match moves[0] {
            Action::Move { from, to, .. } => {
                assert_eq!(from, "oikos/ui/a");
                assert_eq!(to, &existing[1]);
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn refused_subscription_fails() {
        assert!(
            check_suback(&SubAck::new(
                1,
                vec![SubscribeReasonCode::Success(QoS::AtLeastOnce)]
            ))
            .is_ok()
        );
        assert!(check_suback(&SubAck::new(1, vec![SubscribeReasonCode::Failure])).is_err());
        assert!(check_suback(&SubAck::new(1, vec![])).is_err());
    }

    #[test]
    fn empty_read_is_refused() {
        let err = require_retained(&[]).unwrap_err().to_string();
        assert!(err.contains("ACL"), "{err}");
        assert!(require_retained(&[msg("zigbee2mqtt/bridge/state", b"online")]).is_ok());
    }

    #[test]
    fn restore_selects_only_old_and_new_names() {
        let backup = [
            msg("oikos/ui/a", b"1"),
            msg("zigbee2mqtt/lamp", b"2"),
            msg("nidoo/ui/b", b"3"),
            msg("frigate/available", b"online"),
        ];
        let topics: Vec<&str> = restore_selection(&backup)
            .iter()
            .map(|m| m.topic.as_str())
            .collect();
        assert_eq!(topics, ["oikos/ui/a", "nidoo/ui/b"]);
    }

    #[test]
    fn verify_flags_any_casing() {
        let lines = verify_retained(&[msg("nidoo/x", b"OiKoS"), msg("nidoo/y", b"fine")]);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].starts_with("nidoo/x: "));
    }
}
