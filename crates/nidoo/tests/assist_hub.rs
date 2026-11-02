//! The assistant on a hub driven without a broker: plans executed, pending questions, the
//! turn log and the safety range from the home settings.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use nidoo::clock;
use nidoo::config::Config;
use nidoo::hub::{Hub, HubError};
use nidoo_api::{ConnectionStatus, HomeSettings};
use nidoo_assist::view::{AssistRequest, AssistTurn, OutcomeKind, PendingReply, QuestionKind};
use nidoo_climate::doc::{OverrideOrigin, Setpoint, ZoneSettings, ZoneState};
use nidoo_core::command::Publish;
use nidoo_core::model::{Area, AreasDocument};
use tokio::sync::mpsc;

const LED_FIXTURE: &str = "light__0x60a423fffe94d1c6__light.json";
const LED_TOPIC: &str = "nidoo/light/0x60a423fffe94d1c6/light/config";
const LED_STATE_TOPIC: &str = "zigbee2mqtt/rooms/living/led_ceiling";
const LED_COMMAND_TOPIC: &str = "zigbee2mqtt/rooms/living/led_ceiling/set";

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../nidoo-core/tests/fixtures/discovery/"
        ))
        .join(name),
    )
    .expect("fixture")
}

fn hub() -> (Arc<Hub>, mpsc::Receiver<Publish>) {
    let mut cfg = Config::default();
    cfg.topics.discovery_prefixes = vec!["nidoo".to_owned()];
    cfg.climate.enabled = true;
    cfg.commands.confirm_timeout_ms = 50;
    let (tx, rx) = mpsc::channel(256);
    let hub = Hub::new(&cfg, tx);
    hub.set_connection(ConnectionStatus::Online);
    (hub, rx)
}

/// Puts the entity whose id starts with `prefix` in `area`.
async fn place(hub: &Hub, prefix: &str, area: &str) -> String {
    let id = hub
        .snapshot()
        .entities
        .into_iter()
        .find(|e| e.id.starts_with(prefix))
        .unwrap_or_else(|| panic!("no entity {prefix}"))
        .id;
    let mut patch = serde_json::Map::new();
    patch.insert("area".to_owned(), serde_json::Value::from(area));
    hub.set_override(&id, &patch).await.expect("area override");
    id
}

/// The recorded Daikin `n`, placed in `area`.
async fn feed_daikin(hub: &Hub, n: u8, area: &str) -> String {
    let name = format!("climate__daikin_192_168_1_15{n}__climate.json");
    let topic = format!("nidoo/climate/daikin_192_168_1_15{n}/climate/config");
    hub.ingest(&topic, &fixture(&name));
    place(hub, &format!("daikin_192_168_1_15{n}"), area).await
}

fn set_led(hub: &Hub, on: bool) {
    let state: &[u8] = if on {
        br#"{"state":"ON","brightness":100}"#
    } else {
        br#"{"state":"OFF","brightness":100}"#
    };
    hub.ingest(LED_STATE_TOPIC, state);
}

/// A house: Salon (`living`, the LED and a Daikin) and Bureau (`office`, a Daikin), both
/// zones active.
async fn house() -> (Arc<Hub>, mpsc::Receiver<Publish>) {
    let (hub, rx) = hub();
    hub.set_areas(AreasDocument {
        areas: vec![
            Area {
                id: "living".into(),
                name: "Salon".into(),
                ..Area::default()
            },
            Area {
                id: "office".into(),
                name: "Bureau".into(),
                ..Area::default()
            },
        ],
    })
    .await
    .expect("areas");
    hub.ingest(LED_TOPIC, &fixture(LED_FIXTURE));
    place(&hub, "0x60a423fffe94d1c6", "living").await;
    set_led(&hub, false);
    feed_daikin(&hub, 0, "living").await;
    feed_daikin(&hub, 1, "office").await;
    for zone in ["living", "office"] {
        hub.set_climate_zone(
            zone,
            ZoneSettings {
                state: ZoneState::Active,
                ..ZoneSettings::default()
            },
        )
        .expect("zone");
    }
    (hub, rx)
}

