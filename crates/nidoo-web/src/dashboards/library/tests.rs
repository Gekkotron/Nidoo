use nidoo_core::model::{Capabilities, EntityCategory, EntityState, UiOverride};

use super::*;

fn ent(id: &str, name: &str, kind: Kind) -> EntityView {
    EntityView {
        id: id.into(),
        device_id: None,
        name: name.into(),
        device_name: None,
        label: None,
        kind,
        component: kind.component().to_string(),
        protocol_kind: kind,
        device_class: None,
        unit: None,
        icon: None,
        category: EntityCategory::Primary,
        enabled_by_default: true,
        area: "salon".into(),
        part_of: None,
        capabilities: Capabilities::default(),
        ui: UiOverride::default(),
        state: EntityState::default(),
    }
}

fn house(list: Vec<EntityView>) -> BTreeMap<String, EntityView> {
    list.into_iter().map(|e| (e.id.clone(), e)).collect()
}

fn area(id: &str, name: &str) -> AreaView {
    AreaView {
        id: id.into(),
        name: name.into(),
        icon: None,
        order: None,
        entity_count: 0,
        primary_count: 0,
        group_names: Default::default(),
        group_favorites: Default::default(),
    }
}

fn run(query: &str, entities: &BTreeMap<String, EntityView>) -> Vec<Entry> {
    entries(
        Lang::En,
        query,
        entities,
        &[area("salon", "Salon")],
        &[],
        &[],
    )
}

fn of(entries: &[Entry], section: Section) -> Vec<&Entry> {
    entries.iter().filter(|e| e.section == section).collect()
}

#[test]
fn hidden_and_diagnostic_entities_are_left_out() {
    let mut hidden = ent("l2", "Hidden", Kind::Light);
    hidden.ui.hidden = true;
    let mut diag = ent("l3", "Diag", Kind::Light);
    diag.category = EntityCategory::Diagnostic;
    let entities = house(vec![ent("l1", "Lamp", Kind::Light), hidden, diag]);
    let got = run("", &entities);
    assert_eq!(of(&got, Section::Lights).len(), 1);
}

#[test]
fn sub_line_is_room_then_device() {
    let mut light = ent("l1", "Lamp", Kind::Light);
    light.device_name = Some("Hue bulb".into());
    let mut twin = ent("l2", "Light", Kind::Light);
    twin.device_name = Some("light".into());
    let entities = house(vec![light, twin]);
    let got = run("", &entities);
    let subs: Vec<_> = of(&got, Section::Lights)
        .iter()
        .map(|e| e.sub.as_deref())
        .collect();
    assert_eq!(subs, [Some("Salon · Hue bulb"), Some("Salon")]);
}

#[test]
fn search_ignores_accents_and_matches_the_room() {
    let entities = house(vec![
        ent("l1", "Lampe évier", Kind::Light),
        ent("l2", "Other", Kind::Light),
    ]);
    let got = run("EVIER", &entities);
    assert_eq!(of(&got, Section::Lights).len(), 1);
    let got = run("  salon ", &entities);
    assert_eq!(of(&got, Section::Lights).len(), 2);
}

#[test]
fn the_kind_word_finds_entities_in_either_language() {
    let entities = house(vec![
        ent("c1", "Shutter", Kind::Cover),
        ent("l1", "Lamp", Kind::Light),
    ]);
    for query in ["volet", "cover"] {
        let got = run(query, &entities);
        assert_eq!(of(&got, Section::Covers).len(), 1, "{query}");
        assert!(of(&got, Section::Lights).is_empty(), "{query}");
    }
}

#[test]
fn english_words_still_find_the_rows_on_the_french_ui() {
    let flows = [("f1".to_string(), "Night".to_string())];
    let areas = [area("salon", "Salon")];
    let entities = BTreeMap::new();
    let find = |q: &str| entries(Lang::Fr, q, &entities, &areas, &flows, &[]);
    for (query, section) in [
        ("lights", Section::Rooms),
        ("runs the flow", Section::FlowButtons),
        ("today", Section::Energy),
        ("éclairages", Section::Rooms),
        ("lance", Section::FlowButtons),
        ("aujourd", Section::Energy),
    ] {
        assert!(find(query).iter().any(|e| e.section == section), "{query}");
    }
}

#[test]
fn sections_come_in_display_order() {
    let mut lock = ent("k1", "Front door", Kind::Generic);
    lock.component = "lock".into();
    let entities = house(vec![
        ent("s1", "Temp", Kind::Sensor),
        ent("c1", "Shutter", Kind::Cover),
        ent("l1", "Lamp", Kind::Light),
        lock,
    ]);
    let got = entries(
        Lang::En,
        "",
        &entities,
        &[area("salon", "Salon")],
        &[("f1".into(), "Night".into())],
        &[],
    );
    let mut sections: Vec<_> = got.iter().map(|e| e.section).collect();
    sections.dedup();
    assert_eq!(
        sections,
        [
            Section::Lights,
            Section::Covers,
            Section::Sensors,
            Section::Locks,
            Section::Rooms,
            Section::FlowButtons,
            Section::Energy,
            Section::HistoryCharts,
        ]
    );
}

