use super::*;

#[test]
fn setpoint_serde() {
    assert_eq!(serde_json::to_string(&Setpoint::Off).unwrap(), "\"off\"");
    assert_eq!(
        serde_json::to_string(&Setpoint::Temp(20.5)).unwrap(),
        "20.5"
    );
    assert_eq!(
        serde_json::from_str::<Setpoint>("20.5").unwrap(),
        Setpoint::Temp(20.5)
    );
    assert_eq!(
        serde_json::from_str::<Setpoint>("\"off\"").unwrap(),
        Setpoint::Off
    );
    assert!(serde_json::from_str::<Setpoint>("\"on\"").is_err());
}

#[test]
fn hhmm_serde_and_bounds() {
    assert_eq!(
        serde_json::from_str::<HhMm>("\"07:30\"").unwrap(),
        HhMm(450)
    );
    assert_eq!(
        serde_json::from_str::<HhMm>("\"24:00\"").unwrap(),
        HhMm(1440)
    );
    assert_eq!(serde_json::to_string(&HhMm(450)).unwrap(), "\"07:30\"");
    assert!(serde_json::from_str::<HhMm>("\"24:01\"").is_err());
    assert!(serde_json::from_str::<HhMm>("\"7:5\"").is_err());
    assert!(serde_json::from_str::<HhMm>("\"12:60\"").is_err());
}

#[test]
fn day_from_chrono() {
    assert_eq!(Day::from_chrono(chrono::Weekday::Mon), Day::Mon);
    assert_eq!(Day::from_chrono(chrono::Weekday::Sun), Day::Sun);
}

#[test]
fn house_defaults_match_spec() {
    let h: HouseSettings = serde_json::from_str("{}").unwrap();
    assert_eq!(
        (h.frost, h.overheat, h.overcool, h.stale_after_min),
        (7.0, 30.0, 16.0, 60)
    );
    assert_eq!(
        (h.heat_below, h.cool_above, h.away_grace_min),
        (16.0, 24.0, 15)
    );
    assert_eq!(
        (h.learn_days, h.learn_min_days, h.learn_support, h.learn_at),
        (28, 10, 0.6, HhMm(210))
    );
    assert_eq!(h.season, Season::Auto);
    assert!(h.validate().is_ok());
}

#[test]
fn zone_defaults_match_spec() {
    let z: ZoneSettings = serde_json::from_str("{}").unwrap();
    assert_eq!(z.state, ZoneState::Pending);
    assert_eq!(z.manual_hold_max_min, 240);
    assert!(z.rules.is_empty());
    assert!(z.validate().is_ok());
}

fn block(days: &[Day], from: u16, to: u16, sp: f64) -> Block {
    Block {
        days: days.to_vec(),
        from: HhMm(from),
        to: HhMm(to),
        setpoint: Setpoint::Temp(sp),
        evidence: None,
        heated_by: None,
    }
}

fn heat(blocks: Vec<Block>) -> Rules {
    Rules {
        heat: Some(ModeRules {
            base: Setpoint::Temp(17.0),
            blocks,
        }),
        cool: None,
    }
}

#[test]
fn rules_validate_rejects_bad_blocks() {
    let bad = [
        heat(vec![block(&[Day::Mon], 600, 600, 20.0)]),
        heat(vec![block(&[Day::Mon], 700, 600, 20.0)]),
        heat(vec![block(&[], 600, 700, 20.0)]),
        heat(vec![
            block(&[Day::Mon, Day::Tue], 600, 700, 20.0),
            block(&[Day::Tue], 650, 800, 21.0),
        ]),
        heat(vec![block(&[Day::Mon], 600, 700, 41.0)]),
        heat(vec![block(&[Day::Mon], 600, 700, -1.0)]),
        heat(vec![block(&[Day::Mon], 1400, 1441, 20.0)]),
        Rules {
            heat: None,
            cool: Some(ModeRules {
                base: Setpoint::Temp(50.0),
                blocks: vec![],
            }),
        },
    ];
    for r in bad {
        assert!(r.validate().is_err(), "{r:?}");
    }
}

#[test]
fn rules_validate_accepts_block_to_24_00() {
    let r = heat(vec![
        block(&[Day::Mon], 1320, 1440, 19.0),
        block(&[Day::Mon], 360, 1320, 21.0),
    ]);
    assert!(r.validate().is_ok());
    let touching_other_days = heat(vec![
        block(&[Day::Mon], 600, 700, 20.0),
        block(&[Day::Tue], 600, 700, 20.0),
    ]);
    assert!(touching_other_days.validate().is_ok());
    assert!(heat(vec![]).validate().is_ok());
}

