//! Pure helpers behind the alert screens: which alerts to show, how to word their age, how to
//! group them by day and which entity an alert points at.

use std::collections::BTreeMap;

use leptos::prelude::*;
use leptos_router::hooks::use_location;
use nidoo_api::{AlertChannel, AlertView, EntityView};
use time::{Date, OffsetDateTime};

use crate::components::now::unix_now;
use crate::house::House;
use crate::i18n::Lang;
use crate::i18n::security::alert_age;

/// Storage key for the "last visit to the Alerts tab" marker.
#[cfg(target_arch = "wasm32")]
const SEEN_STORAGE_KEY: &str = "oikos.alerts.last_seen";

/// `localStorage`, or `None` when it is missing, blocked or throws (private window, denied
/// permission): the badge then stops persisting, but never panics.
#[cfg(target_arch = "wasm32")]
fn storage() -> Option<web_sys::Storage> {
    web_sys::window().and_then(|w| w.local_storage().ok().flatten())
}

/// Loads the stored "last visit" marker into [`House::last_alerts_seen`] and keeps the stored
/// value in sync with it. A brand-new browser (nothing stored) starts the marker at now, so a
/// first visit never shows a badge for historic alerts.
pub fn install_alerts_badge(house: House) {
    let initial = load_initial_marker();
    house.last_alerts_seen.set(initial);
    #[cfg(target_arch = "wasm32")]
    Effect::new(move |_| {
        let value = house.last_alerts_seen.get();
        if let Some(s) = storage() {
            let _ = s.set_item(SEEN_STORAGE_KEY, &value.to_string());
        }
    });
}

#[cfg(target_arch = "wasm32")]
fn load_initial_marker() -> u64 {
    storage()
        .and_then(|s| s.get_item(SEEN_STORAGE_KEY).ok().flatten())
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or_else(unix_now)
}

#[cfg(not(target_arch = "wasm32"))]
fn load_initial_marker() -> u64 {
    unix_now()
}

/// Keeps [`House::last_alerts_seen`] at `now` whenever the current route is the Alerts page or
/// an alert detail, so the top-bar badge disappears on entry, stays cleared while the user is
/// looking, and resumes counting from the latest alert on the way out. Render it once inside
/// the router.
#[component]
pub fn TrackAlertsSeen() -> impl IntoView {
    let house = expect_context::<House>();
    let location = use_location();
    Effect::new(move |_| {
        let on_alerts = location.pathname.get().starts_with("/alerts");
        // Subscribe to arrivals too, so new alerts during the visit also move the marker.
        house.alerts.with(|_| {});
        if on_alerts {
            house.last_alerts_seen.set(unix_now());
        }
    });
}

/// How many alerts the Home section shows.
pub const RECENT_LIMIT: usize = 5;

/// The channels the Home section draws from.
pub const RECENT_CHANNELS: [AlertChannel; 3] = [
    AlertChannel::Alarm,
    AlertChannel::Security,
    AlertChannel::Camera,
];

/// The first [`RECENT_LIMIT`] alerts of the Home channels, order kept (newest first).
pub fn recent(alerts: &[AlertView]) -> Vec<AlertView> {
    alerts
        .iter()
        .filter(|a| RECENT_CHANNELS.contains(&a.channel))
        .take(RECENT_LIMIT)
        .cloned()
        .collect()
}

/// The alerts of `channel`, or all of them for `None`; order kept.
pub fn filtered(alerts: &[AlertView], channel: Option<AlertChannel>) -> Vec<AlertView> {
    alerts
        .iter()
        .filter(|a| channel.is_none_or(|c| a.channel == c))
        .cloned()
        .collect()
}

/// How many alerts were sent strictly after `last_seen_s` (both unix seconds). The Alerts tab
/// shows this count as a badge; entering `/alerts` moves `last_seen_s` to the current time.
pub fn unseen_count(alerts: &[AlertView], last_seen_s: u64) -> usize {
    alerts.iter().filter(|a| a.sent_at > last_seen_s).count()
}

