//! Pure presentation of the Energy tab: entity views → energy inputs, values and labels. The
//! wording is ported from the app's `EnergyFormat.kt` and `EnergyDescription.kt`; the diagram's
//! geometry lives in `energy_circles`.

use std::collections::BTreeMap;

use nidoo_api::EntityView;
use nidoo_core::energy::{
    EnergyEntity, EnergyNode, EnergyRole, EnergyUi, Flow, IDLE_W, NodeKind, NodeStatus, PowerUnits,
};
use nidoo_core::model::StateValue;

use crate::i18n::energy::{self as words, AriaPart, EnergyText};
use crate::i18n::format::with_unit;
use crate::i18n::{Key, Lang, text};

pub use nidoo_api::energy_entity;

pub fn energy_entities(entities: &BTreeMap<String, EntityView>) -> Vec<EnergyEntity> {
    nidoo_api::energy_entities(entities.values())
}

/// True when the solar roles together produce at least 10 W, so the Energy tab glyph lights up.
pub fn any_solar_active<'a>(entities: impl Iterator<Item = &'a EntityView>) -> bool {
    let watts: f64 = entities
        .filter(|e| {
            e.ui.energy_role.as_deref().and_then(EnergyRole::parse) == Some(EnergyRole::Solar)
        })
        .filter_map(
            |e| match (&e.state.value, PowerUnits::factor(e.unit.as_deref())) {
                (StateValue::Number(n), Some(f)) => Some(*n * f),
                _ => None,
            },
        )
        .sum();
    watts >= IDLE_W
}

/// 0 at 10 W, 1 at 10 kW and above, on a log scale.
pub fn level(watts: Option<f64>) -> f64 {
    match watts {
        Some(w) if w >= 10.0 => ((w.log10() - 1.0) / 3.0).clamp(0.0, 1.0),
        _ => 0.0,
    }
}

/// Not `i18n::format::number`: this one takes min/max fraction digits (kW to 1, counters to 3).
/// `value` with `min..=max` fraction digits, the integer part grouped (narrow no-break space in
/// French, thin space in English) and the language's decimal separator.
fn number(lang: Lang, value: f64, min: usize, max: usize) -> String {
    let (group, point) = match lang {
        Lang::Fr => ('\u{202F}', ','),
        Lang::En => ('\u{2009}', '.'),
    };
    let fixed = format!("{:.max$}", value.abs());
    let (int, frac) = fixed.split_once('.').unwrap_or((fixed.as_str(), ""));
    let mut frac = frac.to_string();
    while frac.len() > min && frac.ends_with('0') {
        frac.pop();
    }
    let mut grouped = String::new();
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            grouped.push(group);
        }
        grouped.push(c);
    }
    let sign = if value < 0.0 && fixed.chars().any(|c| c.is_ascii_digit() && c != '0') {
        "-"
    } else {
        ""
    };
    if frac.is_empty() {
        format!("{sign}{grouped}")
    } else {
        format!("{sign}{grouped}{point}{frac}")
    }
}

/// A power as its number and its unit: ("3.2", "kW") from 1000 W, ("253", "W") below,
/// ("—", "") when unknown.
pub fn power_parts(lang: Lang, watts: Option<f64>) -> (String, &'static str) {
    match watts {
        None => ("—".to_string(), ""),
        Some(w) if w >= 1_000.0 => (number(lang, w / 1_000.0, 1, 1), "kW"),
        Some(w) => (number(lang, w, 0, 0), "W"),
    }
}

/// "3.2 kW" from 1000 W, whole watts below, "—" when unknown.
pub fn power_text(lang: Lang, watts: Option<f64>) -> String {
    let (n, unit) = power_parts(lang, watts);
    with_unit(n, Some(unit).filter(|u| !u.is_empty()))
}

