//! Draft operations on a flow being edited: connect rules, delete, duplicate, add, node ids,
//! new flow ids and the undo history. Pure functions over `Flow`, nothing is sent to the hub.

use std::collections::BTreeSet;

use nidoo_core::flow::{
    ASK_MAX_ANSWERS, AskAnswer, AskParams, Edge, Flow, Lang, MathParams, Node, NodeKind, Position,
    SwitchParams, SwitchRule, math_names,
};

use crate::flow_layout::Pt;

/// Snapshots kept per direction of the undo history.
const HISTORY_CAP: usize = 100;
/// Offset of a duplicated card from its original, in canvas units.
const DUPLICATE_OFFSET: f64 = 24.0;

/// Why an edge cannot be drawn.
#[derive(Debug, PartialEq)]
pub enum ConnectError {
    /// Triggers start runs; nothing flows into them.
    IntoTrigger,
    /// A node cannot feed itself.
    SelfLoop,
    /// The same edge already exists.
    Duplicate,
    /// `to` already reaches `from`.
    Cycle,
    /// An endpoint is not in the flow.
    UnknownNode,
    /// An `if` needs a `true`/`false` port; every other node takes none.
    BadPort,
    /// Only a card with no wires yet can be dropped into a wire.
    AlreadyWired,
}

/// An edge identity: `(from, port, to)`.
pub type EdgeKey = (String, Option<String>, String);

/// Checks whether `from` (on `port`) may feed `to`. Mirrors the hub's validator rules; fan-in
/// and fan-out are allowed.
pub fn can_connect(
    flow: &Flow,
    from: &str,
    port: Option<&str>,
    to: &str,
) -> Result<(), ConnectError> {
    let (Some(source), Some(target)) = (flow.node(from), flow.node(to)) else {
        return Err(ConnectError::UnknownNode);
    };
    if from == to {
        return Err(ConnectError::SelfLoop);
    }
    if target.kind.is_trigger() {
        return Err(ConnectError::IntoTrigger);
    }
    let ports = source.kind.ports();
    let port_ok = match port {
        Some(p) => ports.iter().any(|x| x == p),
        None => ports.is_empty(),
    };
    if !port_ok {
        return Err(ConnectError::BadPort);
    }
    if flow
        .edges
        .iter()
        .any(|e| e.from == from && e.to == to && e.port.as_deref() == port)
    {
        return Err(ConnectError::Duplicate);
    }
    if reaches(flow, to, from) {
        return Err(ConnectError::Cycle);
    }
    Ok(())
}

/// Whether `target` is reachable from `start` along edges, whatever their port.
fn reaches(flow: &Flow, start: &str, target: &str) -> bool {
    let mut seen = BTreeSet::new();
    let mut pending = vec![start];
    while let Some(id) = pending.pop() {
        if id == target {
            return true;
        }
        if seen.insert(id) {
            pending.extend(
                flow.edges
                    .iter()
                    .filter(|e| e.from == id)
                    .map(|e| e.to.as_str()),
            );
        }
    }
    false
}

/// Adds the edge if [`can_connect`] allows it.
pub fn connect(
    flow: &mut Flow,
    from: &str,
    port: Option<&str>,
    to: &str,
) -> Result<(), ConnectError> {
    can_connect(flow, from, port, to)?;
    flow.edges.push(Edge {
        from: from.to_owned(),
        to: to.to_owned(),
        port: port.map(str::to_owned),
    });
    Ok(())
}

/// Checks whether card `id` may be dropped into the wire `key`: the wire exists, the card has
/// an input (not a trigger) and no wire touches it yet.
pub fn can_insert(flow: &Flow, key: &EdgeKey, id: &str) -> Result<(), ConnectError> {
    let node = flow.node(id).ok_or(ConnectError::UnknownNode)?;
    if !flow
        .edges
        .iter()
        .any(|e| e.from == key.0 && e.port == key.1 && e.to == key.2)
    {
        return Err(ConnectError::UnknownNode);
    }
    if node.kind.is_trigger() {
        return Err(ConnectError::IntoTrigger);
    }
    if flow.edges.iter().any(|e| e.from == id || e.to == id) {
        return Err(ConnectError::AlreadyWired);
    }
    Ok(())
}

/// Splits the wire `key` through card `id`: `from → to` becomes `from → id → to`. The new wire
/// leaves the card by its first port (`true` on an `if`). Nothing changes on error.
pub fn insert_into_edge(flow: &mut Flow, key: &EdgeKey, id: &str) -> Result<(), ConnectError> {
    can_insert(flow, key, id)?;
    let out = flow
        .node(id)
        .and_then(|n| n.kind.ports().into_iter().next());
    let mut next = flow.clone();
    remove_edge(&mut next, key);
    connect(&mut next, &key.0, key.1.as_deref(), id)?;
    connect(&mut next, id, out.as_deref(), &key.2)?;
    *flow = next;
    Ok(())
}

/// The params of switch `node`, when it is one.
fn switch_params<'a>(flow: &'a mut Flow, node: &str) -> Option<&'a mut SwitchParams> {
    flow.nodes
        .iter_mut()
        .find(|n| n.id == node)
        .and_then(|n| match &mut n.kind {
            NodeKind::Switch(p) => Some(p),
            _ => None,
        })
}

