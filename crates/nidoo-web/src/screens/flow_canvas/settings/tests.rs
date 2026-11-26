use serde_json::json;

use super::*;

#[test]
fn ask_name_is_checked_and_empty_clears_the_default() {
    let mut p = nidoo_core::flow::AskParams::default();
    assert!(apply_ask_name(&mut p, "Temp").is_err());
    assert_eq!(p.name, None);
    assert!(apply_ask_name(&mut p, " temp ").is_ok());
    assert_eq!(p.name.as_deref(), Some("temp"));
    p.default = Some(21.0);
    assert!(apply_ask_name(&mut p, "").is_ok());
    assert_eq!((p.name, p.default), (None, None));
}

#[test]
fn ask_words_are_split_trimmed_and_cleaned() {
    assert_eq!(ask_words(" yes , sure,, "), vec!["yes", "sure"]);
    assert!(ask_words("  ").is_empty());
    assert_eq!(ask_words_text(&["a".into(), "b".into()]), "a, b");
}

#[test]
fn manual_kind_round_trips_its_value() {
    for (value, kind) in [
        (None, ManualKind::Nothing),
        (Some(Scalar::Bool(true)), ManualKind::On),
        (Some(Scalar::Bool(false)), ManualKind::Off),
        (Some(Scalar::Number(21.5)), ManualKind::Number),
        (Some(Scalar::Text("hi".into())), ManualKind::Text),
    ] {
        assert_eq!(ManualKind::of(value.as_ref()), kind);
        assert_eq!(ManualKind::from_key(kind.key()), Some(kind));
    }
    assert_eq!(
        ManualKind::Number.value("21,5"),
        Some(Some(Scalar::Number(21.5)))
    );
    // An empty or unreadable number box holds no value, never the previous one.
    assert_eq!(ManualKind::Number.value("abc"), Some(None));
    assert_eq!(ManualKind::Number.value(""), Some(None));
    assert_eq!(
        ManualKind::On.value("ignored"),
        Some(Some(Scalar::Bool(true)))
    );
    assert_eq!(ManualKind::Nothing.value("x"), Some(None));
}

#[test]
fn between_gains_an_upper_bound_and_others_lose_it() {
    let mut r = next_rule(None);
    r.value = json!(18);
    set_rule_op(&mut r, RuleOp::Between);
    assert_eq!(r.to, Some(json!(18)));
    r.to = Some(json!(22));
    set_rule_op(&mut r, RuleOp::Between);
    assert_eq!(r.to, Some(json!(22)));
    set_rule_op(&mut r, RuleOp::Gt);
    assert_eq!((r.op, r.to), (RuleOp::Gt, None));
}

#[test]
fn a_new_rule_repeats_the_last_value() {
    assert_eq!(next_rule(None).value, json!(true));
    let mut last = next_rule(None);
    last.value = json!("ON");
    assert_eq!(next_rule(Some(&last)).value, json!("ON"));
}

#[test]
fn rule_ops_round_trip_their_names() {
    for op in RULE_OPS {
        assert_eq!(rule_op_from(op.wire_name()), Some(op));
    }
    assert_eq!(rule_op_from("is_dark"), None);
}

#[test]
fn json_fields_take_a_decimal_comma() {
    assert_eq!(parse_json_input("21,5"), Ok(json!(21.5)));
    assert_eq!(parse_json_input("21.5"), Ok(json!(21.5)));
    assert_eq!(parse_json_input("true"), Ok(json!(true)));
    assert_eq!(parse_json_input("\"ON\""), Ok(json!("ON")));
    assert!(parse_json_input("{oops").is_err());
}

#[test]
fn flipping_to_is_dark_and_back_keeps_the_operands() {
    let mut i = IfParams {
        op: Op::Eq,
        value: Some(json!(true)),
        entity: Some("x".into()),
        field: None,
        missing: Some(Missing::True),
        average: Some("15m".into()),
        offset_minutes: None,
        result: None,
        extra: Default::default(),
    };
    let mut stash = IfStash::default();
    switch_op(&mut i, Op::IsDark, &mut stash);
    assert_eq!(
        (i.value.clone(), i.entity.clone(), i.missing),
        (None, None, None)
    );
    switch_op(&mut i, Op::Eq, &mut stash);
    assert_eq!(i.op, Op::Eq);
    assert_eq!(i.value, Some(json!(true)));
    assert_eq!(i.entity.as_deref(), Some("x"));
    assert_eq!(i.missing, Some(Missing::True));
}

#[test]
fn flipping_to_is_off_peak_and_back_keeps_the_operands() {
    assert_eq!(op_from("is_off_peak"), Some(Op::IsOffPeak));
    assert_eq!(op_str(Op::IsOffPeak), "is_off_peak");
    let mut i = IfParams {
        op: Op::Eq,
        value: Some(json!(true)),
        entity: Some("x".into()),
        field: None,
        missing: Some(Missing::True),
        average: Some("15m".into()),
        offset_minutes: None,
        result: None,
        extra: Default::default(),
    };
    let mut stash = IfStash::default();
    switch_op(&mut i, Op::IsOffPeak, &mut stash);
    assert_eq!(
        (
            i.value.clone(),
            i.entity.clone(),
            i.missing,
            i.average.clone()
        ),
        (None, None, None, None)
    );
    switch_op(&mut i, Op::Eq, &mut stash);
    assert_eq!(i.value, Some(json!(true)));
    assert_eq!(i.entity.as_deref(), Some("x"));
    assert_eq!(i.average.as_deref(), Some("15m"));
}

#[test]
fn only_compare_ops_show_operand_fields() {
    for op in [Op::Eq, Op::Ne, Op::Gt, Op::Lt, Op::Gte, Op::Lte] {
        assert!(takes_operands(op), "{op:?}");
    }
    assert!(!takes_operands(Op::IsDark));
    assert!(!takes_operands(Op::IsOffPeak));
}

#[test]
fn dark_to_off_peak_sets_the_shift_aside() {
    let mut i = IfParams {
        op: Op::IsDark,
        value: None,
        entity: None,
        field: None,
        missing: None,
        average: None,
        offset_minutes: Some(-30),
        result: None,
        extra: Default::default(),
    };
    let mut stash = IfStash::default();
    switch_op(&mut i, Op::IsOffPeak, &mut stash);
    assert_eq!(i.offset_minutes, None);
    switch_op(&mut i, Op::IsDark, &mut stash);
    assert_eq!(i.offset_minutes, Some(-30));
}

#[test]
fn leaving_is_dark_sets_the_shift_aside_and_restores_it() {
    let mut i = IfParams {
        op: Op::IsDark,
        value: None,
        entity: None,
        field: None,
        missing: None,
        average: None,
        offset_minutes: Some(-30),
        result: None,
        extra: Default::default(),
    };
    let mut stash = IfStash::default();
    switch_op(&mut i, Op::Gt, &mut stash);
    assert_eq!(i.offset_minutes, None);
    switch_op(&mut i, Op::IsDark, &mut stash);
    assert_eq!(i.offset_minutes, Some(-30));
}

#[test]
fn switching_between_comparisons_leaves_operands_alone() {
    let mut i = IfParams {
        op: Op::Eq,
        value: Some(json!(1)),
        entity: None,
        field: None,
        missing: None,
        average: None,
        offset_minutes: None,
        result: None,
        extra: Default::default(),
    };
    switch_op(&mut i, Op::Gt, &mut IfStash::default());
    assert_eq!((i.op, i.value), (Op::Gt, Some(json!(1))));
}
