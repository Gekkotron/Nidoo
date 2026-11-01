//! `PushService` behaviour, ported from OikosPush `tests/test_service.py` (minus the
//! MQTT-handler and persistence cases) plus the hub-specific first-sync rules.

use std::cell::Cell;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::future::BoxFuture;
use nidoo_core::command::Publish;
use nidoo_push::Priority;
use nidoo_push::fcm::{FcmTransport, Outcome};
use nidoo_push::service::{PushService, PushSettings, PushState};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio::time::Instant;

const NOW: u64 = 1_700_000_000;
const P: &str = "oikos/push";
const L: &str = "oikos-push";

thread_local! {
    static START: Cell<Option<Instant>> = const { Cell::new(None) };
}

/// Unix seconds that follow tokio's paused clock, starting at `NOW`.
fn clock() -> u64 {
    START.with(|s| {
        let start = s.get().expect("clock started by the harness");
        NOW + start.elapsed().as_secs()
    })
}

#[derive(Clone, Debug)]
struct Call {
    token: String,
    data: BTreeMap<String, String>,
    priority: Priority,
    ttl: u32,
}

/// Records every per-token send and answers from a per-token script (default `Sent`).
/// `delays` slows the next sends down, one entry per call; `delivered` logs completions.
#[derive(Default)]
struct FakeFcm {
    calls: Mutex<Vec<Call>>,
    delivered: Mutex<Vec<Call>>,
    script: Mutex<HashMap<String, VecDeque<Outcome>>>,
    delays: Mutex<VecDeque<Duration>>,
}

impl FakeFcm {
    fn script(&self, token: &str, outcomes: Vec<Outcome>) {
        self.script
            .lock()
            .unwrap()
            .insert(token.to_string(), outcomes.into());
    }

    fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }

    fn tokens(&self) -> Vec<String> {
        self.calls().into_iter().map(|c| c.token).collect()
    }

    fn delay_next(&self, d: Duration) {
        self.delays.lock().unwrap().push_back(d);
    }

    /// `(id, cancel)` of each completed send, in completion order.
    fn delivered(&self) -> Vec<(String, bool)> {
        self.delivered
            .lock()
            .unwrap()
            .iter()
            .map(|c| (c.data["id"].clone(), c.data.contains_key("cancel")))
            .collect()
    }
}

impl FcmTransport for FakeFcm {
    fn send<'a>(
        &'a self,
        token: &'a str,
        data: &'a BTreeMap<String, String>,
        priority: Priority,
        ttl: u32,
    ) -> BoxFuture<'a, Outcome> {
        let call = Call {
            token: token.to_string(),
            data: data.clone(),
            priority,
            ttl,
        };
        self.calls.lock().unwrap().push(call.clone());
        let delay = self.delays.lock().unwrap().pop_front();
        let outcome = self
            .script
            .lock()
            .unwrap()
            .get_mut(token)
            .and_then(VecDeque::pop_front)
            .unwrap_or(Outcome::Sent);
        Box::pin(async move {
            if let Some(d) = delay {
                tokio::time::sleep(d).await;
            }
            self.delivered.lock().unwrap().push(call);
            outcome
        })
    }
}

struct Harness {
    svc: Arc<PushService>,
    rx: mpsc::Receiver<Publish>,
    fcm: Arc<FakeFcm>,
}

fn settings() -> PushSettings {
    PushSettings {
        prefix: P.into(),
        legacy_prefix: L.into(),
        retry_interval: 60,
        max_retries: 10,
        history_size: 100,
        history_days: 7,
    }
}

fn harness_with(settings: PushSettings, with_transport: bool) -> Harness {
    START.with(|s| s.set(Some(Instant::now())));
    let (tx, rx) = mpsc::channel(256);
    let fcm = Arc::new(FakeFcm::default());
    let transport: Option<Arc<dyn FcmTransport>> = if with_transport {
        Some(fcm.clone())
    } else {
        None
    };
    let svc = PushService::new(settings, tx, transport, clock);
    Harness { svc, rx, fcm }
}

fn harness() -> Harness {
    harness_with(settings(), true)
}

/// Lets spawned tasks (timers, sends, the retry worker) run.
async fn flush() {
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
}

async fn advance(secs: f64) {
    tokio::time::advance(Duration::from_secs_f64(secs)).await;
    flush().await;
}

impl Harness {
    /// Connect, acknowledge the subscriptions and wait out the settle; drop the publishes.
    async fn ready(mut self) -> Harness {
        self.svc.on_connected();
        self.svc.on_subscribed();
        advance(2.0).await;
        self.drain();
        self
    }

