use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, Weekday};
use nidoo_climate::decide::DeviceCaps;
use nidoo_climate::doc::{Block, Day, Evidence, HhMm, Mode, ModeRules, Rules, Setpoint, Source};
use nidoo_climate::learn::{
    DeviceHistory, EngineCommand, LearnInput, LearnOutcome, Slot, TargetSample, diff, differs,
    fingerprint, learn, timeline,
};

const ENTITY: &str = "climate.room";

/// A Monday.
fn monday() -> NaiveDate {
    let d = NaiveDate::from_ymd_opt(2026, 10, 5).unwrap();
    assert_eq!(d.weekday(), Weekday::Mon);
    d
}

fn at(date: NaiveDate, hhmm: &str) -> NaiveDateTime {
    let (h, m) = hhmm.split_once(':').unwrap();
    date.and_hms_opt(h.parse().unwrap(), m.parse().unwrap(), 0)
        .unwrap()
}

/// One day of heat targets: `(time, target)` pairs.
fn day_of(date: NaiveDate, points: &[(&str, f64)]) -> Vec<TargetSample> {
    points
        .iter()
        .map(|(t, v)| TargetSample {
            at: at(date, t),
            target: Some(*v),
            mode: Some("heat".into()),
        })
        .collect()
}

/// One day where `None` means the device is off.
fn day_with(date: NaiveDate, mode: &str, points: &[(&str, Option<f64>)]) -> Vec<TargetSample> {
    points
        .iter()
        .map(|(t, v)| TargetSample {
            at: at(date, t),
            target: *v,
            mode: Some(v.map_or("off", |_| mode).into()),
        })
        .collect()
}

fn history(days: u32, f: impl Fn(NaiveDate) -> Vec<TargetSample>) -> Vec<DeviceHistory> {
    vec![DeviceHistory {
        entity: ENTITY.into(),
        samples: (0..days)
            .flat_map(|i| f(monday() + Duration::days(i64::from(i))))
            .collect(),
    }]
}

fn caps() -> DeviceCaps {
    DeviceCaps {
        hvac_modes: vec!["heat".into(), "cool".into(), "off".into()],
        fan_modes: vec![],
        min_temp: None,
        max_temp: None,
        temp_step: Some(0.5),
    }
}

fn input<'a>(
    devices: &'a [DeviceHistory],
    engine: &'a [EngineCommand],
    caps: &'a DeviceCaps,
    days: u32,
) -> LearnInput<'a> {
    LearnInput {
        mode: Mode::Heat,
        devices,
        engine,
        approved: None,
        first_day: monday(),
        days,
        min_days: 10,
        support: 0.6,
        frost: 7.0,
        caps,
    }
}

fn is_weekday(d: NaiveDate) -> bool {
    !matches!(d.weekday(), Weekday::Sat | Weekday::Sun)
}

/// 17 °C, 20.5 °C from 07:00 to 22:30 on weekdays, flat 17 °C on weekends.
fn standard_day(d: NaiveDate) -> Vec<TargetSample> {
    if is_weekday(d) {
        day_of(d, &[("00:00", 17.0), ("07:00", 20.5), ("22:30", 17.0)])
    } else {
        day_of(d, &[("00:00", 17.0)])
    }
}

fn rules_of(outcome: LearnOutcome) -> ModeRules {
    match outcome {
        LearnOutcome::Rules(r) => r,
        other => panic!("expected rules, got {other:?}"),
    }
}

fn weekdays() -> Vec<Day> {
    vec![Day::Mon, Day::Tue, Day::Wed, Day::Thu, Day::Fri]
}

fn block(days: &[Day], from: u16, to: u16, sp: f64) -> Block {
    Block {
        days: days.to_vec(),
        from: HhMm(from),
        to: HhMm(to),
        setpoint: Setpoint::Temp(sp),
        evidence: None,
    }
}

fn weekday_rules() -> ModeRules {
    ModeRules {
        base: Setpoint::Temp(17.0),
        blocks: vec![block(&weekdays(), 420, 1350, 20.5)],
    }
}

fn heat(m: ModeRules) -> Rules {
    Rules {
        heat: Some(m),
        cool: None,
    }
}

