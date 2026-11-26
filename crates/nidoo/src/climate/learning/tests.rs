use super::*;
use nidoo_climate::doc::{Block, Day, ModeRules, Setpoint};

const PARIS: Tz = chrono_tz::Europe::Paris;

fn at(s: &str) -> NaiveDateTime {
    NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap()
}

/// Unix microseconds of a Paris wall-clock time.
fn paris_micros(s: &str) -> Micros {
    let secs = local_to_unix(PARIS, at(s)).unwrap();
    micros(secs)
}

fn rules(base: f64, block_temp: Option<f64>) -> ModeRules {
    ModeRules {
        base: Setpoint::Temp(base),
        blocks: block_temp
            .map(|t| Block {
                days: vec![Day::Mon, Day::Tue, Day::Wed, Day::Thu, Day::Fri],
                from: HhMm(420),
                to: HhMm(1320),
                setpoint: Setpoint::Temp(t),
                evidence: None,
                heated_by: None,
            })
            .into_iter()
            .collect(),
    }
}

fn heat(r: ModeRules) -> Rules {
    Rules {
        heat: Some(r),
        cool: None,
    }
}

#[test]
fn samples_are_local_sorted_and_collapse_duplicate_times() {
    // October 25th 2026 is the end of summer time in Paris: 00:00 is UTC+2, 12:00 UTC+1.
    let t0 = paris_micros("2026-10-25 00:00:00");
    let t1 = paris_micros("2026-10-25 12:00:00");
    let rows = vec![
        (t1, Some(21.0), Some("heat".to_owned())),
        // The row in force before the window, clamped to its start, then the real one.
        (t0, Some(17.0), Some("heat".to_owned())),
        (t0, Some(18.0), Some("heat".to_owned())),
    ];
    let samples = samples_from_rows(PARIS, rows);
    assert_eq!(samples.len(), 2, "{samples:?}");
    assert_eq!(samples[0].at, at("2026-10-25 00:00:00"));
    assert_eq!(samples[0].target, Some(18.0), "the later row wins");
    assert_eq!(samples[1].at, at("2026-10-25 12:00:00"));
    assert_eq!(samples[1].target, Some(21.0));
}

#[test]
fn samples_keep_a_missing_mode_and_target() {
    let t = paris_micros("2026-01-10 08:30:00");
    let samples = samples_from_rows(PARIS, vec![(t, None, None)]);
    assert_eq!(samples[0].at, at("2026-01-10 08:30:00"));
    assert_eq!((samples[0].target, samples[0].mode.clone()), (None, None));
}

fn row(entity: &str, source: &str, at: Micros) -> ClimateDecisionRow {
    ClimateDecisionRow {
        at,
        zone: "z".into(),
        entity_id: entity.into(),
        mode: "heat".into(),
        setpoint: Some(19.0),
        source: source.into(),
        reason: String::new(),
        applied: true,
        adjust: None,
    }
}

#[test]
fn engine_commands_skip_deviceless_and_unknown_rows() {
    let t = paris_micros("2026-01-10 08:30:00");
    let rows = vec![
        row("climate.a", "schedule", t),
        row("", "idle", t),
        row("climate.b", "from_the_future", t),
        row("climate.c", "safety", t),
    ];
    let commands = commands_from_rows(PARIS, &rows);
    assert_eq!(commands.len(), 2);
    assert_eq!(commands[0].entity, "climate.a");
    assert_eq!(commands[0].source, Source::Schedule);
    assert_eq!(commands[0].at, at("2026-01-10 08:30:00"));
    assert_eq!(commands[0].setpoint, Some(19.0));
    assert_eq!(commands[1].source, Source::Safety);
}

#[test]
fn caps_merge_to_the_tightest() {
    let caps = |min, max, step: Option<f64>| DeviceCaps {
        hvac_modes: vec!["heat".into()],
        fan_modes: vec![],
        min_temp: min,
        max_temp: max,
        temp_step: step,
    };
    let merged = merge_caps(&[
        caps(Some(10.0), Some(30.0), Some(0.5)),
        caps(Some(12.0), Some(28.0), Some(1.0)),
        caps(None, None, None),
    ]);
    assert_eq!(merged.min_temp, Some(12.0));
    assert_eq!(merged.max_temp, Some(28.0));
    assert_eq!(merged.temp_step, Some(1.0));
    assert_eq!(merged.hvac_modes, vec!["heat".to_owned()]);
}

