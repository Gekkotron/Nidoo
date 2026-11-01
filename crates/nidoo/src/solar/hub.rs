//! The solar side of the hub: the stored forecast, its retained entities and cache, and the
//! clearing of a device that was switched off.

use std::collections::BTreeMap;
use std::sync::MutexGuard;

use chrono::{DateTime, Utc};
use nidoo_api::{SolarForecastDay, SolarView, WsEvent};
use nidoo_core::command::Publish;
use nidoo_solcast::{Forecast, Ledger};
use serde::{Deserialize, Serialize};

use super::{entities, is_expired, is_stale, view::build_view};
use crate::clock;
use crate::history::solar::SolarForecastRow;
use crate::hub::{Hub, HubError};

/// The payload that makes a sensor's state unknown (the registry reads `None` as unknown; an
/// empty payload would leave the old value in place).
const UNKNOWN: &str = "None";

/// What the hub knows of the solar forecast. `published` only records payloads the broker link
/// took, so a failed publish is retried by the next [`Hub::apply_solar_forecast`].
#[derive(Default)]
pub(crate) struct SolarSlot {
    forecast: Option<Forecast>,
    ledger: Ledger,
    /// Unix time the first failure of the current streak was seen.
    failing_since: Option<u64>,
    /// Unix time of the next planned refresh, set by the poller.
    next_refresh_at: Option<u64>,
    published: BTreeMap<&'static str, String>,
    /// The availability last published; `None` until the first success.
    online: Option<bool>,
    /// Whether the last `publish_solar_configs` got every config and the availability out.
    configs_ok: bool,
}

/// What the retained cache holds: the last forecast and the request ledger, so a restart needs
/// no call. Neither holds the API key or a site id. `forecast` is absent until a first fetch
/// succeeded: the ledger alone still stops a restart loop from spending the day's budget.
#[derive(Serialize, Deserialize)]
struct SolarCache {
    #[serde(default)]
    forecast: Option<Forecast>,
    ledger: Ledger,
}

fn now_utc() -> DateTime<Utc> {
    Utc::now()
}

impl Hub {
    fn solar_slot(&self) -> MutexGuard<'_, SolarSlot> {
        self.solar
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Whether `[solcast] enabled` is on.
    pub fn solar_enabled(&self) -> bool {
        self.solar_enabled
    }

    /// The configured rooftop resource ids.
    pub fn solar_sites(&self) -> Vec<String> {
        self.solar_sites.clone()
    }

    /// This hub's share of the account's daily requests.
    pub fn solar_daily_limit(&self) -> u32 {
        self.solar_daily_limit
    }

    /// The API key file, when `[solcast] api_key_file` sets one.
    pub fn solar_key_file(&self) -> Option<std::path::PathBuf> {
        self.solar_key_file.clone()
    }

    /// The API key itself, when set from the browser.
    pub fn solar_api_key(&self) -> Option<String> {
        self.solar_api_key.clone()
    }

    /// A copy of the request ledger.
    pub fn solar_ledger(&self) -> Ledger {
        self.solar_slot().ledger.clone()
    }

    /// Whether a forecast is held (fetched or restored from the cache).
    pub fn solar_has_forecast(&self) -> bool {
        self.solar_slot().forecast.is_some()
    }

    /// A copy of the held forecast, if any.
    pub fn solar_forecast(&self) -> Option<Forecast> {
        self.solar_slot().forecast.clone()
    }

    /// The view of the held forecast now, with `stale` computed now.
    pub fn solar_view(&self) -> Result<SolarView, HubError> {
        let unavailable = || HubError::Publish("solar unavailable".to_owned());
        if !self.solar_enabled {
            return Err(unavailable());
        }
        let slot = self.solar_slot();
        let forecast = slot.forecast.as_ref().ok_or_else(unavailable)?;
        let now = now_utc();
        let stale = is_stale(
            u64::try_from(forecast.fetched_at).unwrap_or(0),
            slot.failing_since,
            clock::now(),
        );
        Ok(build_view(
            forecast,
            &slot.ledger,
            self.zone(),
            now,
            slot.next_refresh_at,
            self.solar_daily_limit,
            stale,
        ))
    }