fn cams(ids: &[&str]) -> Vec<CameraInfo> {
    ids.iter()
        .map(|id| CameraInfo {
            id: (*id).into(),
            name: nidoo_frigate::pretty(id),
        })
        .collect()
}

fn with_cameras(cameras: &[CameraInfo]) -> Vec<Entry> {
    entries(Lang::En, "", &BTreeMap::new(), &[], &[], cameras)
}

fn camera_ids(entry: &Entry) -> Vec<String> {
    match &entry.config {
        WidgetConfig::Cameras { cameras, .. } => cameras.clone(),
        other => panic!("not a camera config: {other:?}"),
    }
}

#[test]
fn cameras_section_lists_each_camera_and_all() {
    let got = with_cameras(&cams(&["front_door", "garden"]));
    let rows = of(&got, Section::Cameras);
    assert_eq!(rows.len(), 3);
    assert!(rows.iter().all(|e| e.kind == WidgetKind::Camera));
    let all = rows
        .iter()
        .find(|e| e.title == "All cameras")
        .expect("an All cameras row");
    assert_eq!(camera_ids(all), ["front_door", "garden"]);
    let one = rows.iter().find(|e| e.title == "Garden").expect("Garden");
    assert_eq!(camera_ids(one), ["garden"]);
}

#[test]
fn one_camera_has_no_all_row() {
    let got = with_cameras(&cams(&["front_door"]));
    let rows = of(&got, Section::Cameras);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].title, "Front Door");
}

#[test]
fn no_cameras_no_section() {
    assert!(of(&with_cameras(&[]), Section::Cameras).is_empty());
}

#[test]
fn all_cameras_keeps_the_first_four() {
    let got = with_cameras(&cams(&["a", "b", "c", "d", "e"]));
    let all = of(&got, Section::Cameras)
        .into_iter()
        .find(|e| e.title == "All cameras")
        .expect("an All cameras row");
    assert_eq!(camera_ids(all), ["a", "b", "c", "d"]);
}

#[test]
fn initial_span_is_big_for_several_cameras() {
    let two = WidgetConfig::Cameras {
        cameras: vec!["a".into(), "b".into()],
        title: None,
    };
    let one = WidgetConfig::Cameras {
        cameras: vec!["a".into()],
        title: None,
    };
    let entity = WidgetConfig::Entity {
        entity_id: "light.a".into(),
        title: None,
    };
    assert_eq!(initial_span(WidgetKind::Camera, &two), Span::X2x2);
    assert_eq!(initial_span(WidgetKind::Camera, &one), Span::X2x1);
    assert_eq!(initial_span(WidgetKind::Light, &entity), Span::X1x1);
}

#[test]
fn locks_get_their_own_section() {
    let mut lock = ent("lock.front", "Front door", Kind::Generic);
    lock.component = "lock".into();
    let mut siren = ent("siren.x", "Siren", Kind::Generic);
    siren.component = "siren".into();
    let got = run("", &house(vec![lock, siren]));
    let locks = of(&got, Section::Locks);
    assert_eq!(locks.len(), 1);
    assert_eq!(locks[0].kind, WidgetKind::Lock);
    assert!(matches!(
        &locks[0].config,
        WidgetConfig::Entity { entity_id, .. } if entity_id == "lock.front"
    ));
}

#[test]
fn media_players_get_their_own_wide_section() {
    let got = run("", &house(vec![ent("music", "Music", Kind::MediaPlayer)]));
    let players = of(&got, Section::MediaPlayers);
    assert_eq!(players.len(), 1);
    assert_eq!(players[0].kind, WidgetKind::MediaPlayer);
    let config = players[0].config.clone();
    assert_eq!(initial_span(WidgetKind::MediaPlayer, &config), Span::X2x1);
}

#[test]
fn no_lock_no_lock_section() {
    let got = run("", &house(vec![ent("l1", "Lamp", Kind::Light)]));
    assert!(of(&got, Section::Locks).is_empty());
}

#[test]
fn a_sensor_is_offered_as_a_sensor_and_as_a_chart() {
    let entities = house(vec![ent("s1", "Temp", Kind::Sensor)]);
    let got = run("", &entities);
    let sensor = of(&got, Section::Sensors);
    let chart = of(&got, Section::HistoryCharts);
    assert_eq!(sensor[0].kind, WidgetKind::Sensor);
    assert_eq!(
        sensor[0].config,
        WidgetConfig::Entity {
            entity_id: "s1".into(),
            title: None
        }
    );
    assert_eq!(chart[0].kind, WidgetKind::HistoryChart);
    assert_eq!(
        chart[0].config,
        WidgetConfig::History {
            entity_id: "s1".into(),
            range: DashboardHistoryRange::H24
        }
    );
}

