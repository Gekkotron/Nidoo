//! "Notify me when a service is unreachable": a per-service tracker of continuous failures.
//!
//! A service that fails for [`THRESHOLD_SECS`] without a success in between raises one "down"
//! alert; its next success raises one "reachable again" alert, only when the down alert went
//! out. Shorter blips are silent both ways. The state lives in memory only.

use std::sync::MutexGuard;

use serde_json::{Value, json};

use crate::clock;
use crate::hub::Hub;

/// Seconds of continuous failure before the down alert.
pub const THRESHOLD_SECS: u64 = 30 * 60;

/// How often the hub's watch task checks for an outage that crossed the threshold.
pub const WATCH_INTERVAL_SECS: u64 = 60;

/// How long an outage alert stays relevant, in seconds (a day, like the watchdog alerts).
const TTL: u64 = 86_400;

/// The services whose reachability the hub tracks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Service {
    Weather,
    Solar,
}

impl Service {
    /// The id fragment and the `service` alert argument.
    fn key(self) -> &'static str {
        match self {
            Service::Weather => "weather",
            Service::Solar => "solar",
        }
    }

    /// The name the user knows the feature by.
    fn label(self) -> &'static str {
        match self {
            Service::Weather => "Weather",
            Service::Solar => "Solar forecast",
        }
    }

    /// The outside service the hub talks to.
    fn provider(self) -> &'static str {
        match self {
            Service::Weather => "Open-Meteo",
            Service::Solar => "Solcast",
        }
    }
}

/// One service's outage state.
#[derive(Debug)]
pub struct Outage {
    service: Service,
    /// Whether the user asked to be told; nothing is ever alerted while it is off.
    notify: bool,
    failing_since: Option<u64>,
    reason: String,
    alerted: bool,
}

impl Outage {
    pub fn new(service: Service, notify: bool) -> Self {
        Self {
            service,
            notify,
            failing_since: None,
            reason: String::new(),
            alerted: false,
        }
    }

    /// A fetch failed at `now`. The first failure of a run starts the clock; a later one keeps
    /// that start and only updates the reason.
    pub fn failed(&mut self, now: u64, reason: &str) {
        self.failing_since.get_or_insert(now);
        self.reason = reason.to_owned();
    }

    /// A fetch succeeded at `now`: ends the outage, and returns the "reachable again" alert
    /// when a down alert had gone out.
    pub fn succeeded(&mut self, now: u64) -> Option<Value> {
        let since = self.failing_since.take()?;
        let alerted = std::mem::take(&mut self.alerted);
        self.reason.clear();
        (alerted && self.notify).then(|| self.back_alert(since, now))
    }

    /// The periodic check: the down alert once the outage reached the threshold, never twice.
    pub fn tick(&mut self, now: u64) -> Option<Value> {
        let since = self.failing_since?;
        if !self.notify || self.alerted || now.saturating_sub(since) < THRESHOLD_SECS {
            return None;
        }
        self.alerted = true;
        Some(self.down_alert(since))
    }

    fn down_alert(&self, since: u64) -> Value {
        let minutes = THRESHOLD_SECS / 60;
        let service = self.service;
        json!({
            "v": 1,
            "id": format!("service-{}-down-{since}", service.key()),
            "channel": "system",
            "severity": "warning",
            "title": format!("{} unreachable", service.label()),
            "body": format!(
                "No answer from {} for {minutes} min: {}",
                service.provider(),
                self.reason
            ),
            "silent": false,
            "ttl": TTL,
            "loc": {"key": "service.unreachable", "args": {
                "service": service.key(),
                "reason": self.reason,
                "minutes": minutes,
            }},
        })
    }

    fn back_alert(&self, since: u64, now: u64) -> Value {
        let minutes = now.saturating_sub(since) / 60;
        let service = self.service;
        json!({
            "v": 1,
            "id": format!("service-{}-back-{since}", service.key()),
            "channel": "system",
            "severity": "info",
            "title": format!("{} reachable again", service.label()),
            "body": format!(
                "{} answers again after {}",
                service.provider(),
                duration_text(minutes)
            ),
            "silent": false,
            "ttl": TTL,
            "loc": {"key": "service.reachable_again", "args": {
                "service": service.key(),
                "minutes": minutes,
            }},
        })
    }
}

/// `45 min`, `1 h 12 min`, `2 h`.
fn duration_text(minutes: u64) -> String {
    match (minutes / 60, minutes % 60) {
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m} min"),
    }
}

/// Both services' trackers, behind the hub's lock.
pub(crate) struct Outages {
    weather: Outage,
    solar: Outage,
}

impl Outages {
    pub(crate) fn new(weather_notify: bool, solar_notify: bool) -> Self {
        Self {
            weather: Outage::new(Service::Weather, weather_notify),
            solar: Outage::new(Service::Solar, solar_notify),
        }
    }

    fn of(&mut self, service: Service) -> &mut Outage {
        match service {
            Service::Weather => &mut self.weather,
            Service::Solar => &mut self.solar,
        }
    }
}

impl Hub {
    fn outages(&self) -> MutexGuard<'_, Outages> {
        self.outages
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Whether any service asked to be told when unreachable.
    pub fn outage_watch_wanted(&self) -> bool {
        self.weather_notify_unreachable || self.solar_notify_unreachable
    }

    /// A fetch of `service` failed.
    pub fn service_failed(&self, service: Service, reason: &str) {
        self.service_failed_at(service, clock::now(), reason);
    }