    /// [`Self::solar_view`] with the morning forecasts of the 7 local days before today. Empty
    /// when history is off or the database cannot be read.
    pub async fn solar_view_with_history(&self) -> Result<SolarView, HubError> {
        let mut view = self.solar_view()?;
        let Some(history) = self.history() else {
            return Ok(view);
        };
        let today = now_utc().with_timezone(&self.zone()).date_naive();
        let from = today - chrono::Duration::days(7);
        let rows = match history.db() {
            Ok(db) => db.solar_forecast_days(from, today).await,
            Err(e) => {
                tracing::debug!("solar forecast history unavailable: {e}");
                return Ok(view);
            }
        };
        match rows {
            Ok(rows) => {
                view.history.forecasts = rows
                    .into_iter()
                    .map(|r| SolarForecastDay {
                        date: r.date.format("%Y-%m-%d").to_string(),
                        estimate: r.estimate,
                        low: r.low,
                        high: r.high,
                        made_at: u64::try_from(r.made_at.timestamp()).unwrap_or(0),
                    })
                    .collect();
            }
            Err(e) => tracing::debug!("solar forecast history read failed: {e}"),
        }
        Ok(view)
    }

    /// Keeps today's totals as the day's morning forecast; the first one of a local day wins in
    /// the database. Skipped without history or outside a Tokio runtime.
    fn record_solar_morning_forecast(&self, forecast: &Forecast) {
        let Some(history) = self.history().cloned() else {
            return;
        };
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let now = now_utc();
        let zone = self.zone();
        let today = now.with_timezone(&zone).date_naive();
        let Some(total) = nidoo_solcast::day_totals(&forecast.periods, zone)
            .into_iter()
            .find(|d| d.date == today)
        else {
            return;
        };
        let row = SolarForecastRow {
            date: today,
            estimate: total.estimate,
            low: total.low,
            high: total.high,
            made_at: now,
        };
        runtime.spawn(async move {
            let result = match history.db() {
                Ok(db) => db.insert_solar_forecast_day(&row).await.map(|_| ()),
                Err(_) => return,
            };
            if let Err(e) = result {
                tracing::warn!("storing the morning solar forecast failed: {e}");
            }
        });
    }

    /// Publishes `payload` retained and echoes it into the registry. Whether the broker took it.
    fn publish_solar(&self, topic: String, payload: String) -> bool {
        let published = self
            .publish(Publish {
                topic: topic.clone(),
                payload: payload.clone(),
                retain: true,
            })
            .is_ok();
        if published {
            self.ingest(&topic, payload.as_bytes());
        }
        published
    }

    fn publish_solar_availability(&self, online: bool) -> bool {
        let topic = entities::availability_topic(&self.discovery_prefix);
        self.publish_solar(topic, if online { "online" } else { "offline" }.to_owned())
    }

    /// Publishes the retained cache (forecast and ledger). A refused publish is retried by the
    /// next ledger or forecast change.
    fn publish_solar_cache(&self) {
        let cache = {
            let slot = self.solar_slot();
            SolarCache {
                forecast: slot.forecast.clone(),
                ledger: slot.ledger.clone(),
            }
        };
        let payload = match serde_json::to_string(&cache) {
            Ok(payload) => payload,
            Err(error) => {
                tracing::warn!(%error, "solar: could not serialise the retained cache");
                return;
            }
        };
        let _ = self.publish(Publish {
            topic: entities::cache_topic(&self.discovery_prefix),
            payload,
            retain: true,
        });
    }

    /// Republishes the configs when they never went out or something removed them.
    fn heal_solar_configs(&self) {
        // An empty retained config is deliberately not reacted to: two hubs would ping-pong.
        if !self.solar_slot().configs_ok || self.entity_view("solar_forecast_today").is_none() {
            self.publish_solar_configs();
        }
    }

