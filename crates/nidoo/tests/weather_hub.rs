//! The weather entities as the hub publishes them, and the clearing of a switched-off device.

use std::sync::Arc;

use nidoo::config::Config;
use nidoo::hub::{Hub, HubError};
use nidoo::weather::entities;
use nidoo_api::{
    ConnectionStatus, WeatherCondition, WeatherCurrent, WeatherDay, WeatherPlace, WeatherToday,
    WeatherView, WsEvent,
};
use nidoo_core::command::Publish;
use nidoo_core::model::{Kind, StateValue};
use tokio::sync::mpsc;

fn hub_with(enabled: bool) -> (Arc<Hub>, mpsc::Receiver<Publish>) {
    let mut cfg = Config::default();
    cfg.topics.discovery_prefixes = vec!["oikos".to_owned()];
    cfg.weather.enabled = enabled;
    cfg.weather.city = "Paris".to_owned();
    let (tx, rx) = mpsc::channel(256);
    let hub = Hub::new(&cfg, tx);
    hub.set_connection(ConnectionStatus::Online);
    (hub, rx)
}

fn hub() -> (Arc<Hub>, mpsc::Receiver<Publish>) {
    hub_with(true)
}

fn drain(publisher: &mut mpsc::Receiver<Publish>) -> Vec<Publish> {
    let mut out = Vec::new();
    while let Ok(publish) = publisher.try_recv() {
        out.push(publish);
    }
    out
}

fn sample_view() -> WeatherView {
    WeatherView {
        place: WeatherPlace {
            name: "Paris".into(),
            admin: Some("Ile-de-France".into()),
            country: Some("FR".into()),
        },
        fetched_at: nidoo::clock::now(),
        stale: false,
        current: WeatherCurrent {
            temperature: Some(13.4),
            apparent_temperature: Some(12.0),
            humidity: Some(81.0),
            cloud_cover: Some(90.0),
            precipitation: Some(0.2),
            wind_speed: Some(14.0),
            wind_gusts: Some(30.0),
            wind_direction: Some(250.0),
            pressure: Some(1012.0),
            uv_index: Some(1.0),
            condition: Some(WeatherCondition::Rain),
            daylight: Some(true),
        },
        today: WeatherToday {
            min: Some(9.0),
            max: Some(15.0),
            precipitation: Some(4.0),
            precipitation_probability: Some(80.0),
            wind_gusts_max: Some(45.0),
        },
        rain_next_hour: Some(0.5),
        tonight_min: Some(8.0),
        days: vec![WeatherDay {
            date: "2026-10-24".into(),
            condition: Some(WeatherCondition::Rain),
            min: Some(8.0),
            max: Some(13.5),
            precipitation_probability: Some(85.0),
            apparent_max: Some(12.0),
            precipitation: Some(4.6),
            wind_speed_max: Some(25.4),
            wind_gusts_max: Some(41.0),
            wind_direction: Some(250.0),
            humidity: Some(78.0),
            pressure: Some(1005.8),
            cloud_cover: Some(90.0),
            uv_index_max: Some(1.8),
        }],
    }
}

fn topic<'a>(publishes: &'a [Publish], topic: &str) -> Vec<&'a Publish> {
    publishes.iter().filter(|p| p.topic == topic).collect()
}

#[test]
fn weather_unavailable_before_first_fetch() {
    let (hub, _rx) = hub();
    assert!(matches!(hub.weather_view(), Err(HubError::Publish(_))));
}