#[test]
fn house_validate_rejects() {
    let d = HouseSettings::default;
    let bad = [
        HouseSettings { frost: 30.0, ..d() },
        HouseSettings {
            learn_support: 0.0,
            ..d()
        },
        HouseSettings {
            learn_support: 1.1,
            ..d()
        },
        HouseSettings {
            learn_min_days: 29,
            ..d()
        },
        HouseSettings {
            stale_after_min: 0,
            ..d()
        },
        HouseSettings {
            learn_days: 0,
            learn_min_days: 0,
            ..d()
        },
        HouseSettings {
            learn_min_days: 0,
            ..d()
        },
    ];
    for h in bad {
        assert!(h.validate().is_err(), "{h:?}");
    }
    // overcool >= cool_above is fine, learn_support == 1 is fine
    let h = HouseSettings {
        overcool: 25.0,
        learn_support: 1.0,
        ..d()
    };
    assert!(h.validate().is_ok());
    let h = HouseSettings {
        learn_days: 1,
        learn_min_days: 1,
        ..d()
    };
    assert!(h.validate().is_ok());
}

#[test]
fn zone_validate_rejects() {
    let z = ZoneSettings {
        manual_hold_max_min: 0,
        ..Default::default()
    };
    assert!(z.validate().is_err());
    let z = ZoneSettings {
        rules: heat(vec![block(&[Day::Mon], 700, 600, 20.0)]),
        ..Default::default()
    };
    assert!(z.validate().is_err());
}

fn decision(at: u64) -> Decision {
    Decision {
        zone: "z".into(),
        mode: Some(Mode::Heat),
        setpoint: Some(Setpoint::Temp(20.0)),
        source: Source::Schedule,
        cause: None,
        reason: "r".into(),
        reading: Some(19.0),
        reading_age_s: Some(5),
        outdoor_mean: Some(8.0),
        home: true,
        holiday: false,
        applied: true,
        at,
        entities: BTreeMap::new(),
        eco_restore: BTreeMap::new(),
        adjust: None,
        heaters: BTreeMap::new(),
    }
}

#[test]
fn feedback_defaults_and_old_documents() {
    let z: ZoneSettings = serde_json::from_str("{}").unwrap();
    assert_eq!(z.feedback, FeedbackSettings::default());
    assert!(!z.feedback.enabled);
    assert_eq!(z.feedback.band, 0.5);
    assert_eq!(z.feedback.quiet_limit_min, 360);
}

#[test]
fn feedback_validation() {
    assert!(ZoneSettings::default().validate().is_ok());
    let bad = |f: fn(&mut FeedbackSettings)| {
        let mut z = ZoneSettings::default();
        f(&mut z.feedback);
        assert!(z.validate().is_err());
    };
    bad(|f| f.step = 0.0);
    bad(|f| f.band = 0.0);
    bad(|f| f.interval_min = 0);
    bad(|f| f.quiet_limit_min = 0);
    bad(|f| f.max_below = -1.0);
    bad(|f| f.max_above = -0.5);
}

#[test]
fn presence_defaults_and_old_documents() {
    let z: ZoneSettings = serde_json::from_str("{}").unwrap();
    assert_eq!(z.presence, PresenceSettings::default());
    assert!(!z.presence.enabled);
    assert!(z.presence.sensors.is_empty());
    assert_eq!(z.presence.empty_after_min, 30);
    assert_eq!(z.presence.back_after_min, 5);
    assert_eq!(z.presence.setback, 2.0);
}

#[test]
fn presence_validation() {
    assert!(ZoneSettings::default().validate().is_ok());
    let bad = |f: fn(&mut PresenceSettings)| {
        let mut z = ZoneSettings::default();
        f(&mut z.presence);
        assert!(z.validate().is_err());
    };
    bad(|p| p.empty_after_min = 0);
    bad(|p| p.back_after_min = 0);
    bad(|p| p.setback = 0.0);
    bad(|p| p.setback = f64::NAN);
}

#[test]
fn empty_source_round_trips() {
    assert_eq!(serde_json::to_string(&Source::Empty).unwrap(), "\"empty\"");
    assert_eq!(
        serde_json::from_str::<Source>("\"empty\"").unwrap(),
        Source::Empty
    );
}

