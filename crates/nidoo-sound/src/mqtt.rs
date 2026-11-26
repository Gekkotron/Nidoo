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
    let mut options = MqttOptions::new(
        cfg.client_id.clone(),
        cfg.host.as_deref().unwrap_or("localhost"),
        cfg.port,
    );
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
                    tracing::info!(host = cfg.host.as_deref().unwrap_or("localhost"), port = cfg.port, "connected to the broker");
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
mod tests;