#[test]
fn publish_configs_then_apply_publishes_states_and_online() {
    let (hub, mut rx) = hub();
    hub.publish_weather_configs();
    let out = drain(&mut rx);
    assert_eq!(out.len(), 27);
    assert!(out.iter().all(|p| p.retain));
    let availability = topic(&out, "oikos/weather/availability");
    assert_eq!(availability.len(), 1);
    assert_eq!(availability[0].payload, "offline");
    assert_eq!(
        out.iter().filter(|p| p.topic.ends_with("/config")).count(),
        26
    );

    hub.apply_weather(sample_view());
    let out = drain(&mut rx);
    assert_eq!(
        topic(&out, "oikos/weather/availability")[0].payload,
        "online"
    );
    assert_eq!(out.len(), 27);
    assert!(out.iter().all(|p| p.retain));
    assert!(!hub.weather_view().expect("view").stale);

    // The broker echoes everything back.
    let (hub, mut rx) = self::hub();
    hub.publish_weather_configs();
    hub.apply_weather(sample_view());
    for publish in drain(&mut rx) {
        hub.ingest_message(&publish.topic, publish.payload.as_bytes(), true);
    }
    let view = hub.entity_view("weather_temperature").expect("entity");
    assert_eq!(view.kind, Kind::Sensor);
    assert_eq!(view.state.value, StateValue::Number(13.4));
}

#[test]
fn apply_weather_publishes_only_changes() {
    let (hub, mut rx) = hub();
    let mut events = hub.subscribe();
    hub.apply_weather(sample_view());
    drain(&mut rx);
    hub.apply_weather(sample_view());
    assert!(drain(&mut rx).is_empty());

    let mut changed = sample_view();
    changed.current.temperature = Some(14.0);
    hub.apply_weather(changed);
    let out = drain(&mut rx);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].topic, "oikos/weather/temperature");
    assert_eq!(out[0].payload, "14");

    let mut changes = 0;
    while let Ok(event) = events.try_recv() {
        if matches!(event, WsEvent::WeatherChanged) {
            changes += 1;
        }
    }
    // First view, then the temperature change; the identical second view is silent.
    assert_eq!(changes, 2);
}

#[test]
fn weather_publish_failure_is_retried_next_time() {
    let (hub, mut rx) = hub();
    hub.set_connection(ConnectionStatus::Offline {
        reason: "test".into(),
    });
    hub.apply_weather(sample_view());
    assert!(drain(&mut rx).is_empty());
    assert!(hub.weather_view().is_ok());

    hub.set_connection(ConnectionStatus::Online);
    hub.apply_weather(sample_view());
    let out = drain(&mut rx);
    // The configs lost while offline go out again with the states.
    assert_eq!(state_count(&out), 26);
    assert_eq!(config_count(&out), 26);
}

#[test]
fn set_weather_offline_is_once_per_transition() {
    let (hub, mut rx) = hub();
    hub.set_weather_offline();
    hub.set_weather_offline();
    let out = drain(&mut rx);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].payload, "offline");
}

#[test]
fn disabled_weather_clears_only_its_own_configs() {
    let (hub, mut rx) = hub_with(false);
    let own = entities::configs("oikos")[0].1.to_string();
    hub.ingest_message(
        "oikos/sensor/weather/temperature/config",
        own.as_bytes(),
        true,
    );
    let out = drain(&mut rx);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].topic, "oikos/sensor/weather/temperature/config");
    assert!(out[0].payload.is_empty() && out[0].retain);
    assert!(hub.entity_view("weather_temperature").is_none());

    hub.ingest_message("oikos/weather/temperature", b"13.4", true);
    let out = drain(&mut rx);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].topic, "oikos/weather/temperature");
    assert!(out[0].payload.is_empty() && out[0].retain);

    let foreign = serde_json::json!({
        "name": "Foo",
        "unique_id": "foo",
        "state_topic": "x/foo",
        "device": { "identifiers": ["0xabc"] },
    })
    .to_string();
    hub.ingest_message("oikos/sensor/weather/foo/config", foreign.as_bytes(), true);
    assert!(drain(&mut rx).is_empty());
    assert!(hub.entity_view("foo").is_some());
}

#[test]
fn weather_state_topic_is_not_discovery() {
    let (hub, _rx) = hub();
    hub.ingest_message("oikos/weather/temperature", b"13.4", true);
    hub.ingest_message("oikos/weather/availability", b"online", true);
    assert!(hub.ignored().is_empty());
}

