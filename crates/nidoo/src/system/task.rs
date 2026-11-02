//! The system task: every minute it writes the heartbeat, samples the machine, asks the
//! database, follows the three services and publishes the system sensors. The page reads the
//! last tick from memory, so it never waits for the database.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use chrono::{DateTime, NaiveDate, SecondsFormat};
use nidoo_api::system::ids;
use nidoo_api::{
    ConnectionStatus, DiskView, RestartView, ServiceState, ServiceView, StopKind, SystemService,
    SystemView,
};
use nidoo_core::command::Publish;
use serde_json::{Value, json};
use tokio::task::JoinHandle;
use tokio::time::{MissedTickBehavior, timeout};

use super::CommandsToday;
use super::discovery::{availability_topic, config, config_topic, state_topic};
use super::heartbeat;
use super::restarts::{KEEP_SECS, WINDOW_SECS, uptime_pct};
use super::sample::{Resources, Sampler};
use super::services::{ServiceStatus, bridge_state, broker_state};
use crate::clock;
use crate::history::db::Db;
use crate::hub::Hub;
use crate::outage::{Outage, Service};

/// Seconds between two ticks.
pub const TICK_SECS: u64 = 60;

/// The longest one database call of a tick may take.
const DB_TIMEOUT: Duration = Duration::from_secs(10);

/// The longest the daily retention purge may take; it runs detached from the tick.
const PURGE_TIMEOUT: Duration = Duration::from_secs(300);

/// How long the restart push stays relevant, in seconds.
const RESTART_TTL: u64 = 86_400;

/// One service: its state for the page and its outage tracker for the pushes.
#[derive(Debug)]
struct Watched {
    status: ServiceStatus,
    outage: Outage,
}

impl Watched {
    fn new(service: Service, notify: bool) -> Self {
        Self {
            status: ServiceStatus::default(),
            outage: Outage::new(service, notify),
        }
    }

    /// Records `state` at `now` and returns the push it calls for. Unknown feeds neither a
    /// failure nor a success.
    fn observe(&mut self, state: ServiceState, now: u64, reason: &str) -> Vec<Value> {
        self.status.observe(state, now);
        let mut alerts = Vec::new();
        match state {
            ServiceState::Up => alerts.extend(self.outage.succeeded(now)),
            ServiceState::Down => self.outage.failed(now, reason),
            ServiceState::Unknown => {}
        }
        alerts.extend(self.outage.tick(now));
        alerts
    }

    /// The page's view: a state that cannot be measured has no "since".
    fn view(&self, service: SystemService) -> ServiceView {
        let mut view = self.status.view(service);
        if view.state == ServiceState::Unknown {
            view.since = None;
        }
        view
    }
}

/// What the broker last took for one sensor; reset when the broker link comes back, except
/// `measured`.
#[derive(Debug, Default)]
struct Published {
    /// Measured at least once by this run; until then the config is not published.
    measured: bool,
    config: bool,
    state: Option<String>,
    available: Option<bool>,
}

/// The system figures between two ticks, and what was published of them.
pub struct SystemState {
    /// Built at the first tick, from the setup context's data directory.
    sampler: Option<Arc<Mutex<Sampler>>>,
    commands: CommandsToday,
    broker: Watched,
    zigbee2mqtt: Watched,
    database: Watched,
    resources: Option<Resources>,
    db_bytes: Option<u64>,
    started_at: u64,
    /// This start, until a read of the restart log returns it.
    pending: Option<RestartView>,
    /// Whether the restart log took `pending`; the insert is not repeated.
    pending_logged: bool,
    /// The restart log of the last 30 days, newest first, as last read.
    restarts: Vec<RestartView>,
    first_start: Option<u64>,
    uptime_pct: f64,
    /// What was published of each sensor.
    published: BTreeMap<&'static str, Published>,
    /// The broker link epoch the published memory belongs to.
    epoch: Option<u64>,
    /// The local day of the last retention purge attempt.
    purged_on: Option<NaiveDate>,
}

impl SystemState {
    pub fn new(notify: bool, started_at: u64) -> Self {
        Self {
            sampler: None,
            commands: CommandsToday::default(),
            broker: Watched::new(Service::Broker, notify),
            zigbee2mqtt: Watched::new(Service::Zigbee2mqtt, notify),
            database: Watched::new(Service::Database, notify),
            resources: None,
            db_bytes: None,
            started_at,
            pending: None,
            pending_logged: false,
            restarts: Vec::new(),
            first_start: None,
            uptime_pct: 100.0,
            published: BTreeMap::new(),
            epoch: None,
            purged_on: None,
        }
    }