    /// [`Hub::service_failed`] at a given time.
    pub fn service_failed_at(&self, service: Service, now: u64, reason: &str) {
        self.outages().of(service).failed(now, reason);
    }

    /// A fetch of `service` succeeded; sends the "reachable again" alert when due.
    pub fn service_succeeded(&self, service: Service) {
        self.service_succeeded_at(service, clock::now());
    }

    /// [`Hub::service_succeeded`] at a given time.
    pub fn service_succeeded_at(&self, service: Service, now: u64) {
        let alert = self.outages().of(service).succeeded(now);
        self.raise_system(alert);
    }

    /// Sends the down alert of every service that has been failing long enough.
    pub fn check_outages(&self) {
        self.check_outages_at(clock::now());
    }

    /// [`Hub::check_outages`] at a given time.
    pub fn check_outages_at(&self, now: u64) {
        let alerts = {
            let mut outages = self.outages();
            [outages.weather.tick(now), outages.solar.tick(now)]
        };
        for alert in alerts {
            self.raise_system(alert);
        }
    }

    /// Starts the task that checks the outages every minute; `None` while no switch is on.
    pub fn start_outage_watch(self: &std::sync::Arc<Self>) -> Option<tokio::task::JoinHandle<()>> {
        if !self.outage_watch_wanted() {
            return None;
        }
        let hub = std::sync::Arc::clone(self);
        Some(tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(std::time::Duration::from_secs(WATCH_INTERVAL_SECS));
            loop {
                interval.tick().await;
                hub.check_outages();
            }
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: u64 = 1_000_000;
    const MIN: u64 = 60;

    fn weather() -> Outage {
        Outage::new(Service::Weather, true)
    }

    #[test]
    fn a_blip_under_the_threshold_is_silent_both_ways() {
        let mut o = weather();
        o.failed(T0, "timeout");
        assert!(o.tick(T0 + 29 * MIN).is_none());
        assert!(o.succeeded(T0 + 29 * MIN).is_none());
    }

    #[test]
    fn thirty_minutes_down_alerts_once() {
        let mut o = weather();
        o.failed(T0, "timeout");
        assert!(o.tick(T0 + 29 * MIN + 59).is_none());
        let alert = o.tick(T0 + 30 * MIN).expect("down alert");
        assert_eq!(alert["id"], format!("service-weather-down-{T0}"));
        assert_eq!(alert["channel"], "system");
        assert_eq!(alert["severity"], "warning");
        assert_eq!(alert["title"], "Weather unreachable");
        assert_eq!(
            alert["body"],
            "No answer from Open-Meteo for 30 min: timeout"
        );
        assert_eq!(alert["loc"]["key"], "service.unreachable");
        assert_eq!(alert["loc"]["args"]["service"], "weather");
        assert_eq!(alert["loc"]["args"]["reason"], "timeout");
        assert_eq!(alert["loc"]["args"]["minutes"], 30);
        assert!(o.tick(T0 + 31 * MIN).is_none());
        o.failed(T0 + 40 * MIN, "timeout");
        assert!(o.tick(T0 + 90 * MIN).is_none());
    }

    #[test]
    fn recovery_after_a_down_alert_says_how_long() {
        let mut o = Outage::new(Service::Solar, true);
        o.failed(T0, "daily quota used up");
        assert!(o.tick(T0 + 30 * MIN).is_some());
        let alert = o.succeeded(T0 + 72 * MIN).expect("back alert");
        assert_eq!(alert["severity"], "info");
        assert_eq!(alert["title"], "Solar forecast reachable again");
        assert_eq!(alert["body"], "Solcast answers again after 1 h 12 min");
        assert_eq!(alert["loc"]["key"], "service.reachable_again");
        assert_eq!(alert["id"], format!("service-solar-back-{T0}"));
        assert!(o.succeeded(T0 + 73 * MIN).is_none());
    }

    #[test]
    fn a_second_outage_alerts_again() {
        let mut o = weather();
        o.failed(T0, "x");
        assert!(o.tick(T0 + 30 * MIN).is_some());
        assert!(o.succeeded(T0 + 31 * MIN).is_some());
        o.failed(T0 + 100 * MIN, "y");
        assert!(o.tick(T0 + 130 * MIN).is_some());
    }

    #[test]
    fn a_success_restarts_the_count() {
        let mut o = weather();
        o.failed(T0, "x");
        assert!(o.succeeded(T0 + 20 * MIN).is_none());
        o.failed(T0 + 25 * MIN, "x");
        assert!(o.tick(T0 + 40 * MIN).is_none());
        assert!(o.tick(T0 + 55 * MIN).is_some());
    }

    #[test]
    fn the_reason_follows_the_latest_failure_but_the_start_stays() {
        let mut o = weather();
        o.failed(T0, "timeout");
        o.failed(T0 + 10 * MIN, "HTTP 503");
        let alert = o.tick(T0 + 30 * MIN).expect("down alert");
        assert_eq!(alert["loc"]["args"]["reason"], "HTTP 503");
        assert_eq!(alert["id"], format!("service-weather-down-{T0}"));
    }

    #[test]
    fn a_switch_that_is_off_sends_nothing() {
        let mut o = Outage::new(Service::Weather, false);
        o.failed(T0, "timeout");
        assert!(o.tick(T0 + 120 * MIN).is_none());
        assert!(o.succeeded(T0 + 121 * MIN).is_none());
    }

    #[test]
    fn durations_read_naturally() {
        assert_eq!(duration_text(45), "45 min");
        assert_eq!(duration_text(72), "1 h 12 min");
        assert_eq!(duration_text(120), "2 h");
    }
}