    /// Publishes the states of `view` that differ from what went out, and unknown for those that
    /// vanished. The keys whose payload was sent.
    fn publish_solar_states(&self, view: &SolarView) -> Vec<&'static str> {
        let changed: Vec<(&'static str, String)> = {
            let slot = self.solar_slot();
            let states = entities::states(view);
            let mut changed: Vec<(&'static str, String)> = states
                .iter()
                .filter(|(key, payload)| slot.published.get(key) != Some(payload))
                .cloned()
                .collect();
            // A value that went missing is published as unknown, never left frozen.
            changed.extend(
                slot.published
                    .keys()
                    .filter(|key| !states.iter().any(|(k, _)| k == *key))
                    .map(|key| (*key, UNKNOWN.to_owned())),
            );
            changed
        };
        let mut sent = Vec::new();
        for (key, payload) in changed {
            let topic = entities::state_topic(&self.discovery_prefix, key);
            if self.publish_solar(topic, payload.clone()) {
                let mut slot = self.solar_slot();
                if payload == UNKNOWN {
                    slot.published.remove(key);
                } else {
                    slot.published.insert(key, payload);
                }
                sent.push(key);
            }
        }
        sent
    }

    /// The view of the held forecast at `now`, without the failing-streak staleness (states
    /// carry no `stale`).
    fn solar_state_view(&self, now: DateTime<Utc>) -> Option<SolarView> {
        let slot = self.solar_slot();
        let forecast = slot.forecast.as_ref()?;
        Some(build_view(
            forecast,
            &slot.ledger,
            self.zone(),
            now,
            slot.next_refresh_at,
            self.solar_daily_limit,
            false,
        ))
    }

    /// Stores a fresh forecast, publishes the entities that changed and the cache, and tells the
    /// web when the data did. A payload the broker refused is sent again by the next call.
    pub fn apply_solar_forecast(&self, forecast: Forecast) {
        self.heal_solar_configs();
        self.record_solar_morning_forecast(&forecast);
        let data_changed = {
            let mut slot = self.solar_slot();
            let changed = slot.forecast.as_ref().is_none_or(|old| {
                let mut old = old.clone();
                old.fetched_at = forecast.fetched_at;
                old != forecast
            });
            slot.forecast = Some(forecast);
            slot.failing_since = None;
            changed
        };
        let online = self.solar_slot().online;
        if online != Some(true) && self.publish_solar_availability(true) {
            self.solar_slot().online = Some(true);
        }
        if let Some(view) = self.solar_state_view(now_utc()) {
            self.publish_solar_states(&view);
        }
        self.publish_solar_cache();
        if data_changed {
            self.broadcast(WsEvent::SolarChanged);
        }
    }

    /// Recomputes the entities at now: goes offline once the forecast expired, republishes what
    /// changed, and tells the web when the power now or the energy remaining did.
    pub fn refresh_solar_states(&self) {
        let expired = {
            let slot = self.solar_slot();
            match slot.forecast.as_ref() {
                None => return,
                Some(forecast) => is_expired(forecast, clock::now()),
            }
        };
        if expired {
            self.set_solar_offline();
            return;
        }
        self.heal_solar_configs();
        let online = self.solar_slot().online;
        if online != Some(true) && self.publish_solar_availability(true) {
            self.solar_slot().online = Some(true);
        }
        let Some(view) = self.solar_state_view(now_utc()) else {
            return;
        };
        let sent = self.publish_solar_states(&view);
        if sent
            .iter()
            .any(|key| matches!(*key, "forecast_power_now" | "forecast_remaining_today"))
        {
            self.broadcast(WsEvent::SolarChanged);
        }
    }

    /// Counts one attempted request now, and republishes the cache.
    pub fn record_solar_attempt(&self) {
        self.solar_slot().ledger.record_attempt(now_utc());
        self.publish_solar_cache();
    }

    /// Marks the day's budget as spent (HTTP 429), and republishes the cache.
    pub fn record_solar_quota_exceeded(&self) {
        self.solar_slot()
            .ledger
            .record_quota_exceeded(now_utc(), self.solar_daily_limit);
        self.publish_solar_cache();
    }

