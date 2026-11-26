use super::*;
use nidoo_climate::doc::{Override, OverrideOrigin, ZoneSettings};

fn zone_view(state: ZoneState) -> ZoneView {
    ZoneView {
        eco: None,
        zone: "z".into(),
        name: "Z".into(),
        orphan: false,
        settings: ZoneSettings {
            state,
            ..ZoneSettings::default()
        },
        settings_invalid: None,
        entities: vec![],
        reading: None,
        reading_age_s: None,
        room_sensor: None,
        thermometer: None,
        correction_step_at: None,
        presence: None,
        decision: None,
        overrides: vec![],
        proposal: None,
        learn: LearnStatus::Off,
    }
}

fn decided(mut z: ZoneView, setpoint: Setpoint) -> ZoneView {
    z.decision = Some(Decision {
        eco_restore: Default::default(),
        adjust: None,
        zone: z.zone.clone(),
        mode: Some(Mode::Heat),
        setpoint: Some(setpoint),
        source: Source::Schedule,
        cause: None,
        reason: String::new(),
        reading: None,
        reading_age_s: None,
        outdoor_mean: None,
        home: true,
        holiday: false,
        applied: true,
        at: 0,
        entities: Default::default(),
        heaters: std::collections::BTreeMap::new(),
    });
    z
}

#[test]
fn temp_band_cuts_at_17_and_20() {
    assert_eq!(temp_band(Setpoint::Temp(16.5)), TempBand::Low);
    assert_eq!(temp_band(Setpoint::Temp(17.0)), TempBand::Mid);
    assert_eq!(temp_band(Setpoint::Temp(19.5)), TempBand::Mid);
    assert_eq!(temp_band(Setpoint::Temp(20.0)), TempBand::High);
    assert_eq!(temp_band(Setpoint::Off), TempBand::Off);
    assert_eq!(TempBand::Mid.class(), "climate-temp--mid");
}

#[test]
fn room_subline_reads_each_state() {
    let en = Lang::En;
    let mut orphan = zone_view(ZoneState::Active);
    orphan.orphan = true;
    assert_eq!(room_subline(en, &orphan), "No climate device");
    assert_eq!(room_subline(en, &zone_view(ZoneState::Off)), "Off");
    let pending = zone_view(ZoneState::Pending);
    assert_eq!(room_subline(en, &pending), "Pending · no schedule");
    let mut scheduled = pending.clone();
    scheduled.settings.rules = Rules {
        heat: Some(ModeRules {
            base: Setpoint::Temp(16.0),
            blocks: vec![],
        }),
        cool: None,
    };
    assert_eq!(room_subline(en, &scheduled), "Pending");
    assert_eq!(
        room_subline(en, &zone_view(ZoneState::Active)),
        "Active · no decision yet"
    );
    let active = decided(zone_view(ZoneState::Active), Setpoint::Temp(16.0));
    assert_eq!(room_subline(en, &active), "Heat 16° · Active");
    let mut held = active.clone();
    held.overrides.push(Override {
        entity: None,
        setpoint: Some(Setpoint::Temp(19.5)),
        mode: None,
        until: 0,
        origin: OverrideOrigin::Hand,
    });
    assert_eq!(room_subline(en, &held), "Heat 16° · Active · held");
}

#[test]
fn room_band_follows_the_decision_setpoint() {
    let z = zone_view(ZoneState::Active);
    assert_eq!(room_band(&z), None);
    assert_eq!(
        room_band(&decided(z.clone(), Setpoint::Temp(18.5))),
        Some(TempBand::Mid)
    );
    assert_eq!(room_band(&decided(z, Setpoint::Off)), None);
}

#[test]
fn pick_zone_keeps_a_known_choice_and_falls_back() {
    let ids = vec!["a".to_owned(), "b".to_owned()];
    assert_eq!(pick_zone(&ids, Some("b")).as_deref(), Some("b"));
    assert_eq!(pick_zone(&ids, Some("gone")).as_deref(), Some("a"));
    assert_eq!(pick_zone(&ids, None).as_deref(), Some("a"));
    assert_eq!(pick_zone(&[], Some("a")), None);
}

#[test]
fn only_an_active_zone_can_hold() {
    assert!(can_hold(ZoneState::Active));
    assert!(!can_hold(ZoneState::Pending));
    assert!(!can_hold(ZoneState::Off));
}
use nidoo_climate::doc::{Block, HhMm, ModeRules};

