use super::*;

fn draft(eco: &str, cold: &str, hot: &str) -> Draft {
    Draft {
        eco: eco.into(),
        mode: None,
        indoor: None,
        cold: cold.into(),
        hot: hot.into(),
        off_peak: vec![],
        off_peak_device: None,
    }
}

#[test]
fn save_keeps_the_fields_this_panel_does_not_edit() {
    let stored = HomeSettings {
        energy_view: nidoo_api::EnergyView::Orbite,
        assist_min_c: Some(17.0),
        assist_max_c: Some(24.0),
        ..HomeSettings::default()
    };
    let edited = HomeSettings {
        eco_setpoint: Some(15.0),
        ..HomeSettings::default()
    };
    let saved = keep_stored_fields(&stored, edited);
    assert_eq!(saved.eco_setpoint, Some(15.0));
    assert_eq!(saved.energy_view, nidoo_api::EnergyView::Orbite);
    assert_eq!(
        (saved.assist_min_c, saved.assist_max_c),
        (Some(17.0), Some(24.0))
    );
}

#[test]
fn draft_device_round_trips() {
    let device = nidoo_api::OffPeakDevice {
        entity: "sensor.tariff".into(),
        values: vec!["OFF".into(), "OFF2".into()],
    };
    let settings = HomeSettings {
        off_peak_device: Some(device.clone()),
        ..HomeSettings::default()
    };
    let d = Draft::new(Lang::En, &settings);
    assert_eq!(off_peak_device(&d), Some(device.clone()));
    assert_eq!(draft_to_settings(&d).unwrap().off_peak_device, Some(device));
    let blank = Draft {
        off_peak_device: Some((String::new(), vec!["OFF".into()])),
        ..Draft::default()
    };
    assert_eq!(off_peak_device(&blank), None);
    let twice = Draft {
        off_peak_device: Some((
            "sensor.tariff".into(),
            vec!["B".into(), " A ".into(), "B".into(), "".into()],
        )),
        ..Draft::default()
    };
    assert_eq!(
        off_peak_device(&twice).unwrap().values,
        ["B".to_owned(), "A".to_owned()]
    );
}

#[test]
fn draft_parses_numbers() {
    let s = draft_to_settings(&draft("15,5", "", "27.5")).unwrap();
    assert_eq!(s.eco_setpoint, Some(15.5));
    assert_eq!(s.cold_below, None);
    assert_eq!(s.hot_above, Some(27.5));
    let field = draft_to_settings(&draft("abc", "", "")).unwrap_err();
    assert_eq!(field, Field::Eco);
    let field = draft_to_settings(&draft("", "", "x")).unwrap_err();
    assert_eq!(field, Field::Hot);
}

fn full(eco: &str, mode: Option<&str>) -> Draft {
    Draft {
        eco: eco.into(),
        mode: mode.map(Into::into),
        indoor: None,
        cold: "15".into(),
        hot: "26".into(),
        off_peak: vec![],
        off_peak_device: None,
    }
}

#[test]
fn merge_untouched_field_takes_the_latest() {
    let base = full("15", None);
    let draft = full("15", Some("virtual_mode"));
    let latest = full("17", None);
    let got = merge(&base, &draft, &latest);
    assert_eq!(got.eco, "17");
    assert_eq!(got.mode.as_deref(), Some("virtual_mode"));
}

#[test]
fn merge_edited_field_keeps_the_edit() {
    let base = full("15", None);
    let draft = full("16", None);
    assert_eq!(merge(&base, &draft, &base).eco, "16");
}

#[test]
fn merge_edit_wins_over_a_change_elsewhere() {
    let base = full("15", None);
    let draft = full("16", None);
    let latest = full("18", None);
    assert_eq!(merge(&base, &draft, &latest).eco, "16");
}