    /// Whether the retention purge is due on local day `today`; marks the day when it is.
    fn purge_due(&mut self, today: NaiveDate) -> bool {
        if self.purged_on == Some(today) {
            return false;
        }
        self.purged_on = Some(today);
        true
    }

    /// The logged starts plus this one while the log has not returned it, newest first.
    fn all_restarts(&self) -> Vec<RestartView> {
        let mut all = self.restarts.clone();
        if let Some(pending) = &self.pending
            && !all.iter().any(|r| r.started_at == pending.started_at)
        {
            all.insert(0, pending.clone());
        }
        all
    }
}

/// What one tick learnt from the database; `database: None` when none is configured.
#[derive(Default)]
struct DbRound {
    database: Option<Result<u64, String>>,
    inserted: Option<u64>,
    restarts: Option<(Vec<RestartView>, Option<u64>)>,
}

/// What the rest of the hub says at tick time, read before the system lock is taken.
struct Readings {
    entities: usize,
    devices: usize,
    broker: ServiceState,
    zigbee2mqtt: ServiceState,
    retained: Option<u64>,
    commands_total: u64,
    notify: bool,
    today: NaiveDate,
    epoch: u64,
}

impl Hub {
    fn system_state(&self) -> MutexGuard<'_, SystemState> {
        self.system
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Starts the system task: classifies the previous stop, pushes when it was unexpected
    /// (and "notify me" is on), then ticks every minute. `None` without a setup context.
    pub fn start_system(self: &Arc<Self>) -> Option<JoinHandle<()>> {
        let dir = self.setup.get()?.data_dir.clone();
        self.system_boot(&dir, clock::now());
        let hub = Arc::clone(self);
        Some(tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(TICK_SECS));
            interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
            loop {
                interval.tick().await;
                hub.tick_at(clock::now()).await;
            }
        }))
    }

    /// Reads what the previous run left in `dir` before this run writes its first heartbeat.
    fn system_boot(&self, dir: &Path, boot: u64) {
        let (stop, down_secs) = heartbeat::classify(&heartbeat::read(dir), boot);
        {
            let mut state = self.system_state();
            state.started_at = boot;
            state.pending = Some(RestartView {
                started_at: boot,
                down_secs,
                stop,
            });
        }
        tracing::info!(?stop, ?down_secs, "hub started");
        if stop == StopKind::Unexpected && self.system_notify() {
            self.raise_system(Some(restart_alert(boot, down_secs)));
        }
    }

    /// One tick at `now`: the heartbeat, a machine sample (off the async threads), then
    /// [`Hub::tick_with`].
    pub async fn tick_at(&self, now: u64) {
        let dir = self.setup.get().map(|ctx| ctx.data_dir.clone());
        let sampler = self.system_sampler(dir.as_deref());
        let sampled = tokio::task::spawn_blocking(move || {
            if let Some(dir) = &dir
                && let Err(error) = heartbeat::write_heartbeat(dir, now)
            {
                tracing::warn!(%error, "could not write the heartbeat");
            }
            sampler
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .sample()
        })
        .await;
        let resources = sampled.unwrap_or_else(|error| {
            tracing::warn!(%error, "the machine sample failed");
            Resources {
                cpu: None,
                ram: None,
                disk: None,
                temperature_c: None,
            }
        });
        self.tick_with(now, resources).await;
    }

    /// The rest of a tick with the machine figures given: the database round, the services
    /// and their outage pushes, then the sensors.
    pub async fn tick_with(&self, now: u64, resources: Resources) {
        let today = local_day(self.zone(), now);
        let db = self.system_db_round(now).await;
        let (entities, devices) = self.counts();
        let online = matches!(*self.connection().borrow(), ConnectionStatus::Online);
        let bridge = self.read().bridge().state;
        let epoch = self.link_epoch();
        let readings = Readings {
            entities,
            devices,
            broker: broker_state(online, epoch),
            zigbee2mqtt: bridge_state(bridge),
            retained: self.retained_count(),
            commands_total: self.commands_sent(),
            notify: self.system_notify(),
            today,
            epoch,
        };
        let (alerts, configs) = {
            let mut state = self.system_state();
            let alerts = state.absorb(now, resources, db, &readings);
            let values = state.values(&readings);
            let configs = self.publish_system(&mut state, readings.epoch, values);
            (alerts, configs)
        };
        for alert in alerts {
            self.raise_system(Some(alert));
        }
        for (topic, payload) in configs {
            self.ingest(&topic, payload.as_bytes());
        }
        self.start_purge(now, today);
    }

    /// The system page, from the last tick.
    pub fn system_view(&self) -> SystemView {
        let (entities, devices) = self.counts();
        let retained = self.retained_count();
        let commands_total = self.commands_sent();
        let today = local_day(self.zone(), clock::now());
        let notify = self.system_notify();
        let mut state = self.system_state();
        let resources = state.resources.clone();
        SystemView {
            started_at: state.started_at,
            uptime_pct: state.uptime_pct,
            restarts: state.all_restarts(),
            cpu: resources.as_ref().and_then(|r| r.cpu.clone()),
            ram: resources.as_ref().and_then(|r| r.ram.clone()),
            disk: resources
                .as_ref()
                .and_then(|r| r.disk)
                .map(|(used_bytes, total_bytes)| DiskView {
                    used_bytes,
                    total_bytes,
                    db_bytes: state.db_bytes,
                }),
            temperature_c: resources.as_ref().and_then(|r| r.temperature_c),
            devices: u32::try_from(devices).unwrap_or(u32::MAX),
            entities: u32::try_from(entities).unwrap_or(u32::MAX),
            retained,
            commands_today: state.commands.count(commands_total, today),
            services: vec![
                state.broker.view(SystemService::Broker),
                state.zigbee2mqtt.view(SystemService::Zigbee2mqtt),
                state.database.view(SystemService::Database),
            ],
            notify,
        }
    }

    /// The sampler, built once from the data directory (canonicalised, so the disk holding
    /// it is found through a symlink).
    fn system_sampler(&self, dir: Option<&Path>) -> Arc<Mutex<Sampler>> {
        if let Some(sampler) = &self.system_state().sampler {
            return Arc::clone(sampler);
        }
        let raw = dir.map_or_else(|| PathBuf::from("."), Path::to_path_buf);
        let dir = std::fs::canonicalize(&raw).unwrap_or(raw);
        let built = Arc::new(Mutex::new(Sampler::new(dir)));
        Arc::clone(self.system_state().sampler.get_or_insert(built))
    }

    /// The database part of a tick. Without a database, or when its size query fails, the
    /// other calls are skipped.
    async fn system_db_round(&self, now: u64) -> DbRound {
        let Some(db) = self.history().and_then(|h| h.db().ok()) else {
            return DbRound::default();
        };
        let size = match timeout(DB_TIMEOUT, database_size(db)).await {
            Ok(Ok(size)) => size,
            Ok(Err(error)) => return down(error.to_string()),
            Err(_) => return down("no answer within 10 s".to_owned()),
        };
        let mut round = DbRound {
            database: Some(Ok(size)),
            ..DbRound::default()
        };
        let pending = {
            let state = self.system_state();
            state.pending.clone().filter(|_| !state.pending_logged)
        };
        if let Some(start) = pending {
            match timeout(DB_TIMEOUT, db.insert_restart(&start)).await {
                Ok(Ok(())) => round.inserted = Some(start.started_at),
                Ok(Err(error)) => tracing::warn!(%error, "could not log the hub start"),
                Err(_) => tracing::warn!("logging the hub start timed out"),
            }
        }
        let since = now.saturating_sub(WINDOW_SECS);
        let read = async {
            Ok::<_, sqlx::Error>((db.restarts_since(since).await?, db.first_start().await?))
        };
        match timeout(DB_TIMEOUT, read).await {
            Ok(Ok(restarts)) => round.restarts = Some(restarts),
            Ok(Err(error)) => tracing::warn!(%error, "could not read the restart log"),
            Err(_) => tracing::warn!("reading the restart log timed out"),
        }
        round
    }

    /// Once per local day, starts deleting the system history and restart log past the 90
    /// days, detached so the tick never waits on it. The day is marked before the attempt: a
    /// failure is logged once and waits for the next local day. `None` when nothing started.
    fn start_purge(&self, now: u64, today: NaiveDate) -> Option<JoinHandle<()>> {
        let db = self.history().and_then(|h| h.db().ok())?.clone();
        if !self.system_state().purge_due(today) {
            return None;
        }
        let cutoff = now.saturating_sub(KEEP_SECS);
        Some(tokio::spawn(async move {
            match timeout(PURGE_TIMEOUT, db.purge_system_before(cutoff)).await {
                Ok(Ok(removed)) => tracing::debug!(removed, "purged the old system history"),
                Ok(Err(error)) => {
                    tracing::warn!(%error, "could not purge the old system history; next try tomorrow");
                }
                Err(_) => {
                    tracing::warn!("purging the old system history timed out; next try tomorrow")
                }
            }
        }))
    }

    /// Publishes what changed; returns the configs published, for the local registry. A
    /// new broker link epoch forgets what was published, so everything goes out again. A
    /// figure that cannot be measured now is `offline`, never a stale state.
    fn publish_system(
        &self,
        state: &mut SystemState,
        epoch: u64,
        values: Vec<(&'static str, Option<String>)>,
    ) -> Vec<(String, String)> {
        if state.epoch != Some(epoch) {
            for published in state.published.values_mut() {
                *published = Published {
                    measured: published.measured,
                    ..Published::default()
                };
            }
            state.epoch = Some(epoch);
        }
        let prefix = &self.discovery_prefix;
        let send = |topic: String, payload: String| {
            self.publish(Publish {
                topic,
                payload,
                retain: true,
            })
            .is_ok()
        };
        let mut configs = Vec::new();
        for (id, value) in values {
            let published = state.published.entry(id).or_default();
            published.measured |= value.is_some();
            // A sensor never measured gets no config, only `offline`: a config and state an
            // earlier run left retained must not read as current.
            if published.measured && !published.config {
                let topic = config_topic(prefix, id);
                let payload = config(prefix, id).to_string();
                if !send(topic.clone(), payload.clone()) {
                    continue;
                }
                published.config = true;
                configs.push((topic, payload));
            }
            match value {
                Some(value) => {
                    if published.state.as_ref() != Some(&value)
                        && send(state_topic(prefix, id), value.clone())
                    {
                        published.state = Some(value);
                    }
                    if published.available != Some(true)
                        && send(availability_topic(prefix, id), "online".to_owned())
                    {
                        published.available = Some(true);
                    }
                }
                None => {
                    if published.available != Some(false)
                        && send(availability_topic(prefix, id), "offline".to_owned())
                    {
                        published.available = Some(false);
                    }
                }
            }
        }
        configs
    }
}