/// Plays the broker: logs every publish topic and echoes the LED's new state.
fn echo(mut rx: mpsc::Receiver<Publish>, hub: Arc<Hub>) -> Arc<Mutex<Vec<String>>> {
    let log = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&log);
    tokio::spawn(async move {
        while let Some(publish) = rx.recv().await {
            sink.lock().unwrap().push(publish.topic.clone());
            if publish.topic == LED_COMMAND_TOPIC {
                let on = serde_json::from_str::<serde_json::Value>(&publish.payload)
                    .is_ok_and(|v| v["state"] == "ON");
                set_led(&hub, on);
            }
        }
    });
    log
}

fn drain(rx: &mut mpsc::Receiver<Publish>) -> Vec<Publish> {
    let mut out = Vec::new();
    while let Ok(p) = rx.try_recv() {
        out.push(p);
    }
    out
}

fn text(s: &str) -> AssistRequest {
    AssistRequest {
        client: "page-1".into(),
        text: Some(s.into()),
        reply: None,
        simulate: false,
    }
}

fn reply(r: PendingReply) -> AssistRequest {
    AssistRequest {
        client: "page-1".into(),
        text: None,
        reply: Some(r),
        simulate: false,
    }
}

fn simulated(s: &str) -> AssistRequest {
    AssistRequest {
        simulate: true,
        ..text(s)
    }
}

async fn say(hub: &Arc<Hub>, s: &str) -> AssistTurn {
    hub.assist(text(s)).await.expect("assist")
}

fn pending_id(turn: &AssistTurn) -> String {
    turn.question.as_ref().expect("a question").pending.clone()
}

fn overrides(hub: &Hub, zone: &str) -> Vec<nidoo_climate::doc::Override> {
    hub.climate_view()
        .unwrap()
        .zones
        .into_iter()
        .find(|z| z.zone == zone)
        .expect("zone")
        .overrides
}

#[tokio::test]
async fn lights_act_and_skip_already_on() {
    let (hub, rx) = house().await;
    let log = echo(rx, Arc::clone(&hub));
    let turn = say(&hub, "allume la lumière du salon").await;
    assert_eq!(turn.outcome, OutcomeKind::Acted, "{turn:?}");
    assert!(turn.reply.contains("allumée"), "{}", turn.reply);
    assert!(log.lock().unwrap().iter().any(|t| t == LED_COMMAND_TOPIC));
    assert_eq!(turn.trace.actions.len(), 1);
    assert_eq!(turn.trace.actions[0].result, "ok");

    log.lock().unwrap().clear();
    let turn = say(&hub, "allume le salon").await;
    assert_eq!(turn.outcome, OutcomeKind::Acted);
    assert!(turn.reply.contains("déjà"), "{}", turn.reply);
    assert_eq!(turn.trace.actions[0].result, "skipped");
    assert!(!log.lock().unwrap().iter().any(|t| t == LED_COMMAND_TOPIC));
}

#[tokio::test]
async fn setpoint_override_shape() {
    let (hub, _rx) = house().await;
    let now = clock::now();
    let turn = hub
        .assist_at(text("mets le salon à 20"), now)
        .await
        .expect("assist");
    assert_eq!(turn.outcome, OutcomeKind::Acted, "{turn:?}");
    assert!(
        turn.reply.contains("20 °C") && turn.reply.contains("Salon"),
        "{}",
        turn.reply
    );
    let held = overrides(&hub, "living");
    assert_eq!(held.len(), 1);
    assert_eq!(held[0].entity, None);
    assert_eq!(held[0].setpoint, Some(Setpoint::Temp(20.0)));
    assert_eq!(held[0].mode, None);
    assert_eq!(held[0].origin, OverrideOrigin::User);
    assert_eq!(held[0].until, hub.hand_hold_until("living", now));
}

#[tokio::test]
async fn simulate_publishes_nothing() {
    let (hub, mut rx) = house().await;
    drain(&mut rx);
    for sentence in [
        "allume la lumière du salon",
        "mets le salon à 21",
        "fais-moi un café",
        "il fait combien dans le bureau ?",
    ] {
        let turn = hub.assist(simulated(sentence)).await.expect("assist");
        assert!(turn.reply.starts_with("Simulation :"), "{}", turn.reply);
        assert!(turn.simulate);
    }
    assert!(drain(&mut rx).is_empty());
    assert!(overrides(&hub, "living").is_empty());
}

