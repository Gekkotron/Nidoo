//! Off-peak windows: the house-local wall-clock periods billed at the off-peak rate, and the
//! dated history the hub keeps of them so a past day is split with the windows of that day,
//! and the optional device whose state says off-peak, the windows being its backup.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::flow::Scalar;
use crate::model::{Availability, StateValue};

/// One off-peak period, `start` to `end` as `HH:MM` local wall clock; `end < start` crosses
/// midnight.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct OffPeakWindow {
    pub start: String,
    pub end: String,
}

/// The windows in force from `since` (`YYYY-MM-DD`, house-local) until the next entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct OffPeakEntry {
    pub since: String,
    pub windows: Vec<OffPeakWindow>,
}

/// The entity whose state says off-peak, and the state values that mean it; any other value
/// means peak.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct OffPeakDevice {
    pub entity: String,
    pub values: Vec<String>,
}

const DAY_MINUTES: u16 = 24 * 60;

/// The minute of the day of `HH:MM` (two digits each side, 24-hour).
pub fn parse_hhmm(s: &str) -> Option<u16> {
    let (h, m) = s.split_once(':')?;
    if h.len() != 2 || m.len() != 2 || !(h.bytes().chain(m.bytes())).all(|b| b.is_ascii_digit()) {
        return None;
    }
    let (h, m): (u16, u16) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60).then_some(h * 60 + m)
}

/// The window as minute ranges `[a, b)` on one day; a window crossing midnight is two.
fn ranges(start: u16, end: u16) -> Vec<(u16, u16)> {
    if start < end {
        vec![(start, end)]
    } else {
        vec![(start, DAY_MINUTES), (0, end)]
    }
}

fn minutes(w: &OffPeakWindow) -> Option<(u16, u16)> {
    Some((parse_hhmm(&w.start)?, parse_hhmm(&w.end)?))
}

/// Checks every time parses, no window is empty and no two overlap. Messages start with
/// `off_peak` so a client can point at the field; window numbers are 1-based.
pub fn validate_windows(ws: &[OffPeakWindow]) -> Result<(), String> {
    let mut parsed = Vec::with_capacity(ws.len());
    for (i, w) in ws.iter().enumerate() {
        let (start, end) =
            minutes(w).ok_or_else(|| format!("off_peak window {}: bad time", i + 1))?;
        if start == end {
            return Err(format!("off_peak window {}: start equals end", i + 1));
        }
        parsed.push(ranges(start, end));
    }
    for i in 0..parsed.len() {
        for j in i + 1..parsed.len() {
            let overlap = parsed[i]
                .iter()
                .any(|a| parsed[j].iter().any(|b| a.0 < b.1 && b.0 < a.1));
            if overlap {
                return Err(format!("off_peak windows {} and {} overlap", i + 1, j + 1));
            }
        }
    }
    Ok(())
}

/// Whether `minute` of the day falls in one of the windows (`[start, end)`).
pub fn is_off_peak(ws: &[OffPeakWindow], minute: u16) -> bool {
    ws.iter().filter_map(minutes).any(|(start, end)| {
        ranges(start, end)
            .iter()
            .any(|&(a, b)| (a..b).contains(&minute))
    })
}

/// A reading as the text an [`OffPeakDevice`] value is matched against: `on`/`off`, the
/// shortest number (`1.0` reads `1`) or the text itself.
pub fn reading_text(value: &Scalar) -> String {
    match value {
        Scalar::Bool(true) => "on".to_owned(),
        Scalar::Bool(false) => "off".to_owned(),
        Scalar::Number(n) => n.to_string(),
        Scalar::Text(t) => t.clone(),
    }
}

/// What the device says: `Some(true)` off-peak, `Some(false)` peak, `None` when its reading is
/// not usable (offline, no value, or an empty, `unknown` or `unavailable` state), so the
/// windows decide.
pub fn device_off_peak(
    device: &OffPeakDevice,
    value: &StateValue,
    availability: Availability,
) -> Option<bool> {
    if availability == Availability::Offline {
        return None;
    }
    let text = reading_text(&Scalar::of_state(value)?);
    if matches!(text.as_str(), "" | "unknown" | "unavailable") {
        return None;
    }
    Some(device.values.contains(&text))
}

/// Off-peak at `minute` of the day: the device's answer when it has one, else the windows.
pub fn resolve_off_peak(device: Option<bool>, windows: &[OffPeakWindow], minute: u16) -> bool {
    device.unwrap_or_else(|| is_off_peak(windows, minute))
}

/// Minutes from `minute` (1 to 1440) to the next window start or end, `None` without windows.
pub fn next_boundary(windows: &[OffPeakWindow], minute: u16) -> Option<u16> {
    windows
        .iter()
        .filter_map(minutes)
        .flat_map(|(start, end)| [start, end])
        .map(|edge| (edge + DAY_MINUTES - minute - 1) % DAY_MINUTES + 1)
        .min()
}

fn since(e: &OffPeakEntry) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(&e.since, "%Y-%m-%d").ok()
}

/// The windows in force on `date`: the latest entry dated on or before it, else the oldest
/// entry (it also covers the days before it), else none. Undated entries are skipped.
pub fn windows_for(history: &[OffPeakEntry], date: NaiveDate) -> &[OffPeakWindow] {
    let dated = || history.iter().filter_map(|e| Some((since(e)?, e)));
    dated()
        .filter(|(s, _)| *s <= date)
        .max_by_key(|(s, _)| *s)
        .or_else(|| dated().min_by_key(|(s, _)| *s))
        .map(|(_, e)| e.windows.as_slice())
        .unwrap_or(&[])
}

/// The history after a save of `next` on `today`: unchanged when `next` is what is already in
/// force today, else today's entry is replaced or a new one appended, kept sorted by date.
pub fn record_change(
    history: &[OffPeakEntry],
    next: &[OffPeakWindow],
    today: NaiveDate,
) -> Vec<OffPeakEntry> {
    if windows_for(history, today) == next {
        return history.to_vec();
    }
    let mut out: Vec<OffPeakEntry> = history
        .iter()
        .filter(|e| since(e) != Some(today))
        .cloned()
        .collect();
    out.push(OffPeakEntry {
        since: today.format("%Y-%m-%d").to_string(),
        windows: next.to_vec(),
    });
    out.sort_by_key(since);
    out
}

#[cfg(test)]
mod tests;
