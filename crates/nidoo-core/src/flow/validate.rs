//! Structural validation of a flow.

use super::{Flow, MathParams, NodeKind, command_with_number, math_names, template};

/// Why a flow is not structurally valid.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum FlowError {
    #[error("flow id `{0}` must be 1 to 64 characters of a-z, 0-9, `_` or `-`")]
    BadFlowId(String),
    #[error("node id `{0}` must be 1 to 32 characters of a-z, 0-9, `_` or `-`")]
    BadNodeId(String),
    #[error("node id `{0}` is used twice")]
    DuplicateNodeId(String),
    #[error("edge {edge_index} names a missing node `{node}`")]
    UnknownNode { edge_index: usize, node: String },
    #[error("node `{node}` has no port `{port}`")]
    UnknownPort { node: String, port: String },
    #[error("node `{node}` is {kind} and needs a port")]
    MissingPort { node: String, kind: &'static str },
    #[error("node `{node}` has a single output and takes no port")]
    UnexpectedPort { node: String },
    #[error("cycle through {}", .0.join(" → "))]
    Cycle(Vec<String>),
    #[error("the flow has no trigger node")]
    NoTrigger,
    #[error("node `{node}`: {reason}")]
    BadDuration { node: String, reason: String },
    #[error("node `{node}`: {reason}")]
    BadLimit { node: String, reason: String },
    #[error("node `{node}`: value must be a boolean, a number or a string")]
    BadValue { node: String },
    #[error("node `{node}`: {reason}")]
    BadCron { node: String, reason: String },
    #[error(
        "node `{node}`: an if after a schedule, sun or manual trigger without a value must name an entity"
    )]
    IfNeedsEntity { node: String },
    #[error(
        "node `{node}`: a switch after a schedule, sun or manual trigger without a value must name an entity"
    )]
    SwitchNeedsEntity { node: String },
    #[error("node `{node}`: an average after a manual trigger must name an entity")]
    AverageNeedsEntity { node: String },
    #[error("node `{node}`: a field needs an entity")]
    FieldWithoutEntity { node: String },
    #[error("node `{node}`: `{flow}` is not a valid flow id")]
    BadFlowTarget { node: String, flow: String },
    #[error("node `{node}`: offset_minutes {offset_minutes} is outside [-720, 720]")]
    BadSunOffset { node: String, offset_minutes: i32 },
    #[error("node `{node}`: {reason}")]
    BadOperand { node: String, reason: String },
    #[error("node `{node}`: {reason}")]
    BadNotify { node: String, reason: String },
    #[error("node `{node}`: {reason}")]
    BadMath { node: String, reason: String },
}

/// The push channels. Mirrors `nidoo_push::normalize::CHANNELS` (`nidoo-push` depends on this
/// crate); a test in `nidoo` keeps the two equal.
pub const NOTIFY_CHANNELS: [&str; 7] = [
    "alarm", "security", "camera", "energy", "climate", "system", "misc",
];
/// The push severities. Mirrors `nidoo_push::normalize::SEVERITIES`.
pub const NOTIFY_SEVERITIES: [&str; 3] = ["info", "warning", "critical"];

/// Why `channel` is not a push channel, or `None` when it is one.
pub fn notify_channel_error(channel: &str) -> Option<String> {
    (!NOTIFY_CHANNELS.contains(&channel)).then(|| {
        format!(
            "unknown channel `{channel}` (one of {})",
            NOTIFY_CHANNELS.join(", ")
        )
    })
}

/// Why `severity` is not a push severity, or `None` when it is one.
pub fn notify_severity_error(severity: &str) -> Option<String> {
    (!NOTIFY_SEVERITIES.contains(&severity)).then(|| {
        format!(
            "unknown severity `{severity}` (one of {})",
            NOTIFY_SEVERITIES.join(", ")
        )
    })
}