#[test]
fn window_is_the_whole_days_before_today() {
    // 2026-10-22 12:00 Paris (UTC+2).
    let now = local_to_unix(PARIS, at("2026-10-22 12:00:00")).unwrap();
    let (first, window) = window_of(PARIS, now, 28).unwrap();
    assert_eq!(first, NaiveDate::from_ymd_opt(2026, 9, 24).unwrap());
    assert_eq!(window.days, 28);
    assert_eq!(
        window.from,
        local_to_unix(PARIS, at("2026-09-24 00:00:00")).unwrap()
    );
    assert_eq!(
        window.to,
        local_to_unix(PARIS, at("2026-10-22 00:00:00")).unwrap()
    );
}

#[test]
fn next_learn_is_the_next_local_occurrence() {
    let tz = PARIS;
    let half_past_three = HhMm(210);
    let before = local_to_unix(tz, at("2026-10-22 01:00:00")).unwrap();
    assert_eq!(
        next_learn_at(tz, before, half_past_three),
        local_to_unix(tz, at("2026-10-22 03:30:00")).unwrap()
    );
    let after = local_to_unix(tz, at("2026-10-22 03:30:00")).unwrap();
    assert_eq!(
        next_learn_at(tz, after, half_past_three),
        local_to_unix(tz, at("2026-10-23 03:30:00")).unwrap(),
        "strictly after"
    );
    // Spring forward: 02:30 does not exist on 2026-03-29; the run moves on, never skips.
    let night = local_to_unix(tz, at("2026-03-29 00:00:00")).unwrap();
    let next = next_learn_at(tz, night, HhMm(150));
    assert!(next > night && next - night < 4 * 3600, "{next}");
}

#[test]
fn proposal_id_is_local_minute_and_zone() {
    let now = local_to_unix(PARIS, at("2026-10-23 03:30:12")).unwrap();
    assert_eq!(
        proposal_id(PARIS, now, "living"),
        "p-20261023-033012-living"
    );
}

fn settings_with(rules: Rules) -> ZoneSettings {
    ZoneSettings {
        state: ZoneState::Active,
        rules,
        ..ZoneSettings::default()
    }
}

fn learned(r: ModeRules) -> Vec<(Mode, LearnOutcome)> {
    vec![(Mode::Heat, LearnOutcome::Rules(r))]
}

#[test]
fn no_rules_yet_proposes_initial_rules() {
    let v = verdict(
        &settings_with(Rules::default()),
        &learned(rules(17.0, Some(20.5))),
        None,
        100,
    );
    let Verdict::Propose {
        kind,
        rules,
        changes,
        fingerprint,
    } = v
    else {
        panic!("{v:?}");
    };
    assert_eq!(kind, ProposalKind::Initial);
    assert!(rules.heat.is_some() && rules.cool.is_none());
    assert!(changes[0].starts_with("Base"), "{changes:?}");
    assert_eq!(fingerprint.len(), 16);
}

#[test]
fn approved_rules_only_change_when_the_learner_differs() {
    let approved = heat(rules(17.0, Some(20.5)));
    let same = verdict(
        &settings_with(approved.clone()),
        &learned(rules(17.0, Some(20.5))),
        None,
        100,
    );
    assert_eq!(same, Verdict::UpToDate);
    // A quarter of a degree is within tolerance.
    let near = verdict(
        &settings_with(approved.clone()),
        &learned(rules(17.0, Some(20.7))),
        None,
        100,
    );
    assert_eq!(near, Verdict::UpToDate);
    let moved = verdict(
        &settings_with(approved),
        &learned(rules(17.0, Some(21.5))),
        None,
        100,
    );
    let Verdict::Propose { kind, changes, .. } = moved else {
        panic!("{moved:?}");
    };
    assert_eq!(kind, ProposalKind::Change);
    assert!(changes[0].starts_with("Changed"), "{changes:?}");
}

