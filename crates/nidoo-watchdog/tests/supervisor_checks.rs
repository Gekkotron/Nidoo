use nidoo_watchdog::supervisor::{
    Baseline, Check, Ctx, GapStats, Reading, Sensitivity, baseline, evaluate, physical_range,
};

const NOW: u64 = 1_800_000_000;
const H: u64 = 3600;
const D: u64 = 24 * H;

fn probe() -> Reading {
    Reading {
        entity_id: "sensor.tank".into(),
        name: "Tank temperature".into(),
        device_class: Some("temperature".into()),
        unit: Some("°C".into()),
        value: Some(15.0),
        last_change: Some(NOW - 7 * H),
        last_report: Some(NOW - 60),
        unavailable_since: None,
        baseline: Some(Baseline {
            max_gap: 3 * H,
            changes: 400,
            rest: None,
        }),
    }
}

fn ctx() -> Ctx {
    Ctx {
        now: NOW,
        armed_at: NOW - 30 * D,
        factor: Sensitivity::Normal.factor(),
        silent_paused: false,
    }
}

fn checks(r: &Reading, c: &Ctx) -> Vec<Check> {
    evaluate(r, c).into_iter().map(|f| f.check).collect()
}

#[test]
fn tank_probe_flat_while_reporting_is_stuck() {
    let found = evaluate(&probe(), &ctx());
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].check, Check::Stuck);
    assert_eq!(found[0].evidence.for_secs, 7 * H);
    assert_eq!(found[0].evidence.usual_secs, Some(3 * H));
    assert_eq!(found[0].evidence.value, Some(15.0));
    assert_eq!(found[0].evidence.unit.as_deref(), Some("°C"));

    let fresher = Reading {
        last_change: Some(NOW - 5 * H),
        ..probe()
    };
    assert!(evaluate(&fresher, &ctx()).is_empty(), "6 h floor");
}

#[test]
fn stuck_needs_a_recent_report() {
    let quiet = Reading {
        last_report: Some(NOW - 4 * H),
        ..probe()
    };
    assert!(checks(&quiet, &ctx()).is_empty());

    let very_quiet = Reading {
        last_report: Some(NOW - 7 * H),
        ..probe()
    };
    assert_eq!(checks(&very_quiet, &ctx()), vec![Check::Silent]);
}

#[test]
fn silent_clock_starts_at_arm_time() {
    let never = Reading {
        last_report: None,
        last_change: Some(NOW - 30 * D),
        ..probe()
    };
    let recent_arm = Ctx {
        armed_at: NOW - 2 * H,
        ..ctx()
    };
    assert!(checks(&never, &recent_arm).is_empty());

    let old_arm = Ctx {
        armed_at: NOW - 7 * H,
        ..ctx()
    };
    assert_eq!(checks(&never, &old_arm), vec![Check::Silent]);

    // A report older than the arm time counts from the arm time.
    let stale = Reading {
        last_report: Some(NOW - 20 * D),
        ..never
    };
    assert!(checks(&stale, &recent_arm).is_empty());
}

#[test]
fn silent_paused_finds_nothing() {
    let overdue = Reading {
        last_report: Some(NOW - 10 * H),
        ..probe()
    };
    let paused = Ctx {
        silent_paused: true,
        ..ctx()
    };
    assert!(checks(&overdue, &paused).is_empty());

    let offline = Reading {
        unavailable_since: Some(NOW - 5 * H),
        ..overdue
    };
    assert!(checks(&offline, &paused).is_empty());
}

#[test]
fn offline_waits_the_learned_threshold() {
    // max_gap 3 h × 2 = 6 h.
    let offline = Reading {
        last_change: Some(NOW - 60),
        unavailable_since: Some(NOW - 6 * H - 60),
        ..probe()
    };
    let found = evaluate(&offline, &ctx());
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].check, Check::Silent);
    assert_eq!(found[0].evidence.for_secs, 6 * H + 60);
    assert_eq!(found[0].evidence.usual_secs, None);

    let brief = Reading {
        unavailable_since: Some(NOW - 5 * H),
        ..offline.clone()
    };
    assert!(evaluate(&brief, &ctx()).is_empty());

    // A PC switched off every night: 10 h gaps are its normal, so 12 h off is not a problem.
    let nightly = Reading {
        unavailable_since: Some(NOW - 12 * H),
        baseline: Some(Baseline {
            max_gap: 10 * H,
            changes: 400,
            rest: None,
        }),
        ..offline
    };
    assert!(evaluate(&nightly, &ctx()).is_empty());
    let days = Reading {
        unavailable_since: Some(NOW - 21 * H),
        ..nightly
    };
    assert_eq!(checks(&days, &ctx()), vec![Check::Silent]);
}

#[test]
fn offline_without_a_baseline_waits_a_day() {
    let learning = Reading {
        baseline: None,
        last_change: Some(NOW - 60),
        unavailable_since: Some(NOW - 23 * H),
        ..probe()
    };
    assert!(evaluate(&learning, &ctx()).is_empty());
    let dead = Reading {
        unavailable_since: Some(NOW - 24 * H - 60),
        ..learning
    };
    let found = evaluate(&dead, &ctx());
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].check, Check::Silent);
    assert_eq!(found[0].evidence.usual_secs, None);
}

#[test]
fn resting_value_is_not_stuck() {
    // A plug at 0 W for the whole holiday: 0 is where it rests.
    let resting = Reading {
        baseline: Some(Baseline {
            max_gap: 3 * H,
            changes: 400,
            rest: Some(15.0),
        }),
        ..probe()
    };
    assert!(evaluate(&resting, &ctx()).is_empty());

    let elsewhere = Reading {
        value: Some(15.5),
        ..resting
    };
    assert_eq!(checks(&elsewhere, &ctx()), vec![Check::Stuck]);
}