fn is_valid_id(id: &str, max: usize) -> bool {
    (1..=max).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

/// Whether `id` matches `[a-z0-9_-]{1,64}`, the form of a flow id (a topic segment).
pub fn is_valid_flow_id(id: &str) -> bool {
    is_valid_id(id, 64)
}

/// Whether `id` matches `[a-z0-9_-]{1,32}`, the form of a node id.
pub fn is_valid_node_id(id: &str) -> bool {
    is_valid_id(id, 32)
}

/// Checks the structure of a flow: ids, edges and ports, params, at least one trigger and no
/// cycle. It does not look at entities, which only the hub can resolve.
pub fn validate(flow: &Flow) -> Result<(), FlowError> {
    use super::{
        MAX_AVERAGE_SECS, MAX_LIMIT, MAX_SWITCH_RULES, NodeKind, Op, RuleOp, Scalar, Schedule,
        parse_clock, parse_duration, window,
    };

    if !is_valid_flow_id(&flow.id) {
        return Err(FlowError::BadFlowId(flow.id.clone()));
    }
    let mut seen = std::collections::HashSet::new();
    for node in &flow.nodes {
        if !is_valid_node_id(&node.id) {
            return Err(FlowError::BadNodeId(node.id.clone()));
        }
        if !seen.insert(node.id.as_str()) {
            return Err(FlowError::DuplicateNodeId(node.id.clone()));
        }
    }
    for node in &flow.nodes {
        let field_without_entity = match &node.kind {
            NodeKind::If(p) => p.field.is_some() && p.entity.is_none(),
            NodeKind::Switch(p) => p.field.is_some() && p.entity.is_none(),
            _ => false,
        };
        if field_without_entity {
            return Err(FlowError::FieldWithoutEntity {
                node: node.id.clone(),
            });
        }
        match &node.kind {
            NodeKind::Delay(p) => {
                if let Err(reason) = p.bounds() {
                    return Err(FlowError::BadDuration {
                        node: node.id.clone(),
                        reason,
                    });
                }
            }
            NodeKind::TriggerSilent(p) => {
                if let Err(reason) = window(&p.for_) {
                    return Err(FlowError::BadDuration {
                        node: node.id.clone(),
                        reason: format!("for: {reason}"),
                    });
                }
            }
            NodeKind::Limit(p) => {
                if let Err(reason) = window(&p.per) {
                    return Err(FlowError::BadDuration {
                        node: node.id.clone(),
                        reason: format!("per: {reason}"),
                    });
                }
                if !(1..=MAX_LIMIT).contains(&p.max) {
                    return Err(FlowError::BadLimit {
                        node: node.id.clone(),
                        reason: format!("max {} is outside 1 to {MAX_LIMIT}", p.max),
                    });
                }
            }
            NodeKind::TriggerSchedule(p) => {
                if let Err(e) = Schedule::parse(&p.cron) {
                    return Err(FlowError::BadCron {
                        node: node.id.clone(),
                        reason: e.to_string(),
                    });
                }
            }
            NodeKind::FlowEnable(p) => {
                if let Some(target) = p.flow.as_deref().filter(|t| !is_valid_flow_id(t)) {
                    return Err(FlowError::BadFlowTarget {
                        node: node.id.clone(),
                        flow: target.to_owned(),
                    });
                }
            }
            NodeKind::Notify(p) => {
                let bad = |reason: String| FlowError::BadNotify {
                    node: node.id.clone(),
                    reason,
                };
                if p.title.trim().is_empty() {
                    return Err(bad("title must not be empty".to_owned()));
                }
                let fields = [("title", Some(&p.title)), ("body", p.body.as_ref())];
                for (field, text) in fields {
                    if let Some(Err(err)) = text.map(|text| template::parse(text)) {
                        return Err(bad(format!("{field}: {err}")));
                    }
                }
                if let Some(reason) = p.channel.as_deref().and_then(notify_channel_error) {
                    return Err(bad(reason));
                }
                if let Some(reason) = p.severity.as_deref().and_then(notify_severity_error) {
                    return Err(bad(reason));
                }
            }
            NodeKind::TriggerSun(p) => {
                if !(-720..=720).contains(&p.offset()) {
                    return Err(FlowError::BadSunOffset {
                        node: node.id.clone(),
                        offset_minutes: p.offset(),
                    });
                }
            }
            NodeKind::If(p) if p.op == Op::IsDark => {
                let offending = if p.value.is_some() {
                    Some("value")
                } else if p.entity.is_some() {
                    Some("entity")
                } else if p.missing.is_some() {
                    Some("missing")
                } else if p.average.is_some() {
                    Some("average")
                } else if p.result.is_some() {
                    Some("result")
                } else {
                    None
                };
                if let Some(field) = offending {
                    return Err(FlowError::BadOperand {
                        node: node.id.clone(),
                        reason: format!("{field} is not allowed with op: is_dark"),
                    });
                }
                if !(-720..=720).contains(&p.dark_offset()) {
                    return Err(FlowError::BadSunOffset {
                        node: node.id.clone(),
                        offset_minutes: p.dark_offset(),
                    });
                }
            }
            NodeKind::If(p) if p.op == Op::IsOffPeak => {
                let offending = [
                    ("value", p.value.is_some()),
                    ("entity", p.entity.is_some()),
                    ("missing", p.missing.is_some()),
                    ("average", p.average.is_some()),
                    ("result", p.result.is_some()),
                    ("offset_minutes", p.offset_minutes.is_some()),
                ]
                .into_iter()
                .find_map(|(field, set)| set.then_some(field));
                if let Some(field) = offending {
                    return Err(FlowError::BadOperand {
                        node: node.id.clone(),
                        reason: format!("{field} is not allowed with op: is_off_peak"),
                    });
                }
            }
            NodeKind::If(p) if p.offset_minutes.is_some() => {
                return Err(FlowError::BadOperand {
                    node: node.id.clone(),
                    reason: "offset_minutes is only allowed with op: is_dark".to_owned(),
                });
            }
            NodeKind::If(p) => match &p.value {
                None => {
                    return Err(FlowError::BadOperand {
                        node: node.id.clone(),
                        reason: format!("value is required for op {}", p.op.wire_name()),
                    });
                }
                Some(value) if Scalar::from_json(value).is_none() => {
                    return Err(FlowError::BadValue {
                        node: node.id.clone(),
                    });
                }
                Some(value) => {
                    if p.result.is_some() && !value.is_number() {
                        return Err(FlowError::BadOperand {
                            node: node.id.clone(),
                            reason: "value must be a number with result".to_owned(),
                        });
                    }
                    if let Some(window) = &p.average {
                        let bad = |reason: String| FlowError::BadOperand {
                            node: node.id.clone(),
                            reason,
                        };
                        match parse_duration(window) {
                            Err(e) => return Err(bad(format!("average: {e}"))),
                            Ok(d) if d.as_secs() > MAX_AVERAGE_SECS => {
                                return Err(bad("average must be at most 24h".to_owned()));
                            }
                            Ok(_) => {}
                        }
                        if !value.is_number() {
                            return Err(bad("value must be a number with average".to_owned()));
                        }
                    }
                }
            },
            NodeKind::Switch(p) => {
                let bad = |reason: String| FlowError::BadOperand {
                    node: node.id.clone(),
                    reason,
                };
                if p.rules.is_empty() || p.rules.len() > MAX_SWITCH_RULES {
                    return Err(bad(format!(
                        "rules must hold 1 to {MAX_SWITCH_RULES} entries"
                    )));
                }
                if let Some(window) = &p.average {
                    match parse_duration(window) {
                        Err(e) => return Err(bad(format!("average: {e}"))),
                        Ok(d) if d.as_secs() > MAX_AVERAGE_SECS => {
                            return Err(bad("average must be at most 24h".to_owned()));
                        }
                        Ok(_) => {}
                    }
                }
                for (i, rule) in p.rules.iter().enumerate() {
                    let n = i + 1;
                    if Scalar::from_json(&rule.value).is_none() {
                        return Err(bad(format!(
                            "rule {n}: value must be a boolean, a number or a string"
                        )));
                    }
                    match (&rule.op, &rule.to) {
                        (RuleOp::Between, None) => {
                            return Err(bad(format!("rule {n}: to is required for op between")));
                        }
                        (RuleOp::Between, Some(to)) => match (rule.value.as_f64(), to.as_f64()) {
                            (Some(low), Some(high)) if high < low => {
                                return Err(bad(format!("rule {n}: to must be at least value")));
                            }
                            (Some(_), Some(_)) => {}
                            _ => return Err(bad(format!("rule {n}: between needs numbers"))),
                        },
                        (_, Some(_)) => {
                            return Err(bad(format!(
                                "rule {n}: to is only allowed with op between"
                            )));
                        }
                        (_, None) => {}
                    }
                    if p.result.is_some() && !rule.value.is_number() {
                        return Err(bad(format!("rule {n}: value must be a number with result")));
                    }
                    if p.average.is_some() && !rule.value.is_number() {
                        return Err(bad(format!(
                            "rule {n}: value must be a number with average"
                        )));
                    }
                }
            }
            NodeKind::TimeBetween(p) => {
                let bad = |reason: String| FlowError::BadOperand {
                    node: node.id.clone(),
                    reason,
                };
                let from = parse_clock(&p.from)
                    .map_err(|_| bad(format!("from: `{}` is not a HH:MM time", p.from)))?;
                let to = parse_clock(&p.to)
                    .map_err(|_| bad(format!("to: `{}` is not a HH:MM time", p.to)))?;
                if from == to {
                    return Err(bad("from and to must differ".to_owned()));
                }
            }
            NodeKind::Math(p) => check_math_card(&node.id, p, flow)?,
            _ => {}
        }
    }
    check_math_references(flow)?;
    for (edge_index, edge) in flow.edges.iter().enumerate() {
        for end in [&edge.from, &edge.to] {
            if flow.node(end).is_none() {
                return Err(FlowError::UnknownNode {
                    edge_index,
                    node: end.clone(),
                });
            }
        }
        let Some(from) = flow.node(&edge.from) else {
            continue;
        };
        let ports = from.kind.ports();
        match (&edge.port, ports.is_empty()) {
            (None, false) => {
                return Err(FlowError::MissingPort {
                    node: from.id.clone(),
                    kind: match from.kind.type_name() {
                        "if" => "an if",
                        "switch" => "a switch",
                        "time.between" => "a time.between",
                        other => unreachable!("{other} has no ports"),
                    },
                });
            }
            (Some(_), true) => {
                return Err(FlowError::UnexpectedPort {
                    node: from.id.clone(),
                });
            }
            (Some(port), false) if !ports.iter().any(|p| p == port) => {
                return Err(FlowError::UnknownPort {
                    node: from.id.clone(),
                    port: port.clone(),
                });
            }
            _ => {}
        }
    }
    if flow.triggers().is_empty() {
        return Err(FlowError::NoTrigger);
    }
    if let Some(path) = find_cycle(flow) {
        return Err(FlowError::Cycle(path));
    }
    check_ifs_after_valueless_triggers(flow)
}

/// The per-card rules of a math card: inputs, names, averages and the formula. The hub runs it
/// at save time and the web editor for its live error line, so they never disagree.
pub fn check_math_card(node_id: &str, params: &MathParams, flow: &Flow) -> Result<(), FlowError> {
    use super::{MAX_AVERAGE_SECS, MAX_INPUTS, is_valid_name, parse_duration, parse_formula};

    let bad = |reason: String| FlowError::BadMath {
        node: node_id.to_owned(),
        reason,
    };
    if params.inputs.len() > MAX_INPUTS {
        return Err(bad(format!("at most {MAX_INPUTS} inputs")));
    }
    let mut input_names = std::collections::HashSet::new();
    for input in &params.inputs {
        if !is_valid_name(&input.name) {
            return Err(bad(format!("`{}` is not a valid name", input.name)));
        }
        if !input_names.insert(input.name.as_str()) {
            return Err(bad(format!("input `{}` is used twice", input.name)));
        }
        if let Some(window) = &input.average {
            match parse_duration(window) {
                Err(e) => return Err(bad(format!("average: {e}"))),
                Ok(d) if d.as_secs() > MAX_AVERAGE_SECS => {
                    return Err(bad("average must be at most 24h".to_owned()));
                }
                Ok(_) => {}
            }
        }
    }
    if !is_valid_name(&params.name) {
        return Err(bad(format!("`{}` is not a valid name", params.name)));
    }
    let formula = parse_formula(&params.formula).map_err(|e| bad(e.reason))?;
    let others: std::collections::BTreeSet<&str> = flow
        .nodes
        .iter()
        .filter(|n| n.id != node_id)
        .filter_map(|n| match &n.kind {
            NodeKind::Math(p) => Some(p.name.as_str()),
            _ => None,
        })
        .collect();
    for name in formula.names() {
        if input_names.contains(name) || name == "trigger" {
            continue;
        }
        if name == params.name {
            return Err(bad(format!(
                "a math card cannot read its own result `{name}`"
            )));
        }
        if !others.contains(name) {
            return Err(bad(format!("unknown name `{name}`")));
        }
    }
    Ok(())
}

/// The links to math results: a command's `value_from`, an `if` or `switch`'s `result`.
fn check_math_references(flow: &Flow) -> Result<(), FlowError> {
    let names = math_names(flow);
    for node in &flow.nodes {
        let bad = |reason: String| FlowError::BadMath {
            node: node.id.clone(),
            reason,
        };
        let no_card = |name: &str| bad(format!("no math card is named `{name}`"));
        match &node.kind {
            NodeKind::Command(p) => {
                if let Some(name) = &p.value_from {
                    if !names.contains(name.as_str()) {
                        return Err(no_card(name));
                    }
                    if command_with_number(&p.command, 0.0).is_none() {
                        return Err(bad(
                            "value_from works with set_number, set_temperature, set_position and turn_on"
                                .to_owned(),
                        ));
                    }
                }
            }
            NodeKind::If(p) => {
                if let Some(name) = &p.result {
                    if !names.contains(name.as_str()) {
                        return Err(no_card(name));
                    }
                    if p.entity.is_some() || p.average.is_some() || p.missing.is_some() {
                        return Err(bad(RESULT_CLASH.to_owned()));
                    }
                }
            }
            NodeKind::Switch(p) => {
                if let Some(name) = &p.result {
                    if !names.contains(name.as_str()) {
                        return Err(no_card(name));
                    }
                    if p.entity.is_some() || p.average.is_some() {
                        return Err(bad(RESULT_CLASH.to_owned()));
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

const RESULT_CLASH: &str = "result cannot be combined with entity, average or missing";

/// A schedule, a sun event or a manual trigger without a value has no trigger value, so an `if`
/// or a `switch` reachable from one must read an entity, unless the `if` is `op: is_dark`, which
/// reads the sun, or `op: is_off_peak`, which reads the tariff. `time.between` reads the clock and needs none. A manual trigger with a value
/// does carry one, but an average of the trigger value has no history, so an `if` or a `switch`
/// with an `average` reachable from it must name an entity too.
fn check_ifs_after_valueless_triggers(flow: &Flow) -> Result<(), FlowError> {
    use super::{NodeKind, Op};

    let valueless = flow
        .nodes
        .iter()
        .filter(|n| match &n.kind {
            NodeKind::TriggerSchedule(_) | NodeKind::TriggerSun(_) => true,
            NodeKind::TriggerManual(p) => p.value.is_none(),
            _ => false,
        })
        .collect();
    for node in reachable(flow, valueless) {
        if matches!(&node.kind, NodeKind::If(p)
            if p.entity.is_none() && p.result.is_none()
                && !matches!(p.op, Op::IsDark | Op::IsOffPeak))
        {
            return Err(FlowError::IfNeedsEntity {
                node: node.id.clone(),
            });
        }
        if matches!(&node.kind, NodeKind::Switch(p) if p.entity.is_none() && p.result.is_none()) {
            return Err(FlowError::SwitchNeedsEntity {
                node: node.id.clone(),
            });
        }
    }
    let valued = flow
        .nodes
        .iter()
        .filter(|n| matches!(&n.kind, NodeKind::TriggerManual(p) if p.value.is_some()))
        .collect();
    for node in reachable(flow, valued) {
        let averaged = match &node.kind {
            NodeKind::If(p) => p.entity.is_none() && p.average.is_some(),
            NodeKind::Switch(p) => p.entity.is_none() && p.average.is_some(),
            _ => false,
        };
        if averaged {
            return Err(FlowError::AverageNeedsEntity {
                node: node.id.clone(),
            });
        }
    }
    Ok(())
}

/// Every node reachable from `seeds` (the seeds included), following every port.
fn reachable<'a>(flow: &'a Flow, seeds: Vec<&'a super::Node>) -> Vec<&'a super::Node> {
    let mut seen = std::collections::HashSet::new();
    let mut found = Vec::new();
    let mut pending = seeds;
    while let Some(node) = pending.pop() {
        if !seen.insert(node.id.as_str()) {
            continue;
        }
        found.push(node);
        let mut ports = node.kind.ports();
        if ports.is_empty() {
            ports.push(String::new());
        }
        for port in &ports {
            pending.extend(flow.successors(&node.id, (!port.is_empty()).then_some(port.as_str())));
        }
    }
    found
}

/// Iterative DFS with white/grey/black colouring over every edge, whatever its port. Returns the
/// grey path closed on the node that was reached again.
fn find_cycle(flow: &Flow) -> Option<Vec<String>> {
    use std::collections::HashMap;

    let mut next: HashMap<&str, Vec<&str>> = HashMap::new();
    for edge in &flow.edges {
        next.entry(edge.from.as_str())
            .or_default()
            .push(edge.to.as_str());
    }
    // 0 = white (absent), 1 = grey, 2 = black.
    let mut colour: HashMap<&str, u8> = HashMap::new();
    for start in &flow.nodes {
        if colour.contains_key(start.id.as_str()) {
            continue;
        }
        colour.insert(start.id.as_str(), 1);
        // The grey path, each entry with the index of its next child to visit.
        let mut stack: Vec<(&str, usize)> = vec![(start.id.as_str(), 0)];
        while let Some(&(node, i)) = stack.last() {
            let children = next.get(node).map(Vec::as_slice).unwrap_or(&[]);
            let Some(&child) = children.get(i) else {
                colour.insert(node, 2);
                stack.pop();
                continue;
            };
            if let Some(top) = stack.last_mut() {
                top.1 += 1;
            }
            match colour.get(child).copied().unwrap_or(0) {
                0 => {
                    colour.insert(child, 1);
                    stack.push((child, 0));
                }
                1 => {
                    let from = stack.iter().position(|(n, _)| *n == child).unwrap_or(0);
                    let mut path: Vec<String> =
                        stack[from..].iter().map(|(n, _)| (*n).to_owned()).collect();
                    path.push(child.to_owned());
                    return Some(path);
                }
                _ => {}
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::{Edge, MathParams, Node, NodeKind, reference_flow};
    use crate::flow::{command_with_number, parse_formula};

    fn edge(from: &str, to: &str, port: Option<&str>) -> Edge {
        Edge {
            from: from.into(),
            to: to.into(),
            port: port.map(Into::into),
        }
    }

    #[test]
    fn reference_flow_validates() {
        assert_eq!(validate(&reference_flow()), Ok(()));
    }

    #[test]
    fn if_edge_without_port_is_missing_port() {
        let mut f = reference_flow();
        f.edges[1].port = None;
        assert_eq!(
            validate(&f),
            Err(FlowError::MissingPort {
                node: "moving".into(),
                kind: "an if"
            })
        );
        assert!(
            validate(&f)
                .unwrap_err()
                .to_string()
                .contains("node `moving` is an if and needs a port")
        );
    }

    #[test]
    fn port_on_a_command_is_unexpected() {
        let mut f = reference_flow();
        f.edges.push(edge("on", "wait", Some("true")));
        assert_eq!(
            validate(&f),
            Err(FlowError::UnexpectedPort { node: "on".into() })
        );
    }

    #[test]
    fn unknown_port_name() {
        let mut f = reference_flow();
        f.edges[1].port = Some("maybe".into());
        assert_eq!(
            validate(&f),
            Err(FlowError::UnknownPort {
                node: "moving".into(),
                port: "maybe".into()
            })
        );
    }

    #[test]
    fn edge_to_missing_node() {
        let mut f = reference_flow();
        f.edges[3].to = "nope".into();
        let err = validate(&f).unwrap_err();
        assert_eq!(
            err,
            FlowError::UnknownNode {
                edge_index: 3,
                node: "nope".into()
            }
        );
        assert_eq!(err.to_string(), "edge 3 names a missing node `nope`");
    }

    #[test]
    fn cycle_is_rejected() {
        let mut f = reference_flow();
        f.edges.push(edge("off", "wait", None));
        match validate(&f) {
            Err(FlowError::Cycle(path)) => {
                assert!(path.contains(&"wait".to_owned()) && path.contains(&"off".to_owned()));
                assert!(!path.contains(&"motion".to_owned()));
            }
            other => panic!("expected a cycle, got {other:?}"),
        }
        let text = FlowError::Cycle(vec!["wait".into(), "off".into(), "wait".into()]).to_string();
        assert_eq!(text, "cycle through wait → off → wait");
    }

    #[test]
    fn flow_without_trigger_is_rejected() {
        let mut f = reference_flow();
        f.nodes.remove(0);
        f.edges.remove(0);
        assert_eq!(validate(&f), Err(FlowError::NoTrigger));
    }

    #[test]
    fn random_delay_needs_an_upper_bound_at_least_the_lower_one() {
        let mut f = reference_flow();
        if let NodeKind::Delay(p) = &mut f.nodes[3].kind {
            p.up_to = Some("2h".into());
        }
        assert_eq!(validate(&f), Ok(()));
        if let NodeKind::Delay(p) = &mut f.nodes[3].kind {
            p.up_to = Some("5m".into());
        }
        assert_eq!(validate(&f), Ok(()));
        if let NodeKind::Delay(p) = &mut f.nodes[3].kind {
            p.up_to = Some("4m".into());
        }
        let err = validate(&f).unwrap_err();
        assert_eq!(
            err.to_string(),
            "node `wait`: `up_to` (4m) is shorter than `for` (5m)"
        );
    }

    #[test]
    fn bad_duration_and_bad_if_value() {
        let mut f = reference_flow();
        if let NodeKind::Delay(p) = &mut f.nodes[3].kind {
            p.for_ = "5".into();
        }
        assert!(matches!(
            validate(&f),
            Err(FlowError::BadDuration { node, .. }) if node == "wait"
        ));
        let mut f = reference_flow();
        if let NodeKind::Delay(p) = &mut f.nodes[3].kind {
            p.up_to = Some("2x".into());
        }
        assert!(matches!(
            validate(&f),
            Err(FlowError::BadDuration { node, reason })
                if node == "wait" && reason.starts_with("up_to: ")
        ));
        let mut f = reference_flow();
        if let NodeKind::If(p) = &mut f.nodes[1].kind {
            p.value = Some(serde_json::json!([1]));
        }
        let err = validate(&f).unwrap_err();
        assert_eq!(
            err,
            FlowError::BadValue {
                node: "moving".into()
            }
        );
        assert_eq!(
            err.to_string(),
            "node `moving`: value must be a boolean, a number or a string"
        );
    }

    #[test]
    fn field_without_entity_is_refused() {
        let flow = |kind: &str, params: serde_json::Value| {
            let text = serde_json::json!({
                "id": "f", "name": "F",
                "nodes": [
                    { "id": "t", "type": "trigger.state", "params": { "entity": "r" } },
                    { "id": "mid", "type": kind, "params": params }
                ],
                "edges": [{ "from": "t", "to": "mid" }]
            });
            Flow::decode(text.to_string().as_bytes()).unwrap()
        };
        let if_flow = flow(
            "if",
            serde_json::json!({ "op": "lte", "value": 55, "field": "battery" }),
        );
        let switch_flow = flow(
            "switch",
            serde_json::json!({ "field": "battery", "rules": [{ "op": "lte", "value": 55 }] }),
        );
        for f in [if_flow, switch_flow] {
            let err = validate(&f).unwrap_err();
            assert_eq!(err, FlowError::FieldWithoutEntity { node: "mid".into() });
            assert_eq!(err.to_string(), "node `mid`: a field needs an entity");
        }
    }

    #[test]
    fn bad_if_average() {
        let set = |average: &str, value: serde_json::Value| {
            let mut f = reference_flow();
            if let NodeKind::If(p) = &mut f.nodes[1].kind {
                p.average = Some(average.into());
                p.value = Some(value);
            }
            validate(&f)
        };
        let reason = |r: Result<(), FlowError>| match r {
            Err(FlowError::BadOperand { node, reason }) if node == "moving" => reason,
            other => panic!("unexpected {other:?}"),
        };
        assert_eq!(set("15m", serde_json::json!(70)), Ok(()));
        assert_eq!(set("24h", serde_json::json!(70.5)), Ok(()));
        assert!(reason(set("15", serde_json::json!(70))).starts_with("average: "));
        assert_eq!(
            reason(set("24h1s", serde_json::json!(70))),
            "average must be at most 24h"
        );
        assert_eq!(
            reason(set("15m", serde_json::json!(true))),
            "value must be a number with average"
        );
    }

    #[test]
    fn ids_are_checked() {
        let mut f = reference_flow();
        f.id = "Living LED".into();
        assert_eq!(validate(&f), Err(FlowError::BadFlowId("Living LED".into())));
        let mut f = reference_flow();
        f.nodes[0].id = "A B".into();
        assert_eq!(validate(&f), Err(FlowError::BadNodeId("A B".into())));
        let mut f = reference_flow();
        let dup: Node = f.nodes[4].clone();
        f.nodes.push(dup);
        assert_eq!(validate(&f), Err(FlowError::DuplicateNodeId("off".into())));
        assert!(is_valid_flow_id(&"a".repeat(64)) && !is_valid_flow_id(&"a".repeat(65)));
        assert!(is_valid_node_id(&"a".repeat(32)) && !is_valid_node_id(&"a".repeat(33)));
        assert!(!is_valid_flow_id("") && !is_valid_node_id(""));
    }

    fn schedule_flow(cron: &str, then: &str) -> Flow {
        let node = |id: &str, json: &str| -> Node {
            serde_json::from_str(&format!(r#"{{"id":"{id}",{json}}}"#)).unwrap()
        };
        let mut f = Flow {
            id: "tick_flow".into(),
            name: "t".into(),
            ..Flow::default()
        };
        f.nodes.push(node(
            "tick",
            &format!(r#""type":"trigger.schedule","params":{{"cron":"{cron}"}}"#),
        ));
        f.nodes.push(node(
            "act",
            r#""type":"command","params":{"entity":"l","command":{"type":"turn_on"}}"#,
        ));
        f.nodes.push(node(
            "check",
            r#""type":"if","params":{"op":"eq","value":true}"#,
        ));
        f.edges.push(edge("tick", then, None));
        f
    }

    fn notify_flow(params: &str) -> Flow {
        let mut f = schedule_flow("*/10 * * * *", "act");
        f.nodes.push(
            serde_json::from_str(&format!(
                r#"{{"id":"n","type":"notify","params":{params}}}"#
            ))
            .unwrap(),
        );
        f
    }

    #[test]
    fn validate_rejects_empty_notify_title() {
        let err = validate(&notify_flow(r#"{"title":"  "}"#)).unwrap_err();
        assert_eq!(
            err,
            FlowError::BadNotify {
                node: "n".into(),
                reason: "title must not be empty".into()
            }
        );
        assert!(validate(&notify_flow(r#"{"title":"Door"}"#)).is_ok());
    }

    #[test]
    fn validate_rejects_bad_notify_templates() {
        let err = validate(&notify_flow(r#"{"title":"Temp {x"}"#)).unwrap_err();
        assert!(
            matches!(&err, FlowError::BadNotify { reason, .. }
                if reason.starts_with("title:") && reason.contains("unclosed {")),
            "{err:?}"
        );
        let err = validate(&notify_flow(r#"{"title":"T","body":"{a b}"}"#)).unwrap_err();
        assert!(
            matches!(&err, FlowError::BadNotify { reason, .. } if reason.starts_with("body:")),
            "{err:?}"
        );
        assert!(
            validate(&notify_flow(
                r#"{"title":"{weather_temperature:0}","body":"{{ok}}","lang":"fr"}"#
            ))
            .is_ok()
        );
    }

    #[test]
    fn validate_rejects_unknown_notify_channel() {
        let err = validate(&notify_flow(r#"{"title":"T","channel":"pager"}"#)).unwrap_err();
        assert!(
            matches!(&err, FlowError::BadNotify { reason, .. } if reason.contains("pager")),
            "{err}"
        );
        let err = validate(&notify_flow(r#"{"title":"T","severity":"loud"}"#)).unwrap_err();
        assert!(
            matches!(&err, FlowError::BadNotify { reason, .. } if reason.contains("loud")),
            "{err}"
        );
        assert!(
            validate(&notify_flow(
                r#"{"title":"T","channel":"camera","severity":"critical"}"#
            ))
            .is_ok()
        );
    }

    #[test]
    fn bad_flow_target_is_rejected() {
        let mut f = schedule_flow("*/10 * * * *", "act");
        f.nodes.push(
            serde_json::from_str(
                r#"{"id":"stop","type":"flow.enable","params":{"flow":"Bad Id","enabled":false}}"#,
            )
            .unwrap(),
        );
        let err = validate(&f).unwrap_err();
        assert_eq!(
            err,
            FlowError::BadFlowTarget {
                node: "stop".into(),
                flow: "Bad Id".into()
            }
        );
        assert_eq!(
            err.to_string(),
            "node `stop`: `Bad Id` is not a valid flow id"
        );
    }

    #[test]
    fn bad_cron_is_rejected() {
        let f = schedule_flow("61 * * * *", "act");
        assert!(matches!(
            validate(&f),
            Err(FlowError::BadCron { node, .. }) if node == "tick"
        ));
    }

    #[test]
    fn schedule_alone_is_a_trigger() {
        assert_eq!(validate(&schedule_flow("*/10 * * * *", "act")), Ok(()));
    }

    #[test]
    fn if_without_entity_after_a_schedule_is_rejected() {
        let mut f = schedule_flow("*/10 * * * *", "check");
        f.edges.push(edge("check", "act", Some("true")));
        let err = validate(&f).unwrap_err();
        assert_eq!(
            err,
            FlowError::IfNeedsEntity {
                node: "check".into()
            }
        );
        assert_eq!(
            err.to_string(),
            "node `check`: an if after a schedule, sun or manual trigger without a value must name an entity"
        );
    }

    #[test]
    fn if_without_entity_reached_only_from_a_state_trigger_is_fine() {
        let mut f = schedule_flow("*/10 * * * *", "act");
        let state: Node = serde_json::from_str(
            r#"{"id":"motion","type":"trigger.state","params":{"entity":"m"}}"#,
        )
        .unwrap();
        f.nodes.push(state);
        f.edges.push(edge("motion", "check", None));
        f.edges.push(edge("check", "act", Some("true")));
        assert_eq!(validate(&f), Ok(()));
    }

    fn decode_flow(nodes: serde_json::Value, edges: serde_json::Value) -> Flow {
        let json = serde_json::json!({
            "id": "sun_flow",
            "name": "t",
            "nodes": nodes,
            "edges": edges,
        });
        Flow::decode(json.to_string().as_bytes()).unwrap()
    }

    fn manual_flow(manual_params: serde_json::Value, mid: serde_json::Value) -> Flow {
        decode_flow(
            serde_json::json!([
                { "id": "m", "type": "trigger.manual", "params": manual_params },
                mid,
                { "id": "act", "type": "command",
                  "params": { "entity": "l", "command": { "type": "turn_on" } } },
            ]),
            serde_json::json!([
                { "from": "m", "to": "mid" },
                { "from": "mid", "to": "act", "port": "true" },
            ]),
        )
    }

    fn if_node(id: &str, params: serde_json::Value) -> serde_json::Value {
        serde_json::json!({ "id": id, "type": "if", "params": params })
    }

    #[test]
    fn manual_alone_is_a_trigger() {
        let flow = decode_flow(
            serde_json::json!([
                { "id": "m", "type": "trigger.manual", "params": {} },
                { "id": "act", "type": "command",
                  "params": { "entity": "l", "command": { "type": "turn_on" } } },
            ]),
            serde_json::json!([{ "from": "m", "to": "act" }]),
        );
        assert_eq!(validate(&flow), Ok(()));
    }

    #[test]
    fn manual_round_trips() {
        let json = r#"{"id":"f","name":"t","nodes":[{"id":"m","type":"trigger.manual","params":{"value":21.5}}],"edges":[]}"#;
        let flow = Flow::decode(json.as_bytes()).unwrap();
        let encoded = flow.encode();
        assert!(encoded.contains(r#""type":"trigger.manual""#), "{encoded}");
        assert!(encoded.contains(r#""value":21.5"#), "{encoded}");
        let flow = decode_flow(
            serde_json::json!([{ "id": "m", "type": "trigger.manual", "params": { "value": null } }]),
            serde_json::json!([]),
        );
        match &flow.nodes[0].kind {
            NodeKind::TriggerManual(p) => assert_eq!(p.value, None),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn if_without_entity_after_valueless_manual_is_rejected() {
        let flow = decode_flow(
            serde_json::json!([
                { "id": "m", "type": "trigger.manual", "params": {} },
                if_node("check", serde_json::json!({ "op": "eq", "value": true })),
                { "id": "act", "type": "command",
                  "params": { "entity": "l", "command": { "type": "turn_on" } } },
            ]),
            serde_json::json!([
                { "from": "m", "to": "check" },
                { "from": "check", "to": "act", "port": "true" },
            ]),
        );
        let err = validate(&flow).unwrap_err();
        assert_eq!(
            err,
            FlowError::IfNeedsEntity {
                node: "check".into()
            }
        );
        assert_eq!(
            err.to_string(),
            "node `check`: an if after a schedule, sun or manual trigger without a value must name an entity"
        );
    }

    #[test]
    fn switch_without_entity_after_valueless_manual_is_rejected() {
        let flow = decode_flow(
            serde_json::json!([
                { "id": "m", "type": "trigger.manual", "params": {} },
                { "id": "s", "type": "switch", "params": { "rules": [{ "op": "eq", "value": 1 }] } },
                { "id": "act", "type": "command",
                  "params": { "entity": "l", "command": { "type": "turn_on" } } },
            ]),
            serde_json::json!([
                { "from": "m", "to": "s" },
                { "from": "s", "to": "act", "port": "1" },
            ]),
        );
        let err = validate(&flow).unwrap_err();
        assert_eq!(err, FlowError::SwitchNeedsEntity { node: "s".into() });
        assert_eq!(
            err.to_string(),
            "node `s`: a switch after a schedule, sun or manual trigger without a value must name an entity"
        );
    }

    #[test]
    fn if_without_entity_after_valued_manual_is_fine() {
        let flow = manual_flow(
            serde_json::json!({ "value": true }),
            if_node("mid", serde_json::json!({ "op": "eq", "value": true })),
        );
        assert_eq!(validate(&flow), Ok(()));
    }

    #[test]
    fn average_after_valued_manual_needs_entity() {
        let flow = manual_flow(
            serde_json::json!({ "value": 20 }),
            if_node(
                "mid",
                serde_json::json!({ "op": "gt", "value": 19, "average": "15m" }),
            ),
        );
        let err = validate(&flow).unwrap_err();
        assert_eq!(err, FlowError::AverageNeedsEntity { node: "mid".into() });
        assert_eq!(
            err.to_string(),
            "node `mid`: an average after a manual trigger must name an entity"
        );
    }

    #[test]
    fn average_with_entity_after_manual_is_fine() {
        let flow = manual_flow(
            serde_json::json!({ "value": 20 }),
            if_node(
                "mid",
                serde_json::json!({ "op": "gt", "value": 19, "average": "15m", "entity": "t" }),
            ),
        );
        assert_eq!(validate(&flow), Ok(()));
    }

    #[test]
    fn mixed_paths_valueless_manual_and_state_is_rejected() {
        let flow = decode_flow(
            serde_json::json!([
                { "id": "ev", "type": "trigger.state", "params": { "entity": "t" } },
                { "id": "m", "type": "trigger.manual", "params": {} },
                if_node("mid", serde_json::json!({ "op": "eq", "value": true })),
                { "id": "act", "type": "command",
                  "params": { "entity": "l", "command": { "type": "turn_on" } } },
            ]),
            serde_json::json!([
                { "from": "ev", "to": "mid" },
                { "from": "m", "to": "mid" },
                { "from": "mid", "to": "act", "port": "true" },
            ]),
        );
        assert_eq!(
            validate(&flow),
            Err(FlowError::IfNeedsEntity { node: "mid".into() })
        );
    }

    fn sun_flow(id: &str, which: crate::flow::model::SunWhich, offset: Option<i32>) -> Flow {
        let mut params = serde_json::json!({ "which": which });
        if let Some(offset) = offset {
            params["offset_minutes"] = offset.into();
        }
        decode_flow(
            serde_json::json!([{ "id": id, "type": "trigger.sun", "params": params }]),
            serde_json::json!([]),
        )
    }

    fn flow_with_if_params_json(params_json: &str) -> Flow {
        let params: serde_json::Value = serde_json::from_str(params_json).unwrap();
        decode_flow(
            serde_json::json!([
                { "id": "ev", "type": "trigger.state", "params": { "entity": "t" } },
                { "id": "chk", "type": "if", "params": params },
                { "id": "act", "type": "command",
                  "params": { "entity": "l", "command": { "type": "turn_on" } } },
            ]),
            serde_json::json!([
                { "from": "ev", "to": "chk" },
                { "from": "chk", "to": "act", "port": "true" },
            ]),
        )
    }

    fn sun_then_if_flow(if_params: serde_json::Value) -> Flow {
        decode_flow(
            serde_json::json!([
                { "id": "ev", "type": "trigger.sun", "params": { "which": "sunset" } },
                { "id": "chk", "type": "if", "params": if_params },
                { "id": "act", "type": "command",
                  "params": { "entity": "l", "command": { "type": "turn_on" } } },
            ]),
            serde_json::json!([
                { "from": "ev", "to": "chk" },
                { "from": "chk", "to": "act", "port": "true" },
            ]),
        )
    }

    fn sun_then_is_dark_flow() -> Flow {
        sun_then_if_flow(serde_json::json!({ "op": "is_dark" }))
    }

    fn sun_then_plain_if_flow() -> Flow {
        sun_then_if_flow(serde_json::json!({ "op": "lt", "value": 1, "missing": "false" }))
    }

    #[test]
    fn trigger_sun_offset_boundary() {
        use crate::flow::model::SunWhich;
        assert_eq!(
            validate(&sun_flow("ev", SunWhich::Sunset, Some(720))),
            Ok(())
        );
        assert_eq!(
            validate(&sun_flow("ev", SunWhich::Sunset, Some(-720))),
            Ok(())
        );
        assert!(matches!(
            validate(&sun_flow("ev", SunWhich::Sunset, Some(721))),
            Err(FlowError::BadSunOffset {
                offset_minutes: 721,
                ..
            })
        ));
        assert!(matches!(
            validate(&sun_flow("ev", SunWhich::Sunset, Some(-721))),
            Err(FlowError::BadSunOffset {
                offset_minutes: -721,
                ..
            })
        ));
    }

    #[test]
    fn op_is_dark_rejects_entity_value_and_missing() {
        for (field, patch) in [
            ("value", r#"{"op":"is_dark","value":1}"#),
            ("entity", r#"{"op":"is_dark","entity":"e"}"#),
            ("missing", r#"{"op":"is_dark","missing":"true"}"#),
            ("average", r#"{"op":"is_dark","average":"15m"}"#),
            ("result", r#"{"op":"is_dark","result":"x"}"#),
        ] {
            let err = validate(&flow_with_if_params_json(patch)).unwrap_err();
            assert!(
                matches!(err, FlowError::BadOperand { ref reason, .. } if reason.contains(field)),
                "{field}: {err:?}"
            );
        }
    }

    #[test]
    fn is_dark_offset_is_bounded_and_only_for_is_dark() {
        assert_eq!(
            validate(&sun_then_if_flow(
                serde_json::json!({ "op": "is_dark", "offset_minutes": -45 })
            )),
            Ok(())
        );
        assert!(matches!(
            validate(&sun_then_if_flow(
                serde_json::json!({ "op": "is_dark", "offset_minutes": 721 })
            )),
            Err(FlowError::BadSunOffset {
                offset_minutes: 721,
                ..
            })
        ));
        assert!(matches!(
            validate(&sun_then_if_flow(
                serde_json::json!({ "op": "eq", "entity": "l", "value": 1, "offset_minutes": -45 })
            )),
            Err(FlowError::BadOperand { reason, .. }) if reason.contains("offset_minutes")
        ));
    }

    #[test]
    fn non_is_dark_op_requires_value() {
        let err = validate(&flow_with_if_params_json(r#"{"op":"lt","entity":"t"}"#)).unwrap_err();
        assert!(
            matches!(err, FlowError::BadOperand { ref reason, .. } if reason == "value is required for op lt")
        );
    }

    #[test]
    fn op_is_off_peak_rejects_entity_value_missing_average_and_offset() {
        for (field, patch) in [
            ("value", r#"{"op":"is_off_peak","value":1}"#),
            ("entity", r#"{"op":"is_off_peak","entity":"e"}"#),
            ("missing", r#"{"op":"is_off_peak","missing":"true"}"#),
            ("average", r#"{"op":"is_off_peak","average":"15m"}"#),
            ("result", r#"{"op":"is_off_peak","result":"x"}"#),
            (
                "offset_minutes",
                r#"{"op":"is_off_peak","offset_minutes":5}"#,
            ),
        ] {
            let err = validate(&flow_with_if_params_json(patch)).unwrap_err();
            assert!(
                matches!(err, FlowError::BadOperand { ref reason, .. }
                    if *reason == format!("{field} is not allowed with op: is_off_peak")),
                "{field}: {err:?}"
            );
        }
    }

    fn silent_flow(for_: &str) -> Flow {
        decode_flow(
            serde_json::json!([
                { "id": "quiet", "type": "trigger.silent",
                  "params": { "entity": "binary_sensor.smoke", "for": for_ } },
                { "id": "wait", "type": "delay", "params": { "for": "1s" } },
            ]),
            serde_json::json!([{ "from": "quiet", "to": "wait" }]),
        )
    }

    fn limit_flow(max: u32, per: &str, port: Option<&str>) -> Flow {
        let mut edge = serde_json::json!({ "from": "cap", "to": "wait" });
        if let Some(port) = port {
            edge["port"] = serde_json::json!(port);
        }
        decode_flow(
            serde_json::json!([
                { "id": "go", "type": "trigger.manual", "params": {} },
                { "id": "cap", "type": "limit", "params": { "max": max, "per": per } },
                { "id": "wait", "type": "delay", "params": { "for": "1s" } },
            ]),
            serde_json::json!([{ "from": "go", "to": "cap" }, edge]),
        )
    }

    #[test]
    fn silent_is_a_trigger() {
        assert_eq!(validate(&silent_flow("50m")), Ok(()));
    }

    #[test]
    fn window_bounds() {
        for bad in ["59s", "169h", "5", "0m"] {
            assert!(
                matches!(validate(&silent_flow(bad)), Err(FlowError::BadDuration { ref node, .. }) if node == "quiet"),
                "silent {bad}"
            );
            assert!(
                matches!(validate(&limit_flow(1, bad, None)), Err(FlowError::BadDuration { ref node, .. }) if node == "cap"),
                "limit {bad}"
            );
        }
        for good in ["1m", "168h"] {
            assert_eq!(validate(&silent_flow(good)), Ok(()), "silent {good}");
            assert_eq!(validate(&limit_flow(1, good, None)), Ok(()), "limit {good}");
        }
    }

    #[test]
    fn limit_max_bounds() {
        for bad in [0, 101] {
            assert!(
                matches!(validate(&limit_flow(bad, "1h", None)), Err(FlowError::BadLimit { ref node, .. }) if node == "cap"),
                "max {bad}"
            );
        }
        for good in [1, 100] {
            assert_eq!(
                validate(&limit_flow(good, "1h", None)),
                Ok(()),
                "max {good}"
            );
        }
    }

    #[test]
    fn limit_edge_has_no_port() {
        assert_eq!(
            validate(&limit_flow(2, "1h", Some("x"))),
            Err(FlowError::UnexpectedPort { node: "cap".into() })
        );
    }

    #[test]
    fn if_is_off_peak_is_valid_after_a_schedule() {
        let flow = decode_flow(
            serde_json::json!([
                { "id": "tick", "type": "trigger.schedule", "params": { "cron": "0 * * * *" } },
                { "id": "chk", "type": "if", "params": { "op": "is_off_peak" } },
                { "id": "act", "type": "command",
                  "params": { "entity": "l", "command": { "type": "turn_on" } } },
            ]),
            serde_json::json!([
                { "from": "tick", "to": "chk" },
                { "from": "chk", "to": "act", "port": "true" },
            ]),
        );
        assert_eq!(validate(&flow), Ok(()));
    }

    #[test]
    fn if_after_trigger_off_peak_may_read_the_trigger_value() {
        let flow = decode_flow(
            serde_json::json!([
                { "id": "op", "type": "trigger.off_peak", "params": { "edge": "end" } },
                { "id": "chk", "type": "if", "params": { "op": "eq", "value": "end" } },
                { "id": "act", "type": "command",
                  "params": { "entity": "l", "command": { "type": "turn_on" } } },
            ]),
            serde_json::json!([
                { "from": "op", "to": "chk" },
                { "from": "chk", "to": "act", "port": "true" },
            ]),
        );
        assert_eq!(validate(&flow), Ok(()));
    }

    #[test]
    fn if_is_dark_is_valid_right_after_trigger_sun() {
        assert_eq!(validate(&sun_then_is_dark_flow()), Ok(()));
    }

    #[test]
    fn plain_if_without_entity_after_trigger_sun_is_still_rejected() {
        assert_eq!(
            validate(&sun_then_plain_if_flow()),
            Err(FlowError::IfNeedsEntity { node: "chk".into() })
        );
    }

    fn switch_flow(params: serde_json::Value, edges: serde_json::Value) -> Flow {
        decode_flow(
            serde_json::json!([
                { "id": "m", "type": "trigger.state", "params": { "entity": "sensor" } },
                { "id": "s", "type": "switch", "params": params },
                { "id": "a", "type": "command", "params": { "entity": "l", "command": { "type": "turn_on" } } },
                { "id": "b", "type": "command", "params": { "entity": "l", "command": { "type": "turn_off" } } },
            ]),
            edges,
        )
    }

    fn two_rule_edges() -> serde_json::Value {
        serde_json::json!([
            { "from": "m", "to": "s" },
            { "from": "s", "to": "a", "port": "1" },
            { "from": "s", "to": "b", "port": "2" },
        ])
    }

    fn two_rules() -> serde_json::Value {
        serde_json::json!({ "rules": [
            { "op": "lt", "value": 18 },
            { "op": "between", "value": 18, "to": 22 } ] })
    }

    fn switch_reason(params: serde_json::Value) -> String {
        match validate(&switch_flow(params, two_rule_edges())) {
            Err(FlowError::BadOperand { node, reason }) if node == "s" => reason,
            other => panic!("expected BadOperand on s, got {other:?}"),
        }
    }

    fn time_flow(params: serde_json::Value, edges: serde_json::Value) -> Flow {
        decode_flow(
            serde_json::json!([
                { "id": "m", "type": "trigger.schedule", "params": { "cron": "0 7 * * *" } },
                { "id": "t", "type": "time.between", "params": params },
                { "id": "a", "type": "command", "params": { "entity": "l", "command": { "type": "turn_on" } } },
                { "id": "b", "type": "command", "params": { "entity": "l", "command": { "type": "turn_off" } } },
            ]),
            edges,
        )
    }

    fn time_edges() -> serde_json::Value {
        serde_json::json!([
            { "from": "m", "to": "t" },
            { "from": "t", "to": "a", "port": "in" },
            { "from": "t", "to": "b", "port": "out" },
        ])
    }

    #[test]
    fn valid_switch_and_time_flows_pass() {
        assert_eq!(
            validate(&switch_flow(two_rules(), two_rule_edges())),
            Ok(())
        );
        let with_otherwise = serde_json::json!({
            "average": "15m", "otherwise": true, "all": true,
            "rules": [{ "op": "gte", "value": 1 }, { "op": "between", "value": 2, "to": 2 }] });
        let edges = serde_json::json!([
            { "from": "m", "to": "s" },
            { "from": "s", "to": "a", "port": "otherwise" },
        ]);
        assert_eq!(validate(&switch_flow(with_otherwise, edges)), Ok(()));
        let text = serde_json::json!({ "rules": [{ "op": "eq", "value": "on" }] });
        assert_eq!(validate(&switch_flow(text, serde_json::json!([]))), Ok(()));
        assert_eq!(
            validate(&time_flow(
                serde_json::json!({ "from": "22:00", "to": "06:00" }),
                time_edges()
            )),
            Ok(())
        );
    }

    #[test]
    fn if_and_switch_result_need_a_number_value() {
        let reason = |json: &str| match validate(&flow_with_if_params_json(json)) {
            Err(FlowError::BadOperand { node, reason }) if node == "chk" => reason,
            other => panic!("expected BadOperand on chk, got {other:?}"),
        };
        assert_eq!(
            reason(r#"{"op":"eq","result":"x","value":"on"}"#),
            "value must be a number with result"
        );
        assert_eq!(
            reason(r#"{"op":"eq","result":"x","value":true}"#),
            "value must be a number with result"
        );
        assert_eq!(
            switch_reason(serde_json::json!({
                "result": "x", "rules": [{ "op": "lt", "value": 1 }, { "op": "eq", "value": "on" }]
            })),
            "rule 2: value must be a number with result"
        );
    }

    #[test]
    fn switch_rule_errors() {
        let rules = |r: serde_json::Value| serde_json::json!({ "rules": r });
        assert_eq!(
            switch_reason(rules(serde_json::json!([]))),
            "rules must hold 1 to 10 entries"
        );
        let eleven: Vec<_> = (0..11)
            .map(|i| serde_json::json!({ "op": "eq", "value": i }))
            .collect();
        assert_eq!(
            switch_reason(rules(serde_json::json!(eleven))),
            "rules must hold 1 to 10 entries"
        );
        assert_eq!(
            switch_reason(rules(serde_json::json!([
                { "op": "eq", "value": 1 }, { "op": "eq", "value": [1] }]))),
            "rule 2: value must be a boolean, a number or a string"
        );
        assert_eq!(
            switch_reason(rules(serde_json::json!([{ "op": "between", "value": 1 }]))),
            "rule 1: to is required for op between"
        );
        assert_eq!(
            switch_reason(rules(
                serde_json::json!([{ "op": "lt", "value": 1, "to": 2 }])
            )),
            "rule 1: to is only allowed with op between"
        );
        assert_eq!(
            switch_reason(rules(serde_json::json!([
                { "op": "between", "value": "a", "to": 2 }]))),
            "rule 1: between needs numbers"
        );
        assert_eq!(
            switch_reason(rules(serde_json::json!([
                { "op": "between", "value": 1, "to": true }]))),
            "rule 1: between needs numbers"
        );
        assert_eq!(
            switch_reason(rules(serde_json::json!([
                { "op": "between", "value": 5, "to": 4 }]))),
            "rule 1: to must be at least value"
        );
        assert_eq!(
            switch_reason(serde_json::json!({ "average": "15m", "rules": [
                { "op": "eq", "value": 1 }, { "op": "eq", "value": "x" }] })),
            "rule 2: value must be a number with average"
        );
        assert!(
            switch_reason(serde_json::json!({ "average": "soon", "rules": [
                { "op": "eq", "value": 1 }] }))
            .starts_with("average: ")
        );
        assert_eq!(
            switch_reason(serde_json::json!({ "average": "25h", "rules": [
                { "op": "eq", "value": 1 }] })),
            "average must be at most 24h"
        );
    }

    #[test]
    fn switch_ports_checked() {
        let edges = serde_json::json!([
            { "from": "m", "to": "s" }, { "from": "s", "to": "a", "port": "3" }]);
        assert_eq!(
            validate(&switch_flow(two_rules(), edges)),
            Err(FlowError::UnknownPort {
                node: "s".into(),
                port: "3".into()
            })
        );
        let edges = serde_json::json!([
            { "from": "m", "to": "s" }, { "from": "s", "to": "a", "port": "otherwise" }]);
        assert_eq!(
            validate(&switch_flow(two_rules(), edges)),
            Err(FlowError::UnknownPort {
                node: "s".into(),
                port: "otherwise".into()
            })
        );
        let edges = serde_json::json!([
            { "from": "m", "to": "s" }, { "from": "s", "to": "a" }]);
        let err = validate(&switch_flow(two_rules(), edges)).unwrap_err();
        assert_eq!(
            err,
            FlowError::MissingPort {
                node: "s".into(),
                kind: "a switch"
            }
        );
        assert_eq!(err.to_string(), "node `s` is a switch and needs a port");
    }

    #[test]
    fn switch_after_schedule_needs_entity() {
        let flow = |params: serde_json::Value| {
            decode_flow(
                serde_json::json!([
                    { "id": "tick", "type": "trigger.schedule", "params": { "cron": "0 7 * * *" } },
                    { "id": "s", "type": "switch", "params": params },
                    { "id": "a", "type": "command", "params": { "entity": "l", "command": { "type": "turn_on" } } },
                ]),
                serde_json::json!([
                    { "from": "tick", "to": "s" },
                    { "from": "s", "to": "a", "port": "1" },
                ]),
            )
        };
        let err = validate(&flow(
            serde_json::json!({ "rules": [{ "op": "eq", "value": 1 }] }),
        ))
        .unwrap_err();
        assert_eq!(err, FlowError::SwitchNeedsEntity { node: "s".into() });
        assert_eq!(
            err.to_string(),
            "node `s`: a switch after a schedule, sun or manual trigger without a value must name an entity"
        );
        assert_eq!(
            validate(&flow(serde_json::json!({
                "entity": "sensor", "rules": [{ "op": "eq", "value": 1 }] }))),
            Ok(())
        );
    }

    #[test]
    fn time_between_errors() {
        let reason = |params: serde_json::Value| match validate(&time_flow(params, time_edges())) {
            Err(FlowError::BadOperand { node, reason }) if node == "t" => reason,
            other => panic!("expected BadOperand on t, got {other:?}"),
        };
        assert_eq!(
            reason(serde_json::json!({ "from": "7:00", "to": "08:00" })),
            "from: `7:00` is not a HH:MM time"
        );
        assert_eq!(
            reason(serde_json::json!({ "from": "07:00", "to": "24:00" })),
            "to: `24:00` is not a HH:MM time"
        );
        assert_eq!(
            reason(serde_json::json!({ "from": "07:00", "to": "07:00" })),
            "from and to must differ"
        );
        let edges = serde_json::json!([{ "from": "m", "to": "t" }, { "from": "t", "to": "a" }]);
        let err = validate(&time_flow(
            serde_json::json!({ "from": "07:00", "to": "08:00" }),
            edges,
        ))
        .unwrap_err();
        assert_eq!(
            err,
            FlowError::MissingPort {
                node: "t".into(),
                kind: "a time.between"
            }
        );
    }

    #[test]
    fn time_between_after_schedule_is_fine() {
        // time_flow starts from a schedule trigger and has no entity to name.
        assert_eq!(
            validate(&time_flow(
                serde_json::json!({ "from": "07:00", "to": "08:00" }),
                time_edges()
            )),
            Ok(())
        );
    }

    const INTERP: &str = "round(interp(smoothed, [0, 38, 81, 160, 274, 438, 621, 843, 1062, 1270, 1450, 1650, 1947, 2100, 2180, 2200], [0, 10, 15, 20, 25, 30, 35, 40, 45, 50, 55, 60, 70, 80, 90, 100]))";

    /// A state trigger feeding one math card `m` with the given params, then nothing.
    fn math_flow(params: serde_json::Value) -> Flow {
        Flow::decode(
            serde_json::json!({
                "id": "f", "name": "F",
                "nodes": [
                    { "id": "t", "type": "trigger.state", "params": { "entity": "e" } },
                    { "id": "m", "type": "math", "params": params }
                ],
                "edges": [{ "from": "t", "to": "m" }]
            })
            .to_string()
            .as_bytes(),
        )
        .unwrap()
    }

    fn math_reason(params: serde_json::Value) -> String {
        match validate(&math_flow(params)) {
            Err(FlowError::BadMath { node, reason }) => {
                assert_eq!(node, "m");
                reason
            }
            other => panic!("expected BadMath, got {other:?}"),
        }
    }

    #[test]
    fn math_input_rules() {
        let many: Vec<_> = (0..11)
            .map(|i| serde_json::json!({ "name": format!("a{i}"), "entity": "e" }))
            .collect();
        assert_eq!(
            math_reason(serde_json::json!({ "inputs": many, "formula": "1", "name": "r" })),
            "at most 10 inputs"
        );
        assert_eq!(
            math_reason(serde_json::json!({
                "inputs": [{ "name": "Bad", "entity": "e" }], "formula": "1", "name": "r"
            })),
            "`Bad` is not a valid name"
        );
        assert_eq!(
            math_reason(serde_json::json!({ "formula": "1", "name": "min" })),
            "`min` is not a valid name"
        );
        assert_eq!(
            math_reason(serde_json::json!({
                "inputs": [{ "name": "a", "entity": "e" }, { "name": "a", "entity": "f" }],
                "formula": "a", "name": "r"
            })),
            "input `a` is used twice"
        );
    }

    #[test]
    fn math_input_average_follows_the_if_rules() {
        let with = |average: &str| {
            math_reason(serde_json::json!({
                "inputs": [{ "name": "a", "entity": "e", "average": average }],
                "formula": "a", "name": "r"
            }))
        };
        assert_eq!(with("25h"), "average must be at most 24h");
        assert!(with("soon").starts_with("average: "));
    }

    #[test]
    fn math_formula_rules() {
        let parse_reason = parse_formula("1 +").unwrap_err().reason;
        assert_eq!(
            math_reason(serde_json::json!({ "formula": "1 +", "name": "r" })),
            parse_reason
        );
        assert_eq!(
            math_reason(serde_json::json!({ "formula": "x + 1", "name": "r" })),
            "unknown name `x`"
        );
        assert_eq!(
            math_reason(serde_json::json!({ "formula": "r + 1", "name": "r" })),
            "a math card cannot read its own result `r`"
        );
    }

    #[test]
    fn math_card_may_read_trigger_and_another_card() {
        let mut flow = math_flow(serde_json::json!({ "formula": "trigger", "name": "a" }));
        flow.nodes.push(Node {
            id: "n".into(),
            name: None,
            position: None,
            kind: NodeKind::Math(MathParams {
                formula: "a + 1".into(),
                name: "b".into(),
                ..MathParams::default()
            }),
            extra: Default::default(),
        });
        flow.edges.push(edge("m", "n", None));
        assert_eq!(validate(&flow), Ok(()));
        // Two cards may share a name, and an input may shadow nothing it does not own.
        flow.nodes[2].kind = NodeKind::Math(MathParams {
            formula: "1".into(),
            name: "a".into(),
            ..MathParams::default()
        });
        assert_eq!(validate(&flow), Ok(()));
    }

    /// A trigger, a math card `slider`, then `after` (a node JSON) fed by it.
    fn linked_flow(after: serde_json::Value) -> Flow {
        let mut after = after;
        after["id"] = "x".into();
        Flow::decode(
            serde_json::json!({
                "id": "f", "name": "F",
                "nodes": [
                    { "id": "t", "type": "trigger.state", "params": { "entity": "e" } },
                    { "id": "m", "type": "math",
                      "params": { "formula": "trigger", "name": "slider" } },
                    after
                ],
                "edges": [{ "from": "t", "to": "m" }, { "from": "m", "to": "x" }]
            })
            .to_string()
            .as_bytes(),
        )
        .unwrap()
    }

    fn bad_math(flow: &Flow, node: &str, reason: &str) {
        assert_eq!(
            validate(flow),
            Err(FlowError::BadMath {
                node: node.into(),
                reason: reason.into()
            })
        );
    }

    fn command_node(value_from: &str, command: serde_json::Value) -> serde_json::Value {
        serde_json::json!({ "type": "command", "params": {
            "entity": "n", "command": command, "value_from": value_from } })
    }

    #[test]
    fn value_from_rules() {
        let set_number = serde_json::json!({ "type": "set_number", "value": 0 });
        assert_eq!(
            validate(&linked_flow(command_node("slider", set_number.clone()))),
            Ok(())
        );
        bad_math(
            &linked_flow(command_node("nope", set_number)),
            "x",
            "no math card is named `nope`",
        );
        bad_math(
            &linked_flow(command_node(
                "slider",
                serde_json::json!({ "type": "toggle" }),
            )),
            "x",
            "value_from works with set_number, set_temperature, set_position and turn_on",
        );
    }

    #[test]
    fn result_rules_on_if_and_switch() {
        let if_with = |extra: serde_json::Value| {
            let mut params = serde_json::json!({ "op": "gt", "value": 3, "result": "slider" });
            params
                .as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            linked_flow(serde_json::json!({ "type": "if", "params": params }))
        };
        assert_eq!(validate(&if_with(serde_json::json!({}))), Ok(()));
        let clash = "result cannot be combined with entity, average or missing";
        bad_math(&if_with(serde_json::json!({ "entity": "e" })), "x", clash);
        bad_math(&if_with(serde_json::json!({ "average": "5m" })), "x", clash);
        bad_math(
            &if_with(serde_json::json!({ "missing": "true" })),
            "x",
            clash,
        );
        bad_math(
            &if_with(serde_json::json!({ "result": "nope" })),
            "x",
            "no math card is named `nope`",
        );
        let switch_with = |extra: serde_json::Value| {
            let mut params = serde_json::json!({
                "rules": [{ "op": "gt", "value": 3 }], "result": "slider" });
            params
                .as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            linked_flow(serde_json::json!({ "type": "switch", "params": params }))
        };
        assert_eq!(validate(&switch_with(serde_json::json!({}))), Ok(()));
        bad_math(
            &switch_with(serde_json::json!({ "entity": "e" })),
            "x",
            clash,
        );
        bad_math(
            &switch_with(serde_json::json!({ "result": "nope" })),
            "x",
            "no math card is named `nope`",
        );
    }

    #[test]
    fn result_counts_as_naming_the_value_after_a_schedule() {
        let flow = Flow::decode(
            br#"{ "id": "f", "name": "F", "nodes": [
              { "id": "t", "type": "trigger.schedule", "params": { "cron": "0 * * * *" } },
              { "id": "m", "type": "math", "params": { "formula": "1", "name": "r" } },
              { "id": "i", "type": "if", "params": { "op": "gt", "value": 0, "result": "r" } }
            ], "edges": [{ "from": "t", "to": "m" }, { "from": "m", "to": "i" }] }"#,
        )
        .unwrap();
        assert_eq!(validate(&flow), Ok(()));
    }

    #[test]
    fn renamed_math_card_refuses_stale_value_from() {
        let mut flow = linked_flow(command_node(
            "slider",
            serde_json::json!({ "type": "set_number", "value": 0 }),
        ));
        assert_eq!(validate(&flow), Ok(()));
        if let NodeKind::Math(p) = &mut flow.nodes[1].kind {
            p.name = "percent".into();
        }
        bad_math(&flow, "x", "no math card is named `slider`");
    }

    #[test]
    fn math_flow_is_valid() {
        let flow = Flow::decode(
            serde_json::json!({
                "id": "heater", "name": "Heater", "mode": "single",
                "nodes": [
                    { "id": "t", "type": "trigger.state", "params": { "entity": "grid" } },
                    { "id": "d", "type": "math", "params": {
                        "inputs": [
                            { "name": "grid", "entity": "g" },
                            { "name": "heater", "entity": "h", "average": "1m" }],
                        "formula": "clamp(heater - 100 - grid, 0, 1650)", "name": "desired" } },
                    { "id": "s", "type": "math", "params": {
                        "inputs": [{ "name": "prev", "entity": "virtual_target" }],
                        "formula": "if(abs(desired - prev) > 300, 0.1 * prev + 0.9 * desired, 0.6 * prev + 0.4 * desired)",
                        "name": "smoothed", "save_to": "virtual_target" } },
                    { "id": "p", "type": "math", "params": {
                        "formula": INTERP, "name": "slider", "save_to": "group_heater" } },
                    { "id": "c", "type": "command", "params": {
                        "entity": "group_heater",
                        "command": { "type": "set_number", "value": 0 }, "value_from": "slider" } },
                    { "id": "i", "type": "if", "params": {
                        "op": "gt", "value": 300, "result": "desired" } }
                ],
                "edges": [
                    { "from": "t", "to": "d" }, { "from": "d", "to": "s" },
                    { "from": "s", "to": "p" }, { "from": "p", "to": "c" },
                    { "from": "c", "to": "i" }
                ]
            })
            .to_string()
            .as_bytes(),
        )
        .unwrap();
        assert_eq!(validate(&flow), Ok(()));
    }

    #[test]
    fn math_round_trips() {
        let flow = math_flow(serde_json::json!({
            "inputs": [{ "name": "a", "entity": "e", "average": "5m", "tag": "kept too" }],
            "formula": "a", "name": "r", "save_to": "virtual_x", "note": "kept"
        }));
        let NodeKind::Math(p) = &flow.nodes[1].kind else {
            panic!("not a math card");
        };
        assert_eq!(p.inputs[0].average.as_deref(), Some("5m"));
        assert_eq!(
            p.inputs[0].extra.get("tag"),
            Some(&serde_json::json!("kept too"))
        );
        assert_eq!(p.save_to.as_deref(), Some("virtual_x"));
        assert_eq!(p.extra.get("note"), Some(&serde_json::json!("kept")));
        assert_eq!(Flow::decode(flow.encode().as_bytes()).unwrap(), flow);
        assert_eq!(flow.nodes[1].kind.type_name(), "math");
        assert!(flow.nodes[1].kind.ports().is_empty());
        assert!(!flow.nodes[1].kind.is_trigger());
    }

    #[test]
    fn command_with_number_cases() {
        use crate::model::{Command, Hs};
        assert_eq!(
            command_with_number(&Command::SetPosition { position: 0 }, 104.6),
            Some(Command::SetPosition { position: 100 })
        );
        assert_eq!(
            command_with_number(&Command::SetNumber { value: 0.0 }, 2.5),
            Some(Command::SetNumber { value: 2.5 })
        );
        assert_eq!(
            command_with_number(&Command::SetTemperature { temperature: 0.0 }, 19.5),
            Some(Command::SetTemperature { temperature: 19.5 })
        );
        let colour = Hs { h: 10.0, s: 20.0 };
        assert_eq!(
            command_with_number(
                &Command::TurnOn {
                    brightness: None,
                    color: Some(colour),
                    effect: None
                },
                127.5
            ),
            Some(Command::TurnOn {
                brightness: Some(128),
                color: Some(colour),
                effect: None
            })
        );
        assert_eq!(command_with_number(&Command::Toggle, 1.0), None);
    }
}