/// The big and the small line of a big circle: the number, then the unit and what it does
/// ("W · ↘ import"). Unavailable gives the word alone; an unknown value gives "—".
pub fn circle_texts(lang: Lang, node: &EnergyNode) -> (String, String) {
    if node.status == NodeStatus::Unavailable {
        return (
            words::energy_text(lang, EnergyText::Unavailable).to_string(),
            String::new(),
        );
    }
    if node.value.is_none() {
        return ("—".to_string(), String::new());
    }
    let (n, unit) = power_parts(lang, node.value);
    let (arrow, word) = match (node.kind, node.flow) {
        (NodeKind::Solar, _) => ("", Some(EnergyText::CircleProduction)),
        (NodeKind::Grid, Flow::In) => ("\u{2198} ", Some(EnergyText::CircleImport)),
        (NodeKind::Grid, Flow::Out) => ("\u{2197} ", Some(EnergyText::CircleExport)),
        (NodeKind::Battery, Flow::In) => ("\u{2198} ", Some(EnergyText::CircleDischarging)),
        (NodeKind::Battery, Flow::Out) => ("\u{2197} ", Some(EnergyText::CircleCharging)),
        _ => ("", None),
    };
    let small = match word {
        Some(w) => format!("{unit} \u{b7} {arrow}{}", words::energy_text(lang, w)),
        None => unit.to_string(),
    };
    (n, small)
}

/// A lifetime energy counter: "6 584 kWh", "32.7 kWh", "—" when unknown.
pub fn energy_text(lang: Lang, kwh: Option<f64>) -> String {
    kwh.map_or_else(
        || "—".to_string(),
        |v| with_unit(number(lang, v, 0, 1), Some("kWh")),
    )
}

/// A reading with its unit, one decimal at most: "12.5 L".
pub fn amount_text(lang: Lang, value: f64, unit: Option<&str>) -> String {
    with_unit(
        number(lang, value, 0, 1),
        unit.map(str::trim).filter(|u| !u.is_empty()),
    )
}

/// A meter index to three decimals: "601.359 m³".
pub fn meter_text(lang: Lang, value: f64, unit: Option<&str>) -> String {
    with_unit(
        number(lang, value, 0, 3),
        unit.map(str::trim).filter(|u| !u.is_empty()),
    )
}

/// The node's power, led by ↙ (into the house) or ↗ (out of it) for the grid and the battery.
pub fn grid_text(lang: Lang, node: &EnergyNode) -> String {
    let arrow = match (node.kind, node.flow) {
        (NodeKind::Grid | NodeKind::Battery, Flow::In) => "↙ ",
        (NodeKind::Grid | NodeKind::Battery, Flow::Out) => "↗ ",
        _ => "",
    };
    format!("{arrow}{}", power_text(lang, node.value))
}

/// What a node shows under its circle: "Unavailable", "—", the water reading or the power.
pub fn node_value_text(lang: Lang, node: &EnergyNode) -> String {
    if node.status == NodeStatus::Unavailable {
        return words::energy_text(lang, EnergyText::Unavailable).to_string();
    }
    match (node.kind, node.value) {
        (_, None) => "—".to_string(),
        (NodeKind::Water, Some(v)) => amount_text(lang, v, node.unit.as_deref()),
        _ => grid_text(lang, node),
    }
}

/// A node's name: the consumer's own, the role's, or "+N" for the folded consumers.
pub fn node_label(lang: Lang, node: &EnergyNode) -> String {
    if node.kind == NodeKind::More {
        return format!("+{}", node.count);
    }
    if let Some(name) = &node.name {
        return name.clone();
    }
    match node.kind {
        NodeKind::Solar => text(lang, Key::Solar),
        NodeKind::Grid => words::energy_text(lang, EnergyText::NodeGrid),
        NodeKind::Battery => text(lang, Key::Battery),
        NodeKind::House => text(lang, Key::House),
        NodeKind::Water => words::energy_text(lang, EnergyText::NodeWater),
        NodeKind::Consumer | NodeKind::More => words::energy_text(lang, EnergyText::NodeConsumer),
    }
    .to_string()
}

