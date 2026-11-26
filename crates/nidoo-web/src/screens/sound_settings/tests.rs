use super::*;
use nidoo_core::model::{Capabilities, EntityCategory, Kind, UiOverride};

fn entity(id: &str, device: Option<(&str, &str)>, value: StateValue) -> EntityView {
    EntityView {
        id: id.into(),
        device_id: device.map(|d| d.0.into()),
        name: id.into(),
        device_name: device.map(|d| d.1.into()),
        label: None,
        kind: Kind::Switch,
        component: "switch".into(),
        protocol_kind: Kind::Switch,
        device_class: None,
        unit: None,
        icon: None,
        category: EntityCategory::Config,
        enabled_by_default: true,
        area: "kitchen".into(),
        part_of: None,
        capabilities: Capabilities::default(),
        ui: UiOverride::default(),
        state: EntityState {
            value,
            availability: Availability::Online,
            ..EntityState::default()
        },
    }
}

fn map(list: Vec<EntityView>) -> BTreeMap<String, EntityView> {
    list.into_iter().map(|e| (e.id.clone(), e)).collect()
}

fn full(device: &str, name: &str, allowed: bool) -> Vec<EntityView> {
    let d = Some((device, name));
    vec![
        entity(&format!("{device}_volume"), d, StateValue::Number(30.0)),
        entity(&format!("{device}_speech"), d, StateValue::Number(40.0)),
        entity(&format!("{device}_mute"), d, StateValue::OnOff(false)),
        entity(&format!("{device}_allowed"), d, StateValue::OnOff(allowed)),
        entity(&format!("{device}_connected"), d, StateValue::OnOff(true)),
    ]
}

#[test]
fn groups_the_four_entities_of_a_device_and_sorts_by_name() {
    let mut all = full("nidoo_sound_bbb", "kitchen", true);
    all.extend(full("nidoo_sound_aaa", "Office", false));
    all.push(entity(
        "lamp",
        Some(("zigbee_lamp", "Lamp")),
        StateValue::OnOff(true),
    ));
    all.push(entity("loose", None, StateValue::OnOff(true)));
    let found = satellites(&map(all));
    let names: Vec<_> = found.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["kitchen", "Office"]);
    let office = &found[1];
    assert!(office.allowed.is_some() && office.mute.is_some());
    assert!(office.volume.is_some() && office.connected.is_some());
    assert!(office.speech.is_some());
    assert_eq!(
        office.volume.as_ref().map(|e| e.id.as_str()),
        Some("nidoo_sound_aaa_volume")
    );
    assert_eq!(
        office.speech.as_ref().map(|e| e.id.as_str()),
        Some("nidoo_sound_aaa_speech")
    );
    assert!(!office.is_allowed());
    assert!(found[0].is_allowed() && found[0].is_connected());
}

#[test]
fn an_offline_service_means_not_connected() {
    let mut all = full("nidoo_sound_aaa", "Office", true);
    for e in &mut all {
        e.state.availability = Availability::Offline;
    }
    let found = satellites(&map(all));
    assert!(!found[0].service_online());
    assert!(!found[0].is_connected());
}

#[test]
fn the_room_is_the_volume_entity_room() {
    // The card opens the Volume entity's sheet, and the hub reads the Volume's room: a room
    // changed there must win over the other entities' rooms.
    let mut all = full("nidoo_sound_aaa", "A", true);
    for e in &mut all {
        e.area = if e.id.ends_with("_volume") {
            "office"
        } else {
            "kitchen"
        }
        .into();
    }
    let found = satellites(&map(all));
    assert_eq!(found[0].area, "office");
}

#[test]
fn satellite_of_picks_one_device() {
    let mut all = full("nidoo_sound_aaa", "A", true);
    all.extend(full("nidoo_sound_bbb", "B", false));
    let all = map(all);
    let b = satellite_of(&all, "nidoo_sound_bbb").expect("b");
    assert_eq!(b.name, "B");
    assert!(!b.is_allowed());
    assert!(b.volume.is_some() && b.mute.is_some() && b.connected.is_some());
    assert_eq!(satellite_of(&all, "nidoo_sound_zzz"), None);
}

#[test]
fn satellite_rooms_lists_rooms_holding_one_in_area_order() {
    let mut all = full("nidoo_sound_aaa", "A", true);
    let mut office = full("nidoo_sound_bbb", "B", true);
    for e in &mut office {
        e.area = "office".into();
    }
    all.extend(office);
    let mut loose = full("nidoo_sound_ccc", "C", true);
    for e in &mut loose {
        e.area = UNASSIGNED_AREA.into();
    }
    all.extend(loose);
    let found = satellites(&map(all));
    let area = |id: &str, name: &str| AreaView {
        id: id.into(),
        name: name.into(),
        icon: None,
        order: None,
        entity_count: 0,
        primary_count: 0,
        group_names: Default::default(),
        group_favorites: Default::default(),
    };
    let areas = vec![
        area("office", "Office"),
        area("bath", "Bath"),
        area("kitchen", "Kitchen"),
        area(UNASSIGNED_AREA, "Unassigned"),
    ];
    assert_eq!(
        satellite_rooms(&found, &areas),
        [
            ("office".to_owned(), "Office".to_owned()),
            ("kitchen".to_owned(), "Kitchen".to_owned())
        ]
    );
}

