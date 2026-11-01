//! The broker link: one task polls the rumqttc event loop and feeds the hub, another one
//! owns everything outgoing (subscriptions and publishes). Keeping the two apart means an
//! outbound `.await` can never stall the event loop that drains it.

use std::collections::BTreeSet;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use nidoo_api::ConnectionStatus;
use nidoo_core::command::Publish;
use nidoo_core::registry::Change;
use rumqttc::{AsyncClient, Event, EventLoop, MqttOptions, Outgoing, Packet, QoS, SubscribeFilter};
use tokio::sync::{Notify, mpsc};
use tokio::task::JoinHandle;
use tokio::time::{Instant, Sleep};

use crate::config::{MqttConfig, TopicsConfig};
use crate::hub::Hub;

const KEEP_ALIVE: Duration = Duration::from_secs(30);
/// Zigbee2MQTT's `bridge/devices` runs to hundreds of KB; rumqttc's default is 10 KB.
const MAX_INCOMING_PACKET: usize = 4 * 1024 * 1024;
const MAX_OUTGOING_PACKET: usize = 1024 * 1024;
/// Requests queued towards the event loop.
const REQUEST_CAPACITY: usize = 64;
/// Pause after a connection error before polling (and so reconnecting) again.
const RETRY_DELAY: Duration = Duration::from_secs(1);
/// How long the entity count must stay still after connect before it is logged.
const QUIET_PERIOD: Duration = Duration::from_secs(2);

/// Lets the owner disconnect cleanly.
pub struct Handle {
    client: AsyncClient,
}

impl Handle {
    /// Queues a Disconnect without waiting; the task ends once it is on the wire. `false` when
    /// it could not be queued (the request queue is full or the event loop is gone), in which
    /// case the owner aborts the task instead of waiting for a packet that will never go out.
    pub fn disconnect(&self) -> bool {
        match self.client.try_disconnect() {
            Ok(()) => true,
            Err(err) => {
                tracing::warn!(error = %err, "could not queue the disconnect");
                false
            }
        }
    }
}

/// Connects to the broker and runs the link until [`Handle::disconnect`]; the returned task
/// ends once the Disconnect packet was sent.
pub fn start(
    cfg: MqttConfig,
    topics: TopicsConfig,
    hub: Arc<Hub>,
    publish_rx: mpsc::Receiver<Publish>,
) -> (Handle, JoinHandle<()>) {
    let mut options = MqttOptions::new(cfg.client_id.clone(), cfg.host.clone(), cfg.port);
    if let (Some(user), Some(password)) = (&cfg.user, &cfg.password) {
        options.set_credentials(user, password);
    }
    // A persistent session: a publish handed to rumqttc just before the link dropped is
    // retransmitted on reconnect (`session_present`) instead of being discarded with the
    // request queue, and the broker keeps the QoS 1 messages that arrived meanwhile. Two
    // hubs must therefore never share a `client_id`.
    options
        .set_keep_alive(KEEP_ALIVE)
        .set_clean_session(false)
        .set_max_packet_size(MAX_INCOMING_PACKET, MAX_OUTGOING_PACKET);
    let (client, eventloop) = AsyncClient::new(options, REQUEST_CAPACITY);
    let handle = Handle {
        client: client.clone(),
    };

    let resubscribe = hub.subscriptions_wakeup();
    let full = FullSubscription::default();
    let outbound = tokio::spawn(outbound(
        client,
        Arc::clone(&hub),
        cfg.clone(),
        topics,
        publish_rx,
        Arc::clone(&resubscribe),
        full.clone(),
    ));
    let task = tokio::spawn(async move {
        poll_loop(eventloop, &cfg, &hub, &resubscribe, &full).await;
        outbound.abort();
    });
    (handle, task)
}