    fn drain(&mut self) -> Vec<Publish> {
        let mut out = Vec::new();
        while let Ok(p) = self.rx.try_recv() {
            out.push(p);
        }
        out
    }

    fn history_publishes(&mut self) -> Vec<Publish> {
        self.drain()
            .into_iter()
            .filter(|p| p.topic == format!("{P}/history"))
            .collect()
    }

    fn register(&self, topic: &str, token: &str) {
        let payload = json!({ "token": token }).to_string();
        self.svc.on_message(topic, payload.as_bytes(), true);
    }

    fn notify(&self, source: &str, payload: &Value) {
        let topic = format!("{P}/notification/{source}");
        self.svc
            .on_message(&topic, payload.to_string().as_bytes(), false);
    }
}

fn tok(id: &str) -> String {
    format!("{P}/tokens/{id}")
}

fn v1(over: Value) -> Value {
    let mut base =
        json!({"v": 1, "id": "a1", "channel": "camera", "title": "Front door", "body": "Person"});
    for (k, v) in over.as_object().unwrap() {
        base[k] = v.clone();
    }
    base
}

fn ids(p: &Publish) -> Vec<String> {
    let list: Vec<Value> = serde_json::from_str(&p.payload).unwrap();
    list.iter()
        .map(|e| e["id"].as_str().unwrap().to_string())
        .collect()
}

fn clears(publishes: &[Publish]) -> Vec<String> {
    let mut out: Vec<String> = publishes
        .iter()
        .filter(|p| p.topic.contains("/tokens/") && p.payload.is_empty() && p.retain)
        .map(|p| p.topic.clone())
        .collect();
    out.sort();
    out
}

// ---------- hub-specific first-sync rules ----------

#[tokio::test(start_paused = true)]
async fn first_boot_never_publishes_an_empty_history() {
    let mut h = harness();
    h.svc.on_connected();
    h.svc.on_subscribed();
    h.svc
        .submit("t", &json!({"v": 1, "id": "a", "title": "A"}))
        .unwrap();
    let old = json!([{"v": 1, "id": "old", "title": "Old", "sent_at": NOW}]).to_string();
    h.svc
        .on_message(&format!("{P}/history"), old.as_bytes(), true);
    flush().await;
    assert!(h.drain().is_empty(), "nothing before the settle");
    advance(1.9).await;
    assert!(h.drain().is_empty(), "nothing before the settle");
    advance(0.1).await;
    let pubs = h.drain();
    assert_eq!(pubs.len(), 1, "{pubs:?}");
    assert_eq!(pubs[0].topic, format!("{P}/history"));
    assert!(pubs[0].retain);
    assert_eq!(ids(&pubs[0]), ["a", "old"]);
}

#[tokio::test(start_paused = true)]
async fn legacy_history_seeds_once_when_new_is_empty() {
    let mut h = harness();
    let legacy = json!([{"v": 1, "id": "leg", "title": "L", "sent_at": NOW}]);
    h.svc.on_connected();
    h.svc
        .on_message(&format!("{L}/history"), legacy.to_string().as_bytes(), true);
    h.svc.on_subscribed();
    advance(2.0).await;
    let pubs = h.history_publishes();
    assert_eq!(pubs.len(), 1);
    let published: Value = serde_json::from_str(&pubs[0].payload).unwrap();
    assert_eq!(published, legacy);

    // Second connection: the broker replays both retained lists; the legacy one has moved on.
    h.svc.on_connected();
    let ours = h.history_publishes();
    assert_eq!(ours.len(), 1, "a reconnect republishes the list");
    let other = json!([{"v": 1, "id": "leg2", "title": "L2", "sent_at": NOW}]);
    h.svc
        .on_message(&format!("{L}/history"), other.to_string().as_bytes(), true);
    h.svc
        .on_message(&format!("{P}/history"), ours[0].payload.as_bytes(), true);
    h.svc.on_subscribed();
    advance(2.0).await;
    assert!(h.history_publishes().is_empty());
    h.svc
        .submit("t", &json!({"v": 1, "id": "x", "title": "X"}))
        .unwrap();
    assert_eq!(ids(&h.history_publishes()[0]), ["x", "leg"]);
}

#[tokio::test(start_paused = true)]
async fn retained_empty_array_blocks_legacy_seed() {
    let mut h = harness();
    h.svc.on_connected();
    h.svc.on_message(&format!("{P}/history"), b"[]", true);
    let legacy = json!([{"v": 1, "id": "leg", "title": "L", "sent_at": NOW}]).to_string();
    h.svc
        .on_message(&format!("{L}/history"), legacy.as_bytes(), true);
    h.svc.on_subscribed();
    advance(2.0).await;
    let pubs = h.history_publishes();
    assert_eq!(pubs.len(), 1);
    assert_eq!(pubs[0].payload, "[]");
}