#[test]
fn speak_request_sends_null_for_every_room() {
    let all = serde_json::to_value(speak_request(" Hello ", "")).unwrap();
    assert_eq!(all["text"], "Hello");
    assert_eq!(all["room"], serde_json::Value::Null);
    let one = serde_json::to_value(speak_request("Hello", "kitchen")).unwrap();
    assert_eq!(one["room"], "kitchen");
    assert_eq!(one["satellite"], serde_json::Value::Null);
}

#[test]
fn the_test_request_names_the_satellite_by_its_short_id() {
    let found = satellites(&map(full("nidoo_sound_aaa", "Desk", true)));
    assert_eq!(found[0].short_id(), "aaa");
    let req = test_request(&found[0], Lang::En);
    assert_eq!(req.satellite.as_deref(), Some("aaa"));
    assert_eq!(req.text, "Speaker test: Desk");
    assert_eq!(req.room, None);
}

#[test]
fn the_name_opens_the_volume_page() {
    // The Volume is the satellite's card: the others fold into it, and its room is the
    // one the room lists and the hub read. A room set on another entity's page is lost.
    let found = satellites(&map(full("nidoo_sound_aaa", "Desk", true)));
    assert_eq!(
        found[0].page_entity().map(|e| e.id.as_str()),
        Some("nidoo_sound_aaa_volume")
    );
}

#[test]
fn a_rename_on_the_volume_page_names_the_satellite() {
    // Renaming from the page the name opens sets the Volume's override, not the device.
    let mut all = full("nidoo_sound_aaa", "Media player – Chrome on Android", true);
    all.extend(full("nidoo_sound_bbb", "Kitchen", true));
    let volume = all.iter_mut().find(|e| e.id == "nidoo_sound_aaa_volume");
    volume.expect("volume").ui.name = Some("Salon".into());
    let found = satellites(&map(all));
    let names: Vec<_> = found.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["Kitchen", "Salon"]);
}

#[test]
fn a_blank_rename_keeps_the_device_name() {
    let mut all = full("nidoo_sound_aaa", "Desk", true);
    all[0].ui.name = Some("  ".into());
    assert_eq!(satellites(&map(all))[0].name, "Desk");
}

#[test]
fn remove_path_names_the_satellite_by_its_short_id() {
    let found = satellites(&map(full("nidoo_sound_aaa", "Desk", true)));
    assert_eq!(remove_path(&found[0]), "/api/v1/sound/satellites/aaa");
}

#[test]
fn the_room_moves_every_entity_of_the_satellite() {
    let mut all = full("nidoo_sound_aaa", "Desk", true);
    all.push(entity(
        "nidoo_sound_aaa_address",
        Some(("nidoo_sound_aaa", "Desk")),
        StateValue::Text("192.0.2.1".into()),
    ));
    all.extend(full("nidoo_sound_bbb", "Other", true));
    let ids = device_entity_ids(&map(all), "nidoo_sound_aaa");
    assert_eq!(
        ids,
        [
            "nidoo_sound_aaa_address",
            "nidoo_sound_aaa_allowed",
            "nidoo_sound_aaa_connected",
            "nidoo_sound_aaa_mute",
            "nidoo_sound_aaa_speech",
            "nidoo_sound_aaa_volume",
        ]
    );
}

#[test]
fn room_choices_leave_out_the_catch_all() {
    let area = |id: &str, name: &str| AreaView {
        id: id.into(),
        name: name.into(),
        icon: None,
        order: None,
        entity_count: 0,
        primary_count: 0,
        group_names: Default::default(),
        group_favorites: Default::default(),
    };
    let areas = vec![
        area("office", "Office"),
        area(UNASSIGNED_AREA, "Unassigned"),
        area("kitchen", "Kitchen"),
    ];
    assert_eq!(
        room_choices(&areas),
        [
            ("office".to_owned(), "Office".to_owned()),
            ("kitchen".to_owned(), "Kitchen".to_owned())
        ]
    );
}

#[test]
fn no_satellite_without_the_prefix() {
    assert!(satellites(&map(full("other_aaa", "X", true))).is_empty());
}

#[test]
fn status_counts_the_satellites_waiting() {
    let mut all = full("nidoo_sound_aaa", "A", false);
    all.extend(full("nidoo_sound_bbb", "B", false));
    all.extend(full("nidoo_sound_ccc", "C", true));
    let found = satellites(&map(all));
    assert_eq!(status_label(&found, Lang::En), "2 waiting");
    assert_eq!(status_label(&found, Lang::Fr), "2 en attente");
    assert_eq!(status_label(&found[2..], Lang::En), "");
}
