//! Pure logic behind the Flows page: the rows, search, filters, counts, sorting, grouping by
//! room and the remembered preferences. No leptos and no browser types, so everything here is
//! tested natively (the screen lives in `screens/flows/`).

use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, BTreeSet};
use std::hash::{Hash, Hasher};
use std::mem::discriminant;

use nidoo_api::{AreaView, EntityView, FlowView, RunStatus};
use nidoo_core::flow::{
    Flow, FlowChoice, FlowLabel, FlowLabelCatalog, FlowMode, LabelAction, LabelColor, NodeKind,
    plan_switch,
};
use nidoo_core::model::UNASSIGNED_AREA;
use serde::{Deserialize, Serialize};

use crate::flow_edit::fold_accents;
use crate::flows::trigger_summary;
use crate::i18n::Lang;
use crate::i18n::flows::{FlowText, flow_text};

/// The key of the section holding flows with no (known) room.
pub const NO_ROOM_KEY: &str = "__no_room__";

/// The key of the section holding flows with no (known) label.
pub const NO_LABEL_KEY: &str = "__no_label__";

/// What starts a flow; one chip each in the top bar. `Manual` is a flow with a Manual card.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerKind {
    State,
    Schedule,
    Sun,
    Signal,
    Manual,
    OffPeak,
    Alarm,
    /// A flow that starts before a wake-up alarm.
    WakeUp,
    Silent,
}

/// The status tab: every flow, running ones, switched-off ones, or those needing a look.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatusFilter {
    #[default]
    All,
    On,
    Off,
    Attention,
}

/// How the list is ordered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sort {
    #[default]
    Name,
    LastRun,
}

/// Whether the list is cut into room sections, label sections, or not at all.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Grouping {
    #[default]
    Room,
    Label,
    None,
}

/// Dense rows or cards.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewMode {
    #[default]
    List,
    Cards,
}

/// One flow with everything the page derives from it once.
#[derive(Clone, Debug, PartialEq)]
pub struct ListRow {
    pub view: FlowView,
    /// The trigger node(s) in words, as `flows::trigger_summary` prints them.
    pub trigger_text: String,
    /// Empty for an invalid flow.
    pub kinds: BTreeSet<TriggerKind>,
    /// A known area id, else `None`.
    pub room: Option<String>,
    pub room_name: Option<String>,
    /// The flow's labels that the catalog knows, in catalog order.
    pub labels: Vec<FlowLabel>,
    pub attention: bool,
}

/// The search box, the status tab and the trigger chips.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Filters {
    pub query: String,
    pub status: StatusFilter,
    /// Empty means every trigger kind.
    pub triggers: BTreeSet<TriggerKind>,
}

/// The numbers on the status tabs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StatusCounts {
    pub all: usize,
    pub on: usize,
    pub off: usize,
    pub attention: usize,
}

/// What a section header offers besides its title.
#[derive(Clone, Debug, PartialEq)]
pub enum SectionHead {
    /// A room (or "No room"): the title only.
    Plain,
    /// A choice: a picker of its labels, in catalog order.
    Choice {
        choice: FlowChoice,
        members: Vec<FlowLabel>,
    },
    /// A standalone label: turn all its flows on or off.
    Label(FlowLabel),
    /// The flows with no label.
    NoLabel,
}

/// A room, a choice or a label, and its flows.
#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    /// The area id or [`NO_ROOM_KEY`]; `choice:<id>`, `label:<id>` or [`NO_LABEL_KEY`].
    pub key: String,
    pub title: String,
    pub head: SectionHead,
    pub rows: Vec<ListRow>,
    /// Whether any row needs attention.
    pub attention: bool,
}

/// What the page remembers between visits (`localStorage`). A missing field takes its default;
/// a value that does not decode (wrong type, unknown variant) resets all of them, see
/// [`decode_prefs`].
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FlowListPrefs {
    pub view: ViewMode,
    pub grouping: Grouping,
    pub sort: Sort,
    pub status: StatusFilter,
    pub triggers: BTreeSet<TriggerKind>,
    /// Keys of the collapsed sections.
    pub folded: BTreeSet<String>,
}

/// A flow needs attention when it is invalid or its latest finished run (skipping runs still
/// going and skipped ones) failed.
pub fn needs_attention(view: &FlowView) -> bool {
    view.invalid.is_some()
        || view
            .runs
            .iter()
            .find(|r| matches!(r.status, RunStatus::Done | RunStatus::Failed))
            .is_some_and(|r| r.status == RunStatus::Failed)
}

