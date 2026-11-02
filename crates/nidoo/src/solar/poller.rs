//! The task that keeps the solar forecast fresh within the daily request budget: it asks
//! [`nidoo_solcast::decide`] when to call Solcast, fetches every site, and hands the merged
//! result to the hub.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Duration as ChronoDuration, NaiveDate, Utc};
use chrono_tz::Tz;
use nidoo_api::ConnectionStatus;
use nidoo_core::flow::sun::sun_events;
use nidoo_solcast::{
    ApiKey, BASE_URL, Client, Decision, Ledger, SolcastError, decide, local_midnight, merge,
    refreshes_per_day, slots,
};
use tokio::task::JoinHandle;

use crate::config::read_key_file;
use crate::hub::Hub;
use crate::outage::Service;

/// How long the poller waits after the subscription is acknowledged, so the broker can replay
/// the retained cache before the first decision. Without the cache a restart would look like
/// a first start.
pub const CACHE_GRACE: Duration = Duration::from_secs(10);

/// The longest the poller waits for the broker's SubAck before going on anyway.
pub const SUBACK_TIMEOUT: Duration = Duration::from_secs(60);

/// The shortest sleep between two decisions.
const MIN_SLEEP: Duration = Duration::from_secs(1);

/// Sleep used once the key was refused: nothing more is called until a restart.
const FAR_FUTURE: Duration = Duration::from_secs(24 * 3600);

/// Days of forecast kept after today.
const KEEP_DAYS: i64 = 8;

impl Hub {
    /// Starts the Solcast poller; `None` while `[solcast] enabled` is off, in which case
    /// nothing ever leaves the LAN.
    pub fn start_solar(self: &Arc<Self>) -> Option<JoinHandle<()>> {
        self.solar_enabled()
            .then(|| spawn_with(self, BASE_URL.to_owned(), CACHE_GRACE))
    }
}

/// Spawns [`run`] against `base_url` (tests point it at a stub), waiting for the SubAck, then `cache_grace`,
/// after the broker is online before the first decision.
pub fn spawn_with(hub: &Arc<Hub>, base_url: String, cache_grace: Duration) -> JoinHandle<()> {
    tokio::spawn(run(Arc::clone(hub), base_url, cache_grace))
}

/// How one round of calls ended.
enum Round {
    Fetched,
    Failed,
    Unauthorized,
}

/// The poller. Loops until the task is aborted; only a missing key file or a refused key
/// stops the calls.
pub async fn run(hub: Arc<Hub>, base_url: String, cache_grace: Duration) {
    let key = match hub.solar_key_file() {
        _ if hub.solar_api_key().is_some_and(|k| !k.trim().is_empty()) => {
            hub.solar_api_key().unwrap_or_default().trim().to_owned()
        }
        None => {
            tracing::error!("solar: no API key file is configured");
            return;
        }
        Some(path) => match read_key_file(&path) {
            Ok(key) if !key.is_empty() => key,
            Ok(_) => {
                tracing::error!(path = %path.display(), "solar: the API key file is empty");
                return;
            }
            Err(e) => {
                tracing::error!(path = %path.display(), error = %e, "solar: cannot read the API key file");
                return;
            }
        },
    };
    let client = match Client::new(base_url, ApiKey::new(key)) {
        Ok(client) => client,
        Err(e) => {
            tracing::error!(error = %e, "solar: cannot build the HTTP client");
            return;
        }
    };
    let mut connection = hub.connection();
    if connection
        .wait_for(|status| matches!(status, ConnectionStatus::Online))
        .await
        .is_err()
    {
        return;
    }
    // Once, from this task only, so it never interleaves with `apply_solar_forecast`.
    hub.publish_solar_configs();
    let mut subscribed = hub.solar_subscribed();
    match tokio::time::timeout(SUBACK_TIMEOUT, subscribed.wait_for(|done| *done)).await {
        Ok(Ok(_)) => {}
        Ok(Err(_)) => return,
        Err(_) => tracing::warn!(
            "solar: the broker did not acknowledge the subscription in time; going on without the cache guarantee"
        ),
    }
    tokio::time::sleep(cache_grace).await;

    let sites = hub.solar_sites();
    let limit = hub.solar_daily_limit();
    let site_count = u32::try_from(sites.len()).unwrap_or(u32::MAX);
    let mut unauthorized = false;
    let mut warned = false;
    loop {
        let now = Utc::now();
        let day_slots = planned_slots(&hub, now, limit, site_count);
        let decide_now = |unauthorized: bool, now: DateTime<Utc>| {
            next_decision(
                unauthorized,
                now,
                &hub.solar_ledger(),
                hub.solar_has_forecast(),
                &day_slots,
                limit,
                site_count,
            )
        };
        if matches!(decide_now(unauthorized, now), Decision::CallNow) {
            match fetch_round(&hub, &client, &sites, &mut warned).await {
                Round::Fetched => warned = false,
                Round::Failed => {}
                Round::Unauthorized => unauthorized = true,
            }
        }
        // Decide again: the round changed the ledger, and what is advertised as the next
        // refresh must be what the decision really waits for.
        let now = Utc::now();
        let wake = match decide_now(unauthorized, now) {
            Decision::SleepUntil(at) => Some(at),
            Decision::CallNow => None,
        };
        let advertised = if unauthorized { None } else { wake };
        hub.set_solar_next_refresh(advertised.and_then(|t| u64::try_from(t.timestamp()).ok()));
        hub.refresh_solar_states();
        tokio::time::sleep(sleep_for(now, wake)).await;
    }
}

