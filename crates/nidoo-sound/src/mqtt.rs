//! The link to Nidoo: every satellite appears through Home Assistant MQTT
//! discovery as one device with a Volume number, a Mute switch, an Allowed
//! switch (config) and a Connected binary sensor (diagnostic). Commands come
//! back on `nidoo-sound/<short id>/<control>/set`.

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::time::Duration;

use rumqttc::{AsyncClient, Event, LastWill, MqttOptions, Packet, QoS};
use serde_json::json;
use tokio::sync::mpsc;

use crate::config::MqttSettings;
use crate::group::{Control, Published};
use crate::hold::{HOLD_TOPIC, HoldRequest, parse_hold};
use crate::http::Satellites;
use crate::speech::{SPEAK_TOPIC, SpeakRequest, Waiting, parse_speak};
use crate::state::{PlayerPrefs, short_id};

/// Topic root for this service's own topics (the hub owns `nidoo/`).
pub const TOPIC_ROOT: &str = "nidoo-sound";
/// "A sentence finished playing": `{"id", "played"}`, not retained.
pub const SPOKEN_TOPIC: &str = "nidoo-sound/spoken";
/// `online` / `offline` (last will), retained.
pub const STATUS_TOPIC: &str = "nidoo-sound/status";

/// Largest message read from the broker. `<ui_prefix>/+` brings the hub's retained
/// documents (`_types` is 17 KiB, `_home` grows with the house): over rumqttc's 10 KiB
/// default the link drops, the will marks every satellite offline and no name is read.
const MAX_INCOMING_PACKET: usize = 1024 * 1024;
/// Largest message sent; discovery configs and states stay small.
const MAX_OUTGOING_PACKET: usize = 64 * 1024;

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
                base("Music volume", "volume"),
                // `device_class: volume` tells the assistant this is the music volume.
                json!({"command_topic": command("volume"), "min": 0, "max": 100, "step": 1,
                       "mode": "slider", "icon": "mdi:volume-high", "device_class": "volume"}),
            ),
        ),
        (
            "number",
            "speech",
            with(
                base("Speech volume", "speech"),
                json!({"command_topic": command("speech"), "min": 0, "max": 100, "step": 1,
                       "mode": "slider", "icon": "mdi:account-voice"}),
            ),
        ),
        (
            "switch",
            "mute",
            with(
                base("Mute", "mute"),
                // `device_class: mute` tells the assistant this mutes the speaker.
                json!({"command_topic": command("mute"), "payload_on": "ON", "payload_off": "OFF",
                       "icon": "mdi:volume-off", "device_class": "mute"}),
            ),
        ),
        (
            "switch",
            "allowed",
            with(
                base("Allowed", "allowed"),
                json!({"command_topic": command("allowed"), "payload_on": "ON", "payload_off": "OFF",
                       "icon": "mdi:speaker-play"}),
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
        (
            "sensor",
            "address",
            with(
                base("Address", "address"),
                json!({"entity_category": "diagnostic", "icon": "mdi:ip-network"}),
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

/// Retained state values for one satellite; the address is `""` when unknown.
pub fn state_messages(
    p: &PlayerPrefs,
    connected: bool,
    address: Option<Ipv4Addr>,
) -> Vec<(String, String)> {
    let sid = short_id(&p.client_id);
    vec![
        (format!("{TOPIC_ROOT}/{sid}/volume"), p.volume.to_string()),
        (
            format!("{TOPIC_ROOT}/{sid}/speech"),
            p.speech_volume.to_string(),
        ),
        (format!("{TOPIC_ROOT}/{sid}/mute"), on_off(p.muted).into()),
        (
            format!("{TOPIC_ROOT}/{sid}/allowed"),
            on_off(p.allowed).into(),
        ),
        (
            format!("{TOPIC_ROOT}/{sid}/connected"),
            on_off(connected).into(),
        ),
        (
            format!("{TOPIC_ROOT}/{sid}/address"),
            address.map(|a| a.to_string()).unwrap_or_default(),
        ),
    ]
}

/// Empty retained payloads on every topic of one satellite: its discovery configs first (the
/// hub drops the entities), then its state values.
pub fn removal_messages(p: &PlayerPrefs, discovery_prefix: &str) -> Vec<(String, String)> {
    discovery_messages(p, discovery_prefix)
        .into_iter()
        .chain(state_messages(p, false, None))
        .map(|(topic, _)| (topic, String::new()))
        .collect()
}

/// A command from `nidoo-sound/<sid>/<volume|speech|mute|allowed|remove>/set`; `remove` takes any
/// payload.
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
        "volume" | "speech" => {
            let v: f64 = text.parse().ok()?;
            if !v.is_finite() {
                return None;
            }
            let volume = v.round().clamp(0.0, 100.0) as u8;
            Some(if object == "volume" {
                Control::SetVolume { short_id, volume }
            } else {
                Control::SetSpeech { short_id, volume }
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
        "remove" => Some(Control::Remove { short_id }),
        _ => None,
    }
}

/// Keeps the broker in step with the group: discovery + state on every
/// change, everything again on every (re)connect and when Home Assistant
/// discovery restarts (`<prefix>/status` = `online`).
/// What a message from the broker asks for.
#[derive(Debug, PartialEq, Eq)]
pub enum Incoming {
    /// Home Assistant (or the hub) came online: announce discovery again.
    HaOnline,
    Control(Control),
    Speak(SpeakRequest),
    Hold(HoldRequest),
    Ignored,
}

/// A satellite's override retained by the hub on `<ui_prefix>/nidoo_sound_<sid>_volume`
/// (its Music volume entity is its identity): `(sid, name given in Nidoo)`. `None` as the
/// name when the override is cleared or names nothing. `None` for every other topic.
pub fn parse_rename(
    topic: &str,
    payload: &[u8],
    ui_prefix: &str,
) -> Option<(String, Option<String>)> {
    let sid = topic
        .strip_prefix(ui_prefix)?
        .strip_prefix("/nidoo_sound_")?
        .strip_suffix("_volume")?;
    if sid.is_empty() || sid.contains('/') {
        return None;
    }
    // The override's `name`, as the hub's `UiOverride` reads it; nothing else matters here.
    let name = serde_json::from_slice::<serde_json::Value>(payload)
        .ok()
        .and_then(|v| v.get("name")?.as_str().map(|n| n.trim().to_owned()))
        .filter(|n| !n.is_empty());
    Some((sid.to_owned(), name))
}

/// Sorts one incoming message.
pub fn route(topic: &str, payload: &[u8], ha_status: &str) -> Incoming {
    if topic == ha_status {
        return if payload == b"online" {
            Incoming::HaOnline
        } else {
            Incoming::Ignored
        };
    }
    if topic == SPEAK_TOPIC {
        return parse_speak(payload).map_or(Incoming::Ignored, Incoming::Speak);
    }
    if topic == HOLD_TOPIC {
        return parse_hold(payload).map_or(Incoming::Ignored, Incoming::Hold);
    }
    parse_command(topic, payload).map_or(Incoming::Ignored, Incoming::Control)
}

/// The broker session: keep-alive, login, the `offline` will and the packet limits.
fn mqtt_options(cfg: &MqttSettings) -> MqttOptions {
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
    options.set_max_packet_size(MAX_INCOMING_PACKET, MAX_OUTGOING_PACKET);
    options
}

#[allow(clippy::too_many_arguments)]
pub async fn run_mqtt(
    cfg: &MqttSettings,
    discovery_prefix: &str,
    ui_prefix: &str,
    satellites: Satellites,
    mut publish: mpsc::Receiver<Published>,
    control: mpsc::Sender<Control>,
    speak: mpsc::Sender<Waiting>,
    hold: mpsc::Sender<HoldRequest>,
) -> anyhow::Result<()> {
    let (client, mut eventloop) = AsyncClient::new(mqtt_options(cfg), 256);
    let ha_status = format!("{discovery_prefix}/status");
    let mut known: HashMap<String, (PlayerPrefs, bool, Option<Ipv4Addr>)> = HashMap::new();
    let mut announced: HashMap<String, String> = HashMap::new();

    loop {
        tokio::select! {
            event = eventloop.poll() => match event {
                Ok(Event::Incoming(Packet::ConnAck(_))) => {
                    tracing::info!(host = %cfg.host, port = cfg.port, "connected to the broker");
                    client.publish(STATUS_TOPIC, QoS::AtLeastOnce, true, "online").await?;
                    client.subscribe(format!("{TOPIC_ROOT}/+/+/set"), QoS::AtLeastOnce).await?;
                    client.subscribe(ha_status.clone(), QoS::AtLeastOnce).await?;
                    client.subscribe(SPEAK_TOPIC, QoS::AtLeastOnce).await?;
                    client.subscribe(HOLD_TOPIC, QoS::AtLeastOnce).await?;
                    // The names given in Nidoo, for the HTTP API (retained: all come back now).
                    client.subscribe(format!("{ui_prefix}/+"), QoS::AtLeastOnce).await?;
                    announced.clear();
                    for (prefs, connected, address) in known.values() {
                        send(&client, discovery_prefix, prefs, *connected, *address, &mut announced).await?;
                    }
                }
                Ok(Event::Incoming(Packet::Publish(p))) if p.topic.starts_with(ui_prefix) => {
                    if let Some((sid, name)) = parse_rename(&p.topic, &p.payload, ui_prefix) {
                        tracing::debug!(%sid, ?name, "name given in Nidoo");
                        satellites.set_name(&sid, name);
                    }
                }
                Ok(Event::Incoming(Packet::Publish(p))) => match route(&p.topic, &p.payload, &ha_status) {
                    Incoming::HaOnline => {
                        announced.clear();
                        for (prefs, connected, address) in known.values() {
                            send(&client, discovery_prefix, prefs, *connected, *address, &mut announced).await?;
                        }
                    }
                    Incoming::Control(c) => {
                        tracing::debug!(?c, "command from Nidoo");
                        if control.send(c).await.is_err() {
                            return Ok(());
                        }
                    }
                    Incoming::Speak(req) => {
                        tracing::info!(id = %req.id, players = req.players.len(), "speak");
                        if speak.try_send((tokio::time::Instant::now(), req)).is_err() {
                            tracing::warn!("speech queue full, sentence dropped");
                        }
                    }
                    Incoming::Hold(req) => {
                        tracing::debug!(id = %req.id, players = req.players.len(), for_ms = req.for_ms, "hold");
                        if hold.send(req).await.is_err() {
                            return Ok(());
                        }
                    }
                    Incoming::Ignored => tracing::debug!(topic = %p.topic, "ignored message"),
                },
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!(error = %e, "broker connection lost; retrying");
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
            },
            msg = publish.recv() => {
                let (short_id, prefs, connected, address) = match msg {
                    Some(Published::Player { short_id, prefs, connected, address }) => {
                        (short_id, prefs, connected, address)
                    }
                    Some(Published::Removed { prefs }) => {
                        let sid = short_id(&prefs.client_id);
                        for (topic, payload) in removal_messages(&prefs, discovery_prefix) {
                            client.publish(topic, QoS::AtLeastOnce, true, payload).await?;
                        }
                        known.remove(&sid);
                        announced.remove(&prefs.client_id);
                        continue;
                    }
                    Some(Published::Spoken { id, played }) => {
                        let payload = json!({"id": id, "played": played}).to_string();
                        client.publish(SPOKEN_TOPIC, QoS::AtLeastOnce, false, payload).await?;
                        continue;
                    }
                    None => {
                    let _ = client.publish(STATUS_TOPIC, QoS::AtLeastOnce, true, "offline").await;
                    let _ = client.disconnect().await;
                    // Let the event loop flush the last messages.
                    let _ = tokio::time::timeout(Duration::from_secs(2), async {
                        while eventloop.poll().await.is_ok() {}
                    })
                    .await;
                    return Ok(());
                    }
                };
                send(&client, discovery_prefix, &prefs, connected, address, &mut announced).await?;
                known.insert(short_id, (prefs, connected, address));
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
    address: Option<Ipv4Addr>,
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
    for (topic, payload) in state_messages(prefs, connected, address) {
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
            speech_volume: 45,
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
                    Kind::Number,
                    format!("nidoo_sound_{sid}_speech"),
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
                    // Primary, not Config: the web UI's search and room
                    // pages leave Config entities out, and a new satellite
                    // plays nothing until this is switched on.
                    EntityCategory::Primary
                ),
                (
                    Kind::BinarySensor,
                    format!("nidoo_sound_{sid}_connected"),
                    EntityCategory::Diagnostic
                ),
                (
                    Kind::Sensor,
                    format!("nidoo_sound_{sid}_address"),
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
        let msgs = state_messages(&p, false, None);
        assert!(msgs.contains(&(format!("nidoo-sound/{sid}/volume"), "45".into())));
        assert!(msgs.contains(&(format!("nidoo-sound/{sid}/mute"), "ON".into())));
        assert!(msgs.contains(&(format!("nidoo-sound/{sid}/allowed"), "ON".into())));
        assert!(msgs.contains(&(format!("nidoo-sound/{sid}/connected"), "OFF".into())));
    }

    #[test]
    fn speech_command_parses() {
        let t = "nidoo-sound/abc/speech/set";
        assert_eq!(
            parse_command(t, b"72.6"),
            Some(Control::SetSpeech {
                short_id: "abc".into(),
                volume: 73
            })
        );
        assert_eq!(parse_command(t, b"nan"), None);
    }

    #[test]
    fn volume_and_mute_carry_their_markers() {
        let sid = short_id(ID);
        let configs: Vec<serde_json::Value> = discovery_messages(&prefs(), "homeassistant")
            .iter()
            .map(|(_, payload)| serde_json::from_str(payload).unwrap())
            .collect();
        let find = |uid: String| configs.iter().find(|c| c["unique_id"] == uid).unwrap();
        assert_eq!(
            find(format!("nidoo_sound_{sid}_volume"))["device_class"],
            "volume"
        );
        assert_eq!(
            find(format!("nidoo_sound_{sid}_mute"))["device_class"],
            "mute"
        );
        assert!(
            find(format!("nidoo_sound_{sid}_speech"))
                .get("device_class")
                .is_none()
        );
    }

    #[test]
    fn speech_entity_is_published() {
        let sid = short_id(ID);
        let mut p = prefs();
        p.speech_volume = 62;
        let configs: Vec<serde_json::Value> = discovery_messages(&p, "homeassistant")
            .iter()
            .map(|(_, payload)| serde_json::from_str(payload).unwrap())
            .collect();
        let find = |uid: String| configs.iter().find(|c| c["unique_id"] == uid).unwrap();
        let speech = find(format!("nidoo_sound_{sid}_speech"));
        assert_eq!(speech["name"], "Speech volume");
        assert_eq!(
            speech["command_topic"],
            format!("nidoo-sound/{sid}/speech/set")
        );
        assert_eq!(
            find(format!("nidoo_sound_{sid}_volume"))["name"],
            "Music volume"
        );
        assert!(
            state_messages(&p, false, None)
                .contains(&(format!("nidoo-sound/{sid}/speech"), "62".into()))
        );
        assert!(
            removal_messages(&p, "homeassistant")
                .contains(&(format!("nidoo-sound/{sid}/speech"), String::new()))
        );
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

    #[test]
    fn incoming_messages_are_routed() {
        use crate::speech::{SPEAK_TOPIC, SpeakRequest};
        let ha = "homeassistant/status";
        let speak = br#"{"id":"c","url":"/api/v1/assist/speech/c.wav","players":["abc"]}"#;
        assert_eq!(
            route(SPEAK_TOPIC, speak, ha),
            Incoming::Speak(SpeakRequest {
                id: "c".into(),
                url: "/api/v1/assist/speech/c.wav".into(),
                players: vec!["abc".into()],
            })
        );
        assert_eq!(route(SPEAK_TOPIC, b"{}", ha), Incoming::Ignored);
        assert_eq!(route(ha, b"online", ha), Incoming::HaOnline);
        assert_eq!(route(ha, b"offline", ha), Incoming::Ignored);
        assert!(matches!(
            route("nidoo-sound/abc/mute/set", b"ON", ha),
            Incoming::Control(Control::SetMute { muted: true, .. })
        ));
        assert_eq!(route("elsewhere", b"x", ha), Incoming::Ignored);
    }

    #[test]
    fn the_hubs_retained_documents_fit_in_one_packet() {
        // `<ui_prefix>/+` brings `_types` back on every connect; a packet over the limit
        // drops the link, so the satellites flap offline and keep their own names.
        let options = mqtt_options(&MqttSettings::default());
        let types = nidoo_core::types::table_json().len();
        assert!(
            options.max_packet_size() >= 16 * types,
            "limit {} for a {types}-byte type table",
            options.max_packet_size()
        );
    }

    #[test]
    fn rename_comes_from_the_volume_entity_override() {
        let ui = "nidoo/ui";
        let topic = "nidoo/ui/nidoo_sound_0123456789ab_volume";
        assert_eq!(
            parse_rename(topic, br#"{"name":" Living room ","area":"living"}"#, ui),
            Some(("0123456789ab".into(), Some("Living room".into())))
        );
        // Cleared override, override without a name, blank name: back to the satellite's own.
        for payload in [&b""[..], br#"{"area":"living"}"#, br#"{"name":"  "}"#] {
            assert_eq!(
                parse_rename(topic, payload, ui),
                Some(("0123456789ab".into(), None)),
                "{}",
                String::from_utf8_lossy(payload)
            );
        }
        for other in [
            "nidoo/ui/nidoo_sound_0123456789ab_mute",
            "nidoo/ui/light_kitchen",
            "nidoo/ui/_areas",
            "nidoo/ui/nidoo_sound__volume",
            "other/ui/nidoo_sound_0123456789ab_volume",
        ] {
            assert_eq!(parse_rename(other, br#"{"name":"X"}"#, ui), None, "{other}");
        }
    }

    #[test]
    fn hold_topic_is_routed() {
        let ha = "homeassistant/status";
        assert_eq!(
            route(
                HOLD_TOPIC,
                br#"{"id":"x","players":["abc"],"for_ms":30000}"#,
                ha
            ),
            Incoming::Hold(HoldRequest {
                id: "x".into(),
                players: vec!["abc".into()],
                for_ms: 30_000,
            })
        );
        assert_eq!(route(HOLD_TOPIC, b"nope", ha), Incoming::Ignored);
    }

    #[test]
    fn removal_clears_every_retained_topic_discovery_first() {
        let p = prefs();
        let msgs = removal_messages(&p, "homeassistant");
        let discovery: Vec<String> = discovery_messages(&p, "homeassistant")
            .into_iter()
            .map(|(t, _)| t)
            .collect();
        let state: Vec<String> = state_messages(&p, true, None)
            .into_iter()
            .map(|(t, _)| t)
            .collect();
        let topics: Vec<String> = msgs.iter().map(|(t, _)| t.clone()).collect();
        assert_eq!(topics, [discovery, state].concat());
        assert!(msgs.iter().all(|(_, payload)| payload.is_empty()));
    }

    #[test]
    fn an_empty_discovery_payload_removes_the_entity_in_the_hub() {
        for (topic, payload) in removal_messages(&prefs(), "homeassistant") {
            let TopicClass::Discovery(t) = classify(&topic, &["homeassistant".into()], &[]) else {
                continue;
            };
            assert!(
                matches!(parse(&t, payload.as_bytes(), &topic), Ok(Parsed::Removed)),
                "{topic}"
            );
        }
    }

    #[test]
    fn remove_command_parses() {
        assert_eq!(
            parse_command("nidoo-sound/0123456789ab/remove/set", b"PRESS"),
            Some(Control::Remove {
                short_id: "0123456789ab".into()
            })
        );
        assert_eq!(
            parse_command("nidoo-sound/0123456789ab/remove/set", b""),
            Some(Control::Remove {
                short_id: "0123456789ab".into()
            })
        );
    }

    #[test]
    fn address_state_is_dotted_or_empty() {
        let sid = short_id(ID);
        let topic = format!("nidoo-sound/{sid}/address");
        let with = state_messages(&prefs(), true, Some("192.0.2.10".parse().unwrap()));
        assert!(with.contains(&(topic.clone(), "192.0.2.10".into())));
        let without = state_messages(&prefs(), false, None);
        assert!(without.contains(&(topic, String::new())));
    }
}