fn strip(mut m: ModeRules) -> ModeRules {
    for b in &mut m.blocks {
        b.evidence = None;
    }
    m
}

fn cmd(date: NaiveDate, hhmm: &str, sp: f64, source: Source) -> EngineCommand {
    EngineCommand {
        entity: ENTITY.into(),
        at: at(date, hhmm),
        setpoint: Some(sp),
        mode: "heat".into(),
        source,
        adjust: None,
    }
}

#[test]
fn not_enough_days() {
    let h = history(6, standard_day);
    let c = caps();
    assert_eq!(
        learn(&input(&h, &[], &c, 6)),
        LearnOutcome::NotEnough {
            days: 6,
            needed: 10
        }
    );
}

#[test]
fn learns_weekday_block() {
    let h = history(28, standard_day);
    let c = caps();
    let rules = rules_of(learn(&input(&h, &[], &c, 28)));
    assert_eq!(rules.base, Setpoint::Temp(17.0));
    assert_eq!(rules.blocks.len(), 1);
    let b = &rules.blocks[0];
    assert_eq!(b.days, weekdays());
    assert_eq!((b.from, b.to), (HhMm(420), HhMm(1350)));
    assert_eq!(b.setpoint, Setpoint::Temp(20.5));
    assert_eq!(b.evidence, Some(Evidence { agree: 20, of: 20 }));
    assert!(heat(rules).validate().is_ok());
}

#[test]
fn noisy_day_lowers_evidence_not_block() {
    let flat = [
        NaiveDate::from_ymd_opt(2026, 10, 7).unwrap(),
        NaiveDate::from_ymd_opt(2026, 10, 14).unwrap(),
        NaiveDate::from_ymd_opt(2026, 10, 21).unwrap(),
    ];
    let h = history(28, |d| {
        if flat.contains(&d) {
            day_of(d, &[("00:00", 17.0)])
        } else {
            standard_day(d)
        }
    });
    let c = caps();
    let rules = rules_of(learn(&input(&h, &[], &c, 28)));
    assert_eq!(rules.blocks.len(), 1);
    assert_eq!(
        rules.blocks[0].evidence,
        Some(Evidence { agree: 17, of: 20 })
    );
}

#[test]
fn weak_pattern_dropped() {
    let h = history(28, |d| {
        if is_weekday(d) && d < monday() + Duration::days(14) {
            standard_day(d)
        } else {
            day_of(d, &[("00:00", 17.0)])
        }
    });
    let c = caps();
    let rules = rules_of(learn(&input(&h, &[], &c, 28)));
    assert!(rules.blocks.is_empty(), "{rules:?}");
    assert_eq!(rules.base, Setpoint::Temp(17.0));
}

#[test]
fn off_at_night_learns_off_base() {
    let h = history(14, |d| {
        day_with(
            d,
            "heat",
            &[("00:00", None), ("10:00", Some(20.0)), ("20:00", None)],
        )
    });
    let c = caps();
    let rules = rules_of(learn(&input(&h, &[], &c, 14)));
    assert_eq!(rules.base, Setpoint::Off);
    assert_eq!(rules.blocks.len(), 1);
    let b = &rules.blocks[0];
    assert_eq!(b.days.len(), 7);
    assert_eq!((b.from, b.to), (HhMm(600), HhMm(1200)));
    assert_eq!(b.setpoint, Setpoint::Temp(20.0));
    assert_eq!(b.evidence, Some(Evidence { agree: 14, of: 14 }));
    assert!(heat(rules).validate().is_ok());
}

#[test]
fn cool_mode_blocks_below_base() {
    let h = history(14, |d| {
        day_with(
            d,
            "cool",
            &[
                ("00:00", Some(28.0)),
                ("08:00", Some(24.0)),
                ("20:00", Some(28.0)),
            ],
        )
    });
    let c = caps();
    let mut inp = input(&h, &[], &c, 14);
    inp.mode = Mode::Cool;
    let rules = rules_of(learn(&inp));
    assert_eq!(rules.base, Setpoint::Temp(28.0));
    assert_eq!(rules.blocks.len(), 1);
    let b = &rules.blocks[0];
    assert_eq!(b.days.len(), 7);
    assert_eq!((b.from, b.to), (HhMm(480), HhMm(1200)));
    assert_eq!(b.setpoint, Setpoint::Temp(24.0));
}