#[test]
fn weather_value_that_disappears_is_cleared() {
    let (hub, mut rx) = hub();
    hub.apply_weather(sample_view());
    for publish in drain(&mut rx) {
        hub.ingest_message(&publish.topic, publish.payload.as_bytes(), true);
    }
    for (topic, config) in entities::configs("oikos") {
        hub.ingest_message(&topic, config.to_string().as_bytes(), true);
    }
    hub.ingest_message("oikos/weather/uv_index", b"1", true);
    assert_eq!(
        hub.entity_view("weather_uv_index")
            .expect("entity")
            .state
            .value,
        StateValue::Number(1.0)
    );

    let mut gone = sample_view();
    gone.current.uv_index = None;
    hub.apply_weather(gone.clone());
    let out = drain(&mut rx);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].topic, "oikos/weather/uv_index");
    assert_eq!(out[0].payload, "None");
    assert_eq!(
        hub.entity_view("weather_uv_index")
            .expect("entity")
            .state
            .value,
        StateValue::Unknown
    );

    // Cleared once: the same view again publishes nothing.
    hub.apply_weather(gone);
    assert!(drain(&mut rx).is_empty());
}

#[test]
fn publish_configs_makes_states_republish() {
    let (hub, mut rx) = hub();
    hub.apply_weather(sample_view());
    drain(&mut rx);
    hub.publish_weather_configs();
    drain(&mut rx);
    hub.apply_weather(sample_view());
    let out = drain(&mut rx);
    assert_eq!(
        out.iter()
            .filter(|p| p.topic != "oikos/weather/availability")
            .count(),
        26
    );
}

#[test]
fn restart_publishes_unknown_for_values_a_previous_run_left_retained() {
    let (hub, mut rx) = hub();
    hub.publish_weather_configs();
    drain(&mut rx);
    let mut view = sample_view();
    view.current.uv_index = None;
    hub.apply_weather(view);
    let out = drain(&mut rx);
    let uv = topic(&out, "oikos/weather/uv_index");
    assert_eq!(uv.len(), 1);
    assert_eq!(uv[0].payload, "None");
    assert!(uv[0].retain);
    // 25 values, the unknown uv_index and the availability: each goes out once.
    assert_eq!(out.len(), 27);
}

fn config_count(out: &[Publish]) -> usize {
    out.iter().filter(|p| p.topic.ends_with("/config")).count()
}

fn state_count(out: &[Publish]) -> usize {
    out.iter()
        .filter(|p| !p.topic.ends_with("/config") && p.topic != "oikos/weather/availability")
        .count()
}

#[test]
fn configs_lost_while_offline_are_republished_by_the_next_apply() {
    let (hub, mut rx) = hub();
    hub.set_connection(ConnectionStatus::Offline {
        reason: "test".into(),
    });
    hub.publish_weather_configs();
    assert!(drain(&mut rx).is_empty());
    assert!(hub.entity_view("weather_temperature").is_none());

    hub.set_connection(ConnectionStatus::Online);
    hub.apply_weather(sample_view());
    let out = drain(&mut rx);
    assert_eq!(config_count(&out), 26);
    assert_eq!(state_count(&out), 26);
    assert!(hub.entity_view("weather_temperature").is_some());

    // Healthy now: nothing is republished.
    hub.apply_weather(sample_view());
    assert!(drain(&mut rx).is_empty());
}

#[test]
fn removed_entity_makes_the_next_apply_republish_the_configs() {
    let (hub, mut rx) = hub();
    hub.publish_weather_configs();
    hub.apply_weather(sample_view());
    drain(&mut rx);
    assert!(hub.entity_view("weather_temperature").is_some());

    hub.ingest_message("oikos/sensor/weather/temperature/config", b"", true);
    assert!(hub.entity_view("weather_temperature").is_none());
    // No reaction to the empty retained message itself.
    assert!(drain(&mut rx).is_empty());

    hub.apply_weather(sample_view());
    let out = drain(&mut rx);
    assert_eq!(config_count(&out), 26);
    assert_eq!(state_count(&out), 26);
    assert!(hub.entity_view("weather_temperature").is_some());
}