#[test]
fn a_newly_learned_mode_is_a_change_and_keeps_the_other_mode() {
    let approved = heat(rules(17.0, None));
    let outcomes = vec![
        (Mode::Heat, LearnOutcome::Rules(rules(17.0, None))),
        (Mode::Cool, LearnOutcome::Rules(rules(26.0, None))),
    ];
    let Verdict::Propose { rules, .. } =
        verdict(&settings_with(approved.clone()), &outcomes, None, 100)
    else {
        panic!("a cool schedule appeared");
    };
    assert_eq!(rules.heat, approved.heat);
    assert!(rules.cool.is_some());
    // A mode that was not learned keeps the approved rules.
    let only_cool = vec![(Mode::Cool, LearnOutcome::Rules(rules_cool()))];
    let Verdict::Propose { rules, .. } =
        verdict(&settings_with(approved.clone()), &only_cool, None, 100)
    else {
        panic!("cool differs");
    };
    assert_eq!(rules.heat, approved.heat);
}

#[test]
fn a_mode_within_tolerance_keeps_its_approved_rules_and_a_stable_fingerprint() {
    let approved = heat(rules(17.0, Some(20.5)));
    let settings = settings_with(approved.clone());
    // Heat jitters by a quarter of a degree and some minutes; cool is new.
    let run = |jitter: f64| {
        let outcomes = vec![
            (
                Mode::Heat,
                LearnOutcome::Rules(rules(17.0 + jitter, Some(20.5 + jitter))),
            ),
            (Mode::Cool, LearnOutcome::Rules(rules_cool())),
        ];
        let Verdict::Propose {
            rules, fingerprint, ..
        } = verdict(&settings, &outcomes, None, 100)
        else {
            panic!("cool changed");
        };
        (rules, fingerprint)
    };
    let (first, fp_first) = run(0.2);
    let (second, fp_second) = run(-0.3);
    assert_eq!(
        first.heat, approved.heat,
        "sub-threshold heat is not edited"
    );
    assert_eq!(second.heat, approved.heat);
    assert_eq!(
        fp_first, fp_second,
        "the same proposal on consecutive nights"
    );
}

fn rules_cool() -> ModeRules {
    rules(27.0, None)
}

#[test]
fn not_enough_days_reports_the_best_count() {
    let outcomes = vec![
        (
            Mode::Heat,
            LearnOutcome::NotEnough {
                days: 6,
                needed: 10,
            },
        ),
        (
            Mode::Cool,
            LearnOutcome::NotEnough {
                days: 2,
                needed: 10,
            },
        ),
    ];
    assert_eq!(
        verdict(&settings_with(Rules::default()), &outcomes, None, 100),
        Verdict::NotEnough {
            days: 6,
            needed: 10
        }
    );
    // One mode with enough data is a proposal even when the other falls short.
    let mixed = vec![
        outcomes[1].1.clone(),
        LearnOutcome::Rules(rules(17.0, None)),
    ];
    let mixed: Vec<(Mode, LearnOutcome)> =
        [Mode::Cool, Mode::Heat].into_iter().zip(mixed).collect();
    assert!(matches!(
        verdict(&settings_with(Rules::default()), &mixed, None, 100),
        Verdict::Propose { .. }
    ));
    assert_eq!(
        verdict(&settings_with(Rules::default()), &[], None, 100),
        Verdict::UpToDate
    );
}

#[test]
fn a_remembered_rejection_skips_until_it_expires() {
    let proposed = learned(rules(17.0, Some(20.5)));
    let Verdict::Propose { fingerprint, .. } =
        verdict(&settings_with(Rules::default()), &proposed, None, 100)
    else {
        panic!("proposes");
    };
    let mut settings = settings_with(Rules::default());
    settings.rejected = vec![Rejection {
        fingerprint,
        until: 1_000,
    }];
    assert_eq!(verdict(&settings, &proposed, None, 999), Verdict::Rejected);
    assert!(matches!(
        verdict(&settings, &proposed, None, 1_000),
        Verdict::Propose { .. }
    ));
}

#[test]
fn the_same_pending_proposal_is_not_published_again() {
    let proposed = learned(rules(17.0, Some(20.5)));
    let settings = settings_with(Rules::default());
    let Verdict::Propose {
        kind,
        rules,
        changes,
        fingerprint,
    } = verdict(&settings, &proposed, None, 100)
    else {
        panic!("proposes");
    };
    let pending = Proposal {
        id: "p-1".into(),
        zone: "z".into(),
        kind,
        created: 100,
        window: Window {
            from: 0,
            to: 1,
            days: 1,
        },
        rules,
        changes,
        fingerprint,
    };
    assert_eq!(
        verdict(&settings, &proposed, Some(&pending), 200),
        Verdict::AlreadyPending
    );
}