#[tokio::test(start_paused = true)]
async fn missing_cleared_or_non_array_history_still_seeds() {
    let legacy = json!([{"v": 1, "id": "leg", "title": "L", "sent_at": NOW}]);
    for new in [
        None,
        Some(&b""[..]),
        Some(&b"{\"a\": 1}"[..]),
        Some(&b"not json"[..]),
    ] {
        let mut h = harness();
        h.svc.on_connected();
        if let Some(raw) = new {
            h.svc.on_message(&format!("{P}/history"), raw, true);
        }
        h.svc
            .on_message(&format!("{L}/history"), legacy.to_string().as_bytes(), true);
        h.svc.on_subscribed();
        advance(2.0).await;
        let pubs = h.history_publishes();
        assert_eq!(pubs.len(), 1, "{new:?}");
        assert_eq!(ids(&pubs[0]), ["leg"], "{new:?}");
    }
}

#[tokio::test(start_paused = true)]
async fn own_history_echo_is_ignored() {
    let mut h = harness();
    h.svc.on_connected();
    let echo = json!([{"v": 1, "id": "echo", "title": "E", "sent_at": NOW}]).to_string();
    h.svc
        .on_message(&format!("{P}/history"), echo.as_bytes(), false);
    h.svc.on_subscribed();
    advance(2.0).await;
    let pubs = h.history_publishes();
    assert_eq!(pubs.len(), 1);
    assert!(
        ids(&pubs[0]).is_empty(),
        "a non-retained list is not adopted"
    );

    h.svc
        .submit("t", &json!({"v": 1, "id": "a", "title": "A"}))
        .unwrap();
    h.svc
        .on_message(&format!("{P}/history"), echo.as_bytes(), false);
    h.svc
        .submit("t", &json!({"v": 1, "id": "b", "title": "B"}))
        .unwrap();
    let pubs = h.history_publishes();
    assert_eq!(pubs.len(), 2);
    assert_eq!(ids(&pubs[1]), ["b", "a"]);
}

#[tokio::test(start_paused = true)]
async fn stale_settle_timer_is_ignored() {
    let h = harness();
    h.svc.on_connected();
    h.svc.on_subscribed();
    advance(1.0).await;
    h.svc.on_connected();
    advance(1.5).await;
    assert_eq!(h.svc.status().state, PushState::Syncing);
    h.svc.on_subscribed();
    advance(2.0).await;
    assert_eq!(h.svc.status().state, PushState::Ready);
}

#[tokio::test(start_paused = true)]
async fn same_token_on_legacy_and_new_topic_is_sent_once_and_cleared_on_both() {
    let mut h = harness().ready().await;
    h.register(&tok("a"), "t1");
    h.register(&format!("{L}/tokens/a"), "t1");
    assert_eq!(h.svc.status().installs, 2);
    h.fcm.script("t1", vec![Outcome::Invalid]);
    h.svc
        .submit("t", &json!({"v": 1, "id": "a", "title": "A"}))
        .unwrap();
    flush().await;
    assert_eq!(h.fcm.tokens(), ["t1"]);
    let pubs = h.drain();
    assert_eq!(clears(&pubs), [format!("{L}/tokens/a"), tok("a")]);
    assert_eq!(h.svc.status().installs, 0);
    assert_eq!(h.svc.status().queued, 0);
}

#[tokio::test(start_paused = true)]
async fn no_transport_keeps_history_and_reports_off() {
    let h = harness_with(settings(), false);
    assert_eq!(h.svc.status().state, PushState::Off);
    let mut h2 = h.ready().await;
    h2.register(&tok("a"), "tokA");
    h2.svc
        .submit("t", &json!({"v": 1, "id": "a", "title": "A"}))
        .unwrap();
    flush().await;
    let pubs = h2.history_publishes();
    assert_eq!(ids(&pubs[0]), ["a"]);
    assert!(h2.fcm.calls().is_empty());
    let status = h2.svc.status();
    assert_eq!(status.state, PushState::Off);
    assert_eq!(status.queued, 0);

    // Not synced (reconnect) and still no transport: nothing is queued either.
    h2.svc.on_connected();
    h2.svc
        .submit("t", &json!({"v": 1, "id": "b", "title": "B"}))
        .unwrap();
    assert_eq!(h2.svc.status().queued, 0);
    assert_eq!(h2.svc.status().state, PushState::Off);
}