#[test]
fn rest_is_a_value_held_by_three_of_the_longest_gaps() {
    let t = NOW - 10 * D;
    let plug = GapStats {
        entity_id: "sensor.plug_power".into(),
        changes: 900,
        first: NOW - 14 * D,
        top_gaps: vec![
            (t, t + 10 * H, 0.0),
            (t + D, t + D + 9 * H, 0.0),
            (t + 2 * D, t + 2 * D + 8 * H, 3.5),
            (t + 3 * D, t + 3 * D + 7 * H, 0.0),
            (t + 4 * D, t + 4 * D + 6 * H, 3.5),
        ],
    };
    let b = baseline(&plug, NOW, &[]).unwrap();
    assert_eq!(b.max_gap, 10 * H);
    assert_eq!(b.rest, Some(0.0));

    // Two gaps at one value is a coincidence, not a rest.
    let mut probe = plug.clone();
    probe.top_gaps[3].2 = 21.5;
    assert_eq!(baseline(&probe, NOW, &[]).unwrap().rest, None);

    // A recorded problem (stuck at 0) does not teach 0 as the rest.
    let stuck = [(t + 3 * D, t + 3 * D + H)];
    assert_eq!(baseline(&plug, NOW, &stuck).unwrap().rest, None);
}

#[test]
fn no_baseline_means_only_the_range_check() {
    let flat = Reading {
        baseline: None,
        last_change: Some(NOW - 10 * D),
        ..probe()
    };
    assert!(evaluate(&flat, &ctx()).is_empty());

    let wet = Reading {
        device_class: Some("humidity".into()),
        unit: Some("%".into()),
        value: Some(140.0),
        ..flat
    };
    assert_eq!(checks(&wet, &ctx()), vec![Check::Impossible]);
}

#[test]
fn battery_flat_for_14_days_has_no_baseline() {
    let stats = GapStats {
        entity_id: "sensor.battery".into(),
        changes: 3,
        first: NOW - 14 * D,
        top_gaps: vec![
            (NOW - 14 * D, NOW - 2 * D, 100.0),
            (NOW - 2 * D, NOW - D, 100.0),
        ],
    };
    assert_eq!(baseline(&stats, NOW, &[]), None);

    let young = GapStats {
        changes: 500,
        first: NOW - 6 * D,
        ..stats
    };
    assert_eq!(baseline(&young, NOW, &[]), None);
}

#[test]
fn excluded_outage_does_not_inflate_the_baseline() {
    let t = NOW - 12 * D;
    let u = NOW - 3 * D;
    let stats = GapStats {
        entity_id: "sensor.tank".into(),
        changes: 300,
        first: NOW - 14 * D,
        top_gaps: vec![
            (t, t + 5 * D, 15.0),
            (u, u + 4 * H, 16.0),
            (u + D, u + D + 3 * H, 17.0),
            (u + 2 * D, u + 2 * D + 2 * H, 18.0),
            (u - D, u - D + H, 19.0),
        ],
    };
    assert_eq!(baseline(&stats, NOW, &[]).map(|b| b.max_gap), Some(5 * D));

    let outage = [(t + D, t + 3 * D)];
    assert_eq!(
        baseline(&stats, NOW, &outage),
        Some(Baseline {
            max_gap: 4 * H,
            changes: 300,
            rest: None,
        })
    );

    // Every gap overlaps a problem: fall back to the smallest of the five.
    let everything = [(NOW - 20 * D, NOW)];
    assert_eq!(
        baseline(&stats, NOW, &everything).map(|b| b.max_gap),
        Some(H)
    );
}

#[test]
fn ranges() {
    assert_eq!(
        physical_range(Some("temperature"), Some("°C")),
        Some((-50.0, 150.0))
    );
    assert_eq!(
        physical_range(Some("humidity"), Some("%")),
        Some((0.0, 100.0))
    );
    assert_eq!(
        physical_range(Some("battery"), Some("%")),
        Some((0.0, 100.0))
    );
    assert_eq!(
        physical_range(Some("illuminance"), Some("lx")),
        Some((0.0, f64::INFINITY))
    );
    assert_eq!(
        physical_range(Some("pressure"), Some("hPa")),
        Some((800.0, 1200.0))
    );
    assert_eq!(
        physical_range(Some("voltage"), Some("V")),
        Some((0.0, 300.0))
    );
    assert_eq!(
        physical_range(Some("co2"), Some("ppm")),
        Some((0.0, 10_000.0))
    );
    assert_eq!(physical_range(Some("power"), Some("W")), None);
    assert_eq!(physical_range(None, Some("°C")), None);
    // A unit other than the table's: not range-checked.
    assert_eq!(physical_range(Some("temperature"), Some("°F")), None);
    assert_eq!(physical_range(Some("pressure"), Some("kPa")), None);
}

#[test]
fn sensitivity_and_check_names() {
    assert_eq!(Sensitivity::Low.factor(), 3.0);
    assert_eq!(Sensitivity::Normal.factor(), 2.0);
    assert_eq!(Sensitivity::High.factor(), 1.5);
    assert_eq!(Check::Stuck.as_str(), "stuck");
    assert_eq!(Check::Silent.as_str(), "silent");
    assert_eq!(Check::Impossible.as_str(), "impossible");
    assert_eq!(
        serde_json::to_string(&Check::Impossible).unwrap(),
        "\"impossible\""
    );
}
