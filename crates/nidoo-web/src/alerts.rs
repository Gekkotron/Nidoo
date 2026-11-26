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
const SEEN_STORAGE_KEY: &str = "nidoo.alerts.last_seen";

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
mod tests;