impl SystemState {
    /// Takes in one tick's figures; returns the outage pushes due.
    fn absorb(
        &mut self,
        now: u64,
        resources: Resources,
        db: DbRound,
        readings: &Readings,
    ) -> Vec<Value> {
        self.resources = Some(resources);
        if let Some(started_at) = db.inserted
            && self
                .pending
                .as_ref()
                .is_some_and(|p| p.started_at == started_at)
        {
            self.pending_logged = true;
        }
        if let Some((restarts, first_start)) = db.restarts {
            self.restarts = restarts;
            self.first_start = first_start;
            if let Some(pending) = &self.pending
                && self
                    .restarts
                    .iter()
                    .any(|r| r.started_at == pending.started_at)
            {
                self.pending = None;
            }
        }
        self.uptime_pct = uptime_pct(&self.all_restarts(), self.first_start, now, WINDOW_SECS);

        let (database, reason) = match &db.database {
            None => (ServiceState::Unknown, String::new()),
            Some(Ok(_)) => (ServiceState::Up, String::new()),
            Some(Err(error)) => (ServiceState::Down, error.clone()),
        };
        self.db_bytes = db.database.and_then(Result::ok);

        for watched in [&mut self.broker, &mut self.zigbee2mqtt, &mut self.database] {
            watched.outage.set_notify(readings.notify);
        }
        let mut alerts = self
            .broker
            .observe(readings.broker, now, "the hub is not connected");
        alerts.extend(
            self.zigbee2mqtt
                .observe(readings.zigbee2mqtt, now, "the bridge is offline"),
        );
        alerts.extend(self.database.observe(database, now, &reason));
        alerts
    }

