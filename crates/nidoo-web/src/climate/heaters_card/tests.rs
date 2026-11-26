use nidoo_climate::doc::{Block, Day, Heater, HhMm, ModeRules, Rules, Setpoint};

use super::*;

fn heater(id: &str) -> Heater {
    Heater {
        entity: id.into(),
        ..Heater::default()
    }
}

fn block(by: Option<&[&str]>) -> Block {
    Block {
        days: vec![Day::Mon],
        from: HhMm(360),
        to: HhMm(480),
        setpoint: Setpoint::Temp(21.0),
        evidence: None,
        heated_by: by.map(|v| v.iter().map(|s| (*s).to_owned()).collect()),
    }
}

fn ids(v: &[&str]) -> Option<Vec<String>> {
    Some(v.iter().map(|s| (*s).to_owned()).collect())
}

#[test]
fn remove_heater_clears_every_reference() {
    let mut s = ZoneSettings {
        heaters: vec![heater("stove"), heater("rad")],
        heated_by: ids(&["stove"]),
        rules: Rules {
            heat: Some(ModeRules {
                base: Setpoint::Temp(19.0),
                blocks: vec![
                    block(Some(&["stove", "rad"])),
                    block(Some(&["stove"])),
                    block(None),
                ],
            }),
            cool: None,
        },
        ..ZoneSettings::default()
    };
    remove_heater(&mut s, "stove");
    assert_eq!(s.heaters, vec![heater("rad")]);
    assert_eq!(s.heated_by, None);
    let blocks = &s.rules.heat.as_ref().unwrap().blocks;
    assert_eq!(blocks[0].heated_by, ids(&["rad"]));
    assert_eq!(blocks[1].heated_by, None);
    assert_eq!(blocks[2].heated_by, None);
}

#[test]
fn remove_heater_keeps_the_rest_of_a_default() {
    let mut s = ZoneSettings {
        heaters: vec![heater("stove"), heater("rad")],
        heated_by: ids(&["unit", "stove"]),
        ..ZoneSettings::default()
    };
    remove_heater(&mut s, "stove");
    assert_eq!(s.heated_by, ids(&["unit"]));
}

#[test]
fn removing_the_last_heater_clears_every_choice() {
    // With no heater left the choices are hidden: none may stay behind.
    let mut s = ZoneSettings {
        heaters: vec![heater("stove")],
        heated_by: ids(&["A", "stove"]),
        rules: Rules {
            heat: Some(ModeRules {
                base: Setpoint::Temp(19.0),
                blocks: vec![block(Some(&["A"]))],
            }),
            cool: None,
        },
        ..ZoneSettings::default()
    };
    remove_heater(&mut s, "stove");
    assert!(s.heaters.is_empty());
    assert_eq!(s.heated_by, None);
    assert_eq!(s.rules.heat.as_ref().unwrap().blocks[0].heated_by, None);
}

#[test]
fn cool_down_stays_within_the_hubs_range() {
    assert_eq!(cool_down(-5.0), 0);
    assert_eq!(cool_down(14.6), 15);
    assert_eq!(cool_down(900.0), 240);
}

fn strings(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| (*s).to_owned()).collect()
}

#[test]
fn ticking_a_heater_writes_an_explicit_default() {
    let s = ZoneSettings {
        heaters: vec![heater("stove")],
        ..ZoneSettings::default()
    };
    let (units, all) = (strings(&["pump"]), strings(&["pump", "stove"]));
    // None shows the units ticked; adding the stove makes both explicit.
    assert_eq!(
        toggled_default(&s, &units, &all, "stove"),
        Some(ids(&["pump", "stove"]))
    );
    // The pump is the only one ticked: it stays.
    assert_eq!(toggled_default(&s, &units, &all, "pump"), None);
    // With both ticked, unticking the pump leaves the stove.
    let both = ZoneSettings {
        heated_by: ids(&["pump", "stove"]),
        ..s
    };
    assert_eq!(
        toggled_default(&both, &units, &all, "pump"),
        Some(ids(&["stove"]))
    );
}

#[test]
fn a_default_equal_to_the_units_goes_back_to_none() {
    let s = ZoneSettings {
        heaters: vec![heater("stove")],
        heated_by: ids(&["pump", "stove"]),
        ..ZoneSettings::default()
    };
    let (units, all) = (strings(&["pump"]), strings(&["pump", "stove"]));
    assert_eq!(toggled_default(&s, &units, &all, "stove"), Some(None));
}

#[test]
fn the_last_ticked_heater_stays() {
    let s = ZoneSettings {
        heaters: vec![heater("stove")],
        heated_by: ids(&["stove"]),
        ..ZoneSettings::default()
    };
    let (units, all) = (strings(&["pump"]), strings(&["pump", "stove"]));
    assert_eq!(toggled_default(&s, &units, &all, "stove"), None);
}

fn labels(v: &[(&str, &str)]) -> Vec<(String, String)> {
    v.iter()
        .map(|(id, label)| ((*id).to_owned(), (*label).to_owned()))
        .collect()
}

#[test]
fn a_reason_names_heaters_and_units_by_their_labels() {
    let named = labels(&[
        ("trv", "Valve (Bedroom)"),
        ("trv2", "Valve (Office)"),
        ("0x1_switch", "Stove (Garage)"),
    ]);
    assert_eq!(
        name_ids(
            "Schedule block 1: 20.0 °C; 0x1_switch cooling down until 14:20; \
                 trv2 held until 15:00; trv unreachable",
            &named
        ),
        "Schedule block 1: 20.0 °C; Stove (Garage) cooling down until 14:20; \
             Valve (Office) held until 15:00; Valve (Bedroom) unreachable"
    );
}

#[test]
fn naming_ids_never_rewrites_a_label_or_a_longer_word() {
    // `trv` is a prefix of `trv2`; the label of `rad` holds `trv`; `radiant` holds `rad`.
    let named = labels(&[
        ("trv", "Valve"),
        ("trv2", "Office"),
        ("rad", "Radiator trv"),
    ]);
    assert_eq!(
        name_ids("rad on; trv2 off; trv off; radiant", &named),
        "Radiator trv on; Office off; Valve off; radiant"
    );
    assert_eq!(
        name_ids("Schedule base: 17.0 °C", &[]),
        "Schedule base: 17.0 °C"
    );
}

#[test]
fn heater_labels_lists_units_first() {
    let s = ZoneSettings {
        heaters: vec![heater("stove"), heater("rad")],
        ..ZoneSettings::default()
    };
    let units = vec![("heat_pump".to_owned(), "Heat pump".to_owned())];
    let got = heater_labels(&s, &units, |id| id.to_uppercase());
    assert_eq!(
        got,
        vec![
            ("heat_pump".to_owned(), "Heat pump".to_owned()),
            ("stove".to_owned(), "STOVE".to_owned()),
            ("rad".to_owned(), "RAD".to_owned()),
        ]
    );
}