/// The params of ask `node`, when it is one.
fn ask_params<'a>(flow: &'a mut Flow, node: &str) -> Option<&'a mut AskParams> {
    flow.nodes
        .iter_mut()
        .find(|n| n.id == node)
        .and_then(|n| match &mut n.kind {
            NodeKind::Ask(p) => Some(p),
            _ => None,
        })
}

/// Rewrites the numeric exit ports of `node`'s edges through `map` (old 0-based rule index to
/// the new one, `None` drops the wire). Other ports are left alone. Returns the edges dropped.
fn remap_exits(flow: &mut Flow, node: &str, map: impl Fn(usize) -> Option<usize>) -> usize {
    let before = flow.edges.len();
    flow.edges.retain_mut(|e| {
        if e.from != node {
            return true;
        }
        let Some(old) = e
            .port
            .as_deref()
            .and_then(|p| p.parse::<usize>().ok())
            .and_then(|n| n.checked_sub(1))
        else {
            return true;
        };
        match map(old) {
            Some(new) => {
                e.port = Some((new + 1).to_string());
                true
            }
            None => false,
        }
    });
    before - flow.edges.len()
}

/// Appends `rule` to switch `node`; its new exit has no wire yet. Does nothing on another node.
pub fn add_rule(flow: &mut Flow, node: &str, rule: SwitchRule) {
    if let Some(p) = switch_params(flow, node) {
        p.rules.push(rule);
    }
}

/// Appends `answer` to ask `node`; its new exit has no wire yet. Does nothing on another node
/// or when the card already has [`ASK_MAX_ANSWERS`] answers.
pub fn add_answer(flow: &mut Flow, node: &str, answer: AskAnswer) {
    if let Some(p) = ask_params(flow, node)
        && p.answers.len() < ASK_MAX_ANSWERS
    {
        p.answers.push(answer);
    }
}

/// Moves answer `from` to position `to` (0-based) in ask `node`; every wire follows its answer
/// and the `none` exit stays put. Does nothing when an index is out of range.
pub fn move_answer(flow: &mut Flow, node: &str, from: usize, to: usize) {
    let Some(p) = ask_params(flow, node) else {
        return;
    };
    if from >= p.answers.len() || to >= p.answers.len() || from == to {
        return;
    }
    let answer = p.answers.remove(from);
    p.answers.insert(to, answer);
    remap_exits(flow, node, |i| Some(moved_index(i, from, to)));
}

/// Where item `i` lands when item `from` moves to `to`.
fn moved_index(i: usize, from: usize, to: usize) -> usize {
    if i == from {
        to
    } else if from < to && i > from && i <= to {
        i - 1
    } else if to < from && i >= to && i < from {
        i + 1
    } else {
        i
    }
}

/// Deletes answer `index` (0-based) from ask `node`: its wires go, later exits are renumbered,
/// `none` is untouched. Returns how many wires were removed.
pub fn delete_answer(flow: &mut Flow, node: &str, index: usize) -> usize {
    let Some(p) = ask_params(flow, node) else {
        return 0;
    };
    if index >= p.answers.len() {
        return 0;
    }
    p.answers.remove(index);
    remap_exits(flow, node, |i| match i.cmp(&index) {
        std::cmp::Ordering::Less => Some(i),
        std::cmp::Ordering::Equal => None,
        std::cmp::Ordering::Greater => Some(i - 1),
    })
}

/// Moves rule `from` to position `to` (0-based) in switch `node`; every wire follows its rule.
/// Does nothing when an index is out of range.
pub fn move_rule(flow: &mut Flow, node: &str, from: usize, to: usize) {
    let Some(p) = switch_params(flow, node) else {
        return;
    };
    if from >= p.rules.len() || to >= p.rules.len() || from == to {
        return;
    }
    let rule = p.rules.remove(from);
    p.rules.insert(to, rule);
    remap_exits(flow, node, |i| {
        Some(if i == from {
            to
        } else if from < to && i > from && i <= to {
            i - 1
        } else if to < from && i >= to && i < from {
            i + 1
        } else {
            i
        })
    });
}

/// Deletes rule `index` (0-based) from switch `node`: its wires go, later exits are renumbered.
/// Returns how many wires were removed.
pub fn delete_rule(flow: &mut Flow, node: &str, index: usize) -> usize {
    let Some(p) = switch_params(flow, node) else {
        return 0;
    };
    if index >= p.rules.len() {
        return 0;
    }
    p.rules.remove(index);
    remap_exits(flow, node, |i| match i.cmp(&index) {
        std::cmp::Ordering::Less => Some(i),
        std::cmp::Ordering::Equal => None,
        std::cmp::Ordering::Greater => Some(i - 1),
    })
}

/// Turns the `otherwise` exit of switch `node` on or off; off drops its wires. Returns how many
/// wires were removed.
pub fn set_otherwise(flow: &mut Flow, node: &str, on: bool) -> usize {
    let Some(p) = switch_params(flow, node) else {
        return 0;
    };
    p.otherwise = on;
    if on {
        return 0;
    }
    let before = flow.edges.len();
    flow.edges
        .retain(|e| !(e.from == node && e.port.as_deref() == Some("otherwise")));
    before - flow.edges.len()
}

