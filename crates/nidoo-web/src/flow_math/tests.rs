use nidoo_core::flow::{Flow, MathInput, MathParams, Node, NodeKind, Scalar};

use super::*;
use crate::i18n::Lang;
use crate::i18n::flows::function_help;

fn input(name: &str, entity: &str, average: Option<&str>) -> MathInput {
    MathInput {
        name: name.into(),
        entity: entity.into(),
        average: average.map(str::to_owned),
        extra: Default::default(),
    }
}

fn params(formula: &str, inputs: Vec<MathInput>) -> MathParams {
    MathParams {
        inputs,
        formula: formula.into(),
        name: "desired".into(),
        save_to: None,
        extra: Default::default(),
    }
}

/// A flow holding `params` as card `m`, and optionally a second card named `other`.
fn flow_of(params: &MathParams, other: Option<&str>) -> Flow {
    let node = |id: &str, p: MathParams| Node {
        id: id.into(),
        name: None,
        position: None,
        kind: NodeKind::Math(p),
        extra: Default::default(),
    };
    let mut nodes = vec![node("m", params.clone())];
    if let Some(name) = other {
        nodes.push(node(
            "n",
            MathParams {
                name: name.into(),
                formula: "1".into(),
                ..Default::default()
            },
        ));
    }
    Flow {
        id: "f".into(),
        name: "F".into(),
        nodes,
        ..Flow::default()
    }
}

fn values(name: &str) -> Option<Scalar> {
    match name {
        "sensor.grid" => Some(Scalar::Number(1330.0)),
        "sensor.heater" => Some(Scalar::Number(5.0)),
        _ => None,
    }
}

fn run(p: &MathParams, other: Option<&str>) -> Preview {
    preview("m", p, &flow_of(p, other), &values)
}

#[test]
fn preview_computes_with_current_values() {
    let p = params(
        "clamp(heater - 100 - grid, 0, 1650)",
        vec![
            input("grid", "sensor.grid", None),
            input("heater", "sensor.heater", None),
        ],
    );
    assert_eq!(
        run(&p, None),
        Preview::Value {
            value: 0.0,
            averaged_now: false
        }
    );
}

#[test]
fn preview_reports_errors_and_unknowns() {
    let grid = || input("grid", "sensor.grid", None);
    let p = params("gird + 1", vec![grid()]);
    assert_eq!(run(&p, None), Preview::Error("unknown name `gird`".into()));

    let p = params("desired2 + 1", vec![grid()]);
    assert_eq!(
        run(&p, Some("desired2")),
        Preview::Unknown("desired2".into())
    );

    let p = params("grid + 1", vec![input("grid", "sensor.missing", None)]);
    assert_eq!(run(&p, None), Preview::Unknown("grid".into()));

    // A run-time failure with today's readings is shown, not refused.
    let p = params("1 / 0", vec![]);
    assert!(matches!(run(&p, None), Preview::Fails(_)));
}

#[test]
fn preview_marks_averages() {
    let p = params("grid", vec![input("grid", "sensor.grid", Some("1m"))]);
    assert_eq!(
        run(&p, None),
        Preview::Value {
            value: 1330.0,
            averaged_now: true
        }
    );
}

#[test]
fn input_names() {
    assert_eq!(input_name_for("Réseau", &[]), "reseau");
    assert_eq!(input_name_for("Chauffe eau", &[]), "chauffe_eau");
    assert_eq!(input_name_for("2nd floor", &[]), "v_2nd_floor");
    assert_eq!(input_name_for("Réseau", &["reseau"]), "reseau2");
    assert_eq!(input_name_for("Min", &[]), "min2");
    assert_eq!(input_name_for("Réseau", &["reseau", "reseau2"]), "reseau3");
    assert_eq!(input_name_for("  !! ", &[]), "v_");
    let long = input_name_for(&"a".repeat(50), &[]);
    assert_eq!(long.len(), 32);
    assert!(is_placeholder_name("input") && is_placeholder_name("input12"));
    assert!(!is_placeholder_name("inputs") && !is_placeholder_name("grid"));
}

#[test]
fn snippets() {
    let snippet = |name: &str| function_snippet(FUNCTIONS.iter().find(|f| f.name == name).unwrap());
    assert_eq!(snippet("clamp"), "clamp(▢, ▢, ▢)");
    assert_eq!(snippet("round"), "round(▢)");
    assert_eq!(snippet("min"), "min(▢, ▢)");
    assert_eq!(snippet("interp"), "interp(▢, [▢], [▢])");
    assert_eq!(snippet("if"), "if(▢, ▢, ▢)");
}

#[test]
fn insert_selects_first_slot() {
    assert_eq!(
        insert_at("a + ", 4, "clamp(▢, ▢, ▢)"),
        ("a + clamp(▢, ▢, ▢)".to_owned(), 10, 11)
    );
    // Char offsets, not bytes: the accent and the slot are one char each.
    assert_eq!(insert_at("é▢", 1, "x"), ("éx▢".to_owned(), 2, 2));
}

#[test]
fn insert_without_slot() {
    assert_eq!(insert_at("", 0, "grid"), ("grid".to_owned(), 4, 4));
}

#[test]
fn suggest_after_two_letters() {
    let labels = |text: &str, names: &[&str]| -> Vec<String> {
        suggestions(text, text.chars().count(), names)
            .into_iter()
            .map(|s| s.label)
            .collect()
    };
    assert_eq!(labels("heater - cl", &["heater"]), ["clamp"]);
    assert_eq!(labels("he", &["heater"]), ["heater"]);
    assert!(labels("h", &["heater"]).is_empty());
    assert_eq!(labels("ma", &["max_w"]), ["max", "max_w"]);
    assert!(labels("a + ", &["heater"]).is_empty());
}

#[test]
fn accept_replaces_word() {
    let s = suggestions("heater - cl", 11, &["heater"]).remove(0);
    assert_eq!(
        accept("heater - cl", 11, &s),
        ("heater - clamp(▢, ▢, ▢)".to_owned(), 15, 16)
    );
    // Text after the caret stays.
    let s = suggestions("cl + 1", 2, &[]).remove(0);
    assert_eq!(accept("cl + 1", 2, &s).0, "clamp(▢, ▢, ▢) + 1");
}

#[test]
fn help_exists_for_every_function() {
    for f in FUNCTIONS.iter() {
        for lang in [Lang::En, Lang::Fr] {
            assert!(
                !function_help(lang, f.name).is_empty(),
                "{} {lang:?}",
                f.name
            );
        }
    }
}

#[test]
fn offsets_convert_between_chars_and_utf16() {
    assert_eq!(char_offset("é▢🌧x", 2), 2);
    assert_eq!(char_offset("é▢🌧x", 4), 3);
    assert_eq!(utf16_offset("é▢🌧x", 3), 4);
    assert_eq!(utf16_offset("ab", 5), 2);
}
