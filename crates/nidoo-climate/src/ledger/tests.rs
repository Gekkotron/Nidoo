use super::*;
use crate::doc::{Mode, Source};

fn target(setpoint: Option<f64>, mode: &str, held: bool) -> DeviceTarget {
    DeviceTarget {
        setpoint,
        mode: mode.into(),
        held,
    }
}

fn ledger_with(setpoint: Option<f64>, mode: &str, at: u64) -> Ledger {
    let mut l = Ledger::default();
    l.record("climate.a", &target(setpoint, mode, false), at);
    l
}

fn decision(applied: bool, at: u64, entities: Vec<(&str, DeviceTarget)>) -> Decision {
    Decision {
        zone: "z".into(),
        mode: Some(Mode::Heat),
        setpoint: None,
        source: Source::Schedule,
        cause: None,
        reason: String::new(),
        reading: None,
        reading_age_s: None,
        outdoor_mean: None,
        home: true,
        holiday: false,
        applied,
        at,
        entities: entities
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
        eco_restore: BTreeMap::new(),
        adjust: None,
        heaters: BTreeMap::new(),
    }
}

#[test]
fn unknown_when_never_commanded() {
    let l = Ledger::default();
    assert_eq!(
        l.classify("climate.a", Some(20.0), Some("heat"), 0.5, 100, None),
        Observed::Unknown
    );
}

#[test]
fn matches_within_half_step() {
    let l = ledger_with(Some(20.0), "heat", 1000);
    assert_eq!(
        l.classify("climate.a", Some(20.2), Some("heat"), 0.5, 5000, None),
        Observed::Matches
    );
}

#[test]
fn none_against_some_is_a_difference() {
    let l = ledger_with(Some(20.0), "heat", 1000);
    assert_eq!(
        l.classify("climate.a", None, Some("heat"), 0.5, 5000, None),
        Observed::HandChange
    );
}

#[test]
fn echo_within_30s() {
    let l = ledger_with(Some(20.0), "heat", 1000);
    assert_eq!(
        l.classify("climate.a", Some(19.0), Some("heat"), 0.5, 1010, None),
        Observed::Echo
    );
}

#[test]
fn echo_window_ends_after_30s() {
    let l = ledger_with(Some(20.0), "heat", 1000);
    let at = |t| l.classify("climate.a", Some(19.0), Some("heat"), 0.5, t, None);
    assert_eq!(at(1000 + ECHO_WINDOW_S), Observed::Echo);
    assert_eq!(at(1000 + ECHO_WINDOW_S + 1), Observed::HandChange);
}

#[test]
fn drift_window_ends_after_120s() {
    let l = ledger_with(Some(20.0), "heat", 1000);
    let at = 5000;
    let since = |s| l.classify("climate.a", Some(19.0), Some("heat"), 0.5, at, Some(s));
    assert_eq!(since(at - DRIFT_WINDOW_S), Observed::Drift);
    assert_eq!(since(at - DRIFT_WINDOW_S - 1), Observed::HandChange);
}

#[test]
fn modes_compare_without_case() {
    let l = ledger_with(Some(20.0), "heat", 1000);
    assert_eq!(
        l.classify("climate.a", Some(20.0), Some("Heat"), 0.5, 5000, None),
        Observed::Matches
    );
    let l = ledger_with(None, "OFF", 1000);
    assert_eq!(
        l.classify("climate.a", Some(21.0), Some("off"), 0.5, 5000, None),
        Observed::Matches
    );
}

#[test]
fn drift_after_coming_back_online() {
    let l = ledger_with(Some(20.0), "heat", 1000);
    let at = 1600;
    assert_eq!(
        l.classify(
            "climate.a",
            Some(19.0),
            Some("heat"),
            0.5,
            at,
            Some(at - 60)
        ),
        Observed::Drift
    );
}

#[test]
fn hand_change_otherwise() {
    let l = ledger_with(Some(20.0), "heat", 1000);
    assert_eq!(
        l.classify("climate.a", Some(19.0), Some("heat"), 0.5, 1600, None),
        Observed::HandChange
    );
    // Online long ago: not drift.
    assert_eq!(
        l.classify("climate.a", Some(19.0), Some("heat"), 0.5, 1600, Some(1000)),
        Observed::HandChange
    );
}

