//! Home Assistant style Jinja templates: rendering semantics and the recorded house templates.

mod common;

use nidoo_core::discovery::expand;
use nidoo_core::template::{TemplateError, Templates};
use serde_json::{Value, json};

fn t() -> Templates {
    Templates::new()
}

#[test]
fn templates_is_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Templates>();
}

#[test]
fn value_json_key() {
    let t = t();
    assert_eq!(
        t.render("{{ value_json.state }}", r#"{"state":"ON"}"#)
            .unwrap(),
        "ON"
    );
}

#[test]
fn raw_value() {
    let t = t();
    assert_eq!(t.render("{{ value | upper }}", "on").unwrap(), "ON");
}

#[test]
fn value_json_is_undefined_for_non_json_payloads() {
    let t = t();
    assert!(matches!(
        t.render("{{ value_json.state }}", "ON"),
        Err(TemplateError::Undefined(_))
    ));
    assert!(matches!(
        t.render("{{ value_json }}", "42"),
        Err(TemplateError::Undefined(_))
    ));
    assert_eq!(t.render("{{ value_json[0] }}", "[7, 8]").unwrap(), "7");
}

#[test]
fn float_round_and_default() {
    let t = t();
    assert_eq!(
        t.render("{{ value_json.t | float | round(1) }}", r#"{"t":21.456}"#)
            .unwrap(),
        "21.5"
    );
    assert_eq!(
        t.render("{{ value_json.x | default(3) }}", "{}").unwrap(),
        "3"
    );
}

#[test]
fn float_and_int_follow_home_assistant_semantics() {
    let t = t();
    assert_eq!(t.render("{{ value | float }}", "21.5").unwrap(), "21.5");
    assert_eq!(t.render("{{ value | int }}", "21.9").unwrap(), "21");
    assert_eq!(t.render("{{ value | int }}", "3.7").unwrap(), "3");
    assert_eq!(t.render("{{ true | float }}", "x").unwrap(), "1.0");
    assert_eq!(
        t.render("{{ value_json.b | int }}", r#"{"b":false}"#)
            .unwrap(),
        "0"
    );
    assert_eq!(t.render("{{ value | float(0) }}", "unknown").unwrap(), "0");
    assert_eq!(
        t.render("{{ value | int(default=-1) }}", "unknown")
            .unwrap(),
        "-1"
    );
    assert_eq!(
        t.render("{{ value_json.x | float(default=1.5) }}", r#"{"x":null}"#)
            .unwrap(),
        "1.5"
    );
    assert!(matches!(
        t.render("{{ value | float }}", "unknown"),
        Err(TemplateError::Undefined(_))
    ));
    assert!(matches!(
        t.render("{{ value | int }}", "unknown"),
        Err(TemplateError::Undefined(_))
    ));
    assert!(matches!(
        t.render("{{ value_json.x | int }}", r#"{"x":null}"#),
        Err(TemplateError::Undefined(_))
    ));
}

#[test]
fn float_rejects_non_finite_values() {
    let t = t();
    for payload in ["nan", "inf", "-inf", "NaN"] {
        assert!(
            matches!(
                t.render("{{ value | float }}", payload),
                Err(TemplateError::Undefined(_))
            ),
            "{payload}"
        );
        assert_eq!(t.render("{{ value | float(0) }}", payload).unwrap(), "0");
    }
}

#[test]
fn is_defined_and_undefined_errors() {
    let t = t();
    assert!(matches!(
        t.render("{{ value_json.nope }}", "{}"),
        Err(TemplateError::Undefined(_))
    ));
    assert!(matches!(
        t.render("{{ value_json.nope | is_defined }}", "{}"),
        Err(TemplateError::Undefined(_))
    ));
    assert_eq!(
        t.render("{{ value_json.yes | is_defined }}", r#"{"yes":"y"}"#)
            .unwrap(),
        "y"
    );
}

#[test]
fn is_number_filter_and_test() {
    let t = t();
    assert_eq!(
        t.render("{{ value_json.n | is_number }}", r#"{"n":4}"#)
            .unwrap(),
        "True"
    );
    assert_eq!(t.render("{{ value | is_number }}", "abc").unwrap(), "False");
    assert_eq!(t.render("{{ value | is_number }}", "4.5").unwrap(), "True");
    assert_eq!(
        t.render(
            "{{ 'yes' if value_json.n is is_number else 'no' }}",
            r#"{"n":4.5}"#
        )
        .unwrap(),
        "yes"
    );
    assert_eq!(
        t.render(
            "{{ 'yes' if value_json.n is number else 'no' }}",
            r#"{"n":"4"}"#
        )
        .unwrap(),
        "no"
    );
}

#[test]
fn if_else_and_iif() {
    let t = t();
    assert_eq!(
        t.render(
            "{{ 'closed' if value_json.contact else 'open' }}",
            r#"{"contact":true}"#
        )
        .unwrap(),
        "closed"
    );
    assert_eq!(
        t.render("{{ (value_json.p > 0) | iif('on','off') }}", r#"{"p":5}"#)
            .unwrap(),
        "on"
    );
    assert_eq!(
        t.render("{{ value_json.p | iif('on','off') }}", r#"{"p":0}"#)
            .unwrap(),
        "off"
    );
    assert_eq!(
        t.render(
            "{{ value_json.p | iif('on','off','none') }}",
            r#"{"p":null}"#
        )
        .unwrap(),
        "none"
    );
    assert_eq!(
        t.render(
            "{{ value_json.p | iif('on','off', if_none='?') }}",
            r#"{"p":null}"#
        )
        .unwrap(),
        "?"
    );
    assert_eq!(
        t.render("{{ value_json.p | iif('on','off') }}", r#"{"p":null}"#)
            .unwrap(),
        "off"
    );
}

#[test]
fn iif_on_undefined_input_is_an_error() {
    let t = t();
    assert!(matches!(
        t.render("{{ value_json.nope | iif('on','off') }}", "{}"),
        Err(TemplateError::Undefined(_))
    ));
    assert!(matches!(
        t.render("{{ value_json.nope | iif('on','off','none') }}", "{}"),
        Err(TemplateError::Undefined(_))
    ));
}

#[test]
fn truncate_follows_jinja2() {
    let t = t();
    // Within length + leeway: untouched.
    assert_eq!(
        t.render("{{ value | truncate(10) }}", "hello world")
            .unwrap(),
        "hello world"
    );
    // Cut at the last space, with the default end.
    assert_eq!(
        t.render("{{ value | truncate(12) }}", "hello wide world of jinja")
            .unwrap(),
        "hello..."
    );
    // killwords cuts mid-word; a custom end and no leeway.
    assert_eq!(
        t.render("{{ value | truncate(5, True, '', 0) }}", "abcdefgh")
            .unwrap(),
        "abcde"
    );
    assert_eq!(
        t.render(
            "{{ value | truncate(6, killwords=True, end='~') }}",
            "abcdefghijklmn"
        )
        .unwrap(),
        "abcde~"
    );
    // The Zigbee2MQTT text sensor template, with a default for a missing key.
    let z2m = "{{ value_json[\"ts\"] | default('',True) | string | truncate(254, True, '', 0) }}";
    assert_eq!(
        t.render(z2m, r#"{"ts":"2026-10-18T21:43:50.006Z"}"#)
            .unwrap(),
        "2026-10-18T21:43:50.006Z"
    );
    assert_eq!(t.render(z2m, r#"{"other":1}"#).unwrap(), "");
    // Length shorter than the end marker is an evaluation error, not a crash.
    assert!(t.render("{{ value | truncate(2) }}", "abcdefghij").is_err());
}

#[test]
fn syntax_error() {
    let t = t();
    assert!(matches!(
        t.render("{{ value_json.", "{}"),
        Err(TemplateError::Syntax(_))
    ));
}

#[test]
fn check_compiles_without_rendering() {
    let t = t();
    assert_eq!(t.check("{{ value_json.state }}"), Ok(()));
    assert_eq!(t.check("{{ now().strftime('%Y') }}"), Ok(()));
    assert!(matches!(t.check("{% if %}"), Err(TemplateError::Syntax(_))));
}

#[test]
fn unknown_filters_tests_and_functions_are_syntax_errors() {
    let t = t();
    assert!(matches!(
        t.render("{{ value | nonexistent_filter }}", "x"),
        Err(TemplateError::Syntax(_))
    ));
    assert!(matches!(
        t.render("{{ value is nonexistent_test }}", "x"),
        Err(TemplateError::Syntax(_))
    ));
    assert!(matches!(
        t.render("{{ now() }}", "x"),
        Err(TemplateError::Syntax(_))
    ));
}

#[test]
fn other_errors_are_neither_syntax_nor_undefined() {
    let t = t();
    assert!(matches!(
        t.render("{{ value | abs }}", "abc"),
        Err(TemplateError::Other(_))
    ));
}

#[test]
fn render_with_extra_context() {
    let t = t();
    assert_eq!(
        t.render_with(
            r#"{"brightness": {{ brightness }}, "state": "{{ value }}"}"#,
            "ON",
            &[("brightness", json!(128))],
        )
        .unwrap(),
        r#"{"brightness": 128, "state": "ON"}"#
    );
    assert_eq!(
        t.render_with("{{ value }}", "raw", &[("value", json!("override"))])
            .unwrap(),
        "override"
    );
}

/// Collects every template string from an expanded discovery config: top-level keys
/// ending in `_template` plus `availability[].value_template`.
fn templates_of(expanded: &Value) -> Vec<String> {
    let mut out = Vec::new();
    let Some(map) = expanded.as_object() else {
        return out;
    };
    for (key, value) in map {
        if let (true, Some(s)) = (key.ends_with("_template"), value.as_str()) {
            out.push(s.to_owned());
        }
    }
    if let Some(items) = map.get("availability").and_then(Value::as_array) {
        for item in items {
            if let Some(s) = item.get("value_template").and_then(Value::as_str) {
                out.push(s.to_owned());
            }
        }
    }
    out
}

#[test]
fn every_recorded_value_template_compiles() {
    let t = t();
    let dir = common::fixture_dir().join("discovery");
    let mut checked = 0usize;
    let mut files = 0usize;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        files += 1;
        let raw: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        for template in templates_of(&expand(raw)) {
            if let Err(e) = t.check(&template) {
                panic!(
                    "{}: template {template:?} does not compile: {e}",
                    path.display()
                );
            }
            checked += 1;
        }
    }
    assert!(
        files > 400,
        "expected the recorded house fixtures, found {files}"
    );
    assert!(
        checked > files,
        "expected at least one template per fixture, checked {checked} in {files} files"
    );
}

#[test]
fn dict_get_with_and_without_default() {
    let t = Templates::new();
    let payload = r#"{"a":{"x":1}}"#;
    assert_eq!(
        t.render("{{ value_json['a'].get('x', 'n') }}", payload)
            .unwrap(),
        "1"
    );
    assert_eq!(
        t.render("{{ value_json['a'].get('y', 'n') }}", payload)
            .unwrap(),
        "n"
    );
}
