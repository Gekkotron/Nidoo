//! The link to Nidoo: every satellite appears through Home Assistant MQTT
//! discovery as one device with a Volume number, a Mute switch, an Allowed
//! switch (config) and a Connected binary sensor (diagnostic). Commands come
//! back on `nidoo-sound/<short id>/<control>/set`.

use std::collections::HashMap;
use std::time::Duration;

use rumqttc::{AsyncClient, Event, LastWill, MqttOptions, Packet, QoS};
use serde_json::json;
use tokio::sync::mpsc;

use crate::config::MqttSettings;
use crate::group::{Control, Published};
use crate::state::{PlayerPrefs, short_id};

/// Topic root for this service's own topics (the hub owns `nidoo/`).
pub const TOPIC_ROOT: &str = "nidoo-sound";
/// `online` / `offline` (last will), retained.
pub const STATUS_TOPIC: &str = "nidoo-sound/status";

fn on_off(b: bool) -> &'static str {
    if b { "ON" } else { "OFF" }
}

/// Retained discovery configs for one satellite.
pub fn discovery_messages(p: &PlayerPrefs, discovery_prefix: &str) -> Vec<(String, String)> {
    let sid = short_id(&p.client_id);
    let node = format!("nidoo_sound_{sid}");
    let device = json!({
        "identifiers": [node],
        "name": p.name,
        "model": "Sound satellite",
        "manufacturer": "Nidoo",
    });
    let base = |name: &str, object: &str| {
        json!({
            "name": name,
            "unique_id": format!("{node}_{object}"),
            "object_id": format!("{node}_{object}"),
            "state_topic": format!("{TOPIC_ROOT}/{sid}/{object}"),
            "availability_topic": STATUS_TOPIC,
            "device": device,
        })
    };
    let with = |mut v: serde_json::Value, extra: serde_json::Value| {
        if let (Some(v), serde_json::Value::Object(extra)) = (v.as_object_mut(), extra) {
            v.extend(extra);
        }
        v
    };
    let command = |object: &str| format!("{TOPIC_ROOT}/{sid}/{object}/set");
    let configs = [
        (
            "number",
            "volume",
            with(
                base("Volume", "volume"),
                json!({"command_topic": command("volume"), "min": 0, "max": 100, "step": 1,
                       "mode": "slider", "icon": "mdi:volume-high"}),
            ),
        ),
        (
            "switch",
            "mute",
            with(
                base("Mute", "mute"),
                json!({"command_topic": command("mute"), "payload_on": "ON", "payload_off": "OFF",
                       "icon": "mdi:volume-off"}),
            ),
        ),
        (
            "switch",
            "allowed",
            with(
                base("Allowed", "allowed"),
                json!({"command_topic": command("allowed"), "payload_on": "ON", "payload_off": "OFF",
                       "entity_category": "config", "icon": "mdi:speaker-play"}),
            ),
        ),
        (
            "binary_sensor",
            "connected",
            with(
                base("Connected", "connected"),
                json!({"device_class": "connectivity", "entity_category": "diagnostic",
                       "payload_on": "ON", "payload_off": "OFF"}),
            ),
        ),
    ];
    configs
        .into_iter()
        .map(|(component, object, v)| {
            (
                format!("{discovery_prefix}/{component}/{node}/{object}/config"),
                v.to_string(),
            )
        })
        .collect()
}

/// Retained state values for one satellite.
pub fn state_messages(p: &PlayerPrefs, connected: bool) -> Vec<(String, String)> {
    let sid = short_id(&p.client_id);
    vec![
        (format!("{TOPIC_ROOT}/{sid}/volume"), p.volume.to_string()),
        (format!("{TOPIC_ROOT}/{sid}/mute"), on_off(p.muted).into()),
        (
            format!("{TOPIC_ROOT}/{sid}/allowed"),
            on_off(p.allowed).into(),
        ),
        (
            format!("{TOPIC_ROOT}/{sid}/connected"),
            on_off(connected).into(),
        ),
    ]
}