#[test]
fn engine_schedule_slots_count_as_agreement() {
    let h = history(28, standard_day);
    let c = caps();
    let engine: Vec<EngineCommand> = (0..28)
        .map(|i| monday() + Duration::days(i))
        .flat_map(|d| {
            if is_weekday(d) {
                vec![
                    cmd(d, "07:00", 20.5, Source::Schedule),
                    cmd(d, "22:30", 17.0, Source::Schedule),
                ]
            } else {
                vec![cmd(d, "00:00", 17.0, Source::Schedule)]
            }
        })
        .collect();
    let approved = weekday_rules();
    let mut inp = input(&h, &engine, &c, 28);
    inp.approved = Some(&approved);
    let learned = rules_of(learn(&inp));
    assert_eq!(strip(learned.clone()), approved);
    assert!(!differs(&approved, &learned));
}

#[test]
fn engine_away_slots_are_no_data() {
    let h = history(28, |d| {
        if is_weekday(d) {
            day_of(
                d,
                &[
                    ("00:00", 17.0),
                    ("07:00", 20.5),
                    ("19:00", 16.0),
                    ("22:30", 17.0),
                ],
            )
        } else {
            day_of(d, &[("00:00", 17.0)])
        }
    });
    let c = caps();
    let engine: Vec<EngineCommand> = (0..28)
        .map(|i| monday() + Duration::days(i))
        .filter(|d| is_weekday(*d))
        .flat_map(|d| {
            vec![
                cmd(d, "07:00", 20.5, Source::Schedule),
                cmd(d, "19:00", 16.0, Source::Away),
                cmd(d, "22:30", 17.0, Source::Schedule),
            ]
        })
        .collect();
    let approved = weekday_rules();
    let mut inp = input(&h, &engine, &c, 28);
    inp.approved = Some(&approved);
    let tl = timeline(&inp);
    assert_eq!(tl[0].1[19 * 4], Slot::None);
    assert_eq!(tl[0].1[12 * 4], Slot::Temp(20.5));
    assert_eq!(tl[0].1[23 * 4], Slot::Temp(17.0));
    let learned = rules_of(learn(&inp));
    assert!(!differs(&approved, &learned), "{learned:?}");
}

#[test]
fn single_hand_change_does_not_change_rules() {
    let odd = NaiveDate::from_ymd_opt(2026, 10, 14).unwrap();
    let h = history(28, |d| {
        if d == odd {
            day_of(d, &[("00:00", 17.0), ("07:00", 22.0), ("22:30", 17.0)])
        } else {
            standard_day(d)
        }
    });
    let c = caps();
    let learned = rules_of(learn(&input(&h, &[], &c, 28)));
    assert!(!differs(&weekday_rules(), &learned), "{learned:?}");
}

#[test]
fn repeated_hand_changes_propose_change() {
    let early_until = monday() + Duration::days(21);
    let h = history(28, |d| {
        if is_weekday(d) && d < early_until {
            day_of(d, &[("00:00", 17.0), ("06:30", 20.5), ("22:30", 17.0)])
        } else {
            standard_day(d)
        }
    });
    let c = caps();
    let learned = rules_of(learn(&input(&h, &[], &c, 28)));
    let approved = weekday_rules();
    assert!(differs(&approved, &learned), "{learned:?}");
    let lines = diff(Some(&heat(approved)), &heat(learned));
    assert!(
        lines.iter().any(|l| l.contains("07:00 → 06:30")),
        "{lines:?}"
    );
}

#[test]
fn heat_floor_at_frost() {
    let h = history(14, |d| {
        day_of(d, &[("00:00", 5.0), ("08:00", 20.0), ("20:00", 5.0)])
    });
    let c = caps();
    let rules = rules_of(learn(&input(&h, &[], &c, 14)));
    assert_eq!(rules.base, Setpoint::Temp(7.0));
    assert_eq!(rules.blocks.len(), 1);
    assert_eq!(rules.blocks[0].setpoint, Setpoint::Temp(20.0));
    assert_eq!(
        (rules.blocks[0].from, rules.blocks[0].to),
        (HhMm(480), HhMm(1200))
    );
}

