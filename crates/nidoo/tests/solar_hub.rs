//! The solar entities, cache and view as the hub publishes them.

use crate::common;

use std::sync::Arc;

use nidoo::config::Config;
use nidoo::hub::{Hub, HubError};
use nidoo::solar::entities;
use nidoo_api::{ConnectionStatus, WsEvent};
use nidoo_core::command::Publish;
use tokio::sync::mpsc;

const SITE: &str = "site-secret";

fn hub_with(enabled: bool) -> (Arc<Hub>, mpsc::Receiver<Publish>) {
    let mut cfg = Config::default();
    cfg.topics.discovery_prefixes = vec!["nidoo".to_owned()];
    cfg.flows.timezone = Some("Europe/Paris".into());
    cfg.solcast.enabled = enabled;
    cfg.solcast.sites = vec![SITE.to_owned()];
    cfg.solcast.daily_limit = 10;
    cfg.solcast.api_key_file = Some("s3cr3t".into());
    let (tx, rx) = mpsc::channel(512);
    let hub = Hub::new(&cfg, tx);
    hub.set_connection(ConnectionStatus::Online);
    (hub, rx)
}

fn drain(publisher: &mut mpsc::Receiver<Publish>) -> Vec<Publish> {
    let mut out = Vec::new();
    while let Ok(publish) = publisher.try_recv() {
        out.push(publish);
    }
    out
}

fn last<'a>(publishes: &'a [Publish], topic: &str) -> Option<&'a Publish> {
    publishes.iter().rfind(|p| p.topic == topic)
}

fn changes(rx: &mut tokio::sync::broadcast::Receiver<WsEvent>) -> usize {
    let mut n = 0;
    while let Ok(event) = rx.try_recv() {
        if matches!(event, WsEvent::SolarChanged) {
            n += 1;
        }
    }
    n
}

#[tokio::test]
async fn apply_publishes_configs_states_availability_and_cache() {
    let (hub, mut rx) = hub_with(true);
    hub.publish_solar_configs();
    let first = drain(&mut rx);
    assert_eq!(
        first
            .iter()
            .filter(|p| p.topic.ends_with("/config") && p.retain)
            .count(),
        entities::KEYS.len()
    );
    assert_eq!(
        last(&first, "nidoo/solar/availability").unwrap().payload,
        "offline"
    );

    hub.apply_solar_forecast(common::solar::forecast_today());
    let out = drain(&mut rx);
    assert_eq!(
        last(&out, "nidoo/solar/availability").unwrap().payload,
        "online"
    );
    let today = last(&out, "nidoo/solar/forecast_today").unwrap();
    assert!(today.retain);
    assert!(today.payload.parse::<f64>().unwrap() > 0.0);
    // The fixture covers today only: tomorrow's values go out as unknown, never frozen.
    assert_eq!(
        last(&out, "nidoo/solar/forecast_tomorrow").unwrap().payload,
        "None"
    );
    let cache = last(&out, "nidoo/solar/cache").unwrap();
    assert!(cache.retain);
    assert!(cache.payload.contains("forecast"));
    assert!(hub.solar_has_forecast());
}

#[tokio::test]
async fn solar_changed_only_when_data_changes() {
    let (hub, mut rx) = hub_with(true);
    let mut events = hub.subscribe();
    hub.publish_solar_configs();
    let forecast = common::solar::forecast_today();
    hub.apply_solar_forecast(forecast.clone());
    assert_eq!(changes(&mut events), 1);
    // Same data, later fetch: nothing to tell.
    let mut again = forecast.clone();
    again.fetched_at += 60;
    hub.apply_solar_forecast(again);
    assert_eq!(changes(&mut events), 0);
    // Other data.
    let mut other = forecast;
    for p in &mut other.periods {
        p.estimate += 0.5;
    }
    hub.apply_solar_forecast(other);
    assert_eq!(changes(&mut events), 1);
    drain(&mut rx);
}

#[tokio::test]
async fn cache_restores_forecast_and_ledger_once() {
    let (source, mut rx) = hub_with(true);
    source.publish_solar_configs();
    source.record_solar_attempt();
    source.apply_solar_forecast(common::solar::forecast_today());
    let cache = last(&drain(&mut rx), "nidoo/solar/cache")
        .unwrap()
        .payload
        .clone();

    let (hub, _rx) = hub_with(true);
    assert!(!hub.solar_has_forecast());
    hub.ingest_message("nidoo/solar/cache", cache.as_bytes(), true);
    assert!(hub.solar_has_forecast());
    assert_eq!(hub.solar_ledger(), source.solar_ledger());
    assert_eq!(hub.solar_ledger().calls, 1);

    // A second cache keeps the held forecast; its higher call count is merged in.
    let (other, mut other_rx) = hub_with(true);
    other.publish_solar_configs();
    other.record_solar_attempt();
    other.record_solar_attempt();
    other.apply_solar_forecast(common::solar::forecast_today());
    let second = last(&drain(&mut other_rx), "nidoo/solar/cache")
        .unwrap()
        .payload
        .clone();
    hub.ingest_message("nidoo/solar/cache", second.as_bytes(), true);
    assert_eq!(hub.solar_ledger().calls, 2);

    // Malformed is swallowed.
    let (fresh, _rx) = hub_with(true);
    fresh.ingest_message("nidoo/solar/cache", b"{nope", true);
    assert!(!fresh.solar_has_forecast());
}

fn cache_payload(rx: &mut mpsc::Receiver<Publish>) -> String {
    last(&drain(rx), "nidoo/solar/cache")
        .unwrap()
        .payload
        .clone()
}