#[test]
fn decision_adjust_is_optional() {
    let mut d = decision(1);
    let json = serde_json::to_string(&d).unwrap();
    assert!(!json.contains("adjust"));
    assert_eq!(
        serde_json::from_str::<Decision>(&json).unwrap().adjust,
        None
    );
    d.adjust = Some(0.5);
    let json = serde_json::to_string(&d).unwrap();
    assert_eq!(
        serde_json::from_str::<Decision>(&json).unwrap().adjust,
        Some(0.5)
    );
    assert!(!decision(1).same_outcome(&d));
}

#[test]
fn same_outcome_ignores_volatile_fields() {
    let a = decision(1);
    let mut b = decision(2);
    b.reading = Some(19.5);
    b.reading_age_s = None;
    b.outdoor_mean = None;
    assert!(a.same_outcome(&b));
    b.reason = "other".into();
    assert!(!a.same_outcome(&b));
    let mut c = decision(1);
    c.entities.insert(
        "e".into(),
        DeviceTarget {
            setpoint: Some(20.0),
            mode: "heat".into(),
            held: false,
        },
    );
    assert!(!a.same_outcome(&c));
}

#[test]
fn documents_roundtrip() {
    let p = Proposal {
        id: "p1".into(),
        zone: "z".into(),
        kind: ProposalKind::Initial,
        created: 10,
        window: Window {
            from: 1,
            to: 2,
            days: 28,
        },
        rules: heat(vec![Block {
            evidence: Some(Evidence { agree: 7, of: 8 }),
            ..block(&[Day::Mon], 360, 1320, 21.0)
        }]),
        changes: vec!["c".into()],
        fingerprint: "f".into(),
    };
    let s = serde_json::to_string(&p).unwrap();
    assert_eq!(serde_json::from_str::<Proposal>(&s).unwrap(), p);
    let o = vec![Override {
        entity: None,
        setpoint: Some(Setpoint::Off),
        mode: Some(Mode::Cool),
        until: 5,
        origin: OverrideOrigin::Hand,
    }];
    let s = serde_json::to_string(&o).unwrap();
    assert_eq!(serde_json::from_str::<Vec<Override>>(&s).unwrap(), o);
    assert!(
        !serde_json::to_string(&block(&[Day::Mon], 1, 2, 3.0))
            .unwrap()
            .contains("evidence")
    );
}

