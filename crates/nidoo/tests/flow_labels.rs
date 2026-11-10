//! Flow labels and choices on the hub: the retained catalog, its edits, and labels on flows.
//! Driven without a broker: `ingest` feeds the hub, the publisher channel is inspected.

use std::sync::Arc;

use nidoo::config::Config;
use nidoo::hub::{Hub, HubError};
use nidoo_api::{ConnectionStatus, LabelEdit, WsEvent};
use nidoo_core::command::Publish;
use nidoo_core::flow::{Flow, FlowLabelCatalog, LabelColor};
use serde_json::json;
use tokio::sync::mpsc;

const LABELS_TOPIC: &str = "nidoo/flow_labels";

fn hub() -> (Arc<Hub>, mpsc::Receiver<Publish>) {
    let mut cfg = Config::default();
    cfg.commands.confirm_timeout_ms = 50;
    cfg.topics.discovery_prefixes = vec!["nidoo".to_owned()];
    let (tx, rx) = mpsc::channel(64);
    let hub = Hub::new(&cfg, tx);
    hub.set_connection(ConnectionStatus::Online);
    (hub, rx)
}

/// The house of the examples: a season choice (current: winter) and a standalone label.
fn seasons() -> serde_json::Value {
    json!({
        "labels": [
            {"id": "hiver", "name": "Hiver", "color": "blue", "choice": "saison"},
            {"id": "ete", "name": "Été", "color": "amber", "choice": "saison"},
            {"id": "vacances", "name": "Vacances", "color": "green"}
        ],
        "choices": [{"id": "saison", "name": "Saison", "current": "hiver"}]
    })
}

fn with_seasons() -> (Arc<Hub>, mpsc::Receiver<Publish>) {
    let (hub, rx) = hub();
    hub.ingest(LABELS_TOPIC, seasons().to_string().as_bytes());
    (hub, rx)
}

/// A valid flow (a manual trigger) carrying `labels`, retained under its topic.
fn ingest_flow(hub: &Hub, id: &str, enabled: bool, labels: &[&str]) {
    let flow = json!({
        "id": id, "name": id.to_uppercase(), "enabled": enabled, "labels": labels,
        "nodes": [{"id": "go", "type": "trigger.manual", "params": {}}]
    });
    hub.ingest(
        &format!("nidoo/automation/{id}"),
        flow.to_string().as_bytes(),
    );
}

/// Every publish queued so far.
fn drain(rx: &mut mpsc::Receiver<Publish>) -> Vec<Publish> {
    let mut out = Vec::new();
    while let Ok(publish) = rx.try_recv() {
        out.push(publish);
    }
    out
}

fn catalog_of(publish: &Publish) -> FlowLabelCatalog {
    assert_eq!(publish.topic, LABELS_TOPIC);
    assert!(publish.retain);
    FlowLabelCatalog::decode(publish.payload.as_bytes()).expect("catalog")
}

fn create_label(name: &str, choice: Option<&str>) -> LabelEdit {
    LabelEdit::CreateLabel {
        name: name.to_owned(),
        choice: choice.map(str::to_owned),
    }
}

#[tokio::test]
async fn create_label_publishes_the_catalog_retained() {
    let (hub, mut rx) = hub();
    let view = hub
        .edit_flow_labels(create_label("  Été ", None))
        .await
        .expect("create");
    assert_eq!(view.catalog.labels[0].id, "ete");
    assert_eq!(view.catalog.labels[0].name, "Été");
    assert_eq!(view.catalog.labels[0].color, LabelColor::Blue);
    let published = drain(&mut rx);
    assert_eq!(published.len(), 1);
    assert_eq!(catalog_of(&published[0]), view.catalog);

    let view = hub
        .edit_flow_labels(create_label("Hiver", None))
        .await
        .expect("create");
    assert_eq!(view.catalog.labels[1].id, "hiver");
    assert_eq!(view.catalog.labels[1].color, LabelColor::Amber);
    assert_eq!(hub.flow_labels(), view);

    let err = hub.edit_flow_labels(create_label(" ", None)).await;
    assert!(matches!(err, Err(HubError::Rejected(_))), "{err:?}");
}