/// Removes the edge; whether it existed.
pub fn remove_edge(flow: &mut Flow, key: &EdgeKey) -> bool {
    let before = flow.edges.len();
    flow.edges
        .retain(|e| !(e.from == key.0 && e.port == key.1 && e.to == key.2));
    flow.edges.len() != before
}

/// Removes the node and every edge touching it, then bridges the gap: each card that fed it
/// (on the same port) now feeds each card it fed, where [`can_connect`] allows. Whether it
/// existed.
pub fn remove_node(flow: &mut Flow, id: &str) -> bool {
    let before = flow.nodes.len();
    let into: Vec<(String, Option<String>)> = flow
        .edges
        .iter()
        .filter(|e| e.to == id)
        .map(|e| (e.from.clone(), e.port.clone()))
        .collect();
    let out: Vec<String> = flow
        .edges
        .iter()
        .filter(|e| e.from == id)
        .map(|e| e.to.clone())
        .collect();
    flow.nodes.retain(|n| n.id != id);
    flow.edges.retain(|e| e.from != id && e.to != id);
    for (from, port) in &into {
        for to in &out {
            let _ = connect(flow, from, port.as_deref(), to);
        }
    }
    flow.nodes.len() != before
}

/// Copies a node (kind, params, name, extra) 24 px down-right with a fresh id and no edges.
pub fn duplicate_node(flow: &mut Flow, id: &str) -> Option<String> {
    let mut copy = flow.node(id)?.clone();
    copy.id = next_node_id(flow, &copy.kind);
    copy.position = copy.position.map(|p| Position {
        x: p.x + DUPLICATE_OFFSET,
        y: p.y + DUPLICATE_OFFSET,
    });
    let new_id = copy.id.clone();
    flow.nodes.push(copy);
    Some(new_id)
}

/// Appends a node of `kind` at `at`; returns its new id. A math card with no name (the library's) is named with the first free result name.
pub fn add_node(flow: &mut Flow, kind: NodeKind, at: Pt) -> String {
    let kind = match kind {
        NodeKind::Math(mut p) if p.name.is_empty() => {
            p.name = free_math_name(flow);
            NodeKind::Math(p)
        }
        kind => kind,
    };
    let id = next_node_id(flow, &kind);
    flow.nodes.push(Node {
        id: id.clone(),
        name: None,
        position: Some(Position { x: at.x, y: at.y }),
        kind,
        extra: Default::default(),
    });
    id
}

/// Writes `lang` into every notify node, so the push speaks the language the flow was saved in.
pub fn stamp_notify_lang(flow: &mut Flow, lang: Lang) {
    for node in &mut flow.nodes {
        if let NodeKind::Notify(params) = &mut node.kind {
            params.lang = Some(lang);
        }
    }
}

/// `<prefix>_<n>` with the lowest free `n >= 1`.
pub fn next_node_id(flow: &Flow, kind: &NodeKind) -> String {
    let prefix = match kind {
        NodeKind::TriggerState(_) => "trigger",
        NodeKind::TriggerSchedule(_) => "schedule",
        NodeKind::TriggerSun(_) => "sun",
        NodeKind::TriggerOffPeak(_) => "off_peak",
        NodeKind::TriggerSilent(_) => "silent",
        NodeKind::Limit(_) => "limit",
        NodeKind::TriggerManual(_) => "manual",
        NodeKind::TriggerSignal(_) => "signal",
        NodeKind::If(_) => "if",
        NodeKind::Delay(_) => "delay",
        NodeKind::Command(_) => "command",
        NodeKind::FlowEnable(_) => "flow_switch",
        NodeKind::FlowLabel(_) => "flow_label",
        NodeKind::Notify(_) => "notify",
        NodeKind::Speak(_) => "speak",
        NodeKind::Ask(_) => "ask",
        NodeKind::Switch(_) => "switch",
        NodeKind::TimeBetween(_) => "time",
        NodeKind::Math(_) => "math",
    };
    (1..)
        .map(|n| format!("{prefix}_{n}"))
        .find(|id| flow.node(id).is_none())
        .expect("an unbounded range always yields a free id")
}

/// The first free of `result`, `result2`, `result3`, … among the flow's math card names.
fn free_math_name(flow: &Flow) -> String {
    let taken = math_names(flow);
    std::iter::once("result".to_owned())
        .chain((2..).map(|n| format!("result{n}")))
        .find(|name| !taken.contains(name.as_str()))
        .expect("an unbounded range always yields a free name")
}

/// A new math card: no inputs or formula yet, named with the first free of `result`, `result2`,
/// `result3`, …
pub fn new_math_card(flow: &Flow) -> NodeKind {
    NodeKind::Math(MathParams {
        inputs: Vec::new(),
        formula: String::new(),
        name: free_math_name(flow),
        save_to: None,
        extra: Default::default(),
    })
}

/// The ASCII spelling of a lowercase accented letter; `None` for any other character.
fn fold_char(c: char) -> Option<&'static str> {
    Some(match c {
        'à' | 'â' | 'ä' => "a",
        'é' | 'è' | 'ê' | 'ë' => "e",
        'î' | 'ï' => "i",
        'ô' | 'ö' => "o",
        'ù' | 'û' | 'ü' => "u",
        'ç' => "c",
        'œ' => "oe",
        'æ' => "ae",
        _ => return None,
    })
}