/// The on-connect subscription the outbound task sent on the current connection and whose
/// SubAck is still awaited, as its filter count. rumqttc assigns packet ids inside the event
/// loop and the client never learns them, so the SubAck is recognised by its return codes:
/// the broker answers one per filter, and the on-connect subscription (every static filter,
/// push included, plus every state topic) is the one carrying exactly that many.
#[derive(Clone, Default)]
struct FullSubscription(Arc<Mutex<Option<usize>>>);

impl FullSubscription {
    fn set(&self, filters: Option<usize>) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = filters;
    }

    /// True (once) when a SubAck with `codes` return codes answers the awaited subscription.
    fn acknowledged_by(&self, codes: usize) -> bool {
        let mut awaited = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if *awaited == Some(codes) {
            *awaited = None;
            true
        } else {
            false
        }
    }
}

/// Polls the event loop: feeds the hub, tracks the connection, logs the registry once it
/// settles after connect. Returns once the Disconnect packet was sent.
async fn poll_loop(
    mut eventloop: EventLoop,
    cfg: &MqttConfig,
    hub: &Hub,
    resubscribe: &Notify,
    full: &FullSubscription,
) {
    let mut failing = false;
    let mut quiet = Timer::new();
    let mut quiet_logged = false;
    loop {
        tokio::select! {
            event = eventloop.poll() => match event {
                Ok(Event::Incoming(Packet::ConnAck(_))) => {
                    failing = false;
                    tracing::info!(host = %cfg.host, port = cfg.port, "connected");
                    // Before `Online`: the outbound task registers its on-connect
                    // subscription only once it sees the new connection.
                    full.set(None);
                    hub.on_connected();
                    hub.set_connection(ConnectionStatus::Online);
                    if !quiet_logged {
                        quiet.arm(QUIET_PERIOD);
                    }
                }
                Ok(Event::Incoming(Packet::Publish(publish))) => {
                    let changes = hub.ingest_message(&publish.topic, &publish.payload, publish.retain);
                    if changes.contains(&Change::SubscriptionsChanged) {
                        resubscribe.notify_one();
                    }
                    let upserted = changes
                        .iter()
                        .filter(|c| matches!(c, Change::EntityUpserted(_)))
                        .count();
                    if upserted > 0 {
                        tracing::debug!(topic = %publish.topic, upserted, "entities upserted");
                        if quiet.armed() {
                            quiet.arm(QUIET_PERIOD);
                        }
                    }
                }
                Ok(Event::Incoming(Packet::SubAck(ack))) => {
                    // Incremental subscriptions (and any sent before the connection dropped)
                    // carry a different count and leave the awaited one in place.
                    if full.acknowledged_by(ack.return_codes.len()) {
                        tracing::debug!(pkid = ack.pkid, "on-connect subscription acknowledged");
                        hub.on_suback();
                    }
                }
                Ok(Event::Outgoing(Outgoing::Disconnect)) => {
                    hub.set_connection(ConnectionStatus::Offline {
                        reason: "disconnected".to_owned(),
                    });
                    return;
                }
                Ok(_) => {}
                Err(err) => {
                    let reason = err.to_string();
                    if !failing {
                        tracing::warn!(error = %reason, "connection lost; retrying");
                        failing = true;
                    }
                    hub.set_connection(ConnectionStatus::Offline { reason });
                    tokio::time::sleep(RETRY_DELAY).await;
                }
            },
            () = &mut quiet => {
                let (entities, devices) = hub.counts();
                tracing::info!(entities, devices, "registry");
                quiet_logged = true;
            }
        }
    }
}

