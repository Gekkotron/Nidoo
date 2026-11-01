//! Rule matching of the `switch` node.

use super::model::{Op, RuleOp, SwitchParams, SwitchRule};
use super::scalar::{CompareError, Scalar, compare};

/// Whether one rule matches `left`. An absent `left` matches nothing but `ne`; a rule whose
/// constants are not scalars matches nothing.
fn rule_matches(rule: &SwitchRule, left: Option<&Scalar>) -> Result<bool, CompareError> {
    let Some(value) = Scalar::from_json(&rule.value) else {
        return Ok(false);
    };
    let op = match rule.op {
        RuleOp::Eq => Op::Eq,
        RuleOp::Ne => Op::Ne,
        RuleOp::Gt => Op::Gt,
        RuleOp::Lt => Op::Lt,
        RuleOp::Gte => Op::Gte,
        RuleOp::Lte => Op::Lte,
        RuleOp::Between => {
            let Some(to) = rule.to.as_ref().and_then(Scalar::from_json) else {
                return Ok(false);
            };
            return Ok(compare(left, Op::Gte, &value)? && compare(left, Op::Lte, &to)?);
        }
    };
    compare(left, op, &value)
}

/// The ports fired by `left`, in rule order: the first match only unless `all`, and
/// `["otherwise"]` when nothing matched and `otherwise` is set.
pub fn switch_exits(
    params: &SwitchParams,
    left: Option<&Scalar>,
) -> Result<Vec<String>, CompareError> {
    let mut exits = Vec::new();
    for (i, rule) in params.rules.iter().enumerate() {
        if rule_matches(rule, left)? {
            exits.push((i + 1).to_string());
            if !params.all {
                break;
            }
        }
    }
    if exits.is_empty() && params.otherwise {
        exits.push("otherwise".to_owned());
    }
    Ok(exits)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn params(v: serde_json::Value) -> SwitchParams {
        serde_json::from_value(v).unwrap()
    }

    fn banded() -> SwitchParams {
        params(json!({ "rules": [
            { "op": "lt", "value": 18 },
            { "op": "between", "value": 18, "to": 22 },
            { "op": "gt", "value": 22 } ], "otherwise": true }))
    }

    fn n(x: f64) -> Scalar {
        Scalar::Number(x)
    }

    #[test]
    fn switch_exits_first_match() {
        let p = banded();
        assert_eq!(switch_exits(&p, Some(&n(17.0))).unwrap(), ["1"]);
        assert_eq!(switch_exits(&p, Some(&n(18.0))).unwrap(), ["2"]);
        assert_eq!(switch_exits(&p, Some(&n(22.0))).unwrap(), ["2"]);
        assert_eq!(switch_exits(&p, Some(&n(22.5))).unwrap(), ["3"]);
    }

    #[test]
    fn switch_exits_all_matches() {
        let p = params(json!({ "rules": [
            { "op": "gte", "value": 10 }, { "op": "lte", "value": 20 } ], "all": true }));
        assert_eq!(switch_exits(&p, Some(&n(15.0))).unwrap(), ["1", "2"]);
    }

    #[test]
    fn switch_exits_unknown_value() {
        assert_eq!(switch_exits(&banded(), None).unwrap(), ["otherwise"]);
        let ne = params(json!({ "rules": [{ "op": "ne", "value": "on" }] }));
        assert_eq!(switch_exits(&ne, None).unwrap(), ["1"]);
        let none = params(json!({ "rules": [{ "op": "gt", "value": 1 }] }));
        assert!(switch_exits(&none, None).unwrap().is_empty());
    }

    #[test]
    fn switch_exits_ordering_on_text_fails() {
        let p = params(json!({ "rules": [{ "op": "gt", "value": 1 }] }));
        assert_eq!(
            switch_exits(&p, Some(&Scalar::Text("on".into()))),
            Err(CompareError::NotNumeric)
        );
    }

    #[test]
    fn switch_exits_between_edges() {
        let p = params(json!({ "rules": [{ "op": "between", "value": 1, "to": 5 }],
            "otherwise": true }));
        assert_eq!(switch_exits(&p, None).unwrap(), ["otherwise"]);
        assert_eq!(
            switch_exits(&p, Some(&Scalar::Text("on".into()))),
            Err(CompareError::NotNumeric)
        );
        let no_to = params(json!({ "rules": [{ "op": "between", "value": 1 }] }));
        assert!(switch_exits(&no_to, Some(&n(3.0))).unwrap().is_empty());
    }
}