#[tokio::test(start_paused = true)]
async fn transient_failure_is_retried_for_failed_tokens_only() {
    let h = harness().ready().await;
    h.register(&tok("a"), "tokA");
    h.register(&tok("b"), "tokB");
    let worker = h.svc.spawn_retry_worker();
    flush().await;
    h.fcm.script("tokA", vec![Outcome::Transient("503".into())]);
    h.fcm.script(
        "tokB",
        vec![
            Outcome::Transient("503".into()),
            Outcome::Transient("503".into()),
        ],
    );
    h.notify("frigate", &v1(json!({"ttl": 1200})));
    flush().await;
    assert_eq!(h.fcm.tokens(), ["tokA", "tokB"]);
    assert_eq!(h.svc.status().queued, 1);

    advance(60.0).await;
    assert_eq!(h.fcm.tokens(), ["tokA", "tokB", "tokA", "tokB"]);
    assert_eq!(h.svc.status().queued, 1, "tokB still failing");

    advance(60.0).await;
    let calls = h.fcm.calls();
    assert_eq!(calls.len(), 5);
    assert_eq!(
        calls[4].token, "tokB",
        "tokA is never re-sent once it succeeded"
    );
    assert_eq!(calls[4].ttl, 1200);
    assert_eq!(calls[4].priority, Priority::High);
    assert_eq!(calls[4].data["id"], "a1");
    assert_eq!(h.svc.status().queued, 0);
    worker.abort();
}

#[tokio::test(start_paused = true)]
async fn submit_rejects_invalid_payload() {
    let mut h = harness().ready().await;
    h.register(&tok("a"), "tokA");
    assert!(h.svc.submit("t", &json!({"v": 1})).is_err());
    flush().await;
    assert!(h.drain().is_empty());
    assert!(h.fcm.calls().is_empty());
    assert_eq!(
        h.svc
            .submit("t", &json!({"v": 1, "id": "ok", "title": "T"}))
            .unwrap(),
        "ok"
    );
}

// ---------- topics ----------

#[tokio::test(start_paused = true)]
async fn filters_list_new_and_legacy_topics() {
    let h = harness();
    assert_eq!(
        h.svc.filters(),
        [
            "oikos/push/notification/+",
            "oikos/push/tokens/+",
            "oikos/push/history",
            "oikos-push/notification/+",
            "oikos-push/tokens/+",
            "oikos-push/history",
        ]
    );
    assert_eq!(h.svc.history_topic(), "oikos/push/history");
    assert_eq!(h.svc.responses_topic(), "oikos/push/responses");
    for t in [
        "oikos/push/notification/x",
        "oikos/push/tokens/a",
        "oikos/push/history",
        "oikos-push/notification/x",
        "oikos-push/tokens/a",
        "oikos-push/history",
    ] {
        assert!(h.svc.wants(t), "{t}");
    }
    for t in [
        "oikos/push/responses",
        "oikos-push/responses",
        "oikos/push/heartbeat",
        "oikos/push/other/x",
        "elsewhere/notification/x",
        "oikos/light/x/config",
    ] {
        assert!(!h.svc.wants(t), "{t}");
    }
}

#[tokio::test(start_paused = true)]
async fn empty_legacy_prefix_disables_legacy_topics() {
    let h = harness_with(
        PushSettings {
            legacy_prefix: String::new(),
            ..settings()
        },
        true,
    );
    assert_eq!(h.svc.filters().len(), 3);
    assert!(!h.svc.wants("oikos-push/notification/x"));
    assert!(!h.svc.wants("/notification/x"));
    assert!(!h.svc.wants("/history"));
}

#[tokio::test(start_paused = true)]
async fn custom_prefix_drives_every_topic() {
    let h = harness_with(
        PushSettings {
            prefix: "/home/push/".into(),
            ..settings()
        },
        true,
    );
    assert_eq!(
        h.svc.filters()[..3],
        [
            "home/push/notification/+",
            "home/push/tokens/+",
            "home/push/history",
        ]
    );
    assert_eq!(h.svc.history_topic(), "home/push/history");
    assert_eq!(h.svc.responses_topic(), "home/push/responses");
}

#[tokio::test(start_paused = true)]
async fn legacy_notification_topic_is_handled_like_the_new_one() {
    let h = harness().ready().await;
    h.register(&format!("{L}/tokens/a"), "tokA");
    h.svc.on_message(
        &format!("{L}/notification/jeedom"),
        v1(json!({})).to_string().as_bytes(),
        false,
    );
    flush().await;
    let calls = h.fcm.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].data["source"], "jeedom");
}