/// Lowercases `text` and folds accented letters to ASCII (`Lumière` becomes `lumiere`), for
/// case- and accent-insensitive matching.
pub(crate) fn fold_accents(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars().flat_map(char::to_lowercase) {
        match fold_char(c) {
            Some(folded) => out.push_str(folded),
            None => out.push(c),
        }
    }
    out
}

/// A new flow's id: the random-version UUID (RFC 9562 v4) spelled by `bytes`, in lowercase
/// hex with hyphens. It never changes, so a flow keeps its address when it is renamed.
pub fn flow_uuid(mut bytes: [u8; 16]) -> String {
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

/// Undo/redo stacks of whole-flow snapshots, 100 deep each. Callers [`record`](Self::record)
/// the state before a gesture once, so a drag of many moves is a single undo step.
#[derive(Debug, Default)]
pub struct History {
    past: Vec<Flow>,
    future: Vec<Flow>,
}

impl History {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records the state before a gesture; clears the redo stack.
    pub fn record(&mut self, before: Flow) {
        self.future.clear();
        push_capped(&mut self.past, before);
    }

    /// Steps back: returns the previous state and keeps `current` for redo.
    pub fn undo(&mut self, current: Flow) -> Option<Flow> {
        let previous = self.past.pop()?;
        push_capped(&mut self.future, current);
        Some(previous)
    }

    /// Steps forward again: returns the undone state and keeps `current` for undo.
    pub fn redo(&mut self, current: Flow) -> Option<Flow> {
        let next = self.future.pop()?;
        push_capped(&mut self.past, current);
        Some(next)
    }

    /// Forgets everything, e.g. when the draft is replaced or saved.
    pub fn clear(&mut self) {
        self.past.clear();
        self.future.clear();
    }

    pub fn can_undo(&self) -> bool {
        !self.past.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.future.is_empty()
    }
}

fn push_capped(stack: &mut Vec<Flow>, flow: Flow) {
    if stack.len() == HISTORY_CAP {
        stack.remove(0);
    }
    stack.push(flow);
}

#[cfg(test)]
mod tests {
    use nidoo_core::flow::{
        CommandParams, DelayParams, FlowEnableParams, FlowLabelParams, IfParams, NotifyParams, Op,
        TriggerScheduleParams, TriggerStateParams,
    };
    use nidoo_core::flow::{RuleOp, SwitchParams, SwitchRule, TimeBetweenParams};
    use serde_json::json;

    use super::*;

    fn node(id: &str, kind: NodeKind) -> Node {
        Node {
            id: id.into(),
            name: None,
            position: None,
            kind,
            extra: Default::default(),
        }
    }
    fn trigger(id: &str) -> Node {
        node(id, NodeKind::TriggerState(TriggerStateParams::default()))
    }
    fn delay(id: &str) -> Node {
        node(id, NodeKind::Delay(DelayParams::default()))
    }
    fn if_kind() -> NodeKind {
        NodeKind::If(IfParams {
            op: Op::Eq,
            value: Some(json!(true)),
            entity: None,
            field: None,
            missing: None,
            average: None,
            offset_minutes: None,
            result: None,
            extra: Default::default(),
        })
    }
    fn flow(nodes: Vec<Node>) -> Flow {
        Flow {
            id: "f".into(),
            name: "F".into(),
            nodes,
            ..Flow::default()
        }
    }

    #[test]
    fn a_manual_card_gets_a_manual_id() {
        let flow = Flow::default();
        let kind = NodeKind::TriggerManual(Default::default());
        assert_eq!(next_node_id(&flow, &kind), "manual_1");
    }

    #[test]
    fn rejects_into_trigger() {
        let f = flow(vec![trigger("t"), delay("d")]);
        assert_eq!(
            can_connect(&f, "d", None, "t"),
            Err(ConnectError::IntoTrigger)
        );
    }

    #[test]
    fn rejects_self_loop() {
        let f = flow(vec![delay("d")]);
        assert_eq!(can_connect(&f, "d", None, "d"), Err(ConnectError::SelfLoop));
    }

    #[test]
    fn rejects_unknown_node() {
        let f = flow(vec![delay("d")]);
        assert_eq!(
            can_connect(&f, "d", None, "x"),
            Err(ConnectError::UnknownNode)
        );
        assert_eq!(
            can_connect(&f, "x", None, "d"),
            Err(ConnectError::UnknownNode)
        );
    }

    #[test]
    fn rejects_duplicate() {
        let mut f = flow(vec![trigger("t"), delay("d")]);
        connect(&mut f, "t", None, "d").unwrap();
        assert_eq!(
            connect(&mut f, "t", None, "d"),
            Err(ConnectError::Duplicate)
        );
        assert_eq!(f.edges.len(), 1);
    }

    #[test]
    fn rejects_cycle() {
        let mut f = flow(vec![delay("a"), delay("b"), delay("c")]);
        connect(&mut f, "a", None, "b").unwrap();
        connect(&mut f, "b", None, "c").unwrap();
        assert_eq!(connect(&mut f, "c", None, "a"), Err(ConnectError::Cycle));
        assert_eq!(f.edges.len(), 2);
    }

    #[test]
    fn if_needs_port() {
        let mut f = flow(vec![trigger("t"), node("i", if_kind()), delay("d")]);
        assert_eq!(can_connect(&f, "i", None, "d"), Err(ConnectError::BadPort));
        assert_eq!(
            can_connect(&f, "i", Some("maybe"), "d"),
            Err(ConnectError::BadPort)
        );
        assert_eq!(
            can_connect(&f, "t", Some("true"), "d"),
            Err(ConnectError::BadPort)
        );
        assert_eq!(connect(&mut f, "i", Some("true"), "d"), Ok(()));
        assert_eq!(connect(&mut f, "i", Some("false"), "d"), Ok(()));
        assert_eq!(f.edges[0].port.as_deref(), Some("true"));
    }

    #[test]
    fn allows_fan_in_and_out() {
        let mut f = flow(vec![trigger("t"), trigger("u"), delay("a"), delay("b")]);
        assert_eq!(connect(&mut f, "t", None, "a"), Ok(()));
        assert_eq!(connect(&mut f, "t", None, "b"), Ok(()));
        assert_eq!(connect(&mut f, "u", None, "a"), Ok(()));
        assert_eq!(f.edges.len(), 3);
    }

    #[test]
    fn remove_edge_by_key() {
        let mut f = flow(vec![trigger("t"), delay("d")]);
        connect(&mut f, "t", None, "d").unwrap();
        let key: EdgeKey = ("t".into(), None, "d".into());
        assert!(remove_edge(&mut f, &key));
        assert!(!remove_edge(&mut f, &key));
        assert!(f.edges.is_empty());
    }

    fn switch_kind(rules: usize, otherwise: bool) -> NodeKind {
        NodeKind::Switch(SwitchParams {
            rules: (0..rules).map(|i| rule(i as i64)).collect(),
            otherwise,
            ..Default::default()
        })
    }
    /// A delay that passes `validate`.
    fn wait(id: &str) -> Node {
        node(
            id,
            NodeKind::Delay(DelayParams {
                for_: "5s".into(),
                ..Default::default()
            }),
        )
    }
    fn rule(v: i64) -> SwitchRule {
        SwitchRule {
            op: RuleOp::Eq,
            value: json!(v),
            to: None,
            extra: Default::default(),
        }
    }
    /// Trigger `t` into switch `s` (3 rules + otherwise), one target per exit.
    fn switch_flow() -> Flow {
        let mut f = flow(vec![
            trigger("t"),
            node("s", switch_kind(3, true)),
            wait("a"),
            wait("b"),
            wait("c"),
            wait("o"),
        ]);
        connect(&mut f, "t", None, "s").unwrap();
        for (port, to) in [("1", "a"), ("2", "b"), ("3", "c"), ("otherwise", "o")] {
            connect(&mut f, "s", Some(port), to).unwrap();
        }
        assert_eq!(nidoo_core::flow::validate(&f), Ok(()));
        f
    }
    fn exits(f: &Flow) -> Vec<(String, String)> {
        let mut v: Vec<_> = f
            .edges
            .iter()
            .filter(|e| e.from == "s")
            .map(|e| (e.port.clone().unwrap(), e.to.clone()))
            .collect();
        v.sort();
        v
    }
    fn pairs(v: &[(&str, &str)]) -> Vec<(String, String)> {
        v.iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect()
    }

    #[test]
    fn delete_rule_renumbers_later_exits() {
        let mut f = switch_flow();
        assert_eq!(delete_rule(&mut f, "s", 1), 1);
        assert_eq!(
            exits(&f),
            pairs(&[("1", "a"), ("2", "c"), ("otherwise", "o")])
        );
        assert_eq!(nidoo_core::flow::validate(&f), Ok(()));
    }

    fn ask_flow() -> Flow {
        let answer = |l: &str| nidoo_core::flow::AskAnswer {
            label: l.into(),
            words: vec![],
        };
        let mut f = flow(vec![
            trigger("t"),
            node(
                "s",
                NodeKind::Ask(nidoo_core::flow::AskParams {
                    text: "Hi".into(),
                    answers: vec![answer("a"), answer("b"), answer("c")],
                    ..Default::default()
                }),
            ),
            wait("a"),
            wait("b"),
            wait("c"),
            wait("o"),
        ]);
        connect(&mut f, "t", None, "s").unwrap();
        for (port, to) in [("1", "a"), ("2", "b"), ("3", "c"), ("none", "o")] {
            connect(&mut f, "s", Some(port), to).unwrap();
        }
        assert_eq!(nidoo_core::flow::validate(&f), Ok(()));
        f
    }

    #[test]
    fn delete_answer_renumbers_later_exits() {
        let mut f = ask_flow();
        assert_eq!(delete_answer(&mut f, "s", 1), 1);
        assert_eq!(exits(&f), pairs(&[("1", "a"), ("2", "c"), ("none", "o")]));
        assert_eq!(nidoo_core::flow::validate(&f), Ok(()));
    }

    #[test]
    fn move_answer_keeps_wires_with_their_answer() {
        let mut f = ask_flow();
        move_answer(&mut f, "s", 0, 2);
        assert_eq!(
            exits(&f),
            pairs(&[("1", "b"), ("2", "c"), ("3", "a"), ("none", "o")])
        );
        let NodeKind::Ask(p) = &f.node("s").unwrap().kind else {
            panic!()
        };
        let labels: Vec<_> = p.answers.iter().map(|a| a.label.as_str()).collect();
        assert_eq!(labels, ["b", "c", "a"]);
        assert_eq!(nidoo_core::flow::validate(&f), Ok(()));
    }

    #[test]
    fn add_answer_appends_and_stops_at_six() {
        let mut f = ask_flow();
        let new = || nidoo_core::flow::AskAnswer {
            label: "x".into(),
            words: vec![],
        };
        add_answer(&mut f, "s", new());
        assert_eq!(exits(&f).len(), 4);
        for _ in 0..5 {
            add_answer(&mut f, "s", new());
        }
        let NodeKind::Ask(p) = &f.node("s").unwrap().kind else {
            panic!()
        };
        assert_eq!(p.answers.len(), nidoo_core::flow::ASK_MAX_ANSWERS);
    }

    #[test]
    fn ask_ids_are_numbered_ask() {
        let f = ask_flow();
        let kind = f.node("s").unwrap().kind.clone();
        assert_eq!(next_node_id(&f, &kind), "ask_1");
    }

    #[test]
    fn move_rule_carries_its_wires() {
        let mut f = switch_flow();
        move_rule(&mut f, "s", 0, 2);
        assert_eq!(
            exits(&f),
            pairs(&[("1", "b"), ("2", "c"), ("3", "a"), ("otherwise", "o")])
        );
        assert_eq!(nidoo_core::flow::validate(&f), Ok(()));
        move_rule(&mut f, "s", 2, 0);
        assert_eq!(
            exits(&f),
            pairs(&[("1", "a"), ("2", "b"), ("3", "c"), ("otherwise", "o")])
        );
        let NodeKind::Switch(p) = &f.node("s").unwrap().kind else {
            panic!()
        };
        let values: Vec<_> = p.rules.iter().map(|r| r.value.clone()).collect();
        assert_eq!(values, [json!(0), json!(1), json!(2)]);
    }

    #[test]
    fn add_rule_appends_without_touching_wires() {
        let mut f = switch_flow();
        add_rule(&mut f, "s", rule(9));
        assert_eq!(f.node("s").unwrap().kind.ports().len(), 5);
        assert_eq!(exits(&f).len(), 4);
        assert_eq!(nidoo_core::flow::validate(&f), Ok(()));
    }

    #[test]
    fn set_otherwise_off_drops_its_wires() {
        let mut f = switch_flow();
        assert_eq!(set_otherwise(&mut f, "s", false), 1);
        assert_eq!(exits(&f), pairs(&[("1", "a"), ("2", "b"), ("3", "c")]));
        assert_eq!(nidoo_core::flow::validate(&f), Ok(()));
        assert_eq!(set_otherwise(&mut f, "s", true), 0);
        assert_eq!(nidoo_core::flow::validate(&f), Ok(()));
    }

    #[test]
    fn can_connect_accepts_switch_and_time_ports() {
        let f = flow(vec![
            node("s", switch_kind(2, false)),
            node(
                "m",
                NodeKind::TimeBetween(TimeBetweenParams {
                    from: "08:00".into(),
                    to: "20:00".into(),
                    extra: Default::default(),
                }),
            ),
            delay("d"),
        ]);
        assert_eq!(can_connect(&f, "s", Some("2"), "d"), Ok(()));
        assert_eq!(
            can_connect(&f, "s", Some("3"), "d"),
            Err(ConnectError::BadPort)
        );
        assert_eq!(can_connect(&f, "s", None, "d"), Err(ConnectError::BadPort));
        assert_eq!(can_connect(&f, "m", Some("in"), "d"), Ok(()));
        assert_eq!(can_connect(&f, "m", Some("out"), "d"), Ok(()));
        assert_eq!(can_connect(&f, "m", None, "d"), Err(ConnectError::BadPort));
    }

    #[test]
    fn splice_uses_first_port() {
        for (kind, first) in [
            (switch_kind(2, true), "1"),
            (
                NodeKind::TimeBetween(TimeBetweenParams {
                    from: "08:00".into(),
                    to: "20:00".into(),
                    extra: Default::default(),
                }),
                "in",
            ),
        ] {
            let mut f = flow(vec![trigger("t"), delay("d"), node("n", kind)]);
            connect(&mut f, "t", None, "d").unwrap();
            let key: EdgeKey = ("t".into(), None, "d".into());
            assert_eq!(insert_into_edge(&mut f, &key, "n"), Ok(()));
            assert_eq!(wires(&f), [("t", None, "n"), ("n", Some(first), "d")]);
        }
    }

    #[test]
    fn insert_splits_the_wire() {
        let mut f = flow(vec![trigger("t"), delay("d"), delay("n")]);
        connect(&mut f, "t", None, "d").unwrap();
        let key: EdgeKey = ("t".into(), None, "d".into());
        assert_eq!(insert_into_edge(&mut f, &key, "n"), Ok(()));
        assert_eq!(wires(&f), [("t", None, "n"), ("n", None, "d")]);
    }

    #[test]
    fn insert_keeps_the_port_and_leaves_an_if_by_true() {
        let mut f = flow(vec![
            trigger("t"),
            node("i", if_kind()),
            delay("d"),
            node("j", if_kind()),
        ]);
        connect(&mut f, "t", None, "i").unwrap();
        connect(&mut f, "i", Some("false"), "d").unwrap();
        let key: EdgeKey = ("i".into(), Some("false".into()), "d".into());
        assert_eq!(insert_into_edge(&mut f, &key, "j"), Ok(()));
        assert!(
            f.edges
                .iter()
                .any(|e| e.from == "i" && e.port.as_deref() == Some("false") && e.to == "j")
        );
        assert!(
            f.edges
                .iter()
                .any(|e| e.from == "j" && e.port.as_deref() == Some("true") && e.to == "d")
        );
        assert_eq!(f.edges.len(), 3);
    }

    #[test]
    fn insert_refuses_wired_cards_triggers_and_unknown_wires() {
        let mut f = flow(vec![trigger("t"), delay("d"), delay("w"), trigger("u")]);
        connect(&mut f, "t", None, "d").unwrap();
        connect(&mut f, "t", None, "w").unwrap();
        let key: EdgeKey = ("t".into(), None, "d".into());
        let before = f.clone();
        assert_eq!(
            insert_into_edge(&mut f, &key, "w"),
            Err(ConnectError::AlreadyWired)
        );
        assert_eq!(
            insert_into_edge(&mut f, &key, "u"),
            Err(ConnectError::IntoTrigger)
        );
        let gone: EdgeKey = ("t".into(), None, "x".into());
        assert_eq!(
            insert_into_edge(&mut f, &gone, "d"),
            Err(ConnectError::UnknownNode)
        );
        assert_eq!(f, before);
    }

    fn wires(f: &Flow) -> Vec<(&str, Option<&str>, &str)> {
        f.edges
            .iter()
            .map(|e| (e.from.as_str(), e.port.as_deref(), e.to.as_str()))
            .collect()
    }

    #[test]
    fn remove_node_bridges_its_neighbours() {
        let mut f = flow(vec![trigger("t"), delay("a"), delay("b")]);
        connect(&mut f, "t", None, "a").unwrap();
        connect(&mut f, "a", None, "b").unwrap();
        assert!(remove_node(&mut f, "a"));
        assert_eq!(wires(&f), [("t", None, "b")]);
        assert_eq!(f.nodes.len(), 2);
        assert!(!remove_node(&mut f, "a"));
    }

    #[test]
    fn remove_node_bridges_every_pair_on_the_feeding_port() {
        let mut f = flow(vec![
            trigger("t"),
            node("i", if_kind()),
            delay("a"),
            delay("b"),
            delay("c"),
        ]);
        connect(&mut f, "t", None, "i").unwrap();
        connect(&mut f, "i", Some("false"), "a").unwrap();
        connect(&mut f, "a", None, "b").unwrap();
        connect(&mut f, "a", None, "c").unwrap();
        // `c` is already fed by `i`: no duplicate wire.
        connect(&mut f, "i", Some("false"), "c").unwrap();
        assert!(remove_node(&mut f, "a"));
        assert_eq!(
            wires(&f),
            [
                ("t", None, "i"),
                ("i", Some("false"), "c"),
                ("i", Some("false"), "b"),
            ]
        );
    }

    #[test]
    fn remove_node_at_an_end_leaves_nothing_to_bridge() {
        let mut f = flow(vec![trigger("t"), delay("a"), delay("b")]);
        connect(&mut f, "t", None, "a").unwrap();
        connect(&mut f, "a", None, "b").unwrap();
        assert!(remove_node(&mut f, "t"));
        assert_eq!(wires(&f), [("a", None, "b")]);
        assert!(remove_node(&mut f, "b"));
        assert!(f.edges.is_empty());
    }

    #[test]
    fn duplicate_offsets_and_unwires() {
        let mut original = delay("delay_1");
        original.name = Some("Wait".into());
        original.position = Some(Position { x: 10.0, y: 20.0 });
        original.extra.insert("note".into(), json!("keep"));
        let mut f = flow(vec![trigger("t"), original.clone()]);
        connect(&mut f, "t", None, "delay_1").unwrap();
        let id = duplicate_node(&mut f, "delay_1").unwrap();
        assert_eq!(id, "delay_2");
        let copy = f.node(&id).unwrap();
        assert_eq!(copy.kind, original.kind);
        assert_eq!(copy.name, original.name);
        assert_eq!(copy.extra, original.extra);
        assert_eq!(copy.position, Some(Position { x: 34.0, y: 44.0 }));
        assert_eq!(f.edges.len(), 1);
        assert_eq!(duplicate_node(&mut f, "nope"), None);
    }

    #[test]
    fn stamp_sets_lang_on_every_notify() {
        let notify = |lang: Option<Lang>| {
            NodeKind::Notify(NotifyParams {
                title: "T".into(),
                lang,
                ..Default::default()
            })
        };
        let mut flow = flow(vec![
            node("a", notify(None)),
            node("b", notify(Some(Lang::En))),
            delay("w"),
        ]);
        let before = flow.nodes[2].clone();
        stamp_notify_lang(&mut flow, Lang::Fr);
        for n in &flow.nodes[..2] {
            assert!(matches!(&n.kind, NodeKind::Notify(p) if p.lang == Some(Lang::Fr)));
        }
        assert_eq!(flow.nodes[2], before);
    }

    #[test]
    fn add_node_places_and_names_it() {
        let mut f = flow(vec![]);
        let id = add_node(&mut f, if_kind(), Pt { x: 5.0, y: 6.0 });
        assert_eq!(id, "if_1");
        let n = f.node("if_1").unwrap();
        assert_eq!(n.position, Some(Position { x: 5.0, y: 6.0 }));
        assert_eq!(n.name, None);
        assert!(n.extra.is_empty());
    }

    #[test]
    fn next_id_fills_gap() {
        let f = flow(vec![node("if_1", if_kind()), node("if_3", if_kind())]);
        assert_eq!(next_node_id(&f, &if_kind()), "if_2");
    }

    #[test]
    fn next_id_prefixes() {
        let f = flow(vec![]);
        let cases = [
            (NodeKind::TriggerState(Default::default()), "trigger_1"),
            (
                NodeKind::TriggerSchedule(TriggerScheduleParams::default()),
                "schedule_1",
            ),
            (NodeKind::Delay(DelayParams::default()), "delay_1"),
            (
                NodeKind::FlowEnable(FlowEnableParams::default()),
                "flow_switch_1",
            ),
            (
                NodeKind::FlowLabel(FlowLabelParams::default()),
                "flow_label_1",
            ),
        ];
        for (kind, want) in cases {
            assert_eq!(next_node_id(&f, &kind), want);
        }
        let command = NodeKind::Command(CommandParams {
            entity: "e".into(),
            command: serde_json::from_value(json!({"type": "turn_on"})).unwrap(),
            value_from: None,
            extra: Default::default(),
        });
        assert_eq!(next_node_id(&f, &command), "command_1");
    }

    #[test]
    fn flow_uuid_is_a_v4_uuid() {
        assert_eq!(flow_uuid([0; 16]), "00000000-0000-4000-8000-000000000000");
        let id = flow_uuid([0xff; 16]);
        assert_eq!(id, "ffffffff-ffff-4fff-bfff-ffffffffffff");
        assert!(nidoo_core::flow::is_valid_flow_id(&id));
    }

    #[test]
    fn history_undo_redo() {
        let (a, b, c) = (
            flow(vec![]),
            flow(vec![delay("d")]),
            flow(vec![trigger("t")]),
        );
        let mut h = History::new();
        assert!(!h.can_undo());
        h.record(a.clone());
        assert!(h.can_undo());
        let back = h.undo(b.clone()).unwrap();
        assert_eq!(back, a);
        assert!(!h.can_undo());
        assert!(h.can_redo());
        assert_eq!(h.redo(a.clone()), Some(b.clone()));
        assert_eq!(h.undo(b.clone()), Some(a.clone()));
        // A new record clears the future.
        h.record(a);
        assert!(!h.can_redo());
        assert_eq!(h.redo(c), None);
        h.clear();
        assert!(!h.can_undo());
        assert_eq!(h.undo(b), None);
    }

    #[test]
    fn history_caps_at_100() {
        let mut h = History::new();
        for i in 0..150 {
            let mut f = flow(vec![]);
            f.name = i.to_string();
            h.record(f);
        }
        let mut current = flow(vec![]);
        let mut steps = 0;
        while let Some(prev) = h.undo(current.clone()) {
            current = prev;
            steps += 1;
        }
        assert_eq!(steps, 100);
        assert_eq!(current.name, "50");
    }

    #[test]
    fn history_records_once_per_gesture() {
        // Callers record the pre-gesture snapshot once, then mutate freely.
        let mut f = flow(vec![trigger("t")]);
        f.nodes[0].position = Some(Position { x: 0.0, y: 0.0 });
        let original = f.clone();
        let mut h = History::new();
        h.record(f.clone());
        for x in [10.0, 20.0, 30.0] {
            f.nodes[0].position = Some(Position { x, y: 0.0 });
        }
        let restored = h.undo(f).unwrap();
        assert_eq!(restored, original);
        assert!(!h.can_undo());
    }

    #[test]
    fn adding_an_unnamed_math_card_names_it() {
        let mut f = flow(vec![]);
        let blank = || NodeKind::Math(MathParams::default());
        let at = Pt { x: 0.0, y: 0.0 };
        let first = add_node(&mut f, blank(), at);
        let second = add_node(&mut f, blank(), at);
        let name = |id: &str| match &f.node(id).unwrap().kind {
            NodeKind::Math(p) => p.name.clone(),
            other => panic!("not a math card: {other:?}"),
        };
        assert_eq!(name(&first), "result");
        assert_eq!(name(&second), "result2");
    }

    #[test]
    fn new_math_card_takes_the_first_free_result_name() {
        let mut f = Flow::default();
        let name = |kind: &NodeKind| match kind {
            NodeKind::Math(p) => p.name.clone(),
            other => panic!("not a math card: {other:?}"),
        };
        assert_eq!(name(&new_math_card(&f)), "result");
        f.nodes.push(node("m1", new_math_card(&f)));
        assert_eq!(name(&new_math_card(&f)), "result2");
        f.nodes.push(node("m2", new_math_card(&f)));
        assert_eq!(name(&new_math_card(&f)), "result3");
    }
}