/// Owns the client's outgoing side: the full subscription on every connect, the debounced
/// subscription diff, the birth message and the hub's publishes.
async fn outbound(
    client: AsyncClient,
    hub: Arc<Hub>,
    cfg: MqttConfig,
    topics: TopicsConfig,
    mut publish_rx: mpsc::Receiver<Publish>,
    resubscribe: Arc<Notify>,
    full: FullSubscription,
) {
    let mut connection = hub.connection();
    let mut online = false;
    let mut subscribed: BTreeSet<String> = BTreeSet::new();
    let mut debounce = Timer::new();
    loop {
        tokio::select! {
            changed = connection.changed() => {
                if changed.is_err() {
                    return;
                }
                let now_online = matches!(*connection.borrow_and_update(), ConnectionStatus::Online);
                // Every change that lands on Online is a (re)connect: the watch coalesces
                // `Online → Offline → Online` into one change, so the previous flag says nothing.
                if now_online {
                    // Subscribe to everything, whether or not the broker resumed the session:
                    // the registry may have moved on while the link was down.
                    subscribed = hub.subscription_topics();
                    let mut extra = hub.push().filters();
                    extra.extend(hub.extra_filters());
                    let filters = valid_filters(
                        static_filters(&topics, &extra)
                            .into_iter()
                            .chain(subscribed.iter().cloned())
                            .collect(),
                    );
                    let count = filters.len();
                    // Registered before the request is queued, so its SubAck cannot beat it.
                    full.set(Some(count));
                    if !subscribe(&client, filters).await {
                        full.set(None);
                    }
                    tracing::info!(topics = count, "subscribed");
                    if cfg.send_ha_birth
                        && let Err(err) = client
                            .publish(&cfg.ha_status_topic, QoS::AtLeastOnce, false, "online")
                            .await
                    {
                        tracing::warn!(error = %err, "could not publish the birth message");
                    }
                    if let Err(err) = hub.publish_types().await {
                        tracing::warn!(error = %err, "could not publish the type table");
                    }
                }
                online = now_online;
            }
            () = resubscribe.notified() => {
                debounce.arm(hub.subscription_debounce());
            }
            () = &mut debounce => {
                if online {
                    let wanted = hub.subscription_topics();
                    let added: BTreeSet<String> = wanted.difference(&subscribed).cloned().collect();
                    let dropped: Vec<String> = subscribed.difference(&wanted).cloned().collect();
                    if !added.is_empty() {
                        tracing::debug!(count = added.len(), "subscribing");
                        subscribe(&client, valid_filters(added)).await;
                    }
                    for topic in dropped {
                        tracing::debug!(%topic, "unsubscribing");
                        if let Err(err) = client.unsubscribe(topic).await {
                            tracing::warn!(error = %err, "could not unsubscribe");
                        }
                    }
                    subscribed = wanted;
                }
            }
            message = publish_rx.recv() => match message {
                Some(publish) => {
                    tracing::debug!(topic = %publish.topic, retain = publish.retain, "publishing");
                    if let Err(err) = client
                        .publish(publish.topic, QoS::AtLeastOnce, publish.retain, publish.payload)
                        .await
                    {
                        tracing::warn!(error = %err, "could not publish");
                    }
                }
                None => return,
            },
        }
    }
}

/// The wildcards that are always subscribed: discovery, the Z2M bridge, the UI sub-tree, the
/// flow definitions and the push topics. A push filter already inside one of the `#` wildcards
/// (`oikos/push/...` under `oikos/#`) is left out: an overlapping subscription may make the
/// broker deliver a notification twice, and so send it twice.
fn static_filters(topics: &TopicsConfig, push: &[String]) -> BTreeSet<String> {
    let wildcards: BTreeSet<String> = topics
        .discovery_prefixes
        .iter()
        .map(|prefix| format!("{prefix}/#"))
        .chain([
            format!("{}/bridge/#", topics.z2m_base),
            format!("{}/#", topics.ui_prefix),
            format!("{}/#", topics.automation_prefix),
        ])
        .collect();
    let covered = |filter: &str| {
        wildcards.iter().any(|wildcard| {
            let base = wildcard.strip_suffix('#').unwrap_or(wildcard);
            filter.starts_with(base)
        })
    };
    let push: Vec<String> = push.iter().filter(|f| !covered(f)).cloned().collect();
    wildcards.into_iter().chain(push).collect()
}