/// "3.2 kilowatts" from 1000 W, else "400 watts".
fn spoken(lang: Lang, watts: f64) -> String {
    if watts >= 1_000.0 {
        words::spoken_power(number(lang, watts / 1_000.0, 1, 1), true)
    } else {
        words::spoken_power(number(lang, watts, 0, 0), false)
    }
}

/// The diagram as one sentence, in the app's order: the role nodes as drawn, every consumer,
/// then water. Unknown values and idle grid or battery are left out.
pub fn aria_sentence(lang: Lang, ui: &EnergyUi) -> String {
    let part = |n: &EnergyNode| -> Option<String> {
        let v = n.value?;
        let say = |p: AriaPart| Some(words::aria_part(lang, p, &spoken(lang, v)));
        match (n.kind, n.flow) {
            (NodeKind::Solar, _) => say(AriaPart::Solar),
            (NodeKind::House, _) => say(AriaPart::House),
            (NodeKind::Grid, Flow::In) => say(AriaPart::Importing),
            (NodeKind::Grid, Flow::Out) => say(AriaPart::Exporting),
            (NodeKind::Battery, Flow::In) => say(AriaPart::BatterySupplies),
            (NodeKind::Battery, Flow::Out) => say(AriaPart::BatteryCharging),
            (NodeKind::Grid | NodeKind::Battery, Flow::Idle) => None,
            (NodeKind::Consumer, _) => Some(format!(
                "{} {}.",
                n.name
                    .as_deref()
                    .unwrap_or(words::energy_text(lang, EnergyText::NodeConsumer)),
                spoken(lang, v)
            )),
            (NodeKind::Water, _) => Some(words::aria_part(
                lang,
                AriaPart::Water,
                &amount_text(lang, v, n.unit.as_deref()),
            )),
            (NodeKind::More, _) => None,
        }
    };
    let roles = ui.diagram.iter().filter(|n| {
        !matches!(
            n.kind,
            NodeKind::Consumer | NodeKind::More | NodeKind::Water
        )
    });
    let water = ui.cards.iter().find(|n| n.kind == NodeKind::Water);
    roles
        .chain(ui.consumers.iter())
        .chain(water)
        .filter_map(part)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Unix milliseconds: the consumer order throttle's clock.
pub fn now_ms() -> u64 {
    #[cfg(target_arch = "wasm32")]
    {
        js_sys::Date::now() as u64
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64)
    }
}

// --- History tiles (kWh today) ---

/// Per-counter kWh-today values for the derived house, split into additions and subtractions.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HouseTodayParts {
    pub plus: Vec<Option<f64>>,
    pub minus: Vec<Option<f64>>,
}

/// Sum `plus - minus` from the derived-house parts. Returns `None` if any part is missing: a
/// partial sum would understate consumption and mislead the viewer.
pub fn derived_house_today(parts: &HouseTodayParts) -> Option<f64> {
    let mut acc = 0.0;
    for v in parts.plus.iter().chain(parts.minus.iter()) {
        v.as_ref()?;
    }
    for v in &parts.plus {
        acc += v.unwrap_or(0.0);
    }
    for v in &parts.minus {
        acc -= v.unwrap_or(0.0);
    }
    Some(acc)
}

/// Returns `interval_ms` when the tab is visible, `None` when it is hidden — so the caller
/// doesn't schedule another timer while nobody is watching.
pub fn next_fire_delay(hidden: bool, interval_ms: u32) -> Option<u32> {
    if hidden { None } else { Some(interval_ms) }
}

/// Whether a mount-edge Effect should fire this run. Returns true only on the first run where
/// `has_now` is true and the previous run (if any) was `false` — the transition from "empty" to
/// "loaded", once. Steady-state re-runs (entity map tick, same loaded-ness) are no-ops.
pub fn should_fire_mount(prev: Option<bool>, has_now: bool) -> bool {
    has_now && !prev.unwrap_or(false)
}