    /// Notes a failed planned refresh: the streak starts now unless one is running.
    pub fn record_solar_failure(&self) {
        let mut slot = self.solar_slot();
        if slot.failing_since.is_none() {
            slot.failing_since = Some(clock::now());
        }
    }

    /// Sets the unix time of the next planned refresh (`None` when none is planned).
    pub fn set_solar_next_refresh(&self, at: Option<u64>) {
        self.solar_slot().next_refresh_at = at;
    }

    /// Marks the entities unavailable, once per transition.
    pub fn set_solar_offline(&self) {
        let online = self.solar_slot().online;
        if online != Some(false) && self.publish_solar_availability(false) {
            self.solar_slot().online = Some(false);
        }
    }

    /// Publishes every entity's retained config, and `offline` until a forecast lands.
    pub fn publish_solar_configs(&self) {
        // Every key is seeded with a payload no state can equal, so the first apply after a
        // (re)start republishes each present value and sends `None` for each absent one: what a
        // persistent broker kept from the previous run never stays frozen.
        self.solar_slot().published = entities::KEYS
            .iter()
            .map(|key| (*key, String::new()))
            .collect();
        let mut failed = 0;
        for (topic, config) in entities::configs(&self.discovery_prefix) {
            if !self.publish_solar(topic, config.to_string()) {
                failed += 1;
            }
        }
        let availability_ok = self.publish_solar_availability(false);
        if availability_ok {
            self.solar_slot().online = Some(false);
        }
        if failed > 0 || !availability_ok {
            tracing::warn!(
                failed,
                availability_ok,
                "solar: some entity configs were not published; retrying on the next update"
            );
        }
        self.solar_slot().configs_ok = failed == 0 && availability_ok;
    }

    /// Loads the retained cache on the cache topic while `[solcast]` is on and no forecast is
    /// held yet; its ledger is always merged in (never lowering recorded calls). Whether `topic` is the cache topic.
    pub(crate) fn ingest_solar_cache(&self, topic: &str, payload: &[u8]) -> bool {
        if topic != entities::cache_topic(&self.discovery_prefix) {
            return false;
        }
        if !self.solar_enabled || payload.is_empty() {
            return true;
        }
        let cache: SolarCache = match serde_json::from_slice(payload) {
            Ok(cache) => cache,
            Err(error) => {
                tracing::warn!(%error, "solar: ignoring a malformed retained cache");
                return true;
            }
        };
        let mut slot = self.solar_slot();
        // The ledger is merged so a late or stale cache never lowers a call this run recorded;
        // the forecast is taken only while none is held.
        slot.ledger = slot.ledger.merge(&cache.ledger);
        if slot.forecast.is_none() {
            slot.forecast = cache.forecast;
        }
        true
    }

    /// While `[solcast]` is off, clears what an earlier run left retained: everything under
    /// `P/solar/` (states, availability and cache) and the configs of the solar device. Whether
    /// `topic` was one of them. A publish error is ignored: the next connect replays the
    /// retained message.
    pub(crate) fn clear_disabled_solar(&self, topic: &str, payload: &[u8], retain: bool) -> bool {
        if self.solar_enabled || !retain || payload.is_empty() {
            return false;
        }
        let prefix = self.discovery_prefix.as_str();
        let is_state = topic
            .strip_prefix(prefix)
            .and_then(|rest| rest.strip_prefix("/solar/"))
            .is_some_and(|key| !key.is_empty());
        let is_own_config = || {
            let segments: Vec<&str> = topic
                .strip_prefix(prefix)
                .and_then(|rest| rest.strip_prefix('/'))
                .map(|rest| rest.split('/').collect())
                .unwrap_or_default();
            matches!(segments[..], [_, "solar", _, "config"]) && entities::is_own_config(payload)
        };
        if !(is_state || is_own_config()) {
            return false;
        }
        let _ = self.publish(Publish {
            topic: topic.to_owned(),
            payload: String::new(),
            retain: true,
        });
        true
    }
}