/// The filters to subscribe at QoS 1, skipping (and logging) a topic the protocol cannot take.
fn valid_filters(topics: BTreeSet<String>) -> Vec<SubscribeFilter> {
    topics
        .into_iter()
        .filter(|topic| {
            let valid = rumqttc::valid_filter(topic);
            if !valid {
                tracing::warn!(%topic, "skipping an invalid subscription topic");
            }
            valid
        })
        .map(|topic| SubscribeFilter::new(topic, QoS::AtLeastOnce))
        .collect()
}

/// Queues one Subscribe for `filters`; `false` when nothing was queued.
async fn subscribe(client: &AsyncClient, filters: Vec<SubscribeFilter>) -> bool {
    if filters.is_empty() {
        return false;
    }
    match client.subscribe_many(filters).await {
        Ok(()) => true,
        Err(err) => {
            tracing::warn!(error = %err, "could not subscribe");
            false
        }
    }
}

/// A one-shot timer usable in `select!`: pending until armed, then fires once.
struct Timer {
    sleep: Pin<Box<Sleep>>,
    armed: bool,
}

impl Timer {
    fn new() -> Self {
        Self {
            sleep: Box::pin(tokio::time::sleep(Duration::ZERO)),
            armed: false,
        }
    }

    fn armed(&self) -> bool {
        self.armed
    }

    /// (Re)starts the timer: it fires `after` from now.
    fn arm(&mut self, after: Duration) {
        self.sleep.as_mut().reset(Instant::now() + after);
        self.armed = true;
    }
}

impl std::future::Future for Timer {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<()> {
        if !self.armed {
            return std::task::Poll::Pending;
        }
        match self.sleep.as_mut().poll(cx) {
            std::task::Poll::Ready(()) => {
                self.armed = false;
                std::task::Poll::Ready(())
            }
            std::task::Poll::Pending => std::task::Poll::Pending,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn push_filters(prefix: &str, legacy: &str) -> Vec<String> {
        [prefix, legacy]
            .iter()
            .flat_map(|p| {
                [
                    format!("{p}/notification/+"),
                    format!("{p}/tokens/+"),
                    format!("{p}/history"),
                ]
            })
            .collect()
    }

    #[test]
    fn push_filters_under_a_wildcard_are_left_out() {
        let filters = static_filters(
            &TopicsConfig::default(),
            &push_filters("oikos/push", "oikos-push"),
        );
        assert!(filters.contains("oikos/#"));
        assert!(
            !filters.iter().any(|f| f.starts_with("oikos/push/")),
            "{filters:?}"
        );
        for legacy in [
            "oikos-push/notification/+",
            "oikos-push/tokens/+",
            "oikos-push/history",
        ] {
            assert!(
                filters.contains(legacy),
                "{legacy} missing from {filters:?}"
            );
        }
    }

    #[test]
    fn push_filters_outside_the_wildcards_are_kept() {
        let topics = TopicsConfig {
            discovery_prefixes: vec!["homeassistant".into()],
            ..TopicsConfig::default()
        };
        let filters = static_filters(&topics, &push_filters("oikos/push", "oikos-push"));
        assert!(filters.contains("oikos/push/notification/+"), "{filters:?}");
        assert!(filters.contains("oikos/push/tokens/+"));
        assert!(filters.contains("oikos/push/history"));
        // `oikos-push/...` is not under `oikos/#`: a prefix match stops at the separator.
        assert!(filters.contains("oikos-push/history"));
    }

    #[test]
    fn full_subscription_is_recognised_by_its_count_once() {
        let full = FullSubscription::default();
        assert!(!full.acknowledged_by(3), "nothing awaited yet");
        full.set(Some(12));
        assert!(!full.acknowledged_by(2), "an incremental diff");
        assert!(full.acknowledged_by(12));
        assert!(!full.acknowledged_by(12), "only once per connection");
    }
}