/// The badge label for a count: `None` for zero (badge hidden), the number for 1..=99, `"99+"`
/// for 100 or more.
pub fn badge_text(count: usize) -> Option<String> {
    match count {
        0 => None,
        1..=99 => Some(count.to_string()),
        _ => Some("99+".to_string()),
    }
}

/// How long ago `sent_at` was at `now` (both unix seconds). A time in the future is "Just now".
pub fn relative_time(lang: Lang, sent_at: u64, now: u64) -> String {
    alert_age(lang, sent_at, now)
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum DayLabel {
    Today,
    Yesterday,
    On { year: i32, month: u8, day: u8 },
}

/// The calendar day of `unix` seconds shifted by the browser's time zone. An instant outside
/// the representable range reads as the epoch day.
fn local_date(unix: u64, tz_offset_min: i32) -> Date {
    let shifted = i64::try_from(unix)
        .unwrap_or(i64::MAX)
        .saturating_sub(i64::from(tz_offset_min) * 60);
    OffsetDateTime::from_unix_timestamp(shifted)
        .unwrap_or(OffsetDateTime::UNIX_EPOCH)
        .date()
}

/// The day heading of an alert sent at `sent_at`. `tz_offset_min` is the browser's
/// `Date.getTimezoneOffset()` (minutes to add to local time to get UTC), so local = utc - offset.
/// A time in the future is today.
pub fn day_label(sent_at: u64, now: u64, tz_offset_min: i32) -> DayLabel {
    let day = local_date(sent_at, tz_offset_min);
    let today = local_date(now, tz_offset_min);
    if day >= today {
        DayLabel::Today
    } else if today.previous_day() == Some(day) {
        DayLabel::Yesterday
    } else {
        DayLabel::On {
            year: day.year(),
            month: u8::from(day.month()),
            day: day.day(),
        }
    }
}

/// The alerts in order, one group per run of equal day labels.
pub fn group_by_day(
    alerts: &[AlertView],
    now: u64,
    tz_offset_min: i32,
) -> Vec<(DayLabel, Vec<AlertView>)> {
    let mut groups: Vec<(DayLabel, Vec<AlertView>)> = Vec::new();
    for alert in alerts {
        let label = day_label(alert.sent_at, now, tz_offset_min);
        match groups.last_mut() {
            Some((last, list)) if *last == label => list.push(alert.clone()),
            _ => groups.push((label, vec![alert.clone()])),
        }
    }
    groups
}

/// Lowercase, every run of characters outside `[a-z0-9]` becomes one `_`, no leading or trailing `_`.
pub fn slug(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('_') {
            out.push('_');
        }
    }
    while out.ends_with('_') {
        out.pop();
    }
    out
}

/// The unique id of the entity an alert's `entity_id` points at, by the app's rules in order: the
/// exact unique id; the part after the first dot as a unique id; the slug of that part equal to
/// the slug of an entity's name or unique id. Nothing for a missing or empty id.
pub fn match_entity(
    entity_id: Option<&str>,
    entities: &BTreeMap<String, EntityView>,
) -> Option<String> {
    let wanted = entity_id.filter(|id| !id.is_empty())?;
    if entities.contains_key(wanted) {
        return Some(wanted.to_string());
    }
    let tail = wanted.split_once('.').map_or(wanted, |(_, rest)| rest);
    if entities.contains_key(tail) {
        return Some(tail.to_string());
    }
    let wanted_slug = slug(tail);
    if wanted_slug.is_empty() {
        return None;
    }
    entities
        .values()
        .find(|e| slug(&e.name) == wanted_slug || slug(&e.id) == wanted_slug)
        .map(|e| e.id.clone())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use nidoo_api::{AlertChannel, AlertSeverity, AlertView, EntityView};
    use nidoo_core::model::{Capabilities, EntityCategory, EntityState, Kind, UiOverride};

    use crate::i18n::Lang;

    use super::{
        DayLabel, badge_text, day_label, filtered, group_by_day, match_entity, recent,
        relative_time, unseen_count,
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
}