/// The kinds of trigger a flow has: one per trigger node type, nothing for an invalid flow (its stub has no nodes to read).
pub fn trigger_kinds(view: &FlowView) -> BTreeSet<TriggerKind> {
    if view.invalid.is_some() {
        return BTreeSet::new();
    }
    view.definition
        .nodes
        .iter()
        .filter_map(|n| match n.kind {
            NodeKind::TriggerState(_) => Some(TriggerKind::State),
            NodeKind::TriggerSchedule(_) => Some(TriggerKind::Schedule),
            NodeKind::TriggerSun(_) => Some(TriggerKind::Sun),
            NodeKind::TriggerOffPeak(_) => Some(TriggerKind::OffPeak),
            NodeKind::TriggerSilent(_) => Some(TriggerKind::Silent),
            NodeKind::TriggerSignal(_) => Some(TriggerKind::Signal),
            NodeKind::TriggerManual(_) => Some(TriggerKind::Manual),
            NodeKind::TriggerAlarm(_) => Some(TriggerKind::Alarm),
            NodeKind::TriggerWakeUp(_) => Some(TriggerKind::WakeUp),
            _ => None,
        })
        .collect()
}

/// The rows for `flows`, in the given order. A flow whose area is unknown (or `Unassigned`)
/// has no room; a label id the catalog does not have is left out.
pub fn rows(
    lang: Lang,
    flows: &[FlowView],
    areas: &[AreaView],
    entities: &BTreeMap<String, EntityView>,
    catalog: &FlowLabelCatalog,
) -> Vec<ListRow> {
    let names: Vec<(String, String)> = flows
        .iter()
        .map(|v| (v.definition.id.clone(), v.definition.name.clone()))
        .collect();
    flows
        .iter()
        .map(|view| {
            let area = view
                .definition
                .area
                .as_deref()
                .filter(|id| *id != UNASSIGNED_AREA)
                .and_then(|id| areas.iter().find(|a| a.id == id));
            ListRow {
                trigger_text: trigger_summary(lang, &view.definition, entities, &names),
                kinds: trigger_kinds(view),
                room: area.map(|a| a.id.clone()),
                room_name: area.map(|a| a.name.clone()),
                labels: catalog
                    .labels
                    .iter()
                    .filter(|l| view.definition.labels.contains(&l.id))
                    .cloned()
                    .collect(),
                attention: needs_attention(view),
                view: view.clone(),
            }
        })
        .collect()
}

fn matches_query(row: &ListRow, needle: &str) -> bool {
    needle.is_empty()
        || [
            Some(row.view.definition.name.as_str()),
            row.view.definition.description.as_deref(),
            Some(row.trigger_text.as_str()),
            row.room_name.as_deref(),
        ]
        .into_iter()
        .flatten()
        .chain(row.labels.iter().map(|l| l.name.as_str()))
        .any(|text| fold_accents(text).contains(needle))
}

fn matches_triggers(row: &ListRow, triggers: &BTreeSet<TriggerKind>) -> bool {
    triggers.is_empty() || !row.kinds.is_disjoint(triggers)
}

fn matches_status(row: &ListRow, status: StatusFilter) -> bool {
    match status {
        StatusFilter::All => true,
        StatusFilter::On => row.view.definition.enabled && row.view.invalid.is_none(),
        StatusFilter::Off => !row.view.definition.enabled,
        StatusFilter::Attention => row.attention,
    }
}

/// The rows passing the search, the status tab and the trigger chips, in their order.
pub fn filter(rows: &[ListRow], f: &Filters) -> Vec<ListRow> {
    let needle = fold_accents(f.query.trim());
    rows.iter()
        .filter(|r| {
            matches_query(r, &needle)
                && matches_triggers(r, &f.triggers)
                && matches_status(r, f.status)
        })
        .cloned()
        .collect()
}

/// The tab numbers: the search and trigger chips apply, the status does not.
pub fn status_counts(
    rows: &[ListRow],
    query: &str,
    triggers: &BTreeSet<TriggerKind>,
) -> StatusCounts {
    let needle = fold_accents(query.trim());
    let mut counts = StatusCounts::default();
    for r in rows
        .iter()
        .filter(|r| matches_query(r, &needle) && matches_triggers(r, triggers))
    {
        counts.all += 1;
        counts.on += usize::from(matches_status(r, StatusFilter::On));
        counts.off += usize::from(matches_status(r, StatusFilter::Off));
        counts.attention += usize::from(r.attention);
    }
    counts
}

/// Sorts in place: by name, or newest run first with never-run flows last (ties by name).
pub fn sort_rows(rows: &mut [ListRow], sort: Sort) {
    let name = |r: &ListRow| fold_accents(&r.view.definition.name);
    match sort {
        Sort::Name => rows.sort_by_cached_key(name),
        Sort::LastRun => rows.sort_by_cached_key(|r| {
            let last = r.view.runs.first().map(|run| run.started_at);
            (std::cmp::Reverse(last), name(r))
        }),
    }
}

