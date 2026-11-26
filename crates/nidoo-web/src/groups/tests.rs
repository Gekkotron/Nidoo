use super::*;
use crate::rooms::FAVORITES;
use nidoo_core::model::{
    Availability, Capabilities, CoverState, EntityCategory, EntityState, Kind, LightState,
    StateValue, UiOverride,
};

fn entity(id: &str, kind: Kind, area: &str, ui: UiOverride) -> EntityView {
    EntityView {
        id: id.into(),
        device_id: None,
        name: id.into(),
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
        area: area.into(),
        part_of: None,
        capabilities: Capabilities::default(),
        ui,
        state: EntityState::default(),
    }
}

fn light(id: &str, area: &str) -> EntityView {
    entity(id, Kind::Light, area, UiOverride::default())
}

fn cover(id: &str, area: &str) -> EntityView {
    entity(id, Kind::Cover, area, UiOverride::default())
}

fn area(id: &str) -> AreaView {
    AreaView {
        id: id.into(),
        name: id.into(),
        icon: None,
        order: None,
        entity_count: 1,
        primary_count: 1,
        group_names: Default::default(),
        group_favorites: Default::default(),
    }
}

fn single(id: &str) -> HomeItem {
    HomeItem::Entity(id.into())
}

fn with_cover(mut e: EntityView, state: Option<&str>, position: Option<u8>) -> EntityView {
    e.state.value = StateValue::Cover(CoverState {
        state: state.map(Into::into),
        position,
    });
    e
}

fn with_light(mut e: EntityView, on: bool, brightness: Option<u8>) -> EntityView {
    e.state.value = StateValue::Light(LightState {
        on,
        brightness,
        ..Default::default()
    });
    e
}

fn open(id: &str) -> EntityView {
    with_cover(cover(id, "living"), Some("open"), None)
}

fn closed(id: &str) -> EntityView {
    with_cover(cover(id, "living"), Some("closed"), None)
}

fn no_children() -> BTreeMap<String, EntityView> {
    BTreeMap::new()
}

fn items(all: &[EntityView], areas: &[AreaView], chip: &str) -> Vec<HomeItem> {
    home_items(all.iter(), areas, chip, &BTreeMap::new())
}

fn favourite(mut e: EntityView) -> EntityView {
    e.ui.favorite = true;
    e
}

#[test]
fn hidden_and_energy_members_leave() {
    let hidden = entity(
        "blind_b",
        Kind::Cover,
        "living",
        UiOverride {
            hidden: true,
            ..Default::default()
        },
    );
    let mut grid = light("lamp_b", "living");
    grid.ui.energy_role = Some("grid".into());
    let all = [
        cover("blind_a", "living"),
        hidden,
        light("lamp_a", "living"),
        grid,
    ];
    assert_eq!(
        items(&all, &[area("living")], "living"),
        [single("blind_a"), single("lamp_a")]
    );
}

#[test]
fn pinned_cards_go_first() {
    let pinned = entity(
        "zplug",
        Kind::Switch,
        "living",
        UiOverride {
            order: Some(0),
            ..Default::default()
        },
    );
    let all = [
        entity("aplug", Kind::Switch, "living", UiOverride::default()),
        light("lamp_a", "living"),
        pinned,
    ];
    assert_eq!(
        items(&all, &[area("living")], "living"),
        [single("zplug"), single("aplug"), single("lamp_a")]
    );
}

#[test]
fn favorites_show_favourite_entities_by_area_order() {
    let all = [
        favourite(light("a1", "attic")),
        light("a2", "attic"),
        favourite(light("b1", "bath")),
    ];
    let areas = [area("bath"), area("attic")];
    assert_eq!(items(&all, &areas, FAVORITES), [single("b1"), single("a1")]);
    assert_eq!(favorites_count(all.iter(), &areas, &BTreeMap::new()), 2);
}

#[test]
fn offline_groups_read_unavailable() {
    let mut a = open("a");
    let mut b = open("b");
    a.state.availability = Availability::Offline;
    b.state.availability = Availability::Offline;
    let s = summary(&[a, b], &no_children());
    assert_eq!(cover_text(Lang::En, &s), "Unavailable");
    assert_eq!(light_text(Lang::En, &s), "Unavailable");
}

