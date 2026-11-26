//! Structural validation of a flow.

use super::{
    ASK_MAX_ANSWERS, Flow, MathParams, NodeKind, command_with_number, is_valid_name, math_names,
    template,
};

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
    #[error("node `{node}`: from and to are the same alarm mode, so it could never fire")]
    BadAlarmTrigger { node: String },
    #[error("node `{node}`: {reason}")]
    BadThreshold { node: String, reason: String },
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
    #[error("node `{node}`: choose a label")]
    MissingLabel { node: String },
    #[error("timeout must be 1 to 1440 minutes in node `{node}`")]
    BadBackup { node: String },
    #[error("node `{node}`: offset_minutes {offset_minutes} is outside [-720, 720]")]
    BadSunOffset { node: String, offset_minutes: i32 },
    #[error("node `{node}`: before_minutes {minutes} is outside [0, 240]")]
    BadWakeUpBefore { node: String, minutes: u32 },
    #[error("node `{node}`: catch_up_minutes {minutes} is outside [0, 720]")]
    BadCatchUp { node: String, minutes: u32 },
    #[error("node `{node}`: {reason}")]
    BadClimateStep { node: String, reason: String },
    #[error("node `{node}`: {reason}")]
    BadOperand { node: String, reason: String },
    #[error("node `{node}`: {reason}")]
    BadNotify { node: String, reason: String },
    #[error("node `{node}`: {reason}")]
    BadSpeak { node: String, reason: String },
    #[error("node `{node}`: {reason}")]
    BadAsk { node: String, reason: String },
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
        hold, parse_clock, parse_duration, window,
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
        if node.kind.catch_up_minutes() > 720 {
            return Err(FlowError::BadCatchUp {
                node: node.id.clone(),
                minutes: node.kind.catch_up_minutes(),
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
            NodeKind::TriggerState(p) => {
                if let Some(text) = &p.for_
                    && let Err(reason) = hold(text)
                {
                    return Err(FlowError::BadDuration {
                        node: node.id.clone(),
                        reason: format!("for: {reason}"),
                    });
                }
                let bad = |reason: &str| {
                    Err(FlowError::BadThreshold {
                        node: node.id.clone(),
                        reason: reason.to_owned(),
                    })
                };
                if p.above.is_some() || p.below.is_some() {
                    if p.to.is_some() || p.from.is_some() {
                        return bad("above and below cannot be combined with to or from");
                    }
                    if p.above.is_some_and(|a| !a.is_finite())
                        || p.below.is_some_and(|b| !b.is_finite())
                    {
                        return bad("above and below must be finite numbers");
                    }
                    if let (Some(a), Some(b)) = (p.above, p.below)
                        && a >= b
                    {
                        return bad("above must be lower than below, or no value could match");
                    }
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
            NodeKind::TriggerAlarm(p) => {
                if let (Some(from), Some(to)) = (p.from, p.to)
                    && from == to
                    && from != super::AlarmModeFilter::Armed
                {
                    return Err(FlowError::BadAlarmTrigger {
                        node: node.id.clone(),
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
            NodeKind::FlowLabel(p) => {
                if p.label.trim().is_empty() {
                    return Err(FlowError::MissingLabel {
                        node: node.id.clone(),
                    });
                }
            }
            NodeKind::BackupRun(p) => {
                if !(1..=1440).contains(&p.timeout_minutes) {
                    return Err(FlowError::BadBackup {
                        node: node.id.clone(),
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
            NodeKind::Speak(p) => {
                let bad = |reason: String| FlowError::BadSpeak {
                    node: node.id.clone(),
                    reason,
                };
                if p.text.trim().is_empty() {
                    return Err(bad("text must not be empty".to_owned()));
                }
                if let Err(err) = template::parse(&p.text) {
                    return Err(bad(format!("text: {err}")));
                }
            }
            NodeKind::Ask(p) => {
                let bad = |reason: String| FlowError::BadAsk {
                    node: node.id.clone(),
                    reason,
                };
                if p.text.trim().is_empty() {
                    return Err(bad("text must not be empty".to_owned()));
                }
                if let Err(err) = template::parse(&p.text) {
                    return Err(bad(format!("text: {err}")));
                }
                if p.answers.is_empty() || p.answers.len() > ASK_MAX_ANSWERS {
                    return Err(bad(format!("an ask needs 1 to {ASK_MAX_ANSWERS} answers")));
                }
                let mut labels = std::collections::HashSet::new();
                for answer in &p.answers {
                    let label = super::answer::normalize(&answer.label);
                    if label.is_empty() {
                        return Err(bad("answer label must not be empty".to_owned()));
                    }
                    if !labels.insert(label) {
                        return Err(bad(format!("duplicate answer `{}`", answer.label.trim())));
                    }
                }
                match (&p.name, p.default) {
                    (Some(name), _) if !is_valid_name(name) => {
                        return Err(bad(format!("name: `{name}` is not a valid name")));
                    }
                    (None, Some(_)) => return Err(bad("default needs a name".to_owned())),
                    _ => {}
                }
            }
            NodeKind::TriggerWakeUp(p) => {
                if p.before_minutes > 240 {
                    return Err(FlowError::BadWakeUpBefore {
                        node: node.id.clone(),
                        minutes: p.before_minutes,
                    });
                }
            }
            NodeKind::ClimateHold(p) => {
                check_climate_step(&node.id, &p.zones, p.minutes, p.until.as_deref())?;
                if let Some(mode) = p.mode.as_deref()
                    && mode != "heat"
                    && mode != "cool"
                {
                    return Err(FlowError::BadClimateStep {
                        node: node.id.clone(),
                        reason: format!("mode `{mode}` is not heat or cool"),
                    });
                }
            }
            NodeKind::ClimateEco(p) => {
                check_climate_step(&node.id, &p.zones, p.minutes, p.until.as_deref())?;
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
                        "ask" => "an ask",
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
    let others = crate::flow::math_names(flow);
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

/// The checks `climate.hold` and `climate.eco` share: zones, and exactly one valid duration.
fn check_climate_step(
    node: &str,
    zones: &[String],
    minutes: Option<u32>,
    until: Option<&str>,
) -> Result<(), FlowError> {
    let bad = |reason: &str| FlowError::BadClimateStep {
        node: node.to_owned(),
        reason: reason.to_owned(),
    };
    if zones.is_empty() {
        return Err(bad("choose at least one zone"));
    }
    match (minutes, until) {
        (Some(_), Some(_)) => Err(bad("set minutes or until, not both")),
        (None, None) => Err(bad("set minutes or until")),
        (Some(m), None) if !(1..=1440).contains(&m) => Err(bad("minutes must be 1 to 1440")),
        (None, Some(t)) if !is_clock_time(t) => Err(bad("until must be a time HH:MM")),
        _ => Ok(()),
    }
}

/// Whether `s` is `HH:MM` with two digits each, 00:00 to 23:59.
fn is_clock_time(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 5
        && b[2] == b':'
        && [0, 1, 3, 4].iter().all(|&i| b[i].is_ascii_digit())
        && s[..2].parse::<u32>().is_ok_and(|h| h < 24)
        && s[3..].parse::<u32>().is_ok_and(|m| m < 60)
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
            NodeKind::TriggerSchedule(_) | NodeKind::TriggerSun(_) | NodeKind::TriggerWakeUp(_) => {
                true
            }
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
mod tests;