/// Cuts `rows` into rooms following `areas` (the hub's order, `Unassigned` skipped, empty
/// rooms omitted), then the flows with no room under "No room". Row order is preserved.
pub fn sections(lang: Lang, rows: Vec<ListRow>, areas: &[AreaView]) -> Vec<Section> {
    let mut out: Vec<Section> = areas
        .iter()
        .filter(|a| a.id != UNASSIGNED_AREA)
        .map(|a| Section {
            key: a.id.clone(),
            title: a.name.clone(),
            head: SectionHead::Plain,
            rows: Vec::new(),
            attention: false,
        })
        .collect();
    let mut no_room = Section {
        key: NO_ROOM_KEY.to_owned(),
        title: flow_text(lang, FlowText::NoRoom).to_owned(),
        head: SectionHead::Plain,
        rows: Vec::new(),
        attention: false,
    };
    for row in rows {
        let target = row
            .room
            .as_deref()
            .and_then(|room| out.iter_mut().find(|s| s.key == room))
            .unwrap_or(&mut no_room);
        target.attention |= row.attention;
        target.rows.push(row);
    }
    out.push(no_room);
    out.retain(|s| !s.rows.is_empty());
    out
}

/// Cuts `rows` by label: one section per choice (catalog order; the rows carrying any of its
/// labels), then one per standalone label, then the rows with no label under "No label". A row
/// is in every section it matches, once each. Choice and label sections are kept when empty
/// (their header still switches); an empty "No label" is dropped. Row order is preserved.
pub fn label_sections(lang: Lang, rows: Vec<ListRow>, catalog: &FlowLabelCatalog) -> Vec<Section> {
    let section = |key: String, title: &str, head: SectionHead, rows: Vec<ListRow>| Section {
        key,
        title: title.to_owned(),
        attention: rows.iter().any(|r| r.attention),
        head,
        rows,
    };
    let carrying = |ids: &[&str]| -> Vec<ListRow> {
        rows.iter()
            .filter(|r| r.labels.iter().any(|l| ids.contains(&l.id.as_str())))
            .cloned()
            .collect()
    };
    let mut out = Vec::new();
    for choice in &catalog.choices {
        let members: Vec<FlowLabel> = catalog.members(&choice.id).into_iter().cloned().collect();
        let ids: Vec<&str> = members.iter().map(|l| l.id.as_str()).collect();
        let rows = carrying(&ids);
        out.push(section(
            format!("choice:{}", choice.id),
            &choice.name,
            SectionHead::Choice {
                choice: choice.clone(),
                members,
            },
            rows,
        ));
    }
    let standalone = catalog.labels.iter().filter(|l| {
        l.choice
            .as_deref()
            .is_none_or(|c| catalog.choice(c).is_none())
    });
    for label in standalone {
        let rows = carrying(&[label.id.as_str()]);
        out.push(section(
            format!("label:{}", label.id),
            &label.name,
            SectionHead::Label(label.clone()),
            rows,
        ));
    }
    let unlabelled: Vec<ListRow> = rows.into_iter().filter(|r| r.labels.is_empty()).collect();
    if !unlabelled.is_empty() {
        out.push(section(
            NO_LABEL_KEY.to_owned(),
            flow_text(lang, FlowText::NoLabel),
            SectionHead::NoLabel,
            unlabelled,
        ));
    }
    out
}

/// A label colour's wire name, which is also its CSS modifier (`label-chip--blue`).
pub fn label_color_name(color: LabelColor) -> &'static str {
    match color {
        LabelColor::Blue => "blue",
        LabelColor::Amber => "amber",
        LabelColor::Green => "green",
        LabelColor::Red => "red",
        LabelColor::Purple => "purple",
        LabelColor::Teal => "teal",
        LabelColor::Pink => "pink",
        LabelColor::Slate => "slate",
    }
}

/// The `<For>` key of a row or card: the flow id and a digest of everything it shows (name,
/// switch, restart tag, invalid reason, trigger, icon, room, label chips, attention, newest run and whether
/// the last finished one failed). A run event changes the key of its own row only, so the page
/// rebuilds that row and keeps every other one as it is; steps added to a running run change
/// nothing shown, so they keep the row too.
pub fn row_key(row: &ListRow) -> (String, u64) {
    let flow = &row.view.definition;
    let mut hasher = DefaultHasher::new();
    flow.name.hash(&mut hasher);
    flow.enabled.hash(&mut hasher);
    (flow.mode == FlowMode::Restart).hash(&mut hasher);
    row.view.invalid.hash(&mut hasher);
    row.trigger_text.hash(&mut hasher);
    row.kinds.hash(&mut hasher);
    row.room.hash(&mut hasher);
    for label in &row.labels {
        (&label.id, &label.name, label.color).hash(&mut hasher);
    }
    row.attention.hash(&mut hasher);
    if let Some(run) = row.view.runs.first() {
        run.id.hash(&mut hasher);
        discriminant(&run.status).hash(&mut hasher);
        run.started_at.hash(&mut hasher);
    }
    row.view
        .runs
        .iter()
        .find(|r| matches!(r.status, RunStatus::Done | RunStatus::Failed))
        .map(|r| r.status == RunStatus::Failed)
        .hash(&mut hasher);
    (flow.id.clone(), hasher.finish())
}

