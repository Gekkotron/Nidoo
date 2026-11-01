use nidoo_web::cards::group::{range_of, switch_text, thermostat_text};
use nidoo_web::i18n::Lang;
use nidoo_web::i18n::devices::{DeviceText, device_text};
use nidoo_web::i18n::groups::{members_count, subtitle};
use serde_json::{Map, Value, json};

#[test]
fn switch_group_off_says_all_off() {
    assert_eq!(switch_text(Lang::En, false, None), "All off");
}

#[test]
fn switch_group_on_says_all_on() {
    assert_eq!(switch_text(Lang::En, true, None), "All on");
}

#[test]
fn switch_group_counts_decide_when_members_differ() {
    assert_eq!(switch_text(Lang::En, true, Some((2, 3))), "2 of 3 on");
    assert_eq!(switch_text(Lang::En, true, Some((3, 3))), "All on");
    assert_eq!(switch_text(Lang::En, false, Some((0, 3))), "All off");
}

#[test]
fn switch_group_speaks_french() {
    assert_eq!(switch_text(Lang::Fr, true, Some((2, 3))), "2 sur 3 allumés");
    assert_eq!(switch_text(Lang::Fr, true, Some((1, 3))), "1 sur 3 allumé");
    assert_eq!(switch_text(Lang::Fr, false, None), "Tout éteint");
}

#[test]
fn climate_group_with_targets_19_and_21_shows_the_range() {
    let attributes: Map<String, Value> = json!({ "range": [19.0, 21.0] })
        .as_object()
        .unwrap()
        .clone();
    let range = range_of(&attributes);
    assert_eq!(range, Some([19.0, 21.0]));
    assert_eq!(
        thermostat_text(Lang::En, range, Some("heat"), None),
        "19–21\u{a0}°C"
    );
}

#[test]
fn climate_group_without_range_shows_the_mode_or_mixed() {
    assert_eq!(thermostat_text(Lang::En, None, Some("heat"), None), "Heat");
    assert_eq!(thermostat_text(Lang::En, None, None, None), "Mixed");
}

#[test]
fn malformed_range_is_ignored() {
    let attributes: Map<String, Value> = json!({ "range": [19.0] }).as_object().unwrap().clone();
    assert_eq!(range_of(&attributes), None);
}

#[test]
fn button_group_label_is_press_all() {
    assert_eq!(device_text(Lang::En, DeviceText::PressAll), "Press all");
    assert_eq!(device_text(Lang::Fr, DeviceText::PressAll), "Tout appuyer");
    assert_eq!(thermostat_text(Lang::Fr, None, None, None), "Mixte");
}

mod summaries {
    use std::collections::BTreeMap;

    use nidoo_api::{EntityView, GroupMemberView, GroupView};
    use nidoo_core::model::{
        Capabilities, CoverState, EntityCategory, EntityState, Kind, LightState, StateValue,
        UiOverride,
    };
    use nidoo_web::groups::{cover_text, light_text};
    use nidoo_web::i18n::Lang;

    fn entity(id: &str, kind: Kind, value: StateValue) -> EntityView {
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
            area: String::new(),
            part_of: None,
            capabilities: Capabilities::default(),
            ui: UiOverride::default(),
            state: EntityState {
                value,
                ..Default::default()
            },
        }
    }

    fn group(kind: Kind, members: &[EntityView]) -> BTreeMap<String, GroupView> {
        let view = GroupView {
            id: "g".into(),
            name: "G".into(),
            kind,
            members: members
                .iter()
                .map(|m| GroupMemberView {
                    id: m.id.clone(),
                    name: m.id.clone(),
                    present: true,
                    mismatch: false,
                })
                .collect(),
            hide_members: true,
            entity: entity("group_g", kind, StateValue::Unknown),
            used_by_flows: vec![],
        };
        BTreeMap::from([("g".to_owned(), view)])
    }

    fn by_id(members: &[EntityView]) -> BTreeMap<String, EntityView> {
        members.iter().map(|m| (m.id.clone(), m.clone())).collect()
    }

    fn cover(id: &str, state: &str) -> EntityView {
        entity(
            id,
            Kind::Cover,
            StateValue::Cover(CoverState {
                state: Some(state.into()),
                position: None,
            }),
        )
    }

    fn light(id: &str, on: bool) -> EntityView {
        entity(
            id,
            Kind::Light,
            StateValue::Light(LightState {
                on,
                ..Default::default()
            }),
        )
    }

    #[test]
    fn cover_group_says_all_open_and_n_open_m_closed() {
        let all = [cover("a", "open"), cover("b", "open")];
        let s =
            nidoo_web::cards::group::group_summary("g", &group(Kind::Cover, &all), &by_id(&all))
                .unwrap();
        assert_eq!(cover_text(Lang::En, &s), "All open");
        let mixed = [cover("a", "open"), cover("b", "closed")];
        let s = nidoo_web::cards::group::group_summary(
            "g",
            &group(Kind::Cover, &mixed),
            &by_id(&mixed),
        )
        .unwrap();
        assert_eq!(cover_text(Lang::En, &s), "1 open · 1 closed");
    }

    #[test]
    fn light_group_says_all_on_and_n_of_m_on() {
        let all = [light("a", true), light("b", true)];
        let s =
            nidoo_web::cards::group::group_summary("g", &group(Kind::Light, &all), &by_id(&all))
                .unwrap();
        assert_eq!(light_text(Lang::En, &s), "All on");
        let mixed = [light("a", true), light("b", false)];
        let s = nidoo_web::cards::group::group_summary(
            "g",
            &group(Kind::Light, &mixed),
            &by_id(&mixed),
        )
        .unwrap();
        assert_eq!(light_text(Lang::En, &s), "1 of 2 on");
    }
}

#[test]
fn group_member_counts_agree_in_french() {
    assert_eq!(members_count(Lang::Fr, 0), "0 appareil");
    assert_eq!(members_count(Lang::Fr, 1), "1 appareil");
    assert_eq!(members_count(Lang::Fr, 3), "3 appareils");
    assert_eq!(members_count(Lang::En, 1), "1 device");
    assert_eq!(
        subtitle(Lang::Fr, "salon", nidoo_core::model::Kind::Light, 3),
        "salon · 3 lumières"
    );
}