#[test]
fn deterministic() {
    let mut h = history(28, standard_day);
    let mut second = history(28, |d| {
        day_of(d, &[("00:00", 17.0), ("08:00", 20.0), ("20:00", 17.0)])
    });
    second[0].entity = "climate.other".into();
    h.extend(second);
    let engine: Vec<EngineCommand> = (0..28)
        .map(|i| monday() + Duration::days(i))
        .map(|d| cmd(d, "07:00", 20.5, Source::Schedule))
        .collect();
    let c = caps();
    let first = learn(&input(&h, &engine, &c, 28));
    assert_eq!(first, learn(&input(&h, &engine, &c, 28)));

    let mut shuffled = h.clone();
    shuffled.reverse();
    for dev in &mut shuffled {
        dev.samples.reverse();
        dev.samples.rotate_left(7);
    }
    let mut engine2 = engine.clone();
    engine2.reverse();
    assert_eq!(first, learn(&input(&shuffled, &engine2, &c, 28)));
}

#[test]
fn base_is_weighted_by_duration_not_run_count() {
    let h = history(28, |d| {
        if is_weekday(d) {
            day_of(
                d,
                &[
                    ("00:00", 17.0),
                    ("07:00", 20.0),
                    ("08:00", 18.0),
                    ("09:00", 20.0),
                    ("10:00", 18.0),
                    ("11:00", 20.0),
                    ("12:00", 17.0),
                ],
            )
        } else {
            day_of(d, &[("00:00", 17.0)])
        }
    });
    let c = caps();
    let rules = rules_of(learn(&input(&h, &[], &c, 28)));
    assert_eq!(rules.base, Setpoint::Temp(17.0));
}

#[test]
fn split_group_merges_identical_day_blocks() {
    let h = history(28, |d| {
        if matches!(d.weekday(), Weekday::Mon | Weekday::Wed) {
            day_of(d, &[("00:00", 17.0), ("06:00", 21.0), ("22:00", 17.0)])
        } else {
            day_of(d, &[("00:00", 17.0)])
        }
    });
    let c = caps();
    let mut inp = input(&h, &[], &c, 28);
    inp.support = 0.8;
    let rules = rules_of(learn(&inp));
    assert_eq!(rules.blocks.len(), 1, "{rules:?}");
    let b = &rules.blocks[0];
    assert_eq!(b.days, vec![Day::Mon, Day::Wed]);
    assert_eq!((b.from, b.to), (HhMm(360), HhMm(1320)));
    assert_eq!(b.evidence, Some(Evidence { agree: 8, of: 8 }));
}

#[test]
fn fingerprint_ignores_evidence() {
    let a = heat(weekday_rules());
    let mut b = a.clone();
    b.heat.as_mut().unwrap().blocks[0].evidence = Some(Evidence { agree: 3, of: 4 });
    assert_eq!(fingerprint(&a), fingerprint(&b));
    assert_eq!(fingerprint(&a).len(), 16);
    assert!(
        fingerprint(&a)
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
    );
    b.heat.as_mut().unwrap().blocks[0].setpoint = Setpoint::Temp(21.0);
    assert_ne!(fingerprint(&a), fingerprint(&b));
}

#[test]
fn diff_lines() {
    let old = heat(ModeRules {
        base: Setpoint::Temp(17.0),
        blocks: vec![
            block(&weekdays(), 420, 1350, 20.5),
            block(&[Day::Sat], 540, 720, 21.0),
        ],
    });
    let new = heat(ModeRules {
        base: Setpoint::Temp(16.5),
        blocks: vec![
            block(&weekdays(), 390, 1350, 20.5),
            Block {
                evidence: Some(Evidence { agree: 5, of: 6 }),
                ..block(&[Day::Sat, Day::Sun], 480, 540, 19.0)
            },
            block(&[Day::Mon, Day::Wed], 0, 60, 18.0),
        ],
    });
    assert_eq!(
        diff(Some(&old), &new),
        vec![
            "Base: 17.0 °C → 16.5 °C",
            "Changed: weekdays start 07:00 → 06:30",
            "Removed: Sat 09:00–12:00 at 21.0 °C",
            "New: weekends 08:00–09:00 at 19.0 °C (5 of 6 days)",
            "New: Mon, Wed 00:00–01:00 at 18.0 °C",
        ]
    );
    assert!(diff(Some(&old), &old).is_empty());

    let first = Rules {
        heat: Some(ModeRules {
            base: Setpoint::Temp(17.0),
            blocks: vec![Block {
                evidence: Some(Evidence { agree: 17, of: 20 }),
                ..block(&weekdays(), 420, 1350, 20.5)
            }],
        }),
        cool: None,
    };
    assert_eq!(
        diff(None, &first),
        vec![
            "Base: 17.0 °C",
            "New: weekdays 07:00–22:30 at 20.5 °C (17 of 20 days)"
        ]
    );
    let all = heat(ModeRules {
        base: Setpoint::Off,
        blocks: vec![block(
            &[
                Day::Mon,
                Day::Tue,
                Day::Wed,
                Day::Thu,
                Day::Fri,
                Day::Sat,
                Day::Sun,
            ],
            360,
            1380,
            20.0,
        )],
    });
    assert!(diff(None, &all).contains(&"New: every day 06:00–23:00 at 20.0 °C".to_string()));
}

