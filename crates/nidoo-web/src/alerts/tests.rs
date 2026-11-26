use std::collections::BTreeMap;

use nidoo_api::{AlertChannel, AlertSeverity, AlertView, EntityView};
use nidoo_core::model::{Capabilities, EntityCategory, EntityState, Kind, UiOverride};

use crate::i18n::Lang;

use super::{
    DayLabel, badge_text, day_label, filtered, group_by_day, match_entity, recent, relative_time,
    unseen_count,
};

fn alert(id: &str, channel: AlertChannel, sent_at: u64) -> AlertView {
    AlertView {
        id: id.into(),
        channel,
        severity: AlertSeverity::Info,
        title: id.into(),
        body: String::new(),
        image_url: None,
        video_url: None,
        entity_id: None,
        camera: None,
        actions: vec![],
        source: None,
        silent: false,
        resolved: false,
        resolved_at: None,
        sent_at,
    }
}

fn ids(alerts: &[AlertView]) -> Vec<&str> {
    alerts.iter().map(|a| a.id.as_str()).collect()
}

fn entity(id: &str, name: &str) -> EntityView {
    EntityView {
        id: id.into(),
        device_id: None,
        name: name.into(),
        device_name: None,
        label: None,
        kind: Kind::BinarySensor,
        component: "binary_sensor".to_string(),
        protocol_kind: Kind::BinarySensor,
        device_class: None,
        unit: None,
        icon: None,
        category: EntityCategory::Primary,
        enabled_by_default: true,
        area: "salon".into(),
        part_of: None,
        capabilities: Capabilities::default(),
        ui: UiOverride::default(),
        state: EntityState::default(),
    }
}

// 2026-10-19T00:30:00Z
const NOW: u64 = 1_792_369_800;

#[test]
fn recent_keeps_five_from_the_three_channels() {
    let alerts = [
        alert("a", AlertChannel::Energy, 9),
        alert("b", AlertChannel::Camera, 8),
        alert("c", AlertChannel::Alarm, 7),
        alert("d", AlertChannel::Climate, 6),
        alert("e", AlertChannel::Security, 5),
        alert("f", AlertChannel::Camera, 4),
        alert("g", AlertChannel::Camera, 3),
        alert("h", AlertChannel::Alarm, 2),
    ];
    assert_eq!(ids(&recent(&alerts)), ["b", "c", "e", "f", "g"]);
}

#[test]
fn filtered_by_channel_or_all() {
    let alerts = [
        alert("a", AlertChannel::Energy, 3),
        alert("b", AlertChannel::Camera, 2),
        alert("c", AlertChannel::Energy, 1),
    ];
    assert_eq!(ids(&filtered(&alerts, None)), ["a", "b", "c"]);
    assert_eq!(
        ids(&filtered(&alerts, Some(AlertChannel::Energy))),
        ["a", "c"]
    );
    assert!(filtered(&alerts, Some(AlertChannel::Misc)).is_empty());
}

#[test]
fn relative_time_steps() {
    assert_eq!(relative_time(Lang::En, NOW - 10, NOW), "Just now");
    assert_eq!(relative_time(Lang::En, NOW - 120, NOW), "2\u{A0}min ago");
    assert_eq!(relative_time(Lang::En, NOW - 7200, NOW), "2\u{A0}h ago");
    assert_eq!(relative_time(Lang::En, NOW - 172_800, NOW), "2\u{A0}d ago");
}

#[test]
fn relative_time_future_is_just_now() {
    assert_eq!(relative_time(Lang::En, NOW + 600, NOW), "Just now");
}

#[test]
fn day_label_uses_local_midnight() {
    let tz = -120;
    let now = 1_792_369_800; // 2026-10-19T00:30:00Z
    assert_eq!(day_label(1_792_362_600, now, tz), DayLabel::Today); // 2026-10-18T22:30Z
    assert_eq!(day_label(1_792_359_000, now, tz), DayLabel::Yesterday); // 2026-10-18T21:30Z
    assert_eq!(
        day_label(1_792_065_600, now, tz), // 2026-10-15T12:00Z
        DayLabel::On {
            year: 2026,
            month: 10,
            day: 15
        }
    );
}

#[test]
fn day_label_handles_future() {
    assert_eq!(day_label(NOW + 100_000, NOW, 0), DayLabel::Today);
}

#[test]
fn group_by_day_keeps_order_and_merges_runs() {
    let now = 1_792_411_200; // 2026-10-19T12:00:00Z
    let alerts = [
        alert("a", AlertChannel::Camera, now - 60),
        alert("b", AlertChannel::Camera, now - 3600),
        alert("c", AlertChannel::Camera, now - 86_400),
        alert("d", AlertChannel::Camera, now - 90_000),
        alert("e", AlertChannel::Camera, now - 600_000),
    ];
    let groups = group_by_day(&alerts, now, 0);
    let shape: Vec<(DayLabel, Vec<&str>)> = groups
        .iter()
        .map(|(label, list)| (label.clone(), ids(list)))
        .collect();
    assert_eq!(
        shape,
        vec![
            (DayLabel::Today, vec!["a", "b"]),
            (DayLabel::Yesterday, vec!["c", "d"]),
            (
                DayLabel::On {
                    year: 2026,
                    month: 10,
                    day: 12
                },
                vec!["e"]
            ),
        ]
    );
}

#[test]
fn entity_match_rules() {
    let mut entities = BTreeMap::new();
    for (id, name) in [
        ("binary_sensor.exact", "Exact"),
        ("front_door", "Porch"),
        ("0x1_garage", "Garage Door"),
    ] {
        entities.insert(id.to_string(), entity(id, name));
    }
    // The exact unique id.
    assert_eq!(
        match_entity(Some("binary_sensor.exact"), &entities).as_deref(),
        Some("binary_sensor.exact")
    );
    // The part after the first dot is a unique id.
    assert_eq!(
        match_entity(Some("binary_sensor.front_door"), &entities).as_deref(),
        Some("front_door")
    );
    // The slug of that part equals the slug of an entity name.
    assert_eq!(
        match_entity(Some("binary_sensor.garage_door"), &entities).as_deref(),
        Some("0x1_garage")
    );
    assert_eq!(match_entity(Some("binary_sensor.nope"), &entities), None);
    assert_eq!(match_entity(Some(""), &entities), None);
    assert_eq!(match_entity(None, &entities), None);
}

#[test]
fn slug_collapses_runs_and_trims() {
    assert_eq!(super::slug("  Front -- Door! "), "front_door");
    assert_eq!(super::slug("!!!"), "");
}

#[test]
fn unseen_count_takes_strictly_newer_alerts() {
    let alerts = [
        alert("a", AlertChannel::Camera, 110),
        alert("b", AlertChannel::Camera, 100),
        alert("c", AlertChannel::Camera, 90),
    ];
    // An alert whose sent_at equals the marker is treated as seen (strict >).
    assert_eq!(unseen_count(&alerts, 100), 1);
    assert_eq!(unseen_count(&alerts, 50), 3);
    assert_eq!(unseen_count(&alerts, 200), 0);
    assert_eq!(unseen_count(&[], 100), 0);
}

#[test]
fn badge_text_hides_zero_and_caps_at_99() {
    assert_eq!(badge_text(0), None);
    assert_eq!(badge_text(1).as_deref(), Some("1"));
    assert_eq!(badge_text(99).as_deref(), Some("99"));
    assert_eq!(badge_text(100).as_deref(), Some("99+"));
    assert_eq!(badge_text(9999).as_deref(), Some("99+"));
}