#[tokio::test]
async fn ingest_broadcasts_flow_labels_changed() {
    let (hub, _rx) = hub();
    let mut events = hub.subscribe();
    hub.ingest(LABELS_TOPIC, seasons().to_string().as_bytes());
    let event = events.try_recv().expect("an event");
    let WsEvent::FlowLabelsChanged { labels } = event else {
        panic!("{event:?}")
    };
    assert_eq!(labels.catalog.labels.len(), 3);
    // The same document again changes nothing.
    hub.ingest(LABELS_TOPIC, seasons().to_string().as_bytes());
    assert!(events.try_recv().is_err());
}

#[tokio::test]
async fn undecodable_catalog_keeps_the_previous() {
    let (hub, _rx) = with_seasons();
    let before = hub.flow_labels();
    hub.ingest(LABELS_TOPIC, b"[");
    assert_eq!(hub.flow_labels(), before);
    hub.ingest(LABELS_TOPIC, b"");
    assert!(hub.flow_labels().catalog.labels.is_empty());
}

#[tokio::test]
async fn create_in_choice_and_counts() {
    let (hub, _rx) = hub();
    let view = hub
        .edit_flow_labels(LabelEdit::CreateChoice {
            name: "Saison".into(),
        })
        .await
        .expect("choice");
    assert_eq!(view.catalog.choices[0].id, "saison");
    assert_eq!(view.catalog.choices[0].current, None);
    hub.edit_flow_labels(create_label("Hiver", Some("saison")))
        .await
        .expect("hiver");
    let err = hub
        .edit_flow_labels(create_label("Été", Some("nope")))
        .await;
    assert!(matches!(err, Err(HubError::Rejected(_))), "{err:?}");

    ingest_flow(&hub, "a", true, &["hiver"]);
    ingest_flow(&hub, "b", true, &["hiver", "ghost"]);
    hub.ingest(
        "nidoo/automation/bad",
        br#"{"id":"bad","name":"Bad","labels":["hiver"],"nodes":"oops"}"#,
    );
    let view = hub.flow_labels();
    assert_eq!(view.catalog.labels[0].choice.as_deref(), Some("saison"));
    assert_eq!(view.counts.get("hiver"), Some(&3));
    assert_eq!(view.counts.get("ghost"), None);
}

#[tokio::test]
async fn rename_recolor_and_move() {
    let (hub, _rx) = with_seasons();
    hub.edit_flow_labels(LabelEdit::RenameLabel {
        id: "ete".into(),
        name: "Summer".into(),
    })
    .await
    .expect("rename");
    hub.edit_flow_labels(LabelEdit::RecolorLabel {
        id: "ete".into(),
        color: LabelColor::Teal,
    })
    .await
    .expect("recolor");
    hub.edit_flow_labels(LabelEdit::MoveLabel {
        id: "vacances".into(),
        choice: Some("saison".into()),
    })
    .await
    .expect("move in");
    let view = hub
        .edit_flow_labels(LabelEdit::RenameChoice {
            id: "saison".into(),
            name: "Season".into(),
        })
        .await
        .expect("rename choice");
    let ete = view.catalog.label("ete").expect("ete");
    assert_eq!((ete.name.as_str(), ete.color), ("Summer", LabelColor::Teal));
    assert_eq!(
        view.catalog.label("vacances").unwrap().choice.as_deref(),
        Some("saison")
    );
    let saison = view.catalog.choice("saison").unwrap();
    assert_eq!(saison.name, "Season");
    assert_eq!(saison.current.as_deref(), Some("hiver"));
    let err = hub
        .edit_flow_labels(LabelEdit::RenameLabel {
            id: "nope".into(),
            name: "X".into(),
        })
        .await;
    assert!(
        matches!(&err, Err(HubError::Rejected(m)) if m.contains("unknown label `nope`")),
        "{err:?}"
    );
}

