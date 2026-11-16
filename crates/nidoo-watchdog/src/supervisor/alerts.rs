//! v1 push alerts for a run's opened and recovered problems.

use serde_json::{Map, Value, json};

use super::checks::Check;
use super::problems::{Problem, Step};

/// Alert lifetime, seconds.
const TTL: u64 = 86_400;
/// More problems than this in one run become one grouped push.
const GROUP_OVER: usize = 3;
/// Longest localisation argument the push service accepts, characters.
const ARG_MAX: usize = 200;

/// `s` cut to what the push service accepts as a localisation argument.
fn arg(s: &str) -> Value {
    Value::String(s.chars().take(ARG_MAX).collect())
}

/// The alerts for `step`, in send order. Sets `alert_id` on each opened problem it announced.
/// Empty when `notify` is off; recovered alerts only with `notify_recovered`. Problems closed
/// because their entity vanished never push.
pub fn alerts(step: &mut Step, notify: bool, notify_recovered: bool, now: u64) -> Vec<Value> {
    if !notify {
        return Vec::new();
    }
    let mut out = Vec::new();

    if step.opened.len() > GROUP_OVER {
        let label = common_check(&step.opened).map_or("problem", Check::as_str);
        let id = format!("supervisor-group-{label}-{now}");
        let names: Vec<&str> = step.opened.iter().map(|p| p.name.as_str()).collect();
        out.push(group(&id, "warning", "wrong", "look wrong", &names));
        for p in &mut step.opened {
            p.alert_id = Some(id.clone());
        }
    } else {
        for p in &mut step.opened {
            let alert = opened(p);
            p.alert_id = Some(p.alert_key());
            out.push(alert);
        }
    }

    if notify_recovered {
        let recovered: Vec<&Problem> = step
            .closed
            .iter()
            .filter(|c| !c.silent)
            .map(|c| &c.problem)
            .collect();
        if recovered.len() > GROUP_OVER {
            let id = format!("supervisor-group-recovered-{now}");
            let names: Vec<&str> = recovered.iter().map(|p| p.name.as_str()).collect();
            out.push(group(
                &id,
                "info",
                "recovered",
                "are back to normal",
                &names,
            ));
        } else {
            out.extend(recovered.into_iter().map(back_to_normal));
        }
    }
    out
}

fn common_check(problems: &[Problem]) -> Option<Check> {
    let first = problems.first()?.check;
    problems.iter().all(|p| p.check == first).then_some(first)
}

/// `15.0 °C`, or `15.0` without a unit.
fn value_text(p: &Problem) -> String {
    let value = p
        .evidence
        .value
        .map_or_else(|| "?".to_string(), |v| format!("{v:.1}"));
    match p.evidence.unit.as_deref() {
        Some(unit) if !unit.is_empty() => format!("{value} {unit}"),
        _ => value,
    }
}

fn args(p: &Problem) -> Map<String, Value> {
    let mut args = Map::new();
    args.insert("name".into(), arg(&p.name));
    args.insert("check".into(), json!(p.check.as_str()));
    if let Some(v) = p.evidence.value {
        args.insert("value".into(), json!(format!("{v:.1}")));
    }
    if let Some(unit) = &p.evidence.unit {
        args.insert("unit".into(), arg(unit));
    }
    args.insert(
        "hours".into(),
        json!((p.evidence.for_secs / 3600).to_string()),
    );
    if let Some(usual) = p.evidence.usual_secs {
        args.insert("usual".into(), json!(usual.div_ceil(3600).to_string()));
    }
    args
}

fn opened(p: &Problem) -> Value {
    let hours = p.evidence.for_secs / 3600;
    let (key, title, body) = match p.check {
        Check::Stuck => (
            "supervisor.stuck",
            format!("{} looks stuck", p.name),
            format!(
                "Same value ({}) for {hours} h, usually changes within {} h. Check the sensor.",
                value_text(p),
                p.evidence.usual_secs.unwrap_or(0).div_ceil(3600)
            ),
        ),
        Check::Silent => (
            "supervisor.silent",
            format!("{} stopped reporting", p.name),
            if p.evidence.usual_secs.is_some() {
                format!("No report for {hours} h. Check the sensor or its battery.")
            } else {
                format!("Offline for {hours} h. Check the sensor or its battery.")
            },
        ),
        Check::Impossible => (
            "supervisor.impossible",
            format!("{} reads an impossible value", p.name),
            format!(
                "{} is outside what this sensor can measure. Check the sensor.",
                value_text(p)
            ),
        ),
    };
    json!({
        "v": 1,
        "id": p.alert_key(),
        "channel": "system",
        "severity": "warning",
        "title": title,
        "body": body,
        "entity_id": p.entity_id,
        "actions": [{"id": "mute", "label": "Mute this sensor"}],
        "silent": false,
        "ttl": TTL,
        "loc": {"key": key, "args": args(p)},
    })
}

fn back_to_normal(p: &Problem) -> Value {
    let body = match p.check {
        Check::Stuck => "Its value is changing again.",
        Check::Silent => "It is reporting again.",
        Check::Impossible => "Its value is back in range.",
    };
    json!({
        "v": 1,
        "id": format!("supervisor-recovered-{}-{}-{}", p.check.as_str(), p.entity_id, p.opened_at),
        "channel": "system",
        "severity": "info",
        "title": format!("{} is back to normal", p.name),
        "body": body,
        "entity_id": p.entity_id,
        "silent": false,
        "ttl": TTL,
        "loc": {"key": "supervisor.recovered", "args": args(p)},
    })
}

/// One push for many problems: first three names, then "and N more".
fn group(id: &str, severity: &str, state: &str, verb: &str, names: &[&str]) -> Value {
    let n = names.len();
    let shown = names[..n.min(GROUP_OVER)].join(", ");
    let body = if n > GROUP_OVER {
        format!("{shown} and {} more", n - GROUP_OVER)
    } else {
        shown.clone()
    };
    json!({
        "v": 1,
        "id": id,
        "channel": "system",
        "severity": severity,
        "title": format!("{n} sensors {verb}"),
        "body": body,
        "silent": false,
        "ttl": TTL,
        "loc": {"key": "supervisor.group", "args": {
            "state": state,
            "n": n.to_string(),
            "names": arg(&shown),
            "more": n.saturating_sub(GROUP_OVER).to_string(),
        }},
    })
}