#[test]
fn cover_texts() {
    let all = [open("a"), open("b")];
    let s = summary(&all, &no_children());
    assert_eq!(cover_text(Lang::En, &s), "All open");
    let all = [open("a"), open("b"), closed("c")];
    assert_eq!(
        cover_text(Lang::En, &summary(&all, &no_children())),
        "2 open · 1 closed"
    );
    let all = [closed("a"), closed("b")];
    assert_eq!(
        cover_text(Lang::En, &summary(&all, &no_children())),
        "All closed"
    );
    let all = [
        with_cover(cover("a", "living"), Some("opening"), None),
        with_cover(cover("b", "living"), Some("Closing"), None),
    ];
    let s = summary(&all, &no_children());
    assert_eq!(s.moving, Some(Motion::Opening));
    assert_eq!(cover_text(Lang::En, &s), "Opening…");
    let all = [
        with_cover(cover("a", "living"), Some("closing"), None),
        open("b"),
    ];
    assert_eq!(
        cover_text(Lang::En, &summary(&all, &no_children())),
        "Closing…"
    );
}

#[test]
fn cover_positions() {
    let all = [
        with_cover(cover("a", "living"), None, Some(150)),
        open("b"),
        closed("c"),
    ];
    assert_eq!(summary(&all, &no_children()).positions, [100, 100, 0]);
}

#[test]
fn folded_moving_child_makes_a_cover_move() {
    let mut child = entity(
        "a_moving",
        Kind::BinarySensor,
        "living",
        UiOverride::default(),
    );
    child.name = "Moving".into();
    child.part_of = Some("a".into());
    child.state.value = StateValue::Text("closing".into());
    let mut plain = child.clone();
    plain.id = "b_moving".into();
    plain.part_of = Some("b".into());
    plain.state.value = StateValue::OnOff(true);
    let map: BTreeMap<_, _> = [child, plain]
        .into_iter()
        .map(|e| (e.id.clone(), e))
        .collect();
    let only_a = [open("a"), open("c")];
    assert_eq!(summary(&only_a, &map).moving, Some(Motion::Closing));
    let only_b = [open("b"), open("c")];
    assert_eq!(summary(&only_b, &map).moving, Some(Motion::Opening));
}

#[test]
fn light_texts() {
    let on = |id| with_light(light(id, "living"), true, None);
    let off = |id| with_light(light(id, "living"), false, None);
    let s = summary(&[on("a"), on("b")], &no_children());
    assert_eq!(light_text(Lang::En, &s), "All on");
    let s = summary(&[off("a"), off("b")], &no_children());
    assert_eq!(light_text(Lang::En, &s), "All off");
    assert_eq!(
        light_toggle(&s),
        Command::TurnOn {
            brightness: None,
            color: None,
            effect: None
        }
    );
    let s = summary(&[on("a"), on("b"), off("c")], &no_children());
    assert_eq!(light_text(Lang::En, &s), "2 of 3 on");
    assert_eq!(light_toggle(&s), Command::TurnOff);
}

#[test]
fn availability_is_any_member_online() {
    let mut a = light("a", "living");
    a.state.availability = Availability::Offline;
    let mut b = light("b", "living");
    b.state.availability = Availability::Online;
    assert!(summary(&[a.clone(), b], &no_children()).available);
    a.state.availability = Availability::Offline;
    assert!(!summary(&[a.clone(), a], &no_children()).available);
}

fn offline(mut e: EntityView) -> EntityView {
    e.state.availability = Availability::Offline;
    e
}

#[test]
fn targets_skip_unavailable() {
    let all = [light("a", "living"), offline(light("b", "living"))];
    assert_eq!(targets(&all, &Command::TurnOff), ["a"]);
}

#[test]
fn targets_empty_when_all_offline() {
    let all = [offline(light("a", "living")), offline(light("b", "living"))];
    assert!(targets(&all, &Command::TurnOff).is_empty());
}

#[test]
fn set_position_only_to_capable() {
    let mut a = cover("a", "living");
    a.capabilities.position = true;
    let all = [a, cover("b", "living")];
    assert_eq!(targets(&all, &Command::SetPosition { position: 40 }), ["a"]);
    assert_eq!(targets(&all, &Command::Open), ["a", "b"]);
}

