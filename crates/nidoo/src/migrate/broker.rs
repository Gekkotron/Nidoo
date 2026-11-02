//! Broker side of the switch-over: read every retained message, back it up, publish the
//! renamed copies, check them, then clear the old ones and check again.

use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
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
/// How long a read waits for its DISCONNECT to go out before it lets the connection drop.
const DISCONNECT_TIMEOUT: Duration = Duration::from_secs(2);

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

/// Numbers the connections of this process, so each one has its own client id.
static CONNECTIONS: AtomicU32 = AtomicU32::new(0);

/// A new connection, under a client id no other connection of this run uses: with a shared id,
/// the broker hands the session to the newer connection and throws away what it had not yet
/// read from the older one (Mosquitto: "already connected, closing old connection").
fn connect(o: &BrokerOpts, capacity: usize) -> (AsyncClient, EventLoop) {
    let n = CONNECTIONS.fetch_add(1, Ordering::Relaxed);
    let mut options = MqttOptions::new(
        format!("nidoo-migrate-{}-{n}", std::process::id()),
        o.host.clone(),
        o.port,
    );
    if let (Some(user), Some(password)) = (&o.user, &o.password) {
        options.set_credentials(user, password);
    }
    options
        .set_keep_alive(KEEP_ALIVE)
        .set_max_packet_size(MAX_PACKET, MAX_PACKET);
    AsyncClient::new(options, capacity)
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
    let (client, mut eventloop) = connect(o, REQUEST_CAPACITY);
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
    // Say goodbye instead of dropping the socket. Best effort: the read is complete either way,
    // and no later connection shares this one's client id.
    if client.try_disconnect().is_ok() {
        let _ = tokio::time::timeout(DISCONNECT_TIMEOUT, async {
            // Stops on an error too: polling again would reconnect.
            while !matches!(
                eventloop.poll().await,
                Ok(Event::Outgoing(Outgoing::Disconnect)) | Err(_)
            ) {}
        })
        .await;
    }
    Ok(found.into_values().collect())
}