#[tokio::test]
async fn simulated_confirm_yes_publishes_nothing() {
    let (hub, mut rx) = house().await;
    drain(&mut rx);
    let turn = hub.assist(simulated("mets le salon à 25")).await.unwrap();
    assert_eq!(turn.outcome, OutcomeKind::Confirming);
    let mut yes = reply(PendingReply::Yes {
        pending: pending_id(&turn),
    });
    yes.simulate = true;
    let turn = hub.assist(yes).await.unwrap();
    assert_eq!(turn.outcome, OutcomeKind::Acted);
    assert!(turn.reply.starts_with("Simulation :"), "{}", turn.reply);
    assert!(drain(&mut rx).is_empty());
    assert!(overrides(&hub, "living").is_empty());
}

#[tokio::test]
async fn confirm_yes_then_acts() {
    let (hub, _rx) = house().await;
    let turn = say(&hub, "mets le salon à 25").await;
    assert_eq!(turn.outcome, OutcomeKind::Confirming);
    let question = turn.question.clone().unwrap();
    assert_eq!(question.kind, QuestionKind::Confirm);
    assert!(question.choices.is_empty());
    assert!(overrides(&hub, "living").is_empty());
    let turn = hub
        .assist(reply(PendingReply::Yes {
            pending: question.pending,
        }))
        .await
        .unwrap();
    assert_eq!(turn.outcome, OutcomeKind::Acted, "{turn:?}");
    assert_eq!(turn.text, "Oui");
    assert_eq!(
        overrides(&hub, "living")[0].setpoint,
        Some(Setpoint::Temp(25.0))
    );
}

#[tokio::test]
async fn confirm_no_does_nothing() {
    let (hub, _rx) = house().await;
    let turn = say(&hub, "mets le salon à 25").await;
    let turn = hub
        .assist(reply(PendingReply::No {
            pending: pending_id(&turn),
        }))
        .await
        .unwrap();
    assert_eq!(turn.outcome, OutcomeKind::Answered);
    assert!(overrides(&hub, "living").is_empty());
}

#[tokio::test]
async fn typed_oui_answers_confirm() {
    let (hub, _rx) = house().await;
    say(&hub, "mets le salon à 25").await;
    let turn = say(&hub, "oui").await;
    assert_eq!(turn.outcome, OutcomeKind::Acted, "{turn:?}");
    assert_eq!(
        overrides(&hub, "living")[0].setpoint,
        Some(Setpoint::Temp(25.0))
    );
}

#[tokio::test]
async fn choice_reply_acts() {
    let (hub, _rx) = house().await;
    let turn = say(&hub, "mets le chauffage à 20").await;
    assert_eq!(turn.outcome, OutcomeKind::Asked);
    let question = turn.question.clone().unwrap();
    assert_eq!(question.kind, QuestionKind::Choice);
    // In the house's room order.
    let order: Vec<String> = hub
        .snapshot()
        .areas
        .into_iter()
        .map(|a| a.name)
        .filter(|n| n == "Salon" || n == "Bureau")
        .collect();
    assert_eq!(question.choices, order);
    let bureau = order.iter().position(|n| n == "Bureau").unwrap();
    let turn = hub
        .assist(reply(PendingReply::Choice {
            pending: question.pending,
            choice: bureau,
        }))
        .await
        .unwrap();
    assert_eq!(turn.outcome, OutcomeKind::Acted, "{turn:?}");
    assert_eq!(turn.text, "Bureau");
    assert_eq!(overrides(&hub, "office").len(), 1);
    assert!(overrides(&hub, "living").is_empty());
}

#[tokio::test]
async fn typed_choice_answers_the_question() {
    let (hub, _rx) = house().await;
    say(&hub, "mets le chauffage à 20").await;
    let turn = say(&hub, "le bureau").await;
    assert_eq!(turn.outcome, OutcomeKind::Acted, "{turn:?}");
    assert_eq!(overrides(&hub, "office").len(), 1);
}

#[tokio::test]
async fn pending_expires_after_60s() {
    let (hub, _rx) = house().await;
    let now = clock::now();
    let turn = hub
        .assist_at(text("mets le salon à 25"), now)
        .await
        .unwrap();
    let late = hub
        .assist_at(
            reply(PendingReply::Yes {
                pending: pending_id(&turn),
            }),
            now + 61,
        )
        .await;
    assert!(
        matches!(&late, Err(HubError::Rejected(m)) if m == "question expired"),
        "{late:?}"
    );
    let turn = hub.assist_at(text("oui"), now + 62).await.unwrap();
    assert_eq!(turn.outcome, OutcomeKind::NotUnderstood);
    assert!(overrides(&hub, "living").is_empty());
}