/// The decision for this turn: [`decide`], except that a refused key (`unauthorized`) means no
/// call until a restart, whatever `decide` would say.
fn next_decision(
    unauthorized: bool,
    now: DateTime<Utc>,
    ledger: &Ledger,
    has_forecast: bool,
    slots: &[DateTime<Utc>],
    daily_limit: u32,
    sites: u32,
) -> Decision {
    if unauthorized {
        return Decision::SleepUntil(now + ChronoDuration::seconds(FAR_FUTURE.as_secs() as i64));
    }
    decide(now, ledger, has_forecast, slots, daily_limit, sites)
}

/// Refresh slots of local yesterday, today and tomorrow (a day's slots can straddle UTC
/// midnight).
fn planned_slots(hub: &Hub, now: DateTime<Utc>, limit: u32, sites: u32) -> Vec<DateTime<Utc>> {
    let zone = hub.zone();
    let (lat, lon) = hub.house_point();
    let refreshes = refreshes_per_day(limit, sites);
    let today = now.with_timezone(&zone).date_naive();
    let mut out = Vec::new();
    for offset in -1..=1 {
        let date: NaiveDate = today + ChronoDuration::days(offset);
        let (sunrise, sunset) = local_sun(lat, lon, date, zone);
        out.extend(slots(sunrise, sunset, date, zone, refreshes));
    }
    out
}

/// Sunrise and sunset that fall on the local `date` in `zone`. `sun_events` searches one UTC
/// day, so the local day's events can sit in the UTC day before or after: the three UTC days
/// around `date` are searched and the events are filtered on their local date.
fn local_sun(
    lat: f64,
    lon: f64,
    date: NaiveDate,
    zone: Tz,
) -> (Option<DateTime<Utc>>, Option<DateTime<Utc>>) {
    let on_date = |t: &DateTime<Utc>| t.with_timezone(&zone).date_naive() == date;
    let mut sunrise = None;
    let mut sunset = None;
    for offset in -1..=1 {
        let events = sun_events(lat, lon, date + ChronoDuration::days(offset));
        sunrise = sunrise.or(events.sunrise.filter(on_date));
        sunset = sunset.or(events.sunset.filter(on_date));
    }
    (sunrise, sunset)
}

/// Fetches every site in order. All must succeed for the forecast to be replaced: one failure
/// keeps the previous forecast whole.
async fn fetch_round(hub: &Hub, client: &Client, sites: &[String], warned: &mut bool) -> Round {
    let mut results = Vec::with_capacity(sites.len());
    for site in sites {
        hub.record_solar_attempt();
        match client.site_forecast(site).await {
            Ok(periods) if periods.is_empty() => {
                if !*warned {
                    tracing::warn!(
                        "solar: a site answered without any period; keeping the last forecast"
                    );
                    *warned = true;
                }
                hub.record_solar_failure();
                hub.service_failed(Service::Solar, "answered without any forecast");
                return Round::Failed;
            }
            Ok(periods) => results.push(periods),
            Err(SolcastError::Unauthorized) => {
                tracing::error!(
                    "solar: Solcast rejected the API key; no further calls until restart"
                );
                hub.record_solar_failure();
                hub.service_failed(Service::Solar, "API key refused");
                return Round::Unauthorized;
            }
            Err(SolcastError::QuotaExceeded) => {
                tracing::warn!(
                    "solar: Solcast daily request limit reached; waiting for the next UTC day"
                );
                hub.record_solar_quota_exceeded();
                hub.record_solar_failure();
                hub.service_failed(Service::Solar, "daily quota used up");
                return Round::Failed;
            }
            Err(e) => {
                if !*warned {
                    tracing::warn!(error = %e, "solar: fetching the forecast failed; keeping the last one");
                    *warned = true;
                }
                hub.record_solar_failure();
                hub.service_failed(Service::Solar, &e.to_string());
                return Round::Failed;
            }
        }
    }
    let now = Utc::now();
    let mut forecast = merge(results, now.timestamp());
    let zone = hub.zone();
    let midnight = local_midnight(now.with_timezone(&zone).date_naive(), zone);
    if let Some(previous) = hub.solar_forecast() {
        // Solcast only returns the periods from now on: keep what today already had.
        forecast.keep_past(&previous, midnight);
    }
    forecast.trim(midnight, midnight + ChronoDuration::days(KEEP_DAYS));
    hub.apply_solar_forecast(forecast);
    hub.service_succeeded(Service::Solar);
    let calls_today = hub.solar_ledger().calls_on(now);
    tracing::info!(calls_today, "solar: forecast applied");
    Round::Fetched
}