/// Publishes every `(topic, payload, qos)` retained, on one connection, and returns once the
/// broker has processed them all; fails if the connection drops on the way.
///
/// A flushed socket is not a delivered message: the broker may not have read the bytes yet
/// (Mosquitto reads one packet per socket per turn of its loop), and QoS 0 has no ack. So the
/// publishes are followed by a subscription to a topic nothing is published on: the broker
/// handles a connection's packets in order, so its SUBACK proves every earlier publish was
/// processed, without changing any message's QoS or leaving anything retained. QoS 1 and 2
/// messages are counted by their own acks as well.
async fn publish_all(o: &BrokerOpts, items: Vec<(String, Vec<u8>, u8)>) -> Result<()> {
    if items.is_empty() {
        return Ok(());
    }
    let expected = items.iter().filter(|(_, _, q)| *q > 0).count();
    // Room for every request up front (the publishes, the barrier and the disconnect), so they
    // are all queued without waiting and the event loop is the only future this awaits: a
    // `poll()` dropped half way can lose a packet or reconnect.
    let (client, mut eventloop) = connect(o, items.len() + 2);
    for (topic, payload, qos) in items {
        client
            .try_publish(topic, qos_from(qos), true, payload)
            .context("queueing a publish")?;
    }
    let barrier = format!(
        "nidoo-migrate/barrier/{}",
        eventloop.mqtt_options.client_id()
    );
    client
        .try_subscribe(barrier, QoS::AtMostOnce)
        .context("queueing the barrier subscription")?;
    let mut connected = false;
    let mut acked = 0usize;
    let mut barrier_acked = false;
    let mut disconnecting = false;
    loop {
        match eventloop.poll().await.context("publishing to the broker")? {
            Event::Incoming(Packet::ConnAck(_)) if connected => {
                bail!(
                    "the connection to the broker dropped and came back while publishing: \
                     messages may be lost"
                );
            }
            Event::Incoming(Packet::ConnAck(_)) => connected = true,
            Event::Incoming(Packet::PubAck(_) | Packet::PubComp(_)) => acked += 1,
            Event::Incoming(Packet::SubAck(_)) => barrier_acked = true,
            Event::Outgoing(Outgoing::Disconnect) => return Ok(()),
            _ => {}
        }
        if barrier_acked && acked >= expected && !disconnecting {
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

    /// A broker on a thread that handles one packet at a time and takes `per_publish` for
    /// each PUBLISH, like a loaded Mosquitto: bytes the client flushed are not processed yet.
    /// With `close_after`, it drops the connection after that many PUBLISH packets.
    struct FakeBroker {
        port: u16,
        /// Topics of the PUBLISH packets processed so far, in order.
        processed: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
        /// Client id of every CONNECT.
        clients: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
        /// DISCONNECT packets received.
        disconnects: std::sync::Arc<AtomicU32>,
    }

    fn fake_broker(per_publish: Duration, close_after: Option<usize>) -> FakeBroker {
        use rumqttc::{ConnAck, ConnectReturnCode, PubAck};
        use std::io::{Read, Write};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let processed = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let clients = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let disconnects = std::sync::Arc::new(AtomicU32::new(0));
        let (p, c, d) = (processed.clone(), clients.clone(), disconnects.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let (processed, clients, disconnects) = (p.clone(), c.clone(), d.clone());
                std::thread::spawn(move || {
                    let mut buf = bytes::BytesMut::new();
                    let mut chunk = [0u8; 4096];
                    let mut publishes = 0usize;
                    loop {
                        let packet = match Packet::read(&mut buf, MAX_PACKET) {
                            Ok(packet) => packet,
                            Err(rumqttc::Error::InsufficientBytes(_)) => {
                                match stream.read(&mut chunk) {
                                    Ok(0) | Err(_) => return,
                                    Ok(n) => buf.extend_from_slice(&chunk[..n]),
                                }
                                continue;
                            }
                            Err(_) => return,
                        };
                        let reply = match packet {
                            Packet::Connect(c) => {
                                clients.lock().unwrap().push(c.client_id);
                                Some(Packet::ConnAck(ConnAck::new(
                                    ConnectReturnCode::Success,
                                    false,
                                )))
                            }
                            Packet::Publish(p) => {
                                if close_after == Some(publishes) {
                                    return;
                                }
                                publishes += 1;
                                std::thread::sleep(per_publish);
                                processed.lock().unwrap().push(p.topic.clone());
                                (p.qos != QoS::AtMostOnce)
                                    .then(|| Packet::PubAck(PubAck::new(p.pkid)))
                            }
                            Packet::Subscribe(s) => Some(Packet::SubAck(SubAck::new(
                                s.pkid,
                                vec![SubscribeReasonCode::Success(QoS::AtMostOnce)],
                            ))),
                            Packet::PingReq => Some(Packet::PingResp),
                            Packet::Disconnect => {
                                disconnects.fetch_add(1, Ordering::SeqCst);
                                return;
                            }
                            _ => None,
                        };
                        if let Some(reply) = reply {
                            let mut out = bytes::BytesMut::new();
                            reply.write(&mut out, MAX_PACKET).unwrap();
                            if stream.write_all(&out).is_err() {
                                return;
                            }
                        }
                    }
                });
            }
        });
        FakeBroker {
            port,
            processed,
            clients,
            disconnects,
        }
    }

    /// Whether the fake broker got one DISCONNECT within a second (it counts on its own thread,
    /// after the client sent it).
    async fn got_disconnect(broker: &FakeBroker) -> bool {
        for _ in 0..100 {
            if broker.disconnects.load(Ordering::SeqCst) == 1 {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        false
    }

    fn opts(port: u16) -> BrokerOpts {
        BrokerOpts {
            host: "127.0.0.1".into(),
            port,
            user: None,
            password: None,
        }
    }

    fn items(n: usize, qos: u8) -> Vec<(String, Vec<u8>, u8)> {
        (0..n)
            .map(|i| (format!("nidoo/ui/t{i}"), format!("{i}").into_bytes(), qos))
            .collect()
    }

    /// The CI failure: QoS 0 publishes were flushed to the socket, not yet read by the broker,
    /// when the next connection took the session over. publish_all must return only once the
    /// broker processed every message.
    #[tokio::test]
    async fn publish_all_returns_after_the_broker_processed_every_qos0_message() {
        let broker = fake_broker(Duration::from_millis(5), None);
        publish_all(&opts(broker.port), items(60, 0)).await.unwrap();
        let processed = broker.processed.lock().unwrap().clone();
        let want: Vec<String> = items(60, 0).into_iter().map(|(t, _, _)| t).collect();
        assert_eq!(processed, want);
        assert_eq!(broker.clients.lock().unwrap().len(), 1, "one connection");
        assert!(got_disconnect(&broker).await, "a clean DISCONNECT");
    }

    #[tokio::test]
    async fn publish_all_waits_for_every_qos1_ack() {
        let broker = fake_broker(Duration::from_millis(2), None);
        publish_all(&opts(broker.port), items(150, 1))
            .await
            .unwrap();
        assert_eq!(broker.processed.lock().unwrap().len(), 150);
    }

    #[tokio::test]
    async fn publish_all_fails_when_the_broker_drops_the_connection() {
        let broker = fake_broker(Duration::ZERO, Some(10));
        let err = publish_all(&opts(broker.port), items(60, 0)).await;
        assert!(
            err.is_err(),
            "a dropped connection must fail, not reconnect"
        );
        assert_eq!(broker.clients.lock().unwrap().len(), 1, "no reconnect");
    }

    #[tokio::test]
    async fn read_retained_disconnects_cleanly() {
        let broker = fake_broker(Duration::ZERO, None);
        let msgs = read_retained(&opts(broker.port), Duration::from_millis(100))
            .await
            .unwrap();
        assert!(msgs.is_empty());
        assert!(
            got_disconnect(&broker).await,
            "read_retained dropped the socket without a DISCONNECT"
        );
    }

    #[test]
    fn every_connection_has_its_own_client_id() {
        let o = opts(1883);
        let (_c1, e1) = connect(&o, 1);
        let (_c2, e2) = connect(&o, 1);
        assert_ne!(e1.mqtt_options.client_id(), e2.mqtt_options.client_id());
    }

    #[test]
    fn verify_flags_any_casing() {
        let lines = verify_retained(&[msg("nidoo/x", b"OiKoS"), msg("nidoo/y", b"fine")]);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].starts_with("nidoo/x: "));
    }
}
