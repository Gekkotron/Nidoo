use super::*;
use nidoo_api::{ActionReport, ClimateAction, DeviceAction, FlowAction, Outcome, Problem};
use nidoo_core::model::{Capabilities, Command, EntityCategory, EntityState, Kind, UiOverride};

fn amb(id: &str, order: u32, pinned: bool) -> AmbianceView {
    AmbianceView {
        id: id.into(),
        ambiance: Some(Ambiance {
            id: id.into(),
            name: id.into(),
            subtitle: None,
            icon: None,
            pinned,
            order,
            actions: Vec::new(),
            extra: Default::default(),
        }),
        invalid: None,
    }
}

fn set(ids: &[&str]) -> BTreeSet<String> {
    ids.iter().map(|s| (*s).to_owned()).collect()
}

fn ids(list: &[Ambiance]) -> Vec<&str> {
    list.iter().map(|a| a.id.as_str()).collect()
}

#[test]
fn pinned_for_home_caps_at_four_by_order() {
    let list: Vec<_> = [("e", 5), ("a", 1), ("c", 3), ("b", 2), ("f", 6), ("d", 4)]
        .into_iter()
        .map(|(id, o)| amb(id, o, true))
        .collect();
    assert_eq!(ids(&pinned_for_home(&list)), ["a", "b", "c", "d"]);
}

#[test]
fn pinned_for_home_skips_invalid() {
    let mut list = vec![amb("a", 1, true), amb("b", 2, true)];
    list[0].ambiance = None;
    list[0].invalid = Some("broken".into());
    assert_eq!(ids(&pinned_for_home(&list)), ["b"]);
}

#[test]
fn pinned_for_home_skips_unpinned() {
    let list = vec![amb("a", 1, false), amb("b", 2, true)];
    assert_eq!(ids(&pinned_for_home(&list)), ["b"]);
}

#[test]
fn slug_id_suffixes_on_collision() {
    assert_eq!(
        slug_id("Bonne nuit", &set(&["bonne_nuit"]), "ambiance"),
        "bonne_nuit_2"
    );
    assert_eq!(slug_id("!!!", &set(&[]), "ambiance"), "ambiance");
}

#[test]
fn next_order_after_max() {
    assert_eq!(next_order(&[]), 1);
    let mut list = vec![amb("a", 3, true), amb("b", 7, false), amb("c", 2, true)];
    list.push(AmbianceView {
        id: "x".into(),
        ambiance: None,
        invalid: Some("bad".into()),
    });
    assert_eq!(next_order(&list), 8);
}

#[test]
fn examples_fr_and_en() {
    let fr = examples(Lang::Fr, &set(&[]), 1);
    let names: Vec<_> = fr.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "Bonne nuit",
            "Je m'absente",
            "Soirée tranquille",
            "Un peu de lecture"
        ]
    );
    let en = examples(Lang::En, &set(&["good_night"]), 5);
    let names: Vec<_> = en.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "Good night",
            "I'm heading out",
            "Quiet evening",
            "A little reading"
        ]
    );
    assert_eq!(en[0].id, "good_night_2");
    let unique: BTreeSet<_> = en.iter().map(|a| &a.id).collect();
    assert_eq!(unique.len(), 4);
    assert_eq!(en.iter().map(|a| a.order).collect::<Vec<_>>(), [5, 6, 7, 8]);
    assert!(en.iter().all(|a| a.pinned));
    let icons: Vec<_> = en.iter().map(|a| a.icon.as_deref().unwrap()).collect();
    assert_eq!(icons, ["moon", "door_out", "sofa", "book"]);
    assert_eq!(en[0].subtitle.as_deref(), Some("The house goes to sleep"));
    assert_eq!(en[0].actions.len(), 3);
    assert_eq!(en[1].actions.len(), 2);
    assert!(en[3].actions.is_empty());
    assert_eq!(
        en[2].actions,
        [AmbianceAction::Category(CategoryAction {
            what: Category::Lights,
            to: Target::On,
            brightness: Some(30),
            area: None,
            except: Vec::new(),
        })]
    );
}

fn entity(id: &str, name: &str) -> EntityView {
    EntityView {
        id: id.into(),
        device_id: None,
        name: name.into(),
        device_name: None,
        label: None,
        kind: Kind::Light,
        component: Kind::Light.component().to_string(),
        protocol_kind: Kind::Light,
        device_class: None,
        unit: None,
        icon: None,
        category: EntityCategory::Primary,
        enabled_by_default: true,
        area: "living".into(),
        part_of: None,
        capabilities: Capabilities::default(),
        ui: UiOverride::default(),
        state: EntityState::default(),
    }
}

fn cat(what: Category, to: Target, brightness: Option<u8>, area: Option<&str>) -> AmbianceAction {
    AmbianceAction::Category(CategoryAction {
        what,
        to,
        brightness,
        area: area.map(Into::into),
        except: Vec::new(),
    })
}