    /// Every sensor's state payload, `None` when it cannot be measured now.
    fn values(&mut self, readings: &Readings) -> Vec<(&'static str, Option<String>)> {
        let resources = self.resources.as_ref();
        let cpu = resources.and_then(|r| r.cpu.as_ref());
        let ram = resources.and_then(|r| r.ram.as_ref());
        let disk = resources.and_then(|r| r.disk);
        let service = |watched: &Watched| match watched.status.state() {
            ServiceState::Up => Some("ON".to_owned()),
            ServiceState::Down => Some("OFF".to_owned()),
            ServiceState::Unknown => None,
        };
        let commands = self.commands.count(readings.commands_total, readings.today);
        vec![
            (ids::CPU, cpu.map(|c| format!("{:.1}", c.server_pct))),
            (ids::CPU_HUB, cpu.map(|c| format!("{:.1}", c.hub_pct))),
            (ids::RAM_USED, ram.map(|r| megabytes(r.used_bytes))),
            (ids::RAM_HUB, ram.map(|r| megabytes(r.hub_bytes))),
            (ids::DISK_USED, disk.map(|(used, _)| gigabytes(used))),
            (ids::DB_SIZE, self.db_bytes.map(gigabytes)),
            (
                ids::TEMPERATURE,
                resources
                    .and_then(|r| r.temperature_c)
                    .map(|t| format!("{t:.1}")),
            ),
            (ids::DEVICES, Some(readings.devices.to_string())),
            (ids::ENTITIES, Some(readings.entities.to_string())),
            (ids::RETAINED, readings.retained.map(|n| n.to_string())),
            (ids::COMMANDS, Some(commands.to_string())),
            (ids::STARTED, iso8601(self.started_at)),
            (ids::BROKER, service(&self.broker)),
            (ids::ZIGBEE2MQTT, service(&self.zigbee2mqtt)),
            (ids::DATABASE, service(&self.database)),
        ]
    }
}

