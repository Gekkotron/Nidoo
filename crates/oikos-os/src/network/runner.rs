//! The ping supervisor: one loop per enabled watch, kept in step with the watch store.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use oikos_api::{ConnectionStatus, NetworkWatchSpec};
use oikos_core::command::Publish;
use tokio::sync::broadcast::error::RecvError;
use tokio::task::JoinHandle;
use tokio::time::{Instant, MissedTickBehavior, interval_at, timeout};

use super::probe::{Prober, default_prober};
use super::topics::{availability_topic, state_topic};
use super::tracker::{Probe, Tracker};
use crate::hub::Hub;

/// How long the retained sync must stay silent before the first loops start.
const SETTLE_QUIET: Duration = Duration::from_millis(1500);

/// The longest a ping waits for its reply.
const MAX_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// The running loops; dropping it stops them all.
#[derive(Default)]
struct Loops(BTreeMap<String, (NetworkWatchSpec, JoinHandle<()>)>);

impl Drop for Loops {
    fn drop(&mut self) {
        for (_, task) in self.0.values() {
            task.abort();
        }
    }
}

impl Hub {
    /// Starts the ping supervisor with the real prober; `None` while `[network] enabled` is off.
    pub fn start_network(self: &Arc<Self>) -> Option<JoinHandle<()>> {
        self.start_network_with(default_prober())
    }

    /// Starts the ping supervisor with `prober`; `None` while `[network] enabled` is off.
    pub fn start_network_with(self: &Arc<Self>, prober: Arc<dyn Prober>) -> Option<JoinHandle<()>> {
        if !self.network_enabled {
            return None;
        }
        let hub = Arc::clone(self);
        Some(tokio::spawn(
            async move { hub.supervise_network(prober).await },
        ))
    }

    async fn supervise_network(self: Arc<Self>, prober: Arc<dyn Prober>) {
        let mut rev = self.subscribe_network_rev();
        let mut connection = self.connection();
        while !matches!(*connection.borrow_and_update(), ConnectionStatus::Online) {
            if connection.changed().await.is_err() {
                return;
            }
        }
        let mut seen = self.subscribe_state_seen();
        loop {
            match timeout(SETTLE_QUIET, seen.recv()).await {
                Err(_) => break,
                Ok(Ok(_)) | Ok(Err(RecvError::Lagged(_))) => {}
                Ok(Err(RecvError::Closed)) => return,
            }
        }
        let warned = Arc::new(AtomicBool::new(false));
        let mut loops = Loops::default();
        loop {
            self.reconcile_network(&mut loops, &prober, &warned);
            if rev.changed().await.is_err() {
                return;
            }
        }
    }

    /// Stops the loops of watches that were deleted, paused or changed, and starts the loops
    /// of enabled watches that have none.
    fn reconcile_network(
        self: &Arc<Self>,
        loops: &mut Loops,
        prober: &Arc<dyn Prober>,
        warned: &Arc<AtomicBool>,
    ) {
        let wanted: BTreeMap<String, NetworkWatchSpec> = self
            .network_store()
            .all()
            .iter()
            .map(|(id, spec)| (id.clone(), spec.clone()))
            .collect();
        let stale: Vec<String> = loops
            .0
            .iter()
            .filter(|(id, (spec, _))| wanted.get(*id).is_none_or(|w| !w.enabled || w != spec))
            .map(|(id, _)| id.clone())
            .collect();
        for id in stale {
            if let Some((_, task)) = loops.0.remove(&id) {
                task.abort();
            }
        }
        let mut status = self
            .network_status
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        status.retain(|id, _| wanted.get(id).is_some_and(|w| w.enabled));
        drop(status);
        for (id, spec) in wanted {
            if !spec.enabled || loops.0.contains_key(&id) {
                continue;
            }
            let task = tokio::spawn(Arc::clone(self).watch_loop(
                id.clone(),
                spec.clone(),
                Arc::clone(prober),
                Arc::clone(warned),
            ));
            loops.0.insert(id, (spec, task));
        }
    }

    /// Pings one host forever: a first ping after a random delay within one interval, then one
    /// per interval, publishing what the tracker says changed.
    async fn watch_loop(
        self: Arc<Self>,
        id: String,
        spec: NetworkWatchSpec,
        prober: Arc<dyn Prober>,
        warned: Arc<AtomicBool>,
    ) {
        let every = Duration::from_secs(u64::from(spec.interval_s));
        let probe_timeout = every.min(MAX_PROBE_TIMEOUT);
        let mut ticks = interval_at(Instant::now() + jitter(every), every);
        ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let mut tracker = Tracker::new();
        let mut published_value: Option<bool> = None;
        let mut published_available: Option<bool> = None;
        let mut warned_publish = false;
        loop {
            ticks.tick().await;
            let probe = prober.probe(&spec.host, probe_timeout).await;
            if let Probe::CannotPing(reason) = &probe
                && !warned.swap(true, Ordering::Relaxed)
            {
                tracing::warn!(%reason, "network presence cannot ping");
            }
            tracker.on_probe(&probe, crate::clock::now());
            let prefix = &self.discovery_prefix;
            let mut failed = false;
            if let Some(value) = tracker.value()
                && published_value != Some(value)
            {
                let payload = if value { "ON" } else { "OFF" };
                if self.publish_retained(state_topic(prefix, &id), payload) {
                    published_value = Some(value);
                } else {
                    failed = true;
                }
            }
            if let Some(available) = tracker.available()
                && published_available != Some(available)
            {
                let payload = if available { "online" } else { "offline" };
                if self.publish_retained(availability_topic(prefix, &id), payload) {
                    published_available = Some(available);
                } else {
                    failed = true;
                }
            }
            if failed {
                if !warned_publish {
                    warned_publish = true;
                    tracing::warn!(%id, "network publish failed; retrying on the next ping");
                }
            } else {
                warned_publish = false;
            }
            self.set_network_status(&id, tracker.status());
        }
    }

    /// Publishes retained; false when the broker link refused it.
    fn publish_retained(&self, topic: String, payload: &str) -> bool {
        self.publish(Publish {
            topic,
            payload: payload.to_owned(),
            retain: true,
        })
        .is_ok()
    }
}

/// A random delay within `every`, so the watches do not all ping at the same instant.
fn jitter(every: Duration) -> Duration {
    let mut bytes = [0u8; 4];
    let _ = getrandom::getrandom(&mut bytes);
    every.mul_f64(f64::from(u32::from_le_bytes(bytes)) / f64::from(u32::MAX))
}
