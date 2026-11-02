//! The stateful push service. The hub routes
//! its MQTT traffic here (`wants` / `on_message`, `on_connected`, `on_subscribed`) and the
//! service publishes through the hub's channel. The broker is the database: the history is
//! adopted from the retained `<prefix>/history` at the first sync, the queue is memory only.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;

use nidoo_core::command::Publish;
use serde::Serialize;
use serde_json::Value;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;

use crate::fcm::{FcmTransport, SendResult, send_data};
use crate::history::History;
use crate::normalize::{Alert, Rejected, android_priority, normalize, to_fcm_data, ttl_seconds};
use crate::queue::{Pending, RetryQueue};
use crate::tokens::{TokenStatus, Tokens};

/// Time given to the retained token burst after the subscription is acknowledged.
const SETTLE: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PushSettings {
    pub prefix: String,
    pub retry_interval: u64,
    pub max_retries: u32,
    pub history_size: usize,
    pub history_days: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PushState {
    /// No FCM transport: alerts are kept in the history only.
    Off,
    /// Waiting for the token map to be rebuilt after a (re)connect.
    Syncing,
    Ready,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PushStatus {
    pub state: PushState,
    pub installs: usize,
    pub queued: usize,
}

/// What a topic means to the service. `rest` is whatever follows the sub-tree.
enum Route<'a> {
    Notification(&'a str),
    Token(&'a str),
    History,
}

/// Not `Debug`: the token map holds secret FCM tokens.
struct State {
    /// The history has been adopted (or started empty) once; it is ours from then on.
    loaded: bool,
    /// The token map is trustworthy (subscription acknowledged + settle delay).
    synced: bool,
    /// Bumped on every connect, so a settle timer from an older connection does nothing.
    epoch: u64,
    /// The epoch of the settle timer still waiting, if any.
    settle_epoch: Option<u64>,
    /// Retained `<prefix>/history` seen before the first load.
    candidate: Option<Vec<u8>>,
    /// Alerts received before the first load, in arrival order.
    held: Vec<Alert>,
    history: History,
    tokens: Tokens,
    queue: RetryQueue,
}

/// One FCM send for the sender task: the item, the tokens to send it to now, and whether it
/// came from the retry queue (a failure then counts as one more attempt).
struct Job {
    pending: Pending,
    tokens: Vec<String>,
    retry: bool,
}

pub struct PushService {
    prefix: String,
    retry_interval: u64,
    publisher: mpsc::Sender<Publish>,
    transport: Option<Arc<dyn FcmTransport>>,
    clock: fn() -> u64,
    state: Mutex<State>,
    /// FIFO of sends, drained by one task so alerts reach FCM in the order they were taken
    /// (an alert, then its update or cancel). Fed under the state lock.
    jobs: mpsc::UnboundedSender<Job>,
    /// Taken by the first send, which spawns the sender task (so `new` needs no runtime).
    jobs_rx: Mutex<Option<mpsc::UnboundedReceiver<Job>>>,
}

impl PushService {
    pub fn new(
        settings: PushSettings,
        publisher: mpsc::Sender<Publish>,
        transport: Option<Arc<dyn FcmTransport>>,
        clock: fn() -> u64,
    ) -> Arc<PushService> {
        let state = State {
            loaded: false,
            synced: false,
            epoch: 0,
            settle_epoch: None,
            candidate: None,
            held: Vec::new(),
            history: History::new(settings.history_size, settings.history_days),
            tokens: Tokens::default(),
            queue: RetryQueue::new(settings.retry_interval, settings.max_retries),
        };
        let (jobs, jobs_rx) = mpsc::unbounded_channel();
        Arc::new(PushService {
            prefix: settings.prefix.trim_matches('/').to_string(),
            retry_interval: settings.retry_interval,
            publisher,
            transport,
            clock,
            state: Mutex::new(state),
            jobs,
            jobs_rx: Mutex::new(Some(jobs_rx)),
        })
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The MQTT filters the hub subscribes to for this service.
    pub fn filters(&self) -> Vec<String> {
        let p = &self.prefix;
        vec![
            format!("{p}/notification/+"),
            format!("{p}/tokens/+"),
            format!("{p}/history"),
        ]
    }

    fn route<'a>(&self, topic: &'a str) -> Option<Route<'a>> {
        let rest = topic
            .strip_prefix(self.prefix.as_str())?
            .strip_prefix('/')?;
        if rest == "history" {
            Some(Route::History)
        } else if let Some(source) = rest.strip_prefix("notification/") {
            Some(Route::Notification(source))
        } else {
            rest.strip_prefix("tokens/").map(Route::Token)
        }
    }

    pub fn wants(&self, topic: &str) -> bool {
        self.route(topic).is_some()
    }

    pub fn history_topic(&self) -> String {
        format!("{}/history", self.prefix)
    }

    pub fn responses_topic(&self) -> String {
        format!("{}/responses", self.prefix)
    }

    pub fn status(&self) -> PushStatus {
        let st = self.lock();
        let state = match (&self.transport, st.synced) {
            (None, _) => PushState::Off,
            (Some(_), false) => PushState::Syncing,
            (Some(_), true) => PushState::Ready,
        };
        PushStatus {
            state,
            installs: st.tokens.len(),
            queued: st.queue.len(),
        }
    }

    pub fn on_message(self: &Arc<Self>, topic: &str, payload: &[u8], retain: bool) {
        match self.route(topic) {
            None => tracing::debug!(%topic, "push ignores this topic"),
            Some(Route::Token(install)) => self.on_token(topic, install, payload),
            Some(Route::History) => self.on_history(topic, payload, retain),
            Some(Route::Notification(source)) => {
                if source.is_empty() || source.contains('/') {
                    tracing::warn!(%topic, "ignoring notification on unexpected topic");
                    return;
                }
                let parsed = std::str::from_utf8(payload)
                    .map_err(|e| e.to_string())
                    .and_then(|text| {
                        serde_json::from_str::<Value>(text).map_err(|e| e.to_string())
                    });
                match parsed {
                    Ok(value) => {
                        let _ = self.intake(source, &value);
                    }
                    Err(error) => tracing::warn!(%topic, %error, "dropping non-JSON payload"),
                }
            }
        }
    }

    fn on_token(&self, topic: &str, install: &str, payload: &[u8]) {
        if install.is_empty() || install.contains('/') {
            tracing::warn!(%topic, "ignoring token message on unexpected topic");
            return;
        }
        // Never log the payload: it holds the FCM token.
        match self.lock().tokens.handle(topic, payload) {
            TokenStatus::Ignored => tracing::warn!(%topic, "ignoring malformed token message"),
            TokenStatus::Noop => tracing::debug!(%topic, "token message: nothing to remove"),
            status => tracing::info!(%topic, ?status, "token message"),
        }
    }

    fn on_history(&self, topic: &str, payload: &[u8], retain: bool) {
        if !retain {
            // Our own publish coming back, or a stray live message: never adopted.
            tracing::debug!(%topic, "ignoring non-retained history");
            return;
        }
        let mut st = self.lock();
        if st.loaded {
            return;
        }
        st.candidate = Some(payload.to_vec());
    }

    /// The in-process path (bridges, flows, MCP): like a notification arriving on MQTT.
    pub fn submit(self: &Arc<Self>, source: &str, payload: &Value) -> Result<String, Rejected> {
        self.intake(source, payload)
    }

    fn intake(self: &Arc<Self>, source: &str, payload: &Value) -> Result<String, Rejected> {
        let alert = normalize(source, payload, (self.clock)()).inspect_err(|r| {
            tracing::warn!(%source, reason = %r.reason, "dropping push payload");
        })?;
        if alert.loc.is_none() && payload.get("loc").is_some() && payload["v"] == 1 {
            tracing::warn!(id = %alert.id, "ignoring invalid loc");
        }
        let id = alert.id.clone();
        let mut st = self.lock();
        if !st.loaded {
            st.held.push(alert);
        } else {
            st.history.apply(&alert, (self.clock)());
            self.publish_history(&st);
            if let Some(job) = self.outgoing(&mut st, &alert) {
                self.enqueue(job);
            }
        }
        Ok(id)
    }

    /// After the history holds `alert`: the send to run now, or the queueing while unsynced.
    fn outgoing(&self, st: &mut State, alert: &Alert) -> Option<Job> {
        self.transport.as_ref()?;
        let now = (self.clock)();
        let mut pending = Pending {
            id: alert.id.clone(),
            data: to_fcm_data(alert),
            tokens: None,
            priority: android_priority(alert),
            ttl: ttl_seconds(alert),
            queued_at: now,
            attempts: 0,
            next_at: now,
        };
        if !st.synced {
            // Token map not rebuilt yet: resolve the tokens at send time.
            st.queue.add(pending);
            return None;
        }
        let tokens = st.tokens.tokens();
        if tokens.is_empty() {
            tracing::info!(id = %alert.id, "no registered tokens; alert kept in history only");
            return None;
        }
        pending.tokens = Some(tokens.clone());
        Some(Job {
            pending,
            tokens,
            retry: false,
        })
    }

    pub fn on_connected(&self) {
        let mut st = self.lock();
        st.synced = false;
        st.epoch += 1;
        st.tokens.clear();
        if st.loaded {
            st.history.trim((self.clock)());
            self.publish_history(&st);
        }
    }

    pub fn on_subscribed(self: &Arc<Self>) {
        let epoch = {
            let mut st = self.lock();
            if st.synced || st.settle_epoch == Some(st.epoch) {
                return;
            }
            st.settle_epoch = Some(st.epoch);
            st.epoch
        };
        // The deadline is taken now, not when the task first runs.
        let deadline = tokio::time::Instant::now() + SETTLE;
        let me = Arc::clone(self);
        tokio::spawn(async move {
            tokio::time::sleep_until(deadline).await;
            me.settle(epoch);
        });
    }

    fn settle(self: &Arc<Self>, epoch: u64) {
        let mut st = self.lock();
        if st.epoch != epoch || st.settle_epoch != Some(epoch) {
            return;
        }
        st.settle_epoch = None;
        let held = if st.loaded {
            Vec::new()
        } else {
            self.load(&mut st)
        };
        st.synced = true;
        tracing::info!(installs = st.tokens.len(), "push token map synced");
        for alert in &held {
            if let Some(job) = self.outgoing(&mut st, alert) {
                self.enqueue(job);
            }
        }
    }

    /// The first settle: adopt the retained list, apply the held alerts
    /// and publish. Returns the held alerts, to be sent once synced.
    fn load(&self, st: &mut State) -> Vec<Alert> {
        let now = (self.clock)();
        let candidate = st.candidate.take();
        if let Some(raw) = candidate.filter(|r| !r.trim_ascii().is_empty()) {
            if st.history.replace_from_json(&raw) {
                tracing::info!(entries = st.history.entries().len(), "push history adopted");
            } else {
                tracing::warn!("retained push history is not a JSON array");
            }
        }
        st.loaded = true;
        st.history.trim(now);
        let held = std::mem::take(&mut st.held);
        for alert in &held {
            st.history.apply(alert, now);
        }
        self.publish_history(st);
        held
    }

    pub fn spawn_retry_worker(self: &Arc<Self>) -> JoinHandle<()> {
        let period = Duration::from_secs(self.retry_interval.max(1));
        let mut tick = tokio::time::interval_at(tokio::time::Instant::now(), period);
        tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let me = Arc::clone(self);
        tokio::spawn(async move {
            loop {
                tick.tick().await;
                me.retry_due();
            }
        })
    }

    fn retry_due(self: &Arc<Self>) {
        if self.transport.is_none() {
            return;
        }
        let mut st = self.lock();
        if !st.synced {
            return;
        }
        let live = st.tokens.tokens();
        for pending in st.queue.take_due((self.clock)()) {
            let tokens: Vec<String> = match &pending.tokens {
                None => live.clone(),
                Some(wanted) => wanted
                    .iter()
                    .filter(|t| live.contains(t))
                    .cloned()
                    .collect(),
            };
            if tokens.is_empty() {
                if pending.tokens.is_none() {
                    tracing::info!(id = %pending.id, "no registered tokens; alert kept in history only");
                }
                continue;
            }
            self.enqueue(Job {
                pending,
                tokens,
                retry: true,
            });
        }
    }

    /// Hands a send to the sender task, spawning it on first use. Called under the state lock,
    /// so the FIFO order is the order in which the history saw the alerts.
    fn enqueue(self: &Arc<Self>, job: Job) {
        let rx = self
            .jobs_rx
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(rx) = rx {
            tokio::spawn(run_sender(Arc::downgrade(self), rx));
        }
        // The receiver lives in the sender task until this service is dropped; an error means
        // that task panicked.
        if self.jobs.send(job).is_err() {
            tracing::error!("push sender task is gone; alert not sent");
        }
    }

    /// Applies one send's outcome: dead tokens are cleared on the broker and in the map,
    /// transient ones re-queued alone, message errors logged and dropped.
    fn apply_result(&self, job: Job, result: SendResult) {
        let Job {
            mut pending, retry, ..
        } = job;
        let now = (self.clock)();
        let mut st = self.lock();
        for token in &result.invalid {
            for topic in st.tokens.topics_for_token(token) {
                self.publish(Publish {
                    topic: topic.clone(),
                    payload: String::new(),
                    retain: true,
                });
                st.tokens.remove(&topic);
                tracing::warn!(%topic, "cleared invalid token");
            }
        }
        if result.message_errors > 0 {
            tracing::error!(id = %pending.id, error = %result.last_message_error, "FCM rejected alert");
        }
        if !result.transient.is_empty() {
            pending.tokens = Some(result.transient);
            if retry {
                st.queue.put_back(pending, now);
            } else {
                st.queue.add(pending);
            }
        }
    }

    fn publish_history(&self, st: &State) {
        self.publish(Publish {
            topic: self.history_topic(),
            payload: st.history.to_json(),
            retain: true,
        });
    }

    /// Non-blocking hand-off to the MQTT task; called under the state lock so publishes keep
    /// their order. Only the topic is logged (a payload may be large).
    fn publish(&self, publish: Publish) {
        if let Err(error) = self.publisher.try_send(publish) {
            let topic = match &error {
                mpsc::error::TrySendError::Full(p) | mpsc::error::TrySendError::Closed(p) => {
                    p.topic.clone()
                }
            };
            tracing::warn!(%topic, %error, "push publish dropped");
        }
    }
}

/// The single sender task: one send at a time, in FIFO order, off the state lock. It holds
/// the service weakly between jobs, so dropping the service closes the channel and ends it.
async fn run_sender(service: Weak<PushService>, mut jobs: mpsc::UnboundedReceiver<Job>) {
    while let Some(job) = jobs.recv().await {
        let Some(me) = service.upgrade() else {
            break;
        };
        let Some(transport) = me.transport.clone() else {
            continue;
        };
        let p = &job.pending;
        let result = send_data(&*transport, &p.data, &job.tokens, p.priority, p.ttl).await;
        me.apply_result(job, result);
    }
}