/// The preferences from their stored JSON; anything unreadable gives the defaults.
pub fn decode_prefs(raw: &str) -> FlowListPrefs {
    serde_json::from_str(raw).unwrap_or_default()
}

/// Drops a room the house no longer has (or `Unassigned`) from `flow`, so a save never keeps a
/// stale one.
pub fn clear_stale_area(flow: &mut Flow, areas: &[AreaView]) {
    let known = flow
        .area
        .as_deref()
        .is_some_and(|a| a != UNASSIGNED_AREA && areas.iter().any(|x| x.id == a));
    if !known {
        flow.area = None;
    }
}

/// Drops the label ids the catalog no longer has, so a save never carries a stale one. Call it
/// with the hub's catalog only (see `House::flow_labels_loaded`): an empty placeholder would drop
/// every label.
pub fn clear_stale_labels(flow: &mut Flow, catalog: &FlowLabelCatalog) {
    flow.labels.retain(|id| catalog.label(id).is_some());
}

/// The names of the flows whose label action (a `flow.label` card) names label `id`, sorted.
pub fn label_users(flows: &[FlowView], id: &str) -> Vec<String> {
    let mut names: Vec<String> = flows
        .iter()
        .filter(|f| {
            f.definition
                .nodes
                .iter()
                .any(|n| matches!(&n.kind, NodeKind::FlowLabel(p) if p.label == id))
        })
        .map(|f| f.definition.name.clone())
        .collect();
    names.sort();
    names
}

/// How many flows carry each label id (stale ids included; readers look up known ones).
pub fn label_flow_counts(flows: &[FlowView]) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for flow in flows {
        for id in &flow.definition.labels {
            *counts.entry(id.clone()).or_insert(0) += 1;
        }
    }
    counts
}

/// What a label action reaches, for its card's preview: the valid flows it would turn on and
/// off, counted whatever state they are in now (the card usually runs later). `None` when the
/// action cannot run (an unknown label, or a pick outside a complete choice).
pub fn label_reach(
    catalog: &FlowLabelCatalog,
    flows: &[FlowView],
    label: &str,
    action: LabelAction,
) -> Option<(usize, usize)> {
    let pairs: Vec<(&Flow, bool)> = flows
        .iter()
        .map(|f| (&f.definition, f.invalid.is_some()))
        .collect();
    let plan = plan_switch(catalog, &pairs, label, action).ok()?;
    let valid = || flows.iter().filter(|f| f.invalid.is_none());
    let carries = |f: &FlowView, id: &str| f.definition.labels.iter().any(|l| l == id);
    let on = valid().filter(|f| carries(f, label)).count();
    Some(match action {
        LabelAction::Pick => {
            let others: Vec<&str> = plan
                .choice
                .as_deref()
                .map(|c| catalog.members(c))
                .unwrap_or_default()
                .into_iter()
                .map(|l| l.id.as_str())
                .filter(|id| *id != label)
                .collect();
            let off = valid()
                .filter(|f| !carries(f, label) && others.iter().any(|o| carries(f, o)))
                .count();
            (on, off)
        }
        LabelAction::On => (on, 0),
        LabelAction::Off => (0, on),
    })
}

/// The id of the label a create added: the first label of `after` that `before` lacks.
pub fn new_label_id(before: &FlowLabelCatalog, after: &FlowLabelCatalog) -> Option<String> {
    after
        .labels
        .iter()
        .find(|l| before.label(&l.id).is_none())
        .map(|l| l.id.clone())
}

/// The label whose name matches `name`, ignoring case and accents and surrounding spaces.
pub fn label_by_name<'a>(catalog: &'a FlowLabelCatalog, name: &str) -> Option<&'a FlowLabel> {
    let wanted = fold_accents(name.trim()).to_lowercase();
    catalog
        .labels
        .iter()
        .find(|l| fold_accents(l.name.trim()).to_lowercase() == wanted)
}

/// `labels` with `id` added or removed, sorted and without duplicates, as the hub stores them.
pub fn toggled_labels(labels: &[String], id: &str) -> Vec<String> {
    let mut set: BTreeSet<String> = labels.iter().cloned().collect();
    if !set.remove(id) {
        set.insert(id.to_owned());
    }
    set.into_iter().collect()
}

/// The preferences as the JSON to store.
pub fn encode_prefs(p: &FlowListPrefs) -> String {
    serde_json::to_string(p).unwrap_or_default()
}

#[cfg(test)]
mod tests;