#[tokio::test]
async fn new_sentence_drops_pending() {
    let (hub, _rx) = house().await;
    let asked = say(&hub, "mets le salon à 25").await;
    let turn = say(&hub, "il fait combien dehors ?").await;
    assert_eq!(turn.outcome, OutcomeKind::Answered);
    let late = hub
        .assist(reply(PendingReply::Yes {
            pending: pending_id(&asked),
        }))
        .await;
    assert!(matches!(late, Err(HubError::Rejected(_))), "{late:?}");
    let turn = say(&hub, "oui").await;
    assert_eq!(turn.outcome, OutcomeKind::NotUnderstood);
    assert!(overrides(&hub, "living").is_empty());
}

#[tokio::test]
async fn pending_is_per_client() {
    let (hub, _rx) = house().await;
    say(&hub, "mets le salon à 25").await;
    let other = AssistRequest {
        client: "page-2".into(),
        ..text("oui")
    };
    let turn = hub.assist(other).await.unwrap();
    assert_eq!(turn.outcome, OutcomeKind::NotUnderstood);
    assert!(overrides(&hub, "living").is_empty());
}

#[tokio::test]
async fn empty_request_is_rejected() {
    let (hub, _rx) = house().await;
    let empty = AssistRequest {
        client: "page-1".into(),
        text: None,
        reply: None,
        simulate: false,
    };
    assert!(matches!(
        hub.assist(empty).await,
        Err(HubError::Rejected(_))
    ));
    let blank = text("   ");
    assert!(matches!(
        hub.assist(blank).await,
        Err(HubError::Rejected(_))
    ));
}

#[tokio::test]
async fn mark_wrong_flags_turn() {
    let (hub, _rx) = house().await;
    let turn = say(&hub, "il fait combien dans le bureau ?").await;
    hub.assist_mark_wrong(&turn.id).await.expect("mark");
    let turns = hub.assist_turns(10).await;
    assert!(!turns.kept);
    assert_eq!(turns.turns[0].id, turn.id);
    assert!(turns.turns[0].wrong);
    assert!(matches!(
        hub.assist_mark_wrong("nope").await,
        Err(HubError::UnknownTurn(_))
    ));
}

#[tokio::test]
async fn turns_are_newest_first() {
    let (hub, _rx) = house().await;
    let a = say(&hub, "fais-moi un café").await;
    let b = say(&hub, "il fait combien dehors ?").await;
    let turns = hub.assist_turns(10).await.turns;
    assert_eq!(
        turns.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
        [b.id.as_str(), a.id.as_str()]
    );
    assert_eq!(hub.assist_turns(1).await.turns.len(), 1);
}

#[tokio::test]
async fn range_from_home_settings() {
    let (hub, _rx) = house().await;
    hub.set_home_settings(HomeSettings {
        assist_max_c: Some(25.0),
        ..HomeSettings::default()
    })
    .expect("settings");
    let turn = say(&hub, "mets le salon à 24").await;
    assert_eq!(turn.outcome, OutcomeKind::Acted, "{turn:?}");
    let bad = HomeSettings {
        assist_min_c: Some(22.0),
        assist_max_c: Some(20.0),
        ..HomeSettings::default()
    };
    assert!(matches!(
        hub.set_home_settings(bad),
        Err(HubError::Rejected(_))
    ));
    let bad = HomeSettings {
        assist_max_c: Some(40.0),
        ..HomeSettings::default()
    };
    assert!(matches!(
        hub.set_home_settings(bad),
        Err(HubError::Rejected(_))
    ));
}

#[tokio::test]
async fn without_db_history_is_memory_only() {
    let (hub, _rx) = house().await;
    let first = say(&hub, "fais-moi un café").await;
    for _ in 0..100 {
        say(&hub, "il fait combien dehors ?").await;
    }
    let turns = hub.assist_turns(200).await;
    assert!(!turns.kept);
    assert_eq!(turns.turns.len(), 100);
    assert!(turns.turns.iter().all(|t| t.id != first.id));
}