#[tokio::test]
async fn move_current_label_out_clears_current() {
    let (hub, _rx) = with_seasons();
    let view = hub
        .edit_flow_labels(LabelEdit::MoveLabel {
            id: "hiver".into(),
            choice: None,
        })
        .await
        .expect("move");
    assert_eq!(view.catalog.label("hiver").unwrap().choice, None);
    assert_eq!(view.catalog.choice("saison").unwrap().current, None);
}

#[tokio::test]
async fn delete_label_strips_it_from_flows() {
    let (hub, mut rx) = with_seasons();
    ingest_flow(&hub, "a", true, &["ete", "hiver"]);
    ingest_flow(&hub, "b", false, &["ete"]);
    ingest_flow(&hub, "c", true, &["hiver"]);
    drain(&mut rx);

    let view = hub
        .edit_flow_labels(LabelEdit::DeleteLabel { id: "hiver".into() })
        .await
        .expect("delete");
    assert!(view.catalog.label("hiver").is_none());
    assert_eq!(view.catalog.choice("saison").unwrap().current, None);

    let published = drain(&mut rx);
    let topics: Vec<&str> = published.iter().map(|p| p.topic.as_str()).collect();
    assert_eq!(
        topics,
        ["nidoo/automation/a", "nidoo/automation/c", LABELS_TOPIC]
    );
    let a = Flow::decode(published[0].payload.as_bytes()).unwrap();
    assert_eq!(a.labels, ["ete"]);
    let c = Flow::decode(published[1].payload.as_bytes()).unwrap();
    assert!(c.labels.is_empty());
    assert!(hub.flow("b").unwrap().definition.labels == ["ete"]);
}

#[tokio::test]
async fn delete_choice_keeps_its_labels() {
    let (hub, _rx) = with_seasons();
    let view = hub
        .edit_flow_labels(LabelEdit::DeleteChoice {
            id: "saison".into(),
        })
        .await
        .expect("delete");
    assert!(view.catalog.choices.is_empty());
    assert_eq!(view.catalog.labels.len(), 3);
    assert!(view.catalog.labels.iter().all(|l| l.choice.is_none()));
}

#[tokio::test]
async fn edits_refused_offline() {
    let (hub, mut rx) = with_seasons();
    ingest_flow(&hub, "a", true, &["hiver"]);
    drain(&mut rx);
    hub.set_connection(ConnectionStatus::Connecting);
    let err = hub
        .edit_flow_labels(LabelEdit::DeleteLabel { id: "hiver".into() })
        .await;
    assert!(matches!(err, Err(HubError::Publish(_))), "{err:?}");
    assert!(drain(&mut rx).is_empty());
    assert!(hub.flow_labels().catalog.label("hiver").is_some());
    assert_eq!(hub.flow("a").unwrap().definition.labels, ["hiver"]);
}

#[tokio::test]
async fn catalog_topic_is_not_discovery() {
    let (hub, _rx) = hub();
    let before = hub.snapshot().entities.len();
    let changes = hub.ingest(LABELS_TOPIC, seasons().to_string().as_bytes());
    assert!(changes.is_empty(), "{changes:?}");
    assert_eq!(hub.snapshot().entities.len(), before);
    assert_eq!(hub.flow_labels().catalog.labels.len(), 3);
}

#[tokio::test]
async fn catalog_topic_is_configurable() {
    let mut cfg = Config::default();
    cfg.topics.flow_labels = "home/labels".to_owned();
    let (tx, _rx) = mpsc::channel(8);
    let hub = Hub::new(&cfg, tx);
    hub.ingest("home/labels", seasons().to_string().as_bytes());
    assert_eq!(hub.flow_labels().catalog.labels.len(), 3);
}