fn every_day() -> Vec<Day> {
    Day::ALL.to_vec()
}

/// 15 °C, 19 °C every day from 09:00 to 20:30.
fn living_rules() -> ModeRules {
    ModeRules {
        base: Setpoint::Temp(15.0),
        blocks: vec![block(&every_day(), 540, 1230, 19.0)],
    }
}

fn learn_with(h: &[DeviceHistory], approved: &ModeRules) -> ModeRules {
    let c = caps();
    let mut inp = input(h, &[], &c, 14);
    inp.approved = Some(approved);
    rules_of(learn(&inp))
}

#[test]
fn scattered_hand_changes_keep_the_approved_block() {
    // A real fortnight, Saturday to Wednesday: heated from about 09:00 to 20:30 every day, but
    // nudged between 18 and 20 °C with dips of a few hours, never the same way twice. Before
    // the fix, no daytime stretch had enough days within 0.5 °C of the median, so the approved
    // block was proposed for removal and only 30-minute slivers were kept.
    let days: [&[(&str, f64)]; 12] = [
        &[("00:00", 16.0), ("14:20", 22.0), ("15:40", 16.0)],
        &[
            ("00:10", 15.0),
            ("10:00", 18.0),
            ("13:40", 20.0),
            ("15:50", 19.5),
            ("19:00", 19.0),
            ("22:20", 15.0),
        ],
        &[
            ("08:40", 19.0),
            ("09:20", 18.5),
            ("13:30", 19.5),
            ("18:00", 19.0),
            ("19:40", 19.5),
            ("20:40", 20.0),
            ("21:50", 15.5),
        ],
        &[("08:20", 18.0), ("16:20", 19.5), ("22:20", 18.0)],
        &[
            ("00:20", 16.0),
            ("12:00", 18.0),
            ("13:10", 20.0),
            ("15:00", 19.0),
            ("16:10", 19.5),
            ("18:00", 20.0),
            ("21:15", 15.5),
        ],
        &[
            ("08:31", 18.0),
            ("09:29", 19.0),
            ("10:23", 19.5),
            ("10:47", 19.0),
            ("15:14", 18.5),
            ("18:49", 19.5),
            ("22:01", 15.5),
        ],
        &[
            ("08:55", 18.5),
            ("11:01", 19.5),
            ("14:42", 18.0),
            ("18:58", 19.0),
            ("19:58", 19.5),
            ("20:29", 18.0),
            ("21:00", 15.0),
        ],
        &[
            ("09:00", 18.0),
            ("09:45", 19.0),
            ("11:01", 18.5),
            ("15:02", 16.0),
            ("17:05", 19.5),
            ("19:07", 19.0),
            ("21:02", 15.5),
            ("21:05", 16.0),
            ("22:16", 15.0),
        ],
        &[
            ("08:50", 20.0),
            ("09:00", 19.0),
            ("11:08", 19.5),
            ("15:09", 19.0),
            ("19:45", 18.5),
            ("20:30", 15.0),
        ],
        &[
            ("09:00", 19.0),
            ("12:01", 18.0),
            ("16:01", 19.0),
            ("20:30", 15.0),
        ],
        &[
            ("09:00", 19.0),
            ("11:28", 16.0),
            ("13:17", 18.0),
            ("17:17", 19.0),
            ("18:50", 15.5),
            ("20:30", 15.0),
        ],
        &[
            ("08:20", 14.0),
            ("09:00", 19.0),
            ("10:18", 15.0),
            ("14:19", 19.0),
            ("14:32", 14.0),
            ("18:39", 19.0),
            ("19:53", 18.5),
            ("20:30", 15.0),
        ],
    ];
    let saturday = monday() + Duration::days(5);
    let h = vec![DeviceHistory {
        entity: ENTITY.into(),
        samples: days
            .iter()
            .enumerate()
            .flat_map(|(i, points)| day_of(saturday + Duration::days(i as i64), points))
            .collect(),
    }];
    let engine = [cmd(
        saturday + Duration::days(11),
        "14:32",
        14.0,
        Source::Away,
    )];
    let c = caps();
    let approved = living_rules();
    let mut inp = input(&h, &engine, &c, 12);
    inp.first_day = saturday;
    inp.approved = Some(&approved);
    let learned = rules_of(learn(&inp));
    assert!(!differs(&approved, &learned), "{learned:?}");
}