#[tokio::test(start_paused = true)]
async fn nested_notification_topic_is_dropped() {
    let mut h = harness().ready().await;
    h.register(&tok("a"), "tokA");
    h.svc.on_message(
        &format!("{P}/notification/a/b"),
        v1(json!({})).to_string().as_bytes(),
        false,
    );
    flush().await;
    assert!(h.fcm.calls().is_empty());
    assert!(h.drain().is_empty());
}

// ---------- connect ----------

#[tokio::test(start_paused = true)]
async fn reconnect_republishes_history_retained() {
    let mut h = harness().ready().await;
    h.notify("frigate", &v1(json!({})));
    h.drain();
    h.svc.on_connected();
    let pubs = h.drain();
    assert_eq!(pubs.len(), 1);
    assert!(pubs[0].retain);
    assert_eq!(ids(&pubs[0]), ["a1"]);
}

#[tokio::test(start_paused = true)]
async fn reconnect_rebuilds_token_map() {
    let h = harness().ready().await;
    h.register(&tok("inst-a"), "tokA");
    h.register(&tok("inst-b"), "tokB");
    assert_eq!(h.svc.status().installs, 2);
    h.svc.on_connected();
    assert_eq!(h.svc.status().installs, 0);
    h.register(&tok("inst-a"), "tokA");
    h.svc.on_subscribed();
    advance(2.0).await;
    h.notify("frigate", &v1(json!({})));
    flush().await;
    assert_eq!(h.fcm.tokens(), ["tokA"]);
}

// ---------- tokens ----------

#[tokio::test(start_paused = true)]
async fn token_messages_register_update_and_unregister() {
    let h = harness().ready().await;
    h.register(&tok("inst-a"), "old");
    h.register(&tok("inst-a"), "new");
    h.notify("frigate", &v1(json!({})));
    flush().await;
    assert_eq!(h.fcm.tokens(), ["new"]);
    h.svc.on_message(&tok("inst-a"), b"", true);
    assert_eq!(h.svc.status().installs, 0);
}