/// The derived house's `plus` and `minus` counter ids. `None` when the house has no derived
/// total (no solar or grid, or the inputs are not fully meter-paired).
pub fn plus_minus_ids(
    by_role: &BTreeMap<EnergyRole, Vec<&EnergyEntity>>,
    all: &[EnergyEntity],
) -> Option<(Vec<String>, Vec<String>)> {
    let m = nidoo_core::energy::EnergyMeters::house_meters(by_role, all)?;
    Some((
        m.plus.into_iter().map(|e| e.id.clone()).collect(),
        m.minus.into_iter().map(|e| e.id.clone()).collect(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nidoo_core::energy::EnergyFlow;

    #[test]
    fn group_entities_never_reach_the_energy_list() {
        use nidoo_core::model::{Capabilities, EntityCategory, EntityState, Kind, UiOverride};
        let view = |id: &str| EntityView {
            id: id.into(),
            device_id: None,
            name: id.into(),
            device_name: None,
            label: None,
            kind: Kind::Sensor,
            component: Kind::Sensor.component().to_string(),
            protocol_kind: Kind::Sensor,
            device_class: None,
            unit: None,
            icon: None,
            category: EntityCategory::Primary,
            enabled_by_default: true,
            area: "x".into(),
            part_of: None,
            capabilities: Capabilities::default(),
            ui: UiOverride::default(),
            state: EntityState::default(),
        };
        let map: BTreeMap<String, EntityView> = ["power", "group_all"]
            .into_iter()
            .map(|id| (id.to_owned(), view(id)))
            .collect();
        let ids: Vec<_> = energy_entities(&map).into_iter().map(|e| e.id).collect();
        assert_eq!(ids, ["power"]);
    }

    fn ent(id: &str, name: &str, role: EnergyRole, unit: &str, value: f64) -> EnergyEntity {
        EnergyEntity {
            id: id.to_string(),
            device_id: None,
            name: name.to_string(),
            label: None,
            unit: Some(unit.to_string()),
            value: Some(value),
            available: true,
            role: Some(role),
            invert: false,
            order: None,
            icon: None,
            meter: None,
        }
    }

    #[test]
    fn level_is_log_scaled() {
        assert_eq!(level(Some(10.0)), 0.0);
        assert_eq!(level(Some(10_000.0)), 1.0);
        assert_eq!(level(Some(1_000_000.0)), 1.0);
        assert_eq!(level(Some(5.0)), 0.0);
        assert_eq!(level(None), 0.0);
    }

    #[test]
    fn texts() {
        assert_eq!(power_text(Lang::En, Some(3200.0)), "3.2\u{a0}kW");
        assert_eq!(power_text(Lang::En, Some(253.3)), "253\u{a0}W");
        assert_eq!(power_text(Lang::En, None), "—");
        assert_eq!(
            energy_text(Lang::En, Some(6584.04)),
            "6\u{2009}584\u{a0}kWh"
        );
        assert_eq!(energy_text(Lang::En, Some(32.7)), "32.7\u{a0}kWh");
        assert_eq!(energy_text(Lang::En, None), "—");
        assert_eq!(amount_text(Lang::En, 12.5, Some("L")), "12.5\u{a0}L");
        assert_eq!(meter_text(Lang::En, 601.359, Some("m³")), "601.359\u{a0}m³");
    }

    #[test]
    fn aria_sentence_follows_the_app() {
        // The expectation is written from EnergyDescription.kt, not from the spec's rough
        // example: role nodes in diagram order (solar, grid, battery, house), then every
        // consumer by power, kilowatts only from 1000 W and watts below.
        let es = vec![
            ent("s", "S", EnergyRole::Solar, "W", 3200.0),
            ent("g", "G", EnergyRole::Grid, "W", -400.0),
            ent("a", "Water heater", EnergyRole::Consumer, "W", 1700.0),
            ent("b", "AC", EnergyRole::Consumer, "W", 600.0),
        ];
        let ui = EnergyFlow::build(&es, 0, None);
        assert_eq!(
            aria_sentence(Lang::En, &ui),
            "Solar 3.2\u{a0}kilowatts to the house. Exporting 400\u{a0}watts to the grid. \
             House uses 2.8\u{a0}kilowatts. Water heater 1.7\u{a0}kilowatts. AC 600\u{a0}watts."
        );
    }

    #[test]
    fn labels_and_values() {
        let es = vec![
            ent("g", "G", EnergyRole::Grid, "W", -400.0),
            ent("w", "Water", EnergyRole::Water, "L", 5.0),
        ];
        let ui = EnergyFlow::build(&es, 0, None);
        let grid = ui.node("grid").unwrap();
        assert_eq!(grid_text(Lang::En, grid), "↗ 400\u{a0}W");
        assert_eq!(node_label(Lang::En, grid), "Grid");
        assert_eq!(
            node_value_text(Lang::En, ui.node("water").unwrap()),
            "5\u{a0}L"
        );
    }

    #[test]
    fn circle_texts_name_the_direction() {
        let build = |es: Vec<EnergyEntity>| EnergyFlow::build(&es, 0, None);
        let ui = build(vec![ent("g", "G", EnergyRole::Grid, "W", 253.0)]);
        assert_eq!(
            circle_texts(Lang::En, ui.node("grid").unwrap()),
            ("253".to_string(), "W \u{b7} \u{2198} import".to_string())
        );
        let ui = build(vec![ent("g", "G", EnergyRole::Grid, "W", -1610.0)]);
        assert_eq!(
            circle_texts(Lang::Fr, ui.node("grid").unwrap()),
            ("1,6".to_string(), "kW \u{b7} \u{2197} export".to_string())
        );
        let ui = build(vec![ent("s", "S", EnergyRole::Solar, "W", 3460.0)]);
        assert_eq!(
            circle_texts(Lang::En, ui.node("solar").unwrap()),
            ("3.5".to_string(), "kW \u{b7} production".to_string())
        );
        let ui = build(vec![ent("b", "B", EnergyRole::Battery, "W", -500.0)]);
        assert_eq!(
            circle_texts(Lang::Fr, ui.node("battery").unwrap()),
            ("500".to_string(), "W \u{b7} \u{2197} charge".to_string())
        );
        let ui = build(vec![ent("b", "B", EnergyRole::Battery, "W", 500.0)]);
        assert_eq!(
            circle_texts(Lang::En, ui.node("battery").unwrap()).1,
            "W \u{b7} \u{2198} discharging"
        );
        let mut e = ent("g", "G", EnergyRole::Grid, "W", 1.0);
        e.available = false;
        let ui = build(vec![e]);
        assert_eq!(
            circle_texts(Lang::En, ui.node("grid").unwrap()),
            ("Unavailable".to_string(), String::new())
        );
    }

    #[test]
    fn big_power_reads_in_kilowatts() {
        assert_eq!(
            power_parts(Lang::En, Some(12_345.0)),
            ("12.3".to_string(), "kW")
        );
        assert_eq!(
            power_parts(Lang::En, Some(3200.0)),
            ("3.2".to_string(), "kW")
        );
        assert_eq!(power_parts(Lang::En, Some(253.3)), ("253".to_string(), "W"));
        assert_eq!(power_parts(Lang::En, None), ("\u{2014}".to_string(), ""));
        assert_eq!(power_text(Lang::En, Some(999.0)), "999\u{a0}W");
        assert_eq!(power_text(Lang::En, Some(1000.0)), "1.0\u{a0}kW");
    }

    #[test]
    fn texts_in_french() {
        assert_eq!(power_text(Lang::Fr, Some(3200.0)), "3,2\u{a0}kW");
        assert_eq!(power_text(Lang::Fr, Some(253.3)), "253\u{a0}W");
        assert_eq!(energy_text(Lang::Fr, Some(1250.0)), "1\u{202f}250\u{a0}kWh");
        assert_eq!(energy_text(Lang::Fr, Some(32.7)), "32,7\u{a0}kWh");
        assert_eq!(meter_text(Lang::Fr, 601.359, Some("m³")), "601,359\u{a0}m³");
        assert_eq!(amount_text(Lang::Fr, 12.5, Some("L")), "12,5\u{a0}L");
    }

    #[test]
    fn french_aria_sentence_and_labels() {
        let es = vec![
            ent("s", "S", EnergyRole::Solar, "W", 3200.0),
            ent("g", "G", EnergyRole::Grid, "W", -400.0),
            ent("a", "AC", EnergyRole::Consumer, "W", 600.0),
        ];
        let ui = EnergyFlow::build(&es, 0, None);
        assert_eq!(
            aria_sentence(Lang::Fr, &ui),
            "Solaire 3,2\u{a0}kilowatts vers la maison. Exportation de 400\u{a0}watts vers le réseau. \
             La maison consomme 2,8\u{a0}kilowatts. AC 600\u{a0}watts."
        );
        assert_eq!(node_label(Lang::Fr, ui.node("grid").unwrap()), "Réseau");
        assert_eq!(node_label(Lang::Fr, ui.node("house").unwrap()), "Maison");
    }

    #[test]
    fn solar_glyph_lights_with_units() {
        use nidoo_core::model::{Capabilities, EntityCategory, EntityState, Kind, UiOverride};
        let view = |unit: &str, v: f64| EntityView {
            id: "s".into(),
            device_id: None,
            name: "s".into(),
            device_name: None,
            label: None,
            kind: Kind::Sensor,
            component: "sensor".to_string(),
            protocol_kind: Kind::Sensor,
            device_class: None,
            unit: Some(unit.into()),
            icon: None,
            category: EntityCategory::Primary,
            enabled_by_default: true,
            area: "a".into(),
            part_of: None,
            capabilities: Capabilities::default(),
            ui: UiOverride {
                energy_role: Some("solar".into()),
                ..Default::default()
            },
            state: EntityState {
                value: StateValue::Number(v),
                ..Default::default()
            },
        };
        assert!(any_solar_active([view("kW", 0.5)].iter()));
        assert!(!any_solar_active([view("W", 9.0)].iter()));
        assert!(!any_solar_active([view("Wh", 900.0)].iter()));
    }

    #[test]
    fn next_fire_delay_pauses_when_hidden() {
        assert_eq!(next_fire_delay(false, 300_000), Some(300_000));
        assert_eq!(next_fire_delay(true, 300_000), None);
    }

    #[test]
    fn should_fire_mount_fires_on_loaded_edge_only() {
        assert!(should_fire_mount(None, true));
        assert!(!should_fire_mount(None, false));
        assert!(!should_fire_mount(Some(true), true));
        assert!(should_fire_mount(Some(false), true));
    }

    #[test]
    fn derived_house_today_sums_plus_minus_minus() {
        let m = HouseTodayParts {
            plus: vec![Some(10.0), Some(4.0)],
            minus: vec![Some(1.5)],
        };
        assert_eq!(derived_house_today(&m), Some(12.5));
    }

    #[test]
    fn derived_house_today_is_none_when_any_part_is_missing() {
        let partial = HouseTodayParts {
            plus: vec![Some(10.0), None],
            minus: vec![Some(1.0)],
        };
        assert_eq!(derived_house_today(&partial), None);
        let partial = HouseTodayParts {
            plus: vec![Some(10.0)],
            minus: vec![None],
        };
        assert_eq!(derived_house_today(&partial), None);
    }

    #[test]
    fn derived_house_today_handles_empty_parts() {
        assert_eq!(derived_house_today(&HouseTodayParts::default()), Some(0.0));
    }
}
