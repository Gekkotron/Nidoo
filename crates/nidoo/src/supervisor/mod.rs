//! Sensor supervisor, hub side: the hourly task, the live report clocks, the stored problems,
//! the settings and the Mute reply. The checks themselves are pure, in
//! `nidoo_watchdog::supervisor`.

pub mod store;
pub mod task;

use std::collections::HashMap;
use std::sync::{Arc, MutexGuard};

use nidoo_api::{ProblemView, SupervisorSensitivity, SupervisorSettings, SupervisorView};
use nidoo_watchdog::supervisor::{Baseline, Problem, Sensitivity, Supervisor};

use crate::history::db::Db;
use crate::hub::{Hub, HubError};
use crate::setup::store as settings_store;

/// Every supervisor alert id starts with this; replies to them come back to the supervisor.
pub const ALERT_PREFIX: &str = "supervisor-";

/// What the supervisor keeps between runs.
pub struct SupervisorRuntime {
    pub(crate) settings: SupervisorSettings,
    pub(crate) engine: Supervisor,
    /// When the hub last read a value for each entity.
    pub(crate) last_report: HashMap<String, u64>,
    /// Since when each entity has been seen unavailable (first run that saw it).
    pub(crate) unavailable_since: HashMap<String, u64>,
    /// When the report clocks started: the hub start or the broker link coming back.
    pub(crate) armed_at: u64,
    /// The broker link epoch `armed_at` belongs to.
    pub(crate) epoch: u64,
    pub(crate) baselines: HashMap<String, Baseline>,
    /// When the baselines were last computed.
    pub(crate) baselines_at: Option<u64>,
    /// Supervised sensors without a baseline at the last refresh.
    pub(crate) learning: u32,
    /// The open problems were loaded from the database.
    pub(crate) loaded: bool,
    /// When closed problems were last purged.
    pub(crate) purged_at: Option<u64>,
}

impl SupervisorRuntime {
    pub fn new(settings: SupervisorSettings, now: u64) -> Self {
        Self {
            settings,
            engine: Supervisor::default(),
            last_report: HashMap::new(),
            unavailable_since: HashMap::new(),
            armed_at: now,
            epoch: 0,
            baselines: HashMap::new(),
            baselines_at: None,
            learning: 0,
            loaded: false,
            purged_at: None,
        }
    }
}

/// The checks' sensitivity for the wire setting.
pub fn sensitivity(s: SupervisorSensitivity) -> Sensitivity {
    match s {
        SupervisorSensitivity::Low => Sensitivity::Low,
        SupervisorSensitivity::Normal => Sensitivity::Normal,
        SupervisorSensitivity::High => Sensitivity::High,
    }
}

fn problem_view(p: &Problem) -> ProblemView {
    ProblemView {
        entity_id: p.entity_id.clone(),
        name: p.name.clone(),
        check: p.check.as_str().to_owned(),
        opened_at: p.opened_at,
        evidence: serde_json::to_value(&p.evidence).unwrap_or_default(),
    }
}

impl Hub {
    pub(crate) fn supervisor_state(&self) -> MutexGuard<'_, SupervisorRuntime> {
        self.supervisor
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The history database, when one is configured.
    pub(crate) fn supervisor_db(&self) -> Option<Db> {
        self.history().and_then(|h| h.db().ok()).cloned()
    }

    /// The supervisor page: settings, open problems, sensors still learning.
    pub fn supervisor_view(&self) -> SupervisorView {
        let available = self.supervisor_db().is_some();
        let rt = self.supervisor_state();
        SupervisorView {
            available,
            settings: rt.settings.clone(),
            problems: rt.engine.open().map(problem_view).collect(),
            learning: rt.learning,
        }
    }

    /// Stores new settings (live, no restart), then applies them. Nothing changes when they
    /// cannot be stored.
    pub async fn set_supervisor(&self, s: SupervisorSettings) -> Result<SupervisorView, HubError> {
        let unavailable = |why: &str| HubError::Publish(format!("settings unavailable: {why}"));
        let ctx = self
            .setup
            .get()
            .ok_or_else(|| unavailable("no setup context"))?;
        let stored = s.clone();
        settings_store::save_live(ctx, "system", move |sections| {
            sections.system.supervisor = stored
        })
        .await
        .map_err(unavailable)?;
        let enabled = s.enabled;
        let dropped = self.apply_supervisor_settings(s);
        if let Some(db) = self.supervisor_db() {
            if !enabled {
                // Also the rows of a run that never loaded them.
                if let Err(error) = store::delete_open(&db).await {
                    tracing::warn!(%error, "could not drop the stored problems");
                }
            } else {
                forget_open(&db, &dropped).await;
            }
        }
        Ok(self.supervisor_view())
    }

    /// Takes new settings in memory. Turning the supervisor off drops every open problem, and
    /// an ignored sensor's problems are dropped; both without a push. Returns what was dropped.
    pub(crate) fn apply_supervisor_settings(&self, s: SupervisorSettings) -> Vec<Problem> {
        let mut rt = self.supervisor_state();
        let mut dropped = if s.enabled {
            s.ignored.iter().flat_map(|id| rt.engine.mute(id)).collect()
        } else {
            rt.engine.clear()
        };
        if !s.enabled {
            rt.unavailable_since.clear();
        }
        rt.settings = s;
        dropped.sort_by(|a, b| a.entity_id.cmp(&b.entity_id));
        dropped
    }

    /// A reply on `nidoo/push/responses` to one of the supervisor's alerts. `mute` adds the
    /// sensor to `ignored` and drops its problems without a push; storing that is best effort.
    pub(crate) fn on_supervisor_reply(self: &Arc<Self>, payload: &[u8]) {
        let Ok(reply) = serde_json::from_slice::<serde_json::Value>(payload) else {
            return;
        };
        let (Some(id), Some("mute")) = (
            reply.get("id").and_then(|v| v.as_str()),
            reply.get("action").and_then(|v| v.as_str()),
        ) else {
            return;
        };
        let (settings, dropped) = {
            let mut rt = self.supervisor_state();
            let Some(entity) = rt.engine.entity_for_alert(id).map(str::to_owned) else {
                return;
            };
            let dropped = rt.engine.mute(&entity);
            if !rt.settings.ignored.contains(&entity) {
                rt.settings.ignored.push(entity.clone());
            }
            tracing::info!(%entity, "sensor muted from a push");
            (rt.settings.clone(), dropped)
        };
        if tokio::runtime::Handle::try_current().is_err() {
            return;
        }
        let hub = Arc::clone(self);
        tokio::spawn(async move {
            if let Some(ctx) = hub.setup.get()
                && let Err(error) = settings_store::save_live(ctx, "system", move |sections| {
                    sections.system.supervisor = settings
                })
                .await
            {
                tracing::warn!(%error, "could not store the muted sensor");
            }
            if let Some(db) = hub.supervisor_db() {
                forget_open(&db, &dropped).await;
            }
        });
    }
}

/// Deletes the stored open problems of the entities in `dropped`.
async fn forget_open(db: &Db, dropped: &[Problem]) {
    let mut ids: Vec<&str> = dropped.iter().map(|p| p.entity_id.as_str()).collect();
    ids.dedup();
    for id in ids {
        if let Err(error) = store::delete(db, id).await {
            tracing::warn!(%error, entity = id, "could not drop the stored problem");
        }
    }
}