#[test]
fn consistent_hand_changes_still_move_an_approved_block() {
    let h = history(14, |d| {
        day_of(d, &[("00:00", 15.0), ("10:00", 19.0), ("20:30", 15.0)])
    });
    let approved = living_rules();
    let learned = learn_with(&h, &approved);
    let lines = diff(Some(&heat(approved)), &heat(learned.clone()));
    assert_eq!(
        lines,
        vec!["Changed: every day start 09:00 → 10:00"],
        "{learned:?}"
    );
    assert_eq!(
        learned.blocks[0].evidence,
        Some(Evidence { agree: 14, of: 14 })
    );
}

#[test]
fn an_approved_block_left_unheated_is_removed() {
    let h = history(14, |d| day_of(d, &[("00:00", 15.0)]));
    let approved = living_rules();
    let learned = learn_with(&h, &approved);
    assert!(learned.blocks.is_empty(), "{learned:?}");
    assert_eq!(learned.base, Setpoint::Temp(15.0));
}

#[test]
fn days_without_data_keep_their_approved_block() {
    // Weekdays now heat from 10:00; at weekends the device cools, so there is no heat data and
    // the weekend keeps 09:00.
    let h = history(14, |d| {
        if is_weekday(d) {
            day_of(d, &[("00:00", 15.0), ("10:00", 19.0), ("20:30", 15.0)])
        } else {
            day_with(d, "cool", &[("00:00", Some(25.0))])
        }
    });
    let c = caps();
    let approved = living_rules();
    let mut inp = input(&h, &[], &c, 14);
    inp.approved = Some(&approved);
    inp.min_days = 5;
    let learned = rules_of(learn(&inp));
    assert_eq!(
        strip(learned.clone()).blocks,
        vec![
            block(&weekdays(), 600, 1230, 19.0),
            block(&[Day::Sat, Day::Sun], 540, 1230, 19.0),
        ],
        "{learned:?}"
    );
    assert!(heat(learned).validate().is_ok());
}

#[test]
fn evidence_counts_days_that_agree_with_the_fitted_setpoint() {
    // Weekdays heat to 20 °C, 20.5 °C or 21 °C; the median is 20.5 °C, and on a device in
    // whole degrees the block is 21 °C. The days at 20 °C are a full degree away from it.
    let warm = |d: NaiveDate| match (d - monday()).num_days() % 5 {
        0 | 1 => 20.0,
        2 => 21.0,
        _ => 20.5,
    };
    let h = history(28, |d| {
        if is_weekday(d) {
            day_of(d, &[("00:00", 17.0), ("07:00", warm(d)), ("22:30", 17.0)])
        } else {
            day_of(d, &[("00:00", 17.0)])
        }
    });
    let mut c = caps();
    c.temp_step = Some(1.0);
    let rules = rules_of(learn(&input(&h, &[], &c, 28)));
    assert_eq!(rules.blocks.len(), 1, "{rules:?}");
    let b = &rules.blocks[0];
    assert_eq!(b.setpoint, Setpoint::Temp(21.0));
    let at_20 = (0..28)
        .map(|i| monday() + Duration::days(i))
        .filter(|d| is_weekday(*d) && warm(*d) == 20.0)
        .count() as u32;
    assert!(at_20 > 0);
    assert_eq!(
        b.evidence,
        Some(Evidence {
            agree: 20 - at_20,
            of: 20
        })
    );
}