fn down(reason: String) -> DbRound {
    DbRound {
        database: Some(Err(reason)),
        ..DbRound::default()
    }
}

async fn database_size(db: &Db) -> Result<u64, sqlx::Error> {
    let size: i64 = sqlx::query_scalar("SELECT pg_database_size(current_database())")
        .fetch_one(db.pool())
        .await?;
    Ok(u64::try_from(size).unwrap_or(0))
}

/// The local date of Unix time `now` in `zone`.
fn local_day(zone: chrono_tz::Tz, now: u64) -> NaiveDate {
    DateTime::from_timestamp(i64::try_from(now).unwrap_or(i64::MAX), 0)
        .unwrap_or_default()
        .with_timezone(&zone)
        .date_naive()
}

/// `2026-11-02T08:30:00Z`.
fn iso8601(secs: u64) -> Option<String> {
    DateTime::from_timestamp(i64::try_from(secs).ok()?, 0)
        .map(|t| t.to_rfc3339_opts(SecondsFormat::Secs, true))
}

fn megabytes(bytes: u64) -> String {
    format!("{:.0}", bytes as f64 / 1e6)
}

fn gigabytes(bytes: u64) -> String {
    format!("{:.2}", bytes as f64 / 1e9)
}

/// The push sent after an unexpected stop. The `loc` arguments are strings, as the push
/// service requires; an unknown downtime leaves `minutes` out.
fn restart_alert(boot: u64, down_secs: Option<u64>) -> Value {
    let minutes = down_secs.map(|secs| secs / 60);
    let down = minutes.map_or_else(|| "an unknown time".to_owned(), |m| format!("{m} min"));
    let args = match minutes {
        Some(m) => json!({"minutes": m.to_string()}),
        None => json!({}),
    };
    json!({
        "v": 1,
        "id": format!("hub-restart-{boot}"),
        "channel": "system",
        "severity": "warning",
        "title": "Hub restarted",
        "body": format!("The hub restarted after an unexpected stop (down {down})."),
        "silent": false,
        "ttl": RESTART_TTL,
        "loc": {"key": "hub.restarted_unexpectedly", "args": args},
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restart_alert_texts() {
        let alert = restart_alert(1_000, Some(725));
        assert_eq!(alert["id"], "hub-restart-1000");
        assert_eq!(
            alert["body"],
            "The hub restarted after an unexpected stop (down 12 min)."
        );
        assert_eq!(alert["loc"]["key"], "hub.restarted_unexpectedly");
        assert_eq!(alert["loc"]["args"]["minutes"], "12");
        let unknown = restart_alert(1_000, None);
        assert_eq!(
            unknown["body"],
            "The hub restarted after an unexpected stop (down an unknown time)."
        );
        assert!(unknown["loc"]["args"].as_object().unwrap().is_empty());
    }

    #[test]
    fn restart_alert_passes_the_push_checks() {
        let alert =
            nidoo_push::normalize::normalize("system", &restart_alert(1_000, Some(600)), 1_000)
                .expect("a valid alert");
        assert!(alert.loc.is_some(), "the loc hint was dropped");
    }

    fn readings(today: NaiveDate) -> Readings {
        Readings {
            entities: 0,
            devices: 0,
            broker: ServiceState::Up,
            zigbee2mqtt: ServiceState::Unknown,
            retained: None,
            commands_total: 0,
            notify: false,
            today,
            epoch: 0,
        }
    }

    fn no_resources() -> Resources {
        Resources {
            cpu: None,
            ram: None,
            disk: None,
            temperature_c: None,
        }
    }

    const T0: u64 = 1_800_000_000;

    #[test]
    fn start_stays_until_the_log_reads_it() {
        let today = local_day(chrono_tz::UTC, T0);
        let start = RestartView {
            started_at: T0,
            down_secs: Some(600),
            stop: StopKind::Unexpected,
        };
        let mut state = SystemState::new(false, T0);
        state.pending = Some(start.clone());

        // Logged, but the read of the log failed this tick: the start still shows.
        let logged = DbRound {
            database: Some(Ok(1)),
            inserted: Some(T0),
            restarts: None,
        };
        state.absorb(T0 + 60, no_resources(), logged, &readings(today));
        assert!(state.pending_logged);
        assert_eq!(state.all_restarts(), vec![start.clone()]);
        assert!(state.uptime_pct < 100.0);

        let read = DbRound {
            database: Some(Ok(1)),
            inserted: None,
            restarts: Some((vec![start.clone()], Some(T0))),
        };
        state.absorb(T0 + 120, no_resources(), read, &readings(today));
        assert_eq!(state.pending, None);
        assert_eq!(state.all_restarts(), vec![start]);
    }

    #[test]
    fn purge_is_due_once_per_local_day() {
        let day1 = NaiveDate::from_ymd_opt(2026, 11, 2).expect("date");
        let day2 = NaiveDate::from_ymd_opt(2026, 11, 3).expect("date");
        let mut state = SystemState::new(false, T0);
        assert!(state.purge_due(day1));
        assert!(!state.purge_due(day1));
        assert!(state.purge_due(day2));
        assert!(!state.purge_due(day2));
    }

    #[tokio::test]
    async fn failing_purge_waits_for_the_next_day() {
        let (tx, _rx) = tokio::sync::mpsc::channel(256);
        let hub = Hub::new(&crate::config::Config::default(), tx);
        let cfg = crate::config::HistoryConfig {
            url: "postgres://u:p@127.0.0.1:1/x".to_owned(),
            ..crate::config::HistoryConfig::default()
        };
        hub.set_history(crate::history::History::start(&cfg, hub.zone()).expect("a lazy pool"));
        let today = local_day(hub.zone(), T0);

        // The size query waits out the pool's 5 s acquire timeout; a purge awaited inline
        // would add as much again.
        let started = std::time::Instant::now();
        hub.tick_with(T0, no_resources()).await;
        assert!(
            started.elapsed() < Duration::from_secs(8),
            "{:?}",
            started.elapsed()
        );
        assert_eq!(hub.system_state().purged_on, Some(today));

        // The attempt (failing in the background) took the day.
        assert!(hub.start_purge(T0 + 60, today).is_none());
        let tomorrow = today.succ_opt().expect("a next day");
        let next = hub
            .start_purge(T0 + 86_400, tomorrow)
            .expect("tomorrow's purge");
        next.abort();
    }

    #[test]
    fn formats() {
        assert_eq!(iso8601(0).as_deref(), Some("1970-01-01T00:00:00Z"));
        assert_eq!(megabytes(2_000_400_000), "2000");
        assert_eq!(gigabytes(40_004_000_000), "40.00");
    }
}