/// Time to sleep: until `wake` or the next :00/:30 wall-clock boundary after `now`, whichever
/// is first, and at least [`MIN_SLEEP`].
fn sleep_for(now: DateTime<Utc>, wake: Option<DateTime<Utc>>) -> Duration {
    let boundary = (now.timestamp().div_euclid(1800) + 1) * 1800;
    let to_boundary = Duration::from_secs(u64::try_from(boundary - now.timestamp()).unwrap_or(0));
    let to_wake = wake.map_or(to_boundary, |at| {
        (at - now).to_std().unwrap_or(Duration::ZERO)
    });
    to_boundary.min(to_wake).max(MIN_SLEEP)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    /// (zone, latitude, longitude): places where the sun is up at 00:00Z in summer or the local
    /// day straddles a UTC day.
    const PLACES: [(&str, f64, f64); 3] = [
        ("America/Los_Angeles", 34.05, -118.24),
        ("Australia/Sydney", -33.87, 151.21),
        ("Europe/Paris", 48.86, 2.35),
    ];

    #[test]
    fn local_sun_is_on_the_local_date_with_rise_before_set() {
        for (name, lat, lon) in PLACES {
            let zone: Tz = name.parse().unwrap();
            for date in ["2026-06-21", "2026-12-21", "2026-03-29", "2026-10-25"] {
                let date: NaiveDate = date.parse().unwrap();
                let (rise, set) = local_sun(lat, lon, date, zone);
                let (rise, set) = (rise.unwrap(), set.unwrap());
                assert!(rise < set, "{name} {date}: {rise} {set}");
                assert_eq!(rise.with_timezone(&zone).date_naive(), date, "{name}");
                assert_eq!(set.with_timezone(&zone).date_naive(), date, "{name}");
                let refreshes = slots(Some(rise), Some(set), date, zone, 8);
                assert_eq!(refreshes.len(), 8, "{name} {date}: {refreshes:?}");
            }
        }
    }

    #[test]
    fn local_midnight_after_a_midnight_dst_change() {
        // Chile springs forward at 00:00 on 2026-09-06: midnight does not exist.
        let zone: Tz = "America/Santiago".parse().unwrap();
        let date: NaiveDate = "2026-09-06".parse().unwrap();
        let midnight = local_midnight(date, zone);
        assert_eq!(midnight.with_timezone(&zone).date_naive(), date);
        assert_eq!(
            midnight.with_timezone(&zone).format("%H:%M").to_string(),
            "01:00"
        );
    }

    #[test]
    fn sleep_stops_at_the_half_hour() {
        let now = at("2026-10-24T10:17:00Z");
        assert_eq!(sleep_for(now, None), Duration::from_secs(13 * 60));
        let far = at("2026-10-24T20:00:00Z");
        assert_eq!(sleep_for(now, Some(far)), Duration::from_secs(13 * 60));
    }

    #[test]
    fn sleep_stops_at_an_earlier_wake() {
        let now = at("2026-10-24T10:17:00Z");
        let wake = at("2026-10-24T10:20:00Z");
        assert_eq!(sleep_for(now, Some(wake)), Duration::from_secs(3 * 60));
    }

    #[test]
    fn sleep_has_a_one_second_floor() {
        let now = at("2026-10-24T10:17:00Z");
        assert_eq!(sleep_for(now, Some(now)), MIN_SLEEP);
        assert_eq!(
            sleep_for(now, Some(now - ChronoDuration::hours(1))),
            MIN_SLEEP
        );
        // One second before the boundary.
        assert_eq!(sleep_for(at("2026-10-24T10:29:59Z"), None), MIN_SLEEP);
    }

    #[test]
    fn a_refused_key_never_calls() {
        let now = at("2026-10-24T10:17:00Z");
        let slots = [at("2026-10-24T08:00:00Z")];
        // `decide` alone would call: no forecast, no attempt ever.
        assert_eq!(
            decide(now, &Ledger::default(), false, &slots, 10, 1),
            Decision::CallNow
        );
        assert!(matches!(
            next_decision(true, now, &Ledger::default(), false, &slots, 10, 1),
            Decision::SleepUntil(t) if t > now
        ));
    }

    #[test]
    fn a_failed_first_round_waits_for_the_next_slot() {
        let now = at("2026-10-24T10:17:00Z");
        let next = at("2026-10-24T12:00:00Z");
        let slots = [at("2026-10-24T08:00:00Z"), next];
        let mut ledger = Ledger::default();
        ledger.record_attempt(now);
        assert_eq!(
            next_decision(false, now, &ledger, false, &slots, 10, 1),
            Decision::SleepUntil(next)
        );
    }
}