#[test]
fn hub_messages_map_to_translated_rules() {
    let between = "eco_setpoint must be between 5 and 30 °C";
    assert_eq!(message_key(Field::Eco, between), Some(HomeText::EcoRange));
    assert_eq!(
        message_key(Field::Hot, "hot_above must be between 5 and 35 °C"),
        Some(HomeText::ThresholdRange)
    );
    assert_eq!(
        message_key(Field::Cold, "cold_below must be lower than hot_above"),
        Some(HomeText::OrderRule)
    );
    assert_eq!(
        message_key(Field::Mode, "mode_entity `x` is not a select"),
        Some(HomeText::EntityRule)
    );
    assert_eq!(
        message_key(Field::Indoor, "indoor_entity `x` is not an entity"),
        Some(HomeText::EntityRule)
    );
    assert_eq!(message_key(Field::Eco, "boom"), None);
}

#[test]
fn hub_messages_name_their_field() {
    assert_eq!(
        field_of_message("cold_below must be below hot_above"),
        Some(Field::Cold)
    );
    assert_eq!(
        field_of_message("eco_setpoint must be between 5 and 30"),
        Some(Field::Eco)
    );
    assert_eq!(field_of_message("mode_entity: unknown"), Some(Field::Mode));
    assert_eq!(
        field_of_message("indoor_entity: unknown"),
        Some(Field::Indoor)
    );
    assert_eq!(field_of_message("boom"), None);
}

fn window(start: &str, end: &str) -> OffPeakWindow {
    OffPeakWindow {
        start: start.into(),
        end: end.into(),
    }
}

#[test]
fn refilled_numbers_use_the_language_decimal_mark() {
    let s = draft_to_settings(&draft("15,5", "16", "27.25")).unwrap();
    let fr = Draft::new(Lang::Fr, &s);
    assert_eq!((fr.eco.as_str(), fr.cold.as_str()), ("15,5", "16"));
    assert_eq!(fr.hot, "27,25");
    let en = Draft::new(Lang::En, &s);
    assert_eq!((en.eco.as_str(), en.hot.as_str()), ("15.5", "27.25"));
    // What was refilled reads back as the same numbers in either language.
    assert_eq!(draft_to_settings(&fr).unwrap().eco_setpoint, Some(15.5));
    assert_eq!(draft_to_settings(&en).unwrap().hot_above, Some(27.25));
}

#[test]
fn draft_round_trips_off_peak() {
    let d = Draft {
        off_peak: vec![("01:34".into(), "07:04".into())],
        ..draft("", "", "")
    };
    let s = draft_to_settings(&d).unwrap();
    assert_eq!(s.off_peak, vec![window("01:34", "07:04")]);
    assert!(s.off_peak_history.is_empty());
    assert_eq!(Draft::new(Lang::En, &s).off_peak, d.off_peak);
}

#[test]
fn draft_drops_blank_windows() {
    let d = Draft {
        off_peak: vec![("".into(), "".into()), ("22:00".into(), "06:00".into())],
        ..draft("", "", "")
    };
    assert_eq!(
        draft_to_settings(&d).unwrap().off_peak,
        vec![window("22:00", "06:00")]
    );
}

#[test]
fn merge_keeps_edited_windows() {
    let base = full("15", None);
    let draft = Draft {
        off_peak: vec![("12:05".into(), "14:34".into())],
        ..full("15", None)
    };
    let latest = full("17", None);
    let got = merge(&base, &draft, &latest);
    assert_eq!(got.off_peak, draft.off_peak);
    assert_eq!(got.eco, "17");
    let untouched = merge(&base, &base, &draft);
    assert_eq!(untouched.off_peak, draft.off_peak);
}

#[test]
fn off_peak_messages_map_to_field() {
    let overlap = "off_peak windows 1 and 2 overlap";
    assert_eq!(field_of_message(overlap), Some(Field::OffPeak));
    assert_eq!(
        message_key(Field::OffPeak, overlap),
        Some(HomeText::WindowsOverlap)
    );
    assert_eq!(
        message_key(Field::OffPeak, "off_peak window 1: bad time"),
        Some(HomeText::BadTime)
    );
    assert_eq!(
        message_key(Field::OffPeak, "off_peak window 2: start equals end"),
        Some(HomeText::StartEqualsEnd)
    );
}