#[test]
fn eco_defaults_and_shape() {
    let h: HouseSettings = serde_json::from_str("{}").unwrap();
    assert_eq!((h.eco.heat, h.eco.cool), (16.0, 28.0));
    assert!(h.eco.actions.is_empty());
    let e: EcoSettings =
        serde_json::from_str(r#"{"heat":15,"actions":[{"fan_mode":"low"}]}"#).unwrap();
    assert_eq!(e.fan_mode(), Some("low"));
    assert_eq!(e.setpoint(Mode::Heat), 15.0);
    assert_eq!(e.setpoint(Mode::Cool), 28.0);
    assert_eq!(
        serde_json::to_value(&e.actions).unwrap(),
        serde_json::json!([{ "fan_mode": "low" }])
    );
    assert!(serde_json::from_str::<EcoSettings>(r#"{"actions":[{"swing_mode":"off"}]}"#).is_err());
}

#[test]
fn eco_validation() {
    let mut h = HouseSettings::default();
    h.eco.heat = 6.0;
    assert!(h.validate().unwrap_err().contains("eco"));
    let mut h = HouseSettings::default();
    h.eco.cool = 31.0;
    assert!(h.validate().unwrap_err().contains("eco"));
    // Inside frost..overheat is not enough: a setpoint stays within 0..=40.
    let mut h = HouseSettings {
        overheat: 60.0,
        ..HouseSettings::default()
    };
    h.eco.heat = 45.0;
    assert!(h.validate().unwrap_err().contains("eco heat"));
    let mut h = HouseSettings::default();
    h.eco.cool = f64::NAN;
    assert!(h.validate().unwrap_err().contains("eco cool"));
    let mut h = HouseSettings::default();
    h.eco.actions = vec![
        EcoAction::FanMode("low".into()),
        EcoAction::FanMode("high".into()),
    ];
    assert!(h.validate().is_err());
    let mut h = HouseSettings::default();
    h.eco.actions = vec![EcoAction::FanMode(String::new())];
    assert!(h.validate().is_err());
    let mut h = HouseSettings::default();
    h.eco.actions = vec![EcoAction::FanMode("low".into())];
    assert!(h.validate().is_ok());
}

#[test]
fn old_zone_settings_with_away_setpoint_still_load() {
    let z: ZoneSettings =
        serde_json::from_str(r#"{"state":"active","away_setpoint":{"heat":16,"cool":28}}"#)
            .unwrap();
    assert_eq!(z.state, ZoneState::Active);
}

#[test]
fn decision_eco_restore_is_optional() {
    let d = Decision {
        zone: "z".into(),
        mode: Some(Mode::Heat),
        setpoint: None,
        source: Source::Eco,
        cause: None,
        reason: "r".into(),
        reading: None,
        reading_age_s: None,
        outdoor_mean: None,
        home: true,
        holiday: false,
        applied: false,
        at: 1,
        entities: BTreeMap::new(),
        eco_restore: BTreeMap::new(),
        adjust: None,
        heaters: BTreeMap::new(),
    };
    let s = serde_json::to_string(&d).unwrap();
    assert!(!s.contains("eco_restore"), "{s}");
    assert!(s.contains(r#""source":"eco""#), "{s}");
    assert_eq!(serde_json::from_str::<Decision>(&s).unwrap(), d);
    let mut with = d.clone();
    with.eco_restore.insert(
        "a".into(),
        FanRestore {
            was: "auto".into(),
            set: "low".into(),
            state: FanState::Set,
        },
    );
    assert!(!with.same_outcome(&d));
    let s = serde_json::to_string(&with).unwrap();
    assert_eq!(serde_json::from_str::<Decision>(&s).unwrap(), with);
}

#[test]
fn room_eco_round_trips() {
    let e = RoomEco {
        until: 10,
        set_at: 5,
    };
    let s = serde_json::to_string(&e).unwrap();
    assert_eq!(serde_json::from_str::<RoomEco>(&s).unwrap(), e);
}

fn heater(id: &str) -> Heater {
    Heater {
        entity: id.into(),
        ..Heater::default()
    }
}

fn rules_with_block_heated_by(ids: &[&str]) -> ModeRules {
    ModeRules {
        base: Setpoint::Temp(17.0),
        blocks: vec![Block {
            heated_by: Some(ids.iter().map(|s| s.to_string()).collect()),
            ..block(&[Day::Mon], 540, 600, 20.0)
        }],
    }
}

#[test]
fn old_zone_document_round_trips_unchanged() {
    let json = serde_json::to_string(&ZoneSettings::default()).unwrap();
    assert!(!json.contains("heaters") && !json.contains("heated_by") && !json.contains("fill_in"));
    let block: Block =
        serde_json::from_str(r#"{"days":["mon"],"from":"09:00","to":"10:00","setpoint":20.0}"#)
            .unwrap();
    assert_eq!(block.heated_by, None);
    assert!(!serde_json::to_string(&block).unwrap().contains("heated_by"));
}

#[test]
fn heater_defaults() {
    let h: Heater = serde_json::from_str(r#"{"entity":"sw"}"#).unwrap();
    assert_eq!(
        (h.drive, h.enabled, h.min_off_min, h.min_on_min),
        (HeaterDrive::Thermostat, true, 0, 0)
    );
}

#[test]
fn heaters_validate() {
    let mut s = ZoneSettings {
        heaters: vec![heater("sw"), heater("sw")],
        ..ZoneSettings::default()
    };
    assert!(s.validate().unwrap_err().contains("twice"));
    s.heaters = vec![Heater {
        min_off_min: 241,
        ..heater("sw")
    }];
    assert!(s.validate().unwrap_err().contains("min_off_min"));
    s.heaters = vec![Heater {
        min_on_min: 121,
        ..heater("sw")
    }];
    assert!(s.validate().unwrap_err().contains("min_on_min"));
    s.heaters = vec![heater("group_x")];
    assert!(s.validate().unwrap_err().contains("group"));
}

#[test]
fn heated_by_validates_against_heaters_and_units() {
    let mut s = ZoneSettings {
        heaters: vec![heater("stove")],
        heated_by: Some(vec!["stove".into(), "ac".into()]),
        ..ZoneSettings::default()
    };
    assert!(s.validate_heated_by(&["ac".into()]).is_ok());
    s.heated_by = Some(vec![]);
    assert!(
        s.validate_heated_by(&["ac".into()])
            .unwrap_err()
            .contains("empty")
    );
    s.heated_by = None;
    s.rules.heat = Some(rules_with_block_heated_by(&["gone"]));
    assert!(
        s.validate_heated_by(&["ac".into()])
            .unwrap_err()
            .contains("heat block 0")
    );
    s.rules.heat = None;
    s.rules.cool = Some(rules_with_block_heated_by(&["stove"]));
    assert!(s.validate().unwrap_err().contains("cool block 0"));
}