#[test]
fn any_entity_with_a_chartable_history_is_offered_as_a_chart() {
    let entities = house(vec![
        ent("l1", "Lamp", Kind::Light),
        ent("sw1", "Plug", Kind::Switch),
        ent("b1", "Door", Kind::BinarySensor),
        ent("n1", "Level", Kind::Number),
        ent("a1", "Alarm", Kind::AlarmPanel),
        ent("se1", "Mode", Kind::Select),
        ent("v1", "Robot", Kind::Vacuum),
    ]);
    let got = run("", &entities);
    let mut charted: Vec<_> = of(&got, Section::HistoryCharts)
        .iter()
        .filter_map(|e| target_id(&e.config).map(str::to_owned))
        .collect();
    charted.sort();
    assert_eq!(charted, ["b1", "l1", "n1", "sw1"]);
    // A switch has no card of its own, only the chart.
    assert!(
        got.iter()
            .all(|e| e.section == Section::HistoryCharts || target_id(&e.config) != Some("sw1"))
    );
}

#[test]
fn targets_list_the_rows_of_one_kind() {
    let entities = house(vec![
        ent("l1", "Lamp", Kind::Light),
        ent("l2", "Desk", Kind::Light),
        ent("s1", "Temp", Kind::Sensor),
    ]);
    let areas = [area("salon", "Salon")];
    let flows = [("f1".to_owned(), "Night".to_owned())];
    let lights = targets(Lang::En, WidgetKind::Light, &entities, &areas, &flows);
    assert_eq!(
        lights,
        [
            ("l2".to_owned(), "Desk · Salon".to_owned()),
            ("l1".to_owned(), "Lamp · Salon".to_owned()),
        ]
    );
    let charts = targets(
        Lang::En,
        WidgetKind::HistoryChart,
        &entities,
        &areas,
        &flows,
    );
    assert_eq!(charts.len(), 3);
    let buttons = targets(Lang::En, WidgetKind::FlowButton, &entities, &areas, &flows);
    assert_eq!(buttons[0].0, "f1");
    let rooms = targets(Lang::En, WidgetKind::RoomGroup, &entities, &areas, &flows);
    assert_eq!(rooms[0].0, "salon");
    assert!(targets(Lang::En, WidgetKind::Camera, &entities, &areas, &flows).is_empty());
}

#[test]
fn retarget_keeps_the_title_and_the_range() {
    let mut entity = WidgetConfig::Entity {
        entity_id: "l1".into(),
        title: Some("Hall".into()),
    };
    retarget(&mut entity, "l2".into());
    assert_eq!(
        entity,
        WidgetConfig::Entity {
            entity_id: "l2".into(),
            title: Some("Hall".into())
        }
    );
    let mut chart = WidgetConfig::History {
        entity_id: "s1".into(),
        range: DashboardHistoryRange::D7,
    };
    retarget(&mut chart, "s2".into());
    assert_eq!(
        chart,
        WidgetConfig::History {
            entity_id: "s2".into(),
            range: DashboardHistoryRange::D7
        }
    );
    let mut cams = WidgetConfig::Cameras {
        cameras: vec!["a".into()],
        title: None,
    };
    retarget(&mut cams, "b".into());
    assert_eq!(target_id(&cams), None);
    assert!(matches!(cams, WidgetConfig::Cameras { cameras, .. } if cameras == ["a"]));
}

#[test]
fn empty_sections_have_no_rows() {
    let got = entries(Lang::En, "", &BTreeMap::new(), &[], &[], &[]);
    assert!(got.iter().all(|e| e.section == Section::Energy));
    assert_eq!(got.len(), ENERGY_METRICS.len());
}

#[test]
fn adding_a_light_builds_entity_config_with_smallest_allowed_span() {
    let w = make_widget(
        WidgetKind::Light,
        WidgetConfig::Entity {
            entity_id: "light.a".into(),
            title: None,
        },
    );
    assert_eq!(w.kind, WidgetKind::Light);
    assert_eq!(w.span, Span::X1x1);
    assert_eq!(
        w.config,
        WidgetConfig::Entity {
            entity_id: "light.a".into(),
            title: None
        }
    );
}

#[test]
fn adding_a_history_chart_defaults_to_2x1_and_24h_range() {
    let w = make_widget(
        WidgetKind::HistoryChart,
        WidgetConfig::History {
            entity_id: "x".into(),
            range: DashboardHistoryRange::H24,
        },
    );
    assert_eq!(w.span, Span::X2x1);
    assert!(matches!(
        w.config,
        WidgetConfig::History {
            range: DashboardHistoryRange::H24,
            ..
        }
    ));
}