#[test]
fn mode_change_is_a_change() {
    let l = ledger_with(Some(20.0), "heat", 1000);
    assert_eq!(
        l.classify("climate.a", Some(20.0), Some("off"), 0.5, 1600, None),
        Observed::HandChange
    );
}

#[test]
fn ledger_rebuilt_from_every_latest_decision() {
    let ds = [
        decision(
            true,
            1000,
            vec![
                ("climate.a", target(Some(20.0), "heat", false)),
                ("climate.held", target(Some(18.0), "heat", true)),
            ],
        ),
        decision(
            false,
            2000,
            vec![("climate.b", target(Some(21.0), "heat", false))],
        ),
    ];
    let l = Ledger::from_decisions(&ds);
    let a = l.get("climate.a").unwrap();
    assert_eq!(
        (a.setpoint, a.mode.as_str(), a.at),
        (Some(20.0), "heat", 1000)
    );
    assert!(l.get("climate.held").is_none());
    let b = l.get("climate.b").unwrap();
    assert_eq!((b.setpoint, b.at), (Some(21.0), 2000));
}

#[test]
fn a_later_decision_wins_for_the_same_entity() {
    let ds = [
        decision(
            true,
            1000,
            vec![("climate.a", target(Some(20.0), "heat", false))],
        ),
        decision(
            false,
            2000,
            vec![("climate.a", target(Some(18.0), "heat", false))],
        ),
    ];
    assert_eq!(
        Ledger::from_decisions(&ds)
            .get("climate.a")
            .unwrap()
            .setpoint,
        Some(18.0)
    );
}

#[test]
fn an_older_decision_never_overwrites_a_newer_one() {
    let newer = decision(
        false,
        2000,
        vec![("climate.a", target(Some(18.0), "heat", false))],
    );
    let older = decision(
        true,
        1000,
        vec![("climate.a", target(Some(20.0), "heat", false))],
    );
    let a = Ledger::from_decisions([&newer, &older])
        .get("climate.a")
        .cloned()
        .unwrap();
    assert_eq!((a.setpoint, a.at), (Some(18.0), 2000));
}

#[test]
fn off_ignores_the_setpoint() {
    let l = ledger_with(None, "off", 1000);
    assert_eq!(
        l.classify("climate.a", Some(21.0), Some("off"), 0.5, 5000, None),
        Observed::Matches
    );
    let l = ledger_with(Some(10.0), "off", 1000);
    assert_eq!(
        l.classify("climate.a", None, Some("off"), 0.5, 5000, None),
        Observed::Matches
    );
    // Another mode still differs.
    assert_eq!(
        l.classify("climate.a", Some(10.0), Some("heat"), 0.5, 5000, None),
        Observed::HandChange
    );
}

#[test]
fn an_acknowledged_state_has_no_echo_window() {
    let mut l = Ledger::default();
    l.acknowledge("climate.a", &target(Some(22.0), "heat", false), 1000);
    assert_eq!(
        l.classify("climate.a", Some(23.0), Some("heat"), 0.5, 1005, None),
        Observed::HandChange
    );
    assert_eq!(
        l.classify("climate.a", Some(22.0), Some("heat"), 0.5, 1005, None),
        Observed::Matches
    );
}

#[test]
fn forget_removes_the_entry() {
    let mut l = ledger_with(Some(20.0), "heat", 1000);
    l.forget("climate.a");
    assert!(l.get("climate.a").is_none());
}

#[test]
fn restart_difference_is_hand_change() {
    let ds = [decision(
        true,
        1000,
        vec![("climate.a", target(Some(20.0), "heat", false))],
    )];
    let l = Ledger::from_decisions(&ds);
    assert_eq!(
        l.classify("climate.a", Some(17.0), Some("heat"), 0.5, 5000, None),
        Observed::HandChange
    );
}

#[test]
fn hold_until_caps_at_max() {
    assert_eq!(hold_until(1000, Some(6 * 3600), 240), 1000 + 14400);
    assert_eq!(hold_until(1000, Some(3600), 240), 1000 + 3600);
    assert_eq!(hold_until(1000, None, 240), 1000 + 14400);
}
