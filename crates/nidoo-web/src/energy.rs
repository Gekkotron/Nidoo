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
mod tests;