#[test]
fn a_day_agrees_only_when_it_starts_and_ends_within_30_minutes() {
    // Of 20 weekdays, 4 start two hours early, 4 an hour late and 2 end an hour early; the
    // rest follow 07:00–22:30 exactly. The block stays, carried by the 10 that keep its hours.
    let h = history(28, |d| {
        if !is_weekday(d) {
            return day_of(d, &[("00:00", 17.0)]);
        }
        let (from, to) = match (d - monday()).num_days() {
            0..=3 => ("05:00", "22:30"),
            4 | 7 | 8 | 9 => ("08:00", "22:30"),
            10 | 11 => ("07:00", "21:30"),
            _ => ("07:00", "22:30"),
        };
        day_of(d, &[("00:00", 17.0), (from, 20.5), (to, 17.0)])
    });
    let c = caps();
    let mut i = input(&h, &[], &c, 28);
    i.support = 0.5;
    let rules = rules_of(learn(&i));
    assert_eq!(rules.blocks.len(), 1, "{rules:?}");
    let b = &rules.blocks[0];
    assert_eq!((b.from, b.to), (HhMm(420), HhMm(1350)));
    assert_eq!(b.evidence, Some(Evidence { agree: 10, of: 20 }));
}

#[test]
fn corrected_slots_count_as_the_target() {
    // The device ran 1 degree lower from 12:00 to 16:00: the engine's correction, not a
    // change of the schedule.
    let h = history(14, |d| {
        day_of(
            d,
            &[
                ("00:00", 17.0),
                ("09:00", 19.0),
                ("12:00", 18.0),
                ("16:00", 19.0),
                ("20:30", 17.0),
            ],
        )
    });
    let c = caps();
    let engine: Vec<EngineCommand> = (0..14)
        .map(|i| monday() + Duration::days(i))
        .flat_map(|d| {
            let mut corrected = cmd(d, "12:00", 19.0, Source::Schedule);
            corrected.adjust = Some(-1.0);
            vec![
                cmd(d, "09:00", 19.0, Source::Schedule),
                corrected,
                cmd(d, "16:00", 19.0, Source::Schedule),
            ]
        })
        .collect();
    let rules = rules_of(learn(&input(&h, &engine, &c, 14)));
    assert_eq!(rules.base, Setpoint::Temp(17.0));
    assert_eq!(rules.blocks.len(), 1);
    let b = &rules.blocks[0];
    assert_eq!(b.days.len(), 7);
    assert_eq!((b.from, b.to), (HhMm(540), HhMm(1230)));
    assert_eq!(b.setpoint, Setpoint::Temp(19.0));
}

#[test]
fn empty_slots_do_not_change_what_is_learned() {
    // The device ran 2 degrees lower from 13:00 to 15:00: the room was empty and set back.
    let h = history(14, |d| {
        day_of(
            d,
            &[
                ("00:00", 17.0),
                ("09:00", 19.0),
                ("13:00", 17.0),
                ("15:00", 19.0),
                ("20:30", 17.0),
            ],
        )
    });
    let c = caps();
    let engine: Vec<EngineCommand> = (0..14)
        .map(|i| monday() + Duration::days(i))
        .flat_map(|d| {
            vec![
                cmd(d, "09:00", 19.0, Source::Schedule),
                cmd(d, "13:00", 17.0, Source::Empty),
                cmd(d, "15:00", 19.0, Source::Schedule),
            ]
        })
        .collect();
    let rules = rules_of(learn(&input(&h, &engine, &c, 14)));
    assert_eq!(rules.base, Setpoint::Temp(17.0));
    assert_eq!(rules.blocks.len(), 1, "{:?}", rules.blocks);
    let b = &rules.blocks[0];
    assert_eq!((b.from, b.to), (HhMm(540), HhMm(1230)));
    assert_eq!(b.setpoint, Setpoint::Temp(19.0));
}