#[test]
fn brightness_only_to_dimmable() {
    let mut a = light("a", "living");
    a.capabilities.brightness = true;
    let all = [a, light("b", "living")];
    let dim = Command::TurnOn {
        brightness: Some(80),
        color: None,
        effect: None,
    };
    let plain = Command::TurnOn {
        brightness: None,
        color: None,
        effect: None,
    };
    assert_eq!(targets(&all, &dim), ["a"]);
    assert_eq!(targets(&all, &plain), ["a", "b"]);
}

#[test]
fn means() {
    let mut a = with_cover(cover("a", "living"), None, Some(20));
    a.capabilities.position = true;
    let mut b = with_cover(cover("b", "living"), None, Some(50));
    b.capabilities.position = true;
    let unknown = cover("c", "living");
    assert_eq!(mean_position(&[a, b, unknown.clone()]), 35);
    assert_eq!(mean_position(&[unknown]), 0);

    let dim = |id: &str, on, b| {
        let mut e = with_light(light(id, "living"), on, b);
        e.capabilities.brightness = true;
        e
    };
    let all = [
        dim("a", true, Some(100)),
        dim("b", true, Some(201)),
        dim("c", false, Some(9)),
    ];
    assert_eq!(mean_brightness(&all), 151);
    assert_eq!(mean_brightness(&[dim("a", false, Some(9))]), 255);
}

fn hand_group(id: &str, members: &[&str], hide_members: bool) -> (String, GroupView) {
    let view = GroupView {
        id: id.into(),
        name: id.into(),
        kind: Kind::Light,
        members: members
            .iter()
            .map(|m| nidoo_api::GroupMemberView {
                id: (*m).to_owned(),
                name: (*m).to_owned(),
                present: true,
                mismatch: false,
            })
            .collect(),
        hide_members,
        entity: entity(
            &format!("group_{id}"),
            Kind::Light,
            "living",
            UiOverride::default(),
        ),
        used_by_flows: Vec::new(),
    };
    (id.to_owned(), view)
}

fn ids(items: Vec<HomeItem>) -> Vec<String> {
    items
        .into_iter()
        .map(|i| match i {
            HomeItem::Entity(id) => id,
        })
        .collect()
}

#[test]
fn member_hidden_when_any_group_hides_it() {
    let all = [
        light("lamp", "living"),
        cover("group_a", "living"),
        entity("group_b", Kind::Switch, "living", UiOverride::default()),
    ];
    let groups: BTreeMap<_, _> = [
        hand_group("a", &["lamp"], true),
        hand_group("b", &["lamp"], false),
    ]
    .into();
    let got = ids(home_items(all.iter(), &[area("living")], "living", &groups));
    assert!(got.contains(&"group_a".to_owned()));
    assert!(got.contains(&"group_b".to_owned()));
    assert!(!got.contains(&"lamp".to_owned()));
}

#[test]
fn favourited_member_still_shows_on_favorites() {
    let fav = UiOverride {
        favorite: true,
        ..Default::default()
    };
    let all = [
        entity("lamp", Kind::Light, "living", fav),
        light("other", "living"),
    ];
    let groups: BTreeMap<_, _> = [hand_group("a", &["lamp", "other"], true)].into();
    let got = ids(home_items(
        all.iter(),
        &[area("living")],
        FAVORITES,
        &groups,
    ));
    assert_eq!(got, ["lamp"]);
}

#[test]
fn nothing_hidden_when_no_group_hides_members() {
    let all = [light("lamp", "living"), cover("shutter", "living")];
    let groups: BTreeMap<_, _> = [hand_group("a", &["lamp", "shutter"], false)].into();
    let got = ids(home_items(all.iter(), &[area("living")], "living", &groups));
    assert_eq!(got.len(), 2);
}

#[test]
fn empty_groups_leave_every_chip_unchanged() {
    let fav = UiOverride {
        favorite: true,
        ..Default::default()
    };
    let all = [
        entity("lamp", Kind::Light, "living", fav),
        cover("shutter", "kitchen"),
    ];
    let areas = [area("living"), area("kitchen")];
    for chip in ["living", "kitchen", FAVORITES] {
        let got = home_items(all.iter(), &areas, chip, &BTreeMap::new());
        let with_unhidden = home_items(
            all.iter(),
            &areas,
            chip,
            &[hand_group("a", &["lamp", "shutter"], false)].into(),
        );
        assert_eq!(got, with_unhidden, "{chip}");
    }
    assert_eq!(items(&all, &areas, FAVORITES), [single("lamp")]);
}