#[test]
fn learn_status_text_reads_each_state() {
    assert_eq!(
        learn_status_text(
            Lang::En,
            &LearnStatus::NotEnough {
                days: 6,
                needed: 10
            }
        ),
        "Learning: 6 of 10 days so far"
    );
    assert_eq!(
        learn_status_text(Lang::En, &LearnStatus::HistoryOff),
        "History is off: rules can't be learned"
    );
    assert_eq!(
        learn_status_text(Lang::En, &LearnStatus::Proposed),
        "New rules to review"
    );
    assert_eq!(
        learn_status_text(Lang::En, &LearnStatus::UpToDate),
        "Rules up to date"
    );
    assert_eq!(
        learn_status_text(Lang::En, &LearnStatus::Eco),
        "Learning paused while in eco"
    );
}

#[test]
fn age_text_picks_the_unit() {
    let en = Lang::En;
    assert_eq!(age_text(en, 45), "just now");
    assert_eq!(age_text(en, 600), "10\u{A0}min ago");
    assert_eq!(age_text(en, 7200), "2\u{A0}h ago");
    assert_eq!(age_text(en, 3 * 86_400), "3\u{A0}d ago");
}

fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> u64 {
    use chrono::TimeZone;
    chrono::Utc
        .with_ymd_and_hms(y, mo, d, h, mi, 0)
        .unwrap()
        .timestamp() as u64
}

#[test]
fn clock_and_week_follow_the_house_zone() {
    let paris = parse_zone("Europe/Paris");
    // Monday 2026-01-05 23:30 UTC is Tuesday 00:30 in Paris (UTC+1).
    let now = at(2026, 1, 5, 23, 30);
    assert_eq!(clock_text(now, paris), "00:30");
    assert_eq!(clock_text(now, Tz::UTC), "23:30");
    assert_eq!(week_minute(now, paris), (Day::Tue, 30));
    assert_eq!(week_minute(now, Tz::UTC), (Day::Mon, 23 * 60 + 30));
    assert_eq!(parse_zone("nowhere"), Tz::UTC);
}

#[test]
fn until_text_names_the_day_when_not_today() {
    let paris = parse_zone("Europe/Paris");
    let now = at(2026, 1, 5, 10, 0); // Monday 11:00 in Paris
    assert_eq!(
        until_text(Lang::En, at(2026, 1, 5, 12, 0), now, paris),
        "13:00"
    );
    assert_eq!(
        until_text(Lang::En, at(2026, 1, 6, 6, 0), now, paris),
        "tomorrow 07:00"
    );
    assert_eq!(
        until_text(Lang::En, at(2026, 1, 8, 6, 0), now, paris),
        "Thu 07:00"
    );
    // 23:30 UTC is already tomorrow in Paris.
    assert_eq!(
        until_text(Lang::En, at(2026, 1, 5, 23, 30), now, paris),
        "tomorrow 00:30"
    );
}

#[test]
fn eco_end_names_the_day_and_month() {
    let paris = parse_zone("Europe/Paris");
    // 18:00 UTC is 19:00 in Paris on Tuesday 3 November 2026.
    let until = at(2026, 11, 3, 18, 0);
    assert_eq!(eco_end_text(Lang::En, until, paris), "Tue 3 Nov 19:00");
    assert_eq!(eco_end_text(Lang::Fr, until, paris), "mar. 3 nov. 19:00");
}

#[test]
fn degrees_drop_a_trailing_zero() {
    assert_eq!(degrees_text(Lang::En, 21.0), "21°");
    assert_eq!(degrees_text(Lang::En, 20.5), "20.5°");
    assert_eq!(setpoint_text(Lang::En, Some(Setpoint::Off)), "Off");
}

fn rules(blocks: Vec<Block>) -> Rules {
    Rules {
        heat: Some(ModeRules {
            base: Setpoint::Temp(17.0),
            blocks,
        }),
        cool: None,
    }
}

fn block(days: &[Day], from: &str, to: &str) -> Block {
    Block {
        days: days.to_vec(),
        from: HhMm::parse(from).unwrap(),
        to: HhMm::parse(to).unwrap(),
        setpoint: Setpoint::Temp(21.0),
        evidence: None,
        heated_by: None,
    }
}