#[tokio::test]
async fn forecast_cache_never_lowers_calls_recorded_this_run() {
    let (source, mut rx) = hub_with(true);
    source.publish_solar_configs();
    source.record_solar_attempt();
    source.apply_solar_forecast(common::solar::forecast_today());
    let cache = cache_payload(&mut rx);

    let (hub, _rx) = hub_with(true);
    hub.record_solar_attempt();
    hub.record_solar_attempt();
    hub.record_solar_attempt();
    hub.ingest_message("nidoo/solar/cache", cache.as_bytes(), true);
    assert_eq!(hub.solar_ledger().calls, 3);
    assert!(hub.solar_has_forecast());
}

#[tokio::test]
async fn ledger_only_cache_is_restored_into_a_fresh_hub() {
    let (source, mut rx) = hub_with(true);
    source.publish_solar_configs();
    source.record_solar_attempt();
    source.record_solar_attempt();
    let cache = cache_payload(&mut rx);
    assert!(!cache.contains("\"forecast\":{"));

    let (hub, _rx) = hub_with(true);
    hub.ingest_message("nidoo/solar/cache", cache.as_bytes(), true);
    assert_eq!(hub.solar_ledger(), source.solar_ledger());
    assert_eq!(hub.solar_ledger().calls, 2);
    assert!(!hub.solar_has_forecast());
}

#[tokio::test]
async fn ledger_only_cache_with_fewer_calls_does_not_lower_calls() {
    let (source, mut rx) = hub_with(true);
    source.publish_solar_configs();
    source.record_solar_attempt();
    let cache = cache_payload(&mut rx);

    let (hub, _rx) = hub_with(true);
    hub.record_solar_attempt();
    hub.record_solar_attempt();
    hub.ingest_message("nidoo/solar/cache", cache.as_bytes(), true);
    assert_eq!(hub.solar_ledger().calls, 2);
}

#[tokio::test]
async fn cache_never_contains_key_or_site_ids() {
    let (hub, mut rx) = hub_with(true);
    hub.publish_solar_configs();
    hub.record_solar_attempt();
    hub.record_solar_quota_exceeded();
    hub.record_solar_failure();
    hub.apply_solar_forecast(common::solar::forecast_today());
    hub.refresh_solar_states();
    let out = drain(&mut rx);
    assert!(out.iter().any(|p| p.topic == "nidoo/solar/cache"));
    for publish in &out {
        assert!(!publish.payload.contains(SITE), "{}", publish.topic);
        assert!(!publish.payload.contains("s3cr3t"), "{}", publish.topic);
    }
    let view = serde_json::to_string(&hub.solar_view().unwrap()).unwrap();
    assert!(!view.contains(SITE) && !view.contains("s3cr3t"));
}

#[tokio::test]
async fn disabled_hub_clears_retained_solar_topics() {
    let (hub, mut rx) = hub_with(false);
    for topic in [
        "nidoo/solar/forecast_today",
        "nidoo/solar/availability",
        "nidoo/solar/cache",
    ] {
        hub.ingest_message(topic, b"x", true);
    }
    let config = entities::configs("nidoo").remove(0);
    hub.ingest_message(&config.0, config.1.to_string().as_bytes(), true);
    // Another device's retained config is left alone.
    hub.ingest_message("nidoo/sensor/other/x/config", b"{}", true);
    let out = drain(&mut rx);
    assert_eq!(out.len(), 4);
    assert!(out.iter().all(|p| p.payload.is_empty() && p.retain));
    // A disabled hub never loads the cache.
    assert!(!hub.solar_has_forecast());
}

#[tokio::test]
async fn view_is_503_when_disabled_or_empty() {
    let (disabled, _rx) = hub_with(false);
    disabled.apply_solar_forecast(common::solar::forecast_today());
    assert!(matches!(disabled.solar_view(), Err(HubError::Publish(m)) if m == "solar unavailable"));
    let (empty, _rx) = hub_with(true);
    assert!(matches!(empty.solar_view(), Err(HubError::Publish(m)) if m == "solar unavailable"));
    assert!(empty.solar_view_with_history().await.is_err());
    empty.apply_solar_forecast(common::solar::forecast_today());
    let view = empty.solar_view().unwrap();
    assert!(!view.stale);
    assert_eq!(view.daily_limit, 10);
}

#[tokio::test]
async fn expired_forecast_goes_offline_on_refresh() {
    let (hub, mut rx) = hub_with(true);
    hub.publish_solar_configs();
    hub.apply_solar_forecast(common::solar::forecast_today());
    hub.refresh_solar_states();
    assert_eq!(
        last(&drain(&mut rx), "nidoo/solar/availability")
            .unwrap()
            .payload,
        "online"
    );

    let (old, mut old_rx) = hub_with(true);
    old.publish_solar_configs();
    let fetched = chrono::Utc::now() - chrono::Duration::hours(25);
    old.apply_solar_forecast(common::solar::forecast_covering(fetched));
    drain(&mut old_rx);
    old.refresh_solar_states();
    assert_eq!(
        last(&drain(&mut old_rx), "nidoo/solar/availability")
            .unwrap()
            .payload,
        "offline"
    );
}

#[tokio::test]
async fn view_with_history_is_empty_without_history() {
    let (hub, _rx) = hub_with(true);
    hub.apply_solar_forecast(common::solar::forecast_today());
    let plain = hub.solar_view().unwrap();
    let with = hub.solar_view_with_history().await.unwrap();
    assert!(with.history.forecasts.is_empty());
    assert_eq!(with, plain);
}