#[test]
fn action_sentences() {
    let entities: BTreeMap<_, _> = [entity("lamp", "Desk lamp")]
        .into_iter()
        .map(|e| (e.id.clone(), e))
        .collect();
    let areas: Vec<AreaView> = serde_json::from_str(
        r#"[{"id":"living","name":"Living room","icon":null,"order":null,"entity_count":0,"primary_count":0}]"#,
    )
    .unwrap();
    let flows = vec![("away".to_owned(), "Away lights".to_owned())];
    let zones = vec![("z1".to_owned(), "Bedroom".to_owned())];
    let house = HouseView {
        entities: &entities,
        areas: &areas,
        flows: &flows,
        zones: &zones,
    };
    let s = |a: &AmbianceAction, l| action_sentence(a, &house, l);
    let device = |entity: &str| {
        AmbianceAction::Device(DeviceAction {
            entity: entity.into(),
            command: Command::TurnOff,
        })
    };
    let heat = |zone: Option<&str>, clear| {
        AmbianceAction::Climate(ClimateAction {
            zone: zone.map(Into::into),
            setpoint: Some(serde_json::json!(16)),
            hours: Some(12),
            clear,
            ..Default::default()
        })
    };
    let flow = |f: &str, d| {
        AmbianceAction::Flow(FlowAction {
            flow: f.into(),
            action: d,
        })
    };

    let lights_off = cat(Category::Lights, Target::Off, None, None);
    assert_eq!(s(&lights_off, Lang::En), "Every light in the house: off");
    assert_eq!(
        s(&lights_off, Lang::Fr),
        "Tous les éclairages de la maison : éteindre"
    );
    let dim = cat(Category::Lights, Target::On, Some(30), Some("living"));
    assert_eq!(s(&dim, Lang::En), "Every light in Living room: on at 30 %");
    assert_eq!(
        s(&dim, Lang::Fr),
        "Tous les éclairages — Living room : allumer à 30 %"
    );
    let shutters = cat(Category::Covers, Target::Close, None, None);
    assert_eq!(s(&shutters, Lang::En), "Every shutter in the house: close");
    assert_eq!(
        s(&shutters, Lang::Fr),
        "Tous les volets de la maison : fermer"
    );
    let plugs = cat(Category::Plugs, Target::On, None, None);
    assert_eq!(s(&plugs, Lang::En), "Every plug in the house: on");
    assert_eq!(
        s(&plugs, Lang::Fr),
        "Toutes les prises de la maison : allumer"
    );
    let open = cat(Category::Covers, Target::Open, None, Some("gone"));
    assert_eq!(s(&open, Lang::En), "Every shutter in gone (missing): open");
    assert_eq!(
        s(&open, Lang::Fr),
        "Tous les volets — gone (introuvable) : ouvrir"
    );

    assert_eq!(s(&device("lamp"), Lang::En), "Desk lamp: turn off");
    assert_eq!(s(&device("ghost"), Lang::En), "ghost (missing): turn off");
    assert_eq!(
        s(&device("ghost"), Lang::Fr),
        "ghost (introuvable) : éteindre"
    );

    let on_lamp = |command| {
        AmbianceAction::Device(DeviceAction {
            entity: "lamp".into(),
            command,
        })
    };
    let mode = on_lamp(Command::SetHvacMode {
        mode: "heat".into(),
    });
    assert_eq!(s(&mode, Lang::En), "Desk lamp: set mode to heat");
    assert_eq!(s(&mode, Lang::Fr), "Desk lamp : régler en mode chauffage");
    let control = on_lamp(Command::Control {
        control: "percentage".into(),
        value: Some(serde_json::json!(60)),
    });
    assert_eq!(s(&control, Lang::En), "Desk lamp: set percentage to 60");
    assert_eq!(
        s(&control, Lang::Fr),
        "Desk lamp : régler sur 60 (percentage)"
    );

    assert_eq!(
        s(&heat(None, false), Lang::En),
        "Heating, every zone: 16 °C for 12 h"
    );
    assert_eq!(
        s(&heat(Some("z1"), false), Lang::Fr),
        "Chauffage, Bedroom : 16 °C pendant 12 h"
    );
    assert_eq!(
        s(&heat(None, true), Lang::En),
        "Heating, every zone: back to the schedule"
    );
    assert_eq!(
        s(&heat(None, true), Lang::Fr),
        "Chauffage, toutes les zones : retour au programme"
    );
    assert_eq!(
        s(&heat(Some("z9"), false), Lang::En),
        "Heating, z9 (missing): 16 °C for 12 h"
    );

    assert_eq!(
        s(&flow("away", FlowDo::Enable), Lang::En),
        "Automation Away lights: switch on"
    );
    assert_eq!(
        s(&flow("away", FlowDo::Disable), Lang::Fr),
        "Automatisation Away lights : désactiver"
    );
    assert_eq!(
        s(&flow("nope", FlowDo::Run), Lang::En),
        "Automation nope (missing): run"
    );
}

#[test]
fn failed_count_sums_actions() {
    let problem = |id: &str| Problem {
        id: id.into(),
        reason: "x".into(),
    };
    let line = |index, failed: Vec<Problem>| ActionReport {
        index,
        outcome: Outcome::Failed,
        total: 3,
        failed,
        skipped: vec![problem("s")],
        reason: None,
    };
    let report = ApplyReport {
        ambiance: "a".into(),
        started_at: 0,
        actions: vec![
            line(0, vec![problem("a"), problem("b")]),
            line(1, Vec::new()),
            line(2, vec![problem("c")]),
        ],
    };
    assert_eq!(failed_count(&report), 3);
}