#[test]
fn french_room_lines_and_ages() {
    let fr = Lang::Fr;
    let active = decided(zone_view(ZoneState::Active), Setpoint::Temp(16.5));
    assert_eq!(room_subline(fr, &active), "Chauffage 16,5° · Active");
    let mut held = active.clone();
    held.overrides.push(Override {
        entity: None,
        setpoint: Some(Setpoint::Temp(19.5)),
        mode: None,
        until: 0,
        origin: OverrideOrigin::Hand,
    });
    assert_eq!(
        room_subline(fr, &held),
        "Chauffage 16,5° · Active · dérogation en cours"
    );
    assert_eq!(
        room_subline(fr, &zone_view(ZoneState::Pending)),
        "En attente · aucune programmation"
    );
    assert_eq!(age_text(fr, 45), "à l'instant");
    assert_eq!(age_text(fr, 600), "il y a 10\u{A0}min");
    assert_eq!(degrees_text(fr, 20.5), "20,5°");
    assert_eq!(setpoint_text(fr, Some(Setpoint::Off)), "Arrêt");
    assert_eq!(mode_word(fr, None), "Au repos");
    assert_eq!(source_word(fr, Source::Schedule), "Programmation");
    assert_eq!(
        learn_status_text(fr, &LearnStatus::UpToDate),
        "Règles à jour"
    );
}

#[test]
fn french_until_names_the_day() {
    let paris = parse_zone("Europe/Paris");
    let now = at(2026, 1, 5, 10, 0);
    assert_eq!(
        until_text(Lang::Fr, at(2026, 1, 6, 6, 0), now, paris),
        "demain 07:00"
    );
    assert_eq!(
        until_text(Lang::Fr, at(2026, 1, 8, 6, 0), now, paris),
        "jeu. 07:00"
    );
}

#[test]
fn stamps_are_dated_unless_today() {
    let paris = parse_zone("Europe/Paris");
    let now = at(2026, 1, 5, 10, 0); // Monday 11:00 in Paris
    assert_eq!(
        stamp_text(Lang::En, at(2026, 1, 5, 7, 5), now, paris),
        "08:05"
    );
    // 23:30 UTC on Sunday is already Monday in Paris.
    assert_eq!(
        stamp_text(Lang::En, at(2026, 1, 4, 23, 30), now, paris),
        "00:30"
    );
    assert_eq!(
        stamp_text(Lang::En, at(2026, 1, 4, 22, 30), now, paris),
        "Sun 4 Jan 23:30"
    );
    assert_eq!(
        stamp_text(Lang::Fr, at(2026, 1, 4, 22, 30), now, paris),
        "dim. 4 janv. 23:30"
    );
}

#[test]
fn history_rows_use_the_known_words() {
    assert_eq!(history_source(Lang::Fr, "schedule"), "Programmation");
    assert_eq!(history_source(Lang::Fr, "override"), "Dérogation");
    assert_eq!(history_source(Lang::En, "boost"), "Boost");
    assert_eq!(history_source(Lang::Fr, "boost"), "Boost");
    assert_eq!(source_word(Lang::En, Source::Boost), "Boost");
    assert_eq!(history_source(Lang::En, "away"), "Away");
    assert_eq!(history_source(Lang::En, "eco"), "Eco");
    assert_eq!(history_source(Lang::Fr, "eco"), "Éco");
    assert_eq!(history_source(Lang::En, "empty"), "Empty room");
    assert_eq!(history_source(Lang::Fr, "empty"), "Pièce vide");
    assert_eq!(history_source(Lang::Fr, "mystery_source"), "Mystery source");
}

#[test]
fn next_change_is_the_nearest_block_edge() {
    let r = rules(vec![block(&[Day::Mon, Day::Tue], "07:00", "09:00")]);
    // Monday 06:00: the block starts in an hour.
    assert_eq!(
        minutes_to_next_change(&r, Mode::Heat, Day::Mon, 360),
        Some(60)
    );
    // Monday 08:00: it ends in an hour.
    assert_eq!(
        minutes_to_next_change(&r, Mode::Heat, Day::Mon, 480),
        Some(60)
    );
    // Monday 10:00: next is Tuesday 07:00.
    assert_eq!(
        minutes_to_next_change(&r, Mode::Heat, Day::Mon, 600),
        Some(1260)
    );
    // Wednesday: wraps to next Monday 07:00.
    assert_eq!(
        minutes_to_next_change(&r, Mode::Heat, Day::Wed, 0),
        Some(5 * 1440 + 420)
    );
    assert_eq!(minutes_to_next_change(&r, Mode::Cool, Day::Mon, 0), None);
    assert_eq!(
        minutes_to_next_change(&rules(vec![]), Mode::Heat, Day::Mon, 0),
        None
    );
}

