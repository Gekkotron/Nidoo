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
