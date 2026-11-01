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
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn w(start: &str, end: &str) -> OffPeakWindow {
        OffPeakWindow {
            start: start.to_string(),
            end: end.to_string(),
        }
    }

    fn entry(since: &str, windows: &[OffPeakWindow]) -> OffPeakEntry {
        OffPeakEntry {
            since: since.to_string(),
            windows: windows.to_vec(),
        }
    }

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn parses_hhmm() {
        assert_eq!(parse_hhmm("01:34"), Some(94));
        assert_eq!(parse_hhmm("00:00"), Some(0));
        assert_eq!(parse_hhmm("23:59"), Some(1439));
        for bad in ["24:00", "1:34", "01:60", "0134", "", "01:3a", "+1:34"] {
            assert_eq!(parse_hhmm(bad), None, "{bad}");
        }
    }

    #[test]
    fn off_peak_wraps_midnight() {
        let ws = [w("22:00", "06:00")];
        assert!(is_off_peak(&ws, 23 * 60));
        assert!(is_off_peak(&ws, 0));
        assert!(is_off_peak(&ws, 359));
        assert!(!is_off_peak(&ws, 360));
        assert!(!is_off_peak(&ws, 12 * 60));
    }

    #[test]
    fn off_peak_at_odd_minutes() {
        let ws = [w("01:34", "07:04"), w("12:05", "14:34")];
        assert!(!is_off_peak(&ws, 93));
        assert!(is_off_peak(&ws, 94));
        assert!(!is_off_peak(&ws, 424));
        assert!(is_off_peak(&ws, 12 * 60 + 5));
        assert!(!is_off_peak(&ws, 14 * 60 + 34));
        assert!(!is_off_peak(&[], 100));
    }

    fn device(values: &[&str]) -> OffPeakDevice {
        OffPeakDevice {
            entity: "sensor.tariff".to_owned(),
            values: values.iter().map(|v| (*v).to_owned()).collect(),
        }
    }

    fn text(s: &str) -> StateValue {
        StateValue::Text(s.to_owned())
    }

    #[test]
    fn device_value_in_list_is_off_peak() {
        let dev = device(&["OFF"]);
        assert_eq!(
            device_off_peak(&dev, &text("OFF"), Availability::Online),
            Some(true)
        );
    }

    #[test]
    fn unknown_code_is_peak() {
        let dev = device(&["OFF"]);
        assert_eq!(
            device_off_peak(&dev, &text("NEW"), Availability::Online),
            Some(false)
        );
    }

    #[test]
    fn offline_unknown_and_empty_are_not_usable() {
        let dev = device(&["OFF", "", "unknown", "unavailable"]);
        assert_eq!(
            device_off_peak(&dev, &text("OFF"), Availability::Offline),
            None
        );
        for value in [
            StateValue::Unknown,
            text(""),
            text("unknown"),
            text("unavailable"),
        ] {
            assert_eq!(
                device_off_peak(&dev, &value, Availability::Online),
                None,
                "{value:?}"
            );
        }
        // Most entities never announce availability: unknown availability is still usable.
        assert_eq!(
            device_off_peak(&dev, &text("OFF"), Availability::Unknown),
            Some(true)
        );
    }

    #[test]
    fn bool_and_number_match_their_text_form() {
        assert_eq!(
            device_off_peak(
                &device(&["on"]),
                &StateValue::OnOff(true),
                Availability::Online
            ),
            Some(true)
        );
        assert_eq!(
            device_off_peak(
                &device(&["on"]),
                &StateValue::OnOff(false),
                Availability::Online
            ),
            Some(false)
        );
        assert_eq!(
            device_off_peak(
                &device(&["1"]),
                &StateValue::Number(1.0),
                Availability::Online
            ),
            Some(true)
        );
    }

    #[test]
    fn resolve_prefers_device_then_windows() {
        let ws = [w("22:00", "06:00")];
        assert!(!resolve_off_peak(Some(false), &ws, 23 * 60));
        assert!(resolve_off_peak(None, &ws, 23 * 60));
        assert!(!resolve_off_peak(None, &[], 0));
        assert!(resolve_off_peak(Some(true), &[], 12 * 60));
    }

    #[test]
    fn next_boundary_wraps() {
        let ws = [w("22:00", "06:00")];
        assert_eq!(next_boundary(&ws, 21 * 60), Some(60));
        assert_eq!(next_boundary(&ws, 23 * 60), Some(7 * 60));
        assert_eq!(next_boundary(&ws, 6 * 60), Some(16 * 60));
        assert_eq!(next_boundary(&ws, 22 * 60), Some(8 * 60));
        assert_eq!(next_boundary(&[], 0), None);
    }

    #[test]
    fn validation_messages() {
        assert_eq!(
            validate_windows(&[w("25:00", "06:00")]).unwrap_err(),
            "off_peak window 1: bad time"
        );
        assert_eq!(
            validate_windows(&[w("06:00", "06:00")]).unwrap_err(),
            "off_peak window 1: start equals end"
        );
        assert_eq!(
            validate_windows(&[w("22:00", "06:00"), w("05:00", "07:00")]).unwrap_err(),
            "off_peak windows 1 and 2 overlap"
        );
        assert_eq!(
            validate_windows(&[
                w("01:00", "02:00"),
                w("03:00", "04:00"),
                w("01:30", "01:45")
            ])
            .unwrap_err(),
            "off_peak windows 1 and 3 overlap"
        );
        assert!(validate_windows(&[w("01:34", "07:04"), w("12:05", "14:34")]).is_ok());
        assert!(validate_windows(&[w("22:00", "06:00"), w("06:00", "07:00")]).is_ok());
        assert!(validate_windows(&[]).is_ok());
    }

    #[test]
    fn windows_for_picks_entry_in_force() {
        let h = vec![
            entry("2026-03-01", &[w("01:00", "07:00")]),
            entry("2026-10-24", &[w("02:00", "08:00")]),
        ];
        assert_eq!(windows_for(&h, d("2026-01-10")), &h[0].windows[..]);
        assert_eq!(windows_for(&h, d("2026-10-23")), &h[0].windows[..]);
        assert_eq!(windows_for(&h, d("2026-10-24")), &h[1].windows[..]);
        assert_eq!(windows_for(&h, d("2026-12-01")), &h[1].windows[..]);
        assert!(windows_for(&[], d("2026-10-24")).is_empty());
    }

    #[test]
    fn windows_for_skips_unparsable_entries() {
        let h = vec![
            entry("garbage", &[w("03:00", "04:00")]),
            entry("2026-03-01", &[w("01:00", "07:00")]),
        ];
        assert_eq!(windows_for(&h, d("2026-01-10")), &h[1].windows[..]);
    }

    #[test]
    fn record_change_rules() {
        let a = vec![w("01:00", "07:00")];
        let b = vec![w("02:00", "08:00")];
        let h1 = record_change(&[], &a, d("2026-10-20"));
        assert_eq!(h1, vec![entry("2026-10-20", &a)]);
        // Unchanged windows add no entry.
        assert_eq!(record_change(&h1, &a, d("2026-10-24")), h1);
        let h2 = record_change(&h1, &b, d("2026-10-24"));
        assert_eq!(h2, vec![entry("2026-10-20", &a), entry("2026-10-24", &b)]);
        // A second save on the same day replaces that day's entry.
        let c = vec![w("03:00", "09:00")];
        let h3 = record_change(&h2, &c, d("2026-10-24"));
        assert_eq!(h3, vec![entry("2026-10-20", &a), entry("2026-10-24", &c)]);
        // Clearing is an entry too.
        let h4 = record_change(&h1, &[], d("2026-10-25"));
        assert_eq!(h4.last().unwrap().windows, Vec::<OffPeakWindow>::new());
        assert_eq!(h4.len(), 2);
        assert!(record_change(&[], &[], d("2026-10-25")).is_empty());
    }

    #[test]
    fn serde_shape() {
        let e = entry("2026-10-24", &[w("01:34", "07:04")]);
        assert_eq!(
            serde_json::to_value(&e).unwrap(),
            serde_json::json!({"since": "2026-10-24", "windows": [{"start": "01:34", "end": "07:04"}]})
        );
    }
}