fn climate_entity(id: &str, fans: &[&str]) -> nidoo_api::EntityView {
    nidoo_api::EntityView {
        id: id.into(),
        device_id: None,
        name: id.into(),
        device_name: None,
        label: None,
        kind: nidoo_core::model::Kind::Climate,
        component: "climate".into(),
        protocol_kind: nidoo_core::model::Kind::Climate,
        device_class: None,
        unit: None,
        icon: None,
        category: nidoo_core::model::EntityCategory::Primary,
        enabled_by_default: true,
        area: "a".into(),
        part_of: None,
        capabilities: nidoo_core::model::Capabilities {
            fan_modes: fans.iter().map(|f| f.to_string()).collect(),
            ..Default::default()
        },
        ui: Default::default(),
        state: Default::default(),
    }
}

#[test]
fn fan_mode_choices_unions_and_counts() {
    let a = climate_entity("a", &["auto", "low", "high"]);
    let b = climate_entity("b", &["auto", "quiet"]);
    let (choices, units) = fan_mode_choices([&a, &b]);
    assert_eq!(units, 2);
    assert_eq!(
        choices,
        vec![
            ("auto".to_owned(), 2),
            ("low".to_owned(), 1),
            ("high".to_owned(), 1),
            ("quiet".to_owned(), 1)
        ]
    );
}

#[test]
fn fan_mode_choices_skips_groups_and_non_climate() {
    let group = climate_entity("group_all", &["low"]);
    let mut sensor = climate_entity("s", &["low"]);
    sensor.kind = nidoo_core::model::Kind::Sensor;
    let none = climate_entity("c", &[]);
    let (choices, units) = fan_mode_choices([&group, &sensor, &none]);
    assert!(choices.is_empty());
    assert_eq!(
        units, 1,
        "a climate without fan control still counts as a unit"
    );
}

#[test]
fn eco_until_local_and_dst_gap() {
    let paris = parse_zone("Europe/Paris");
    // 00:00 on 2026-11-01 in Paris (CET) is 23:00 UTC the day before.
    assert_eq!(eco_until("2026-11-01", "00:00", paris), Some(1_793_487_600));
    // 02:30 does not exist on 2026-03-29: the clocks jump to 03:00 (01:00 UTC).
    assert_eq!(eco_until("2026-03-29", "02:30", paris), Some(1_774_746_000));
    assert_eq!(eco_until("2026-11-01", "", paris), Some(1_793_487_600));
    assert_eq!(eco_until("nope", "00:00", paris), None);
}

#[test]
fn in_eco_for_room_and_away() {
    let mut z = zone_view(ZoneState::Active);
    assert!(!in_eco(&z, 100));
    z.eco = Some(nidoo_climate::doc::RoomEco {
        until: 200,
        set_at: 0,
    });
    assert!(in_eco(&z, 100));
    assert!(!in_eco(&z, 200), "ended");
    z.eco = None;
    let mut z = decided(z, Setpoint::Temp(16.0));
    z.decision.as_mut().unwrap().source = Source::Away;
    assert!(in_eco(&z, 100));
}

#[test]
fn step_target_rounds_and_clamps() {
    assert_eq!(step_target(19.0, TARGET_STEP), 19.5);
    assert_eq!(step_target(19.3, TARGET_STEP), 20.0);
    assert_eq!(step_target(0.2, -TARGET_STEP), 0.0);
    assert_eq!(step_target(39.8, TARGET_STEP), 40.0);
}

#[test]
fn hold_minutes_defaults_to_two_hours() {
    assert_eq!(hold_minutes(Some(45)), 45);
    assert_eq!(hold_minutes(None), 120);
}

#[test]
fn start_target_prefers_the_decision() {
    let mut read = zone_view(ZoneState::Active);
    read.reading = Some(19.6);
    assert_eq!(
        start_target(&decided(read.clone(), Setpoint::Temp(18.5))),
        Some(18.5)
    );
    assert_eq!(start_target(&read), Some(20.0));
    assert_eq!(start_target(&zone_view(ZoneState::Active)), None);
}

#[test]
fn correction_summary_reads_in_both_languages() {
    let f = nidoo_climate::doc::FeedbackSettings {
        step: 0.5,
        band: 0.5,
        interval_min: 15,
        ..Default::default()
    };
    assert_eq!(
        correction_summary(Lang::En, &f),
        "Step 0.5 °C · tolerance 0.5 °C · every 15 min"
    );
    assert_eq!(
        correction_summary(Lang::Fr, &f),
        "Pas 0,5 °C · écart 0,5 °C · toutes les 15 min"
    );
}