#[tokio::test(start_paused = true)]
async fn bad_token_messages_are_ignored() {
    let h = harness().ready().await;
    h.register(&tok("inst-a"), "keep");
    h.svc.on_message(&tok("inst-a"), b"not json", true);
    h.svc.on_message(&tok("inst-a"), br#"{"nope":1}"#, true);
    h.svc.on_message(&tok("a/b"), br#"{"token":"x"}"#, true);
    h.svc.on_message(&tok(""), br#"{"token":"y"}"#, true);
    assert_eq!(h.svc.status().installs, 1);
    h.notify("frigate", &v1(json!({})));
    flush().await;
    assert_eq!(h.fcm.tokens(), ["keep"]);
}

// ---------- sending ----------

#[tokio::test(start_paused = true)]
async fn v1_notification_is_sent_with_normalized_data_priority_and_ttl() {
    let h = harness().ready().await;
    h.register(&tok("inst-a"), "tokA");
    h.notify("frigate", &v1(json!({"ttl": 600})));
    flush().await;
    let call = &h.fcm.calls()[0];
    assert_eq!(call.token, "tokA");
    assert_eq!(call.priority, Priority::High, "camera channel");
    assert_eq!(call.ttl, 600);
    assert_eq!(call.data["id"], "a1");
    assert_eq!(call.data["source"], "frigate");
    assert_eq!(call.data["sent_at"], clock().to_string());
    assert_eq!(call.data["v"], "1");
}

#[tokio::test(start_paused = true)]
async fn legacy_notification_is_normalized_before_sending() {
    let h = harness().ready().await;
    h.register(&tok("inst-a"), "tokA");
    let legacy = json!({"title": "Alarme", "body": "Intrusion", "priority": "high",
        "data": {"id": 9, "channelId": "alarme",
                 "actions": "[{\"btn\":\"Désarmer\",\"action\":\"disarm\"}]"}});
    h.notify("jeedom", &legacy);
    flush().await;
    let data = &h.fcm.calls()[0].data;
    assert_eq!(
        (
            data["id"].as_str(),
            data["channel"].as_str(),
            data["severity"].as_str()
        ),
        ("9", "alarm", "warning")
    );
    let actions: Value = serde_json::from_str(&data["actions"]).unwrap();
    assert_eq!(actions, json!([{"id": "disarm", "label": "Désarmer"}]));
}

#[tokio::test(start_paused = true)]
async fn cancel_is_sent_with_300s_ttl() {
    let h = harness().ready().await;
    h.register(&tok("inst-a"), "tokA");
    h.notify("frigate", &json!({"v": 1, "id": "a1", "cancel": true}));
    flush().await;
    let call = &h.fcm.calls()[0];
    assert_eq!(call.ttl, 300);
    assert_eq!(call.data["cancel"], "true");
}

#[tokio::test(start_paused = true)]
async fn normal_priority_for_info_energy() {
    let h = harness().ready().await;
    h.register(&tok("inst-a"), "tokA");
    h.notify("frigate", &v1(json!({"channel": "energy"})));
    flush().await;
    assert_eq!(h.fcm.calls()[0].priority, Priority::Normal);
}

#[tokio::test(start_paused = true)]
async fn history_is_updated_and_republished_retained_after_each_alert() {
    let mut h = harness().ready().await;
    h.register(&tok("inst-a"), "tokA");
    h.notify("frigate", &v1(json!({"id": "a1"})));
    h.notify(
        "frigate",
        &v1(json!({"id": "a2", "severity": "warning", "silent": true})),
    );
    let pubs = h.history_publishes();
    assert_eq!(pubs.len(), 2);
    assert!(pubs.iter().all(|p| p.retain));
    let entries: Vec<Value> = serde_json::from_str(&pubs[1].payload).unwrap();
    assert_eq!(ids(&pubs[1]), ["a2", "a1"]);
    assert_eq!(entries[0]["silent"], true);
    assert_eq!(entries[0]["sent_at"], clock());
    assert!(entries[0]["actions"].is_array());
}

#[tokio::test(start_paused = true)]
async fn no_tokens_means_history_only_and_nothing_queued() {
    let mut h = harness().ready().await;
    h.notify("frigate", &v1(json!({})));
    flush().await;
    assert!(h.fcm.calls().is_empty());
    assert_eq!(h.svc.status().queued, 0);
    let pubs = h.history_publishes();
    assert_eq!(pubs.len(), 1);
    assert_eq!(ids(&pubs[0]), ["a1"]);
}

#[tokio::test(start_paused = true)]
async fn non_notification_topics_are_ignored() {
    let mut h = harness().ready().await;
    h.register(&tok("inst-a"), "tokA");
    let payload = v1(json!({})).to_string();
    for topic in [
        format!("{P}/responses"),
        format!("{P}/history"),
        format!("{P}/heartbeat"),
        format!("{P}/other/x"),
        "elsewhere/notification/x".to_string(),
    ] {
        h.svc.on_message(&topic, payload.as_bytes(), false);
    }
    flush().await;
    assert!(h.fcm.calls().is_empty());
    assert!(h.drain().is_empty());
}

// ---------- failures ----------

#[tokio::test(start_paused = true)]
async fn invalid_token_publishes_empty_retained_payload_and_clears_map() {
    let mut h = harness().ready().await;
    h.register(&tok("inst-a"), "tokA");
    h.fcm.script("tokA", vec![Outcome::Invalid]);
    h.notify("frigate", &v1(json!({})));
    flush().await;
    assert_eq!(clears(&h.drain()), [tok("inst-a")]);
    assert_eq!(h.svc.status().installs, 0);
    assert_eq!(h.svc.status().queued, 0, "invalid tokens are not retried");
}

#[tokio::test(start_paused = true)]
async fn invalid_token_shared_by_two_installs_clears_both_topics() {
    let mut h = harness().ready().await;
    h.register(&tok("inst-a"), "same");
    h.register(&tok("inst-b"), "same");
    h.register(&tok("inst-c"), "other");
    h.fcm.script("same", vec![Outcome::Invalid]);
    h.notify("frigate", &v1(json!({})));
    flush().await;
    assert_eq!(h.fcm.tokens(), ["same", "other"], "shared token sent once");
    assert_eq!(clears(&h.drain()), [tok("inst-a"), tok("inst-b")]);
    assert_eq!(h.svc.status().installs, 1);
}

#[tokio::test(start_paused = true)]
async fn mixed_failures_queue_only_transient_tokens() {
    let mut h = harness().ready().await;
    for (install, token) in [("a", "tokA"), ("b", "tokB"), ("c", "tokC")] {
        h.register(&tok(install), token);
    }
    h.fcm.script("tokB", vec![Outcome::Invalid]);
    h.fcm.script("tokC", vec![Outcome::Transient("503".into())]);
    h.notify("frigate", &v1(json!({})));
    flush().await;
    assert_eq!(h.fcm.tokens(), ["tokA", "tokB", "tokC"]);
    assert_eq!(h.svc.status().queued, 1);
    assert_eq!(clears(&h.drain()), [tok("b")]);

    let worker = h.svc.spawn_retry_worker();
    flush().await;
    assert_eq!(h.fcm.tokens()[3..], ["tokC"], "never re-sent to tokA");
    assert_eq!(h.svc.status().queued, 0);
    worker.abort();
}

#[tokio::test(start_paused = true)]
async fn retry_skips_tokens_no_longer_registered() {
    let h = harness().ready().await;
    h.register(&tok("c"), "tokC");
    h.fcm.script("tokC", vec![Outcome::Transient("503".into())]);
    h.notify("frigate", &v1(json!({})));
    flush().await;
    h.svc.on_message(&tok("c"), b"", true);
    let worker = h.svc.spawn_retry_worker();
    advance(60.0).await;
    assert_eq!(h.fcm.calls().len(), 1, "no second FCM call");
    assert_eq!(h.svc.status().queued, 0, "item dropped, not retried");
    worker.abort();
}

#[tokio::test(start_paused = true)]
async fn message_level_error_keeps_tokens_and_drops_the_alert() {
    let mut h = harness().ready().await;
    h.register(&tok("inst-a"), "tokA");
    h.register(&tok("inst-b"), "tokB");
    let err = Outcome::Message("Android message is too big".into());
    h.fcm.script("tokA", vec![err.clone()]);
    h.fcm.script("tokB", vec![err]);
    h.notify("frigate", &v1(json!({})));
    flush().await;
    assert!(clears(&h.drain()).is_empty());
    assert_eq!(h.svc.status().installs, 2);
    assert_eq!(h.svc.status().queued, 0);
}

// ---------- token map sync ----------

#[tokio::test(start_paused = true)]
async fn sync_needs_the_subscribe_ack_then_the_settle_delay() {
    let h = harness();
    assert_eq!(h.svc.status().state, PushState::Syncing);
    h.svc.on_connected();
    advance(5.0).await;
    assert_eq!(h.svc.status().state, PushState::Syncing);
    h.svc.on_subscribed();
    advance(1.9).await;
    assert_eq!(h.svc.status().state, PushState::Syncing);
    advance(0.1).await;
    assert_eq!(h.svc.status().state, PushState::Ready);
}

#[tokio::test(start_paused = true)]
async fn alert_before_first_sync_is_held_and_sent_at_settle() {
    let mut h = harness();
    h.svc.on_connected();
    h.notify("frigate", &v1(json!({})));
    flush().await;
    assert!(h.drain().is_empty());
    assert!(h.fcm.calls().is_empty());
    assert_eq!(h.svc.status().queued, 0, "held, not queued");
    h.register(&tok("inst-a"), "tokA");
    h.register(&tok("inst-b"), "tokB");
    h.svc.on_subscribed();
    advance(2.0).await;
    assert_eq!(ids(&h.history_publishes()[0]), ["a1"]);
    assert_eq!(h.fcm.tokens(), ["tokA", "tokB"]);
}

#[tokio::test(start_paused = true)]
async fn alert_while_resyncing_is_queued_unresolved_and_sent_once_synced() {
    let mut h = harness().ready().await;
    let worker = h.svc.spawn_retry_worker();
    flush().await;
    h.svc.on_connected();
    h.notify("frigate", &v1(json!({})));
    flush().await;
    assert_eq!(ids(&h.history_publishes().pop().unwrap()), ["a1"]);
    assert!(h.fcm.calls().is_empty());
    assert_eq!(h.svc.status().queued, 1);
    h.register(&tok("inst-a"), "tokA");
    h.register(&tok("inst-b"), "tokB");
    advance(60.0).await;
    assert!(h.fcm.calls().is_empty(), "the worker waits for the sync");
    assert_eq!(h.svc.status().queued, 1);
    h.svc.on_subscribed();
    advance(2.0).await;
    advance(60.0).await;
    assert_eq!(h.fcm.tokens(), ["tokA", "tokB"]);
    assert_eq!(h.svc.status().queued, 0);
    worker.abort();
}

#[tokio::test(start_paused = true)]
async fn unresolved_alert_with_no_tokens_after_sync_is_history_only() {
    let h = harness().ready().await;
    let worker = h.svc.spawn_retry_worker();
    flush().await;
    h.svc.on_connected();
    h.notify("frigate", &v1(json!({})));
    assert_eq!(h.svc.status().queued, 1);
    h.svc.on_subscribed();
    advance(2.0).await;
    advance(60.0).await;
    assert!(h.fcm.calls().is_empty());
    assert_eq!(h.svc.status().queued, 0);
    worker.abort();
}

#[tokio::test(start_paused = true)]
async fn reconnect_resets_synced_until_the_new_subscribe_and_settle() {
    let h = harness().ready().await;
    assert_eq!(h.svc.status().state, PushState::Ready);
    h.svc.on_connected();
    assert_eq!(h.svc.status().state, PushState::Syncing);
    h.svc.on_subscribed();
    assert_eq!(h.svc.status().state, PushState::Syncing);
    advance(2.0).await;
    assert_eq!(h.svc.status().state, PushState::Ready);
}

#[tokio::test(start_paused = true)]
async fn status_serializes_with_snake_case_state() {
    let h = harness().ready().await;
    h.register(&tok("a"), "tokA");
    assert_eq!(
        serde_json::to_value(h.svc.status()).unwrap(),
        json!({"state": "ready", "installs": 1, "queued": 0})
    );
    let off = harness_with(settings(), false);
    assert_eq!(
        serde_json::to_value(off.svc.status()).unwrap()["state"],
        "off"
    );
}

// ---------- garbage ----------

#[tokio::test(start_paused = true)]
async fn garbage_payloads_are_dropped_without_side_effects() {
    let mut h = harness().ready().await;
    h.register(&tok("inst-a"), "tokA");
    let raws: [&[u8]; 11] = [
        b"\xff\xfe\x00",
        b"not json",
        b"",
        b"[1, 2]",
        b"42",
        b"\"text\"",
        b"null",
        b"true",
        br#"{"v": 2, "id": "x", "title": "T"}"#,
        br#"{"v": 1, "title": "no id"}"#,
        br#"{"v": 1, "id": "x"}"#,
    ];
    for raw in raws {
        h.svc.on_message(&format!("{P}/notification/x"), raw, false);
    }
    flush().await;
    assert!(h.fcm.calls().is_empty());
    assert!(h.drain().is_empty());
    assert_eq!(h.svc.status().queued, 0);
}

// ---------- loc ----------

#[tokio::test(start_paused = true)]
async fn a_dropped_loc_still_sends_the_alert() {
    let h = harness().ready().await;
    h.register(&tok("inst-a"), "tokA");
    h.notify("frigate", &v1(json!({"loc": "frigate.object"})));
    flush().await;
    let data = &h.fcm.calls()[0].data;
    assert_eq!(data["id"], "a1");
    assert!(!data.contains_key("loc"));
}

#[tokio::test(start_paused = true)]
async fn a_valid_loc_reaches_fcm() {
    let h = harness().ready().await;
    h.register(&tok("inst-a"), "tokA");
    h.notify(
        "frigate",
        &v1(json!({"loc": {"key": "frigate.object", "args": {"label": "car"}}})),
    );
    flush().await;
    assert_eq!(
        h.fcm.calls()[0].data["loc"],
        r#"{"key":"frigate.object","args":{"label":"car"}}"#
    );
}

// ---------- ordering ----------

#[tokio::test(start_paused = true)]
async fn an_alert_and_its_cancel_reach_fcm_in_order() {
    let h = harness().ready().await;
    h.register(&tok("inst-a"), "tokA");
    h.fcm.delay_next(Duration::from_secs(1));
    h.notify("frigate", &v1(json!({})));
    h.notify("frigate", &json!({"v": 1, "id": "a1", "cancel": true}));
    flush().await;
    assert!(
        h.fcm.delivered().is_empty(),
        "the slow first send holds the queue"
    );
    assert_eq!(h.fcm.calls().len(), 1, "the cancel waits for the alert");
    advance(1.0).await;
    assert_eq!(
        h.fcm.delivered(),
        [("a1".to_string(), false), ("a1".to_string(), true)]
    );
}

#[tokio::test(start_paused = true)]
async fn held_alerts_are_sent_in_arrival_order_at_the_first_settle() {
    let h = harness();
    h.svc.on_connected();
    h.notify("frigate", &v1(json!({})));
    h.notify("frigate", &json!({"v": 1, "id": "a1", "cancel": true}));
    h.register(&tok("inst-a"), "tokA");
    h.fcm.delay_next(Duration::from_millis(500));
    h.svc.on_subscribed();
    advance(2.0).await;
    advance(0.5).await;
    assert_eq!(
        h.fcm.delivered(),
        [("a1".to_string(), false), ("a1".to_string(), true)]
    );
}

#[tokio::test(start_paused = true)]
async fn dropping_the_service_ends_the_sender_task() {
    let Harness { svc, rx, fcm } = harness().ready().await;
    svc.on_message(&tok("inst-a"), br#"{"token":"tokA"}"#, true);
    svc.submit("t", &json!({"v": 1, "id": "a", "title": "A"}))
        .unwrap();
    flush().await;
    assert_eq!(fcm.calls().len(), 1, "the sender task ran");
    let weak = Arc::downgrade(&svc);
    drop(svc);
    flush().await;
    assert!(
        weak.upgrade().is_none(),
        "the sender task holds no strong reference"
    );
    assert_eq!(Arc::strong_count(&fcm), 1, "the transport was released too");
    drop(rx);
}
