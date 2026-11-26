//! Draft operations on a flow being edited: connect rules, delete, duplicate, add, node ids,
//! new flow ids and the undo history. Pure functions over `Flow`, nothing is sent to the hub.

use std::collections::BTreeSet;

use nidoo_core::flow::{
    ASK_MAX_ANSWERS, AskAnswer, AskParams, Edge, Flow, Lang, MathParams, Node, NodeKind, Position,
    SwitchParams, SwitchRule, TriggerSignalParams, TriggerStateParams, math_names,
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
        NodeKind::TriggerAlarm(_) => "alarm",
        NodeKind::TriggerSilent(_) => "silent",
        NodeKind::Limit(_) => "limit",
        NodeKind::TriggerManual(_) => "manual",
        NodeKind::TriggerSignal(_) => "signal",
        NodeKind::If(_) => "if",
        NodeKind::Delay(_) => "delay",
        NodeKind::Command(_) => "command",
        NodeKind::FlowEnable(_) => "flow_switch",
        NodeKind::FlowLabel(_) => "flow_label",
        NodeKind::BackupRun(_) => "backup",
        NodeKind::TriggerWakeUp(_) => "wake_up",
        NodeKind::ClimateHold(_) => "climate_hold",
        NodeKind::ClimateEco(_) => "climate_eco",
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

/// A device trigger pointed at `entity`: a signal trigger when the entity sends signals (a
/// remote, an `event`, a tag), else a state trigger. Another entity resets the filters, which
/// belonged to the old one; an empty `entity` keeps the card's type. Other cards are unchanged.
pub fn retarget_trigger(kind: &NodeKind, entity: String, signal: bool) -> NodeKind {
    let (current, extra) = match kind {
        NodeKind::TriggerState(p) => (&p.entity, &p.extra),
        NodeKind::TriggerSignal(p) => (&p.entity, &p.extra),
        _ => return kind.clone(),
    };
    let was_signal = matches!(kind, NodeKind::TriggerSignal(_));
    let signal = if entity.is_empty() {
        was_signal
    } else {
        signal
    };
    if *current == entity && signal == was_signal {
        return kind.clone();
    }
    let extra = extra.clone();
    if signal {
        NodeKind::TriggerSignal(TriggerSignalParams {
            entity,
            signal: None,
            extra,
        })
    } else {
        NodeKind::TriggerState(TriggerStateParams {
            entity,
            extra,
            ..Default::default()
        })
    }
}

#[cfg(test)]
mod tests;