/// A command from `nidoo-sound/<sid>/<volume|mute|allowed>/set`.
pub fn parse_command(topic: &str, payload: &[u8]) -> Option<Control> {
    let mut parts = topic.split('/');
    let (root, sid, object, set) = (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
    if root != TOPIC_ROOT || set != "set" || parts.next().is_some() {
        return None;
    }
    let short_id = sid.to_owned();
    let text = std::str::from_utf8(payload).ok()?.trim();
    let switch = || match text {
        "ON" => Some(true),
        "OFF" => Some(false),
        _ => None,
    };
    match object {
        "volume" => {
            let v: f64 = text.parse().ok()?;
            if !v.is_finite() {
                return None;
            }
            Some(Control::SetVolume {
                short_id,
                volume: v.round().clamp(0.0, 100.0) as u8,
            })
        }
        "mute" => Some(Control::SetMute {
            short_id,
            muted: switch()?,
        }),
        "allowed" => Some(Control::SetAllowed {
            short_id,
            allowed: switch()?,
        }),
        _ => None,
    }
}

/// Keeps the broker in step with the group: discovery + state on every
/// change, everything again on every (re)connect and when Home Assistant
/// discovery restarts (`<prefix>/status` = `online`).
pub async fn run_mqtt(
    cfg: &MqttSettings,
    discovery_prefix: &str,
    mut publish: mpsc::Receiver<Published>,
    control: mpsc::Sender<Control>,
) -> anyhow::Result<()> {
    let mut options = MqttOptions::new(cfg.client_id.clone(), cfg.host.clone(), cfg.port);
    options.set_keep_alive(Duration::from_secs(30));
    if let Some(user) = &cfg.user {
        options.set_credentials(user.clone(), cfg.password.clone().unwrap_or_default());
    }
    options.set_last_will(LastWill::new(
        STATUS_TOPIC,
        "offline",
        QoS::AtLeastOnce,
        true,
    ));
    let (client, mut eventloop) = AsyncClient::new(options, 256);
    let ha_status = format!("{discovery_prefix}/status");
    let mut known: HashMap<String, (PlayerPrefs, bool)> = HashMap::new();
    let mut announced: HashMap<String, String> = HashMap::new();

    loop {
        tokio::select! {
            event = eventloop.poll() => match event {
                Ok(Event::Incoming(Packet::ConnAck(_))) => {
                    tracing::info!(host = %cfg.host, port = cfg.port, "connected to the broker");
                    client.publish(STATUS_TOPIC, QoS::AtLeastOnce, true, "online").await?;
                    client.subscribe(format!("{TOPIC_ROOT}/+/+/set"), QoS::AtLeastOnce).await?;
                    client.subscribe(ha_status.clone(), QoS::AtLeastOnce).await?;
                    announced.clear();
                    for (prefs, connected) in known.values() {
                        send(&client, discovery_prefix, prefs, *connected, &mut announced).await?;
                    }
                }
                Ok(Event::Incoming(Packet::Publish(p))) => {
                    if p.topic == ha_status {
                        if p.payload.as_ref() == b"online" {
                            announced.clear();
                            for (prefs, connected) in known.values() {
                                send(&client, discovery_prefix, prefs, *connected, &mut announced).await?;
                            }
                        }
                    } else if let Some(c) = parse_command(&p.topic, &p.payload) {
                        tracing::debug!(?c, "command from Nidoo");
                        if control.send(c).await.is_err() {
                            return Ok(());
                        }
                    } else {
                        tracing::debug!(topic = %p.topic, "ignored message");
                    }
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!(error = %e, "broker connection lost; retrying");
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
            },
            msg = publish.recv() => {
                let Some(Published::Player { short_id, prefs, connected }) = msg else {
                    let _ = client.publish(STATUS_TOPIC, QoS::AtLeastOnce, true, "offline").await;
                    let _ = client.disconnect().await;
                    // Let the event loop flush the last messages.
                    let _ = tokio::time::timeout(Duration::from_secs(2), async {
                        while eventloop.poll().await.is_ok() {}
                    })
                    .await;
                    return Ok(());
                };
                send(&client, discovery_prefix, &prefs, connected, &mut announced).await?;
                known.insert(short_id, (prefs, connected));
            }
        }
    }
}

/// Publishes discovery (once per name, per connection) and state.
async fn send(
    client: &AsyncClient,
    discovery_prefix: &str,
    prefs: &PlayerPrefs,
    connected: bool,
    announced: &mut HashMap<String, String>,
) -> anyhow::Result<()> {
    if announced.get(&prefs.client_id) != Some(&prefs.name) {
        for (topic, payload) in discovery_messages(prefs, discovery_prefix) {
            client
                .publish(topic, QoS::AtLeastOnce, true, payload)
                .await?;
        }
        announced.insert(prefs.client_id.clone(), prefs.name.clone());
    }
    for (topic, payload) in state_messages(prefs, connected) {
        client
            .publish(topic, QoS::AtLeastOnce, true, payload)
            .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nidoo_core::discovery::{Parsed, TopicClass, classify, parse};
    use nidoo_core::model::{EntityCategory, Kind};

    const ID: &str = "9kDIII64_oap3z6x5vV92ffB6OvJ6WU5778PBR5u-Bs";

    fn prefs() -> PlayerPrefs {
        PlayerPrefs {
            client_id: ID.into(),
            name: "Kitchen satellite".into(),
            volume: 45,
            muted: false,
            allowed: true,
        }
    }

    #[test]
    fn discovery_parses_in_the_hub() {
        let sid = short_id(ID);
        let mut found = Vec::new();
        for (topic, payload) in discovery_messages(&prefs(), "homeassistant") {
            let TopicClass::Discovery(t) = classify(&topic, &["homeassistant".into()], &[]) else {
                panic!("{topic} is not a discovery topic")
            };
            let Ok(Parsed::Entity(e)) = parse(&t, payload.as_bytes(), &topic) else {
                panic!("{topic} did not parse: {payload}")
            };
            assert_eq!(e.device_name.as_deref(), Some("Kitchen satellite"));
            found.push((e.kind, e.unique_id.clone(), e.category));
        }
        assert_eq!(
            found,
            [
                (
                    Kind::Number,
                    format!("nidoo_sound_{sid}_volume"),
                    EntityCategory::Primary
                ),
                (
                    Kind::Switch,
                    format!("nidoo_sound_{sid}_mute"),
                    EntityCategory::Primary
                ),
                (
                    Kind::Switch,
                    format!("nidoo_sound_{sid}_allowed"),
                    EntityCategory::Config
                ),
                (
                    Kind::BinarySensor,
                    format!("nidoo_sound_{sid}_connected"),
                    EntityCategory::Diagnostic
                ),
            ]
        );
    }

    #[test]
    fn discovery_topics_and_availability() {
        let sid = short_id(ID);
        let msgs = discovery_messages(&prefs(), "homeassistant");
        assert_eq!(
            msgs[0].0,
            format!("homeassistant/number/nidoo_sound_{sid}/volume/config")
        );
        for (_, payload) in &msgs {
            let v: serde_json::Value = serde_json::from_str(payload).unwrap();
            assert_eq!(v["availability_topic"], STATUS_TOPIC);
            assert_eq!(
                v["device"]["identifiers"],
                serde_json::json!([format!("nidoo_sound_{sid}")])
            );
            assert_eq!(v["device"]["manufacturer"], "Nidoo");
        }
        let vol: serde_json::Value = serde_json::from_str(&msgs[0].1).unwrap();
        assert_eq!(
            (
                vol["min"].as_i64(),
                vol["max"].as_i64(),
                vol["mode"].as_str()
            ),
            (Some(0), Some(100), Some("slider"))
        );
        assert_eq!(
            vol["command_topic"],
            format!("nidoo-sound/{sid}/volume/set")
        );
        assert_eq!(vol["state_topic"], format!("nidoo-sound/{sid}/volume"));
    }

    #[test]
    fn state_messages_carry_values() {
        let sid = short_id(ID);
        let mut p = prefs();
        p.muted = true;
        let msgs = state_messages(&p, false);
        assert!(msgs.contains(&(format!("nidoo-sound/{sid}/volume"), "45".into())));
        assert!(msgs.contains(&(format!("nidoo-sound/{sid}/mute"), "ON".into())));
        assert!(msgs.contains(&(format!("nidoo-sound/{sid}/allowed"), "ON".into())));
        assert!(msgs.contains(&(format!("nidoo-sound/{sid}/connected"), "OFF".into())));
    }

    #[test]
    fn volume_command_parses_and_clamps() {
        let t = "nidoo-sound/0123456789ab/volume/set";
        let sid = "0123456789ab".to_owned();
        assert_eq!(
            parse_command(t, b"45"),
            Some(Control::SetVolume {
                short_id: sid.clone(),
                volume: 45
            })
        );
        assert_eq!(
            parse_command(t, b"120"),
            Some(Control::SetVolume {
                short_id: sid.clone(),
                volume: 100
            })
        );
        assert_eq!(
            parse_command(t, b"45.6"),
            Some(Control::SetVolume {
                short_id: sid.clone(),
                volume: 46
            })
        );
        assert_eq!(
            parse_command(t, b"-3"),
            Some(Control::SetVolume {
                short_id: sid,
                volume: 0
            })
        );
        assert_eq!(parse_command(t, b"abc"), None);
    }

    #[test]
    fn mute_and_allowed_commands_parse() {
        let sid = "0123456789ab".to_owned();
        assert_eq!(
            parse_command("nidoo-sound/0123456789ab/mute/set", b"ON"),
            Some(Control::SetMute {
                short_id: sid.clone(),
                muted: true
            })
        );
        assert_eq!(
            parse_command("nidoo-sound/0123456789ab/allowed/set", b"OFF"),
            Some(Control::SetAllowed {
                short_id: sid,
                allowed: false
            })
        );
        assert_eq!(
            parse_command("nidoo-sound/0123456789ab/mute/set", b"maybe"),
            None
        );
        assert_eq!(
            parse_command("nidoo-sound/0123456789ab/connected/set", b"ON"),
            None
        );
        assert_eq!(
            parse_command("elsewhere/0123456789ab/mute/set", b"ON"),
            None
        );
    }
}
