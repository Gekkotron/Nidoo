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
                trigger_text: trigger_summary(lang, &view.definition, entities),
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
mod tests {
    use super::*;
    use nidoo_api::{AreaView, FlowView, RunStatus, RunView};
    use nidoo_core::flow::{Flow, FlowLabelCatalog};
    use nidoo_core::model::UNASSIGNED_AREA;

    fn run(status: RunStatus, started_at: u64) -> RunView {
        RunView {
            id: format!("r{started_at}"),
            flow_id: "f".into(),
            status,
            started_at,
            ended_at: None,
            trigger: None,
            manual: false,
            steps: vec![],
        }
    }

    fn flow(id: &str, name: &str, triggers: &[&str]) -> Flow {
        let nodes: Vec<String> = triggers
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let params = match *t {
                    "trigger.state" => r#"{ "entity": "m" }"#,
                    "trigger.schedule" => r#"{ "cron": "* * * * *" }"#,
                    "trigger.sun" => r#"{ "which": "sunset" }"#,
                    "trigger.off_peak" => r#"{ "edge": "start" }"#,
                    "trigger.silent" => r#"{ "entity": "m", "for": "50m" }"#,
                    "trigger.manual" => "{}",
                    _ => r#"{ "entity": "r" }"#,
                };
                format!(r#"{{ "id": "t{i}", "type": "{t}", "params": {params} }}"#)
            })
            .collect();
        Flow::decode(
            format!(
                r#"{{ "id": "{id}", "name": "{name}", "nodes": [{}], "edges": [] }}"#,
                nodes.join(",")
            )
            .as_bytes(),
        )
        .unwrap()
    }

    fn view(id: &str, name: &str, triggers: &[&str]) -> FlowView {
        FlowView {
            definition: flow(id, name, triggers),
            invalid: None,
            active_run: None,
            runs: vec![],
        }
    }

    fn invalid(id: &str) -> FlowView {
        FlowView {
            definition: flow(id, id, &[]),
            invalid: Some("broken".into()),
            active_run: None,
            runs: vec![],
        }
    }

    #[test]
    fn row_key_changes_only_with_what_the_row_shows() {
        let base = view("f", "Hall light", &["trigger.state"]);
        let key = |v: &FlowView| row_key(&row(v.clone()));
        assert_eq!(key(&base), key(&base));
        assert_eq!(key(&base).0, "f");

        let mut running = base.clone();
        running.runs = vec![run(RunStatus::Running, 5)];
        assert_ne!(key(&running), key(&base), "a new run shows");
        let mut stepped = running.clone();
        stepped.runs[0].steps.push(nidoo_api::StepView {
            node: "t0".into(),
            at: 6,
            outcome: nidoo_api::StepOutcome::Ok,
            detail: String::new(),
            ports: None,
        });
        assert_eq!(key(&stepped), key(&running), "a step shows nothing new");
        let mut done = running.clone();
        done.runs[0].status = RunStatus::Done;
        assert_ne!(key(&done), key(&running), "the run ending shows");

        let mut off = base.clone();
        off.definition.enabled = false;
        assert_ne!(key(&off), key(&base));
        let mut renamed = base.clone();
        renamed.definition.name = "Porch light".into();
        assert_ne!(key(&renamed), key(&base));
        let moved = row_in(base.clone(), "living", "Living room");
        assert_ne!(row_key(&moved), key(&base));
        let mut broken = base.clone();
        broken.invalid = Some("unknown entity".into());
        assert_ne!(key(&broken), key(&base));
    }

    #[test]
    fn clear_stale_area_drops_unknown_and_unassigned() {
        let areas = vec![area("living", "Living room")];
        let with = |a: &str| {
            let mut f = Flow {
                area: Some(a.into()),
                ..Flow::default()
            };
            clear_stale_area(&mut f, &areas);
            f.area
        };
        assert_eq!(with("ghost"), None);
        assert_eq!(with(UNASSIGNED_AREA), None);
        assert_eq!(with("living"), Some("living".into()));
    }

    fn area(id: &str, name: &str) -> AreaView {
        AreaView {
            id: id.into(),
            name: name.into(),
            icon: None,
            order: None,
            entity_count: 0,
            primary_count: 0,
            group_names: Default::default(),
            group_favorites: Default::default(),
        }
    }

    fn row(v: FlowView) -> ListRow {
        rows(
            Lang::En,
            &[v],
            &[],
            &BTreeMap::new(),
            &FlowLabelCatalog::default(),
        )
        .remove(0)
    }

    /// The house of the examples: a season choice and a standalone label.
    fn seasons() -> FlowLabelCatalog {
        serde_json::from_value(serde_json::json!({
            "labels": [
                {"id": "hiver", "name": "Hiver", "color": "blue", "choice": "saison"},
                {"id": "ete", "name": "Été", "color": "amber", "choice": "saison"},
                {"id": "vacances", "name": "Vacances", "color": "green"}
            ],
            "choices": [
                {"id": "saison", "name": "Saison", "current": "hiver"},
                {"id": "mode", "name": "Mode"}
            ]
        }))
        .unwrap()
    }

    fn labelled(id: &str, labels: &[&str]) -> FlowView {
        let mut v = view(id, id, &["trigger.manual"]);
        v.definition.labels = labels.iter().map(|l| (*l).to_owned()).collect();
        v
    }

    fn label_rows(flows: &[FlowView], catalog: &FlowLabelCatalog) -> Vec<ListRow> {
        rows(Lang::En, flows, &[], &BTreeMap::new(), catalog)
    }

    fn picking(name: &str, labels: &[&str], picks: Option<&str>) -> FlowView {
        let mut nodes =
            vec![serde_json::json!({"id": "go", "type": "trigger.manual", "params": {}})];
        if let Some(label) = picks {
            nodes.push(
                serde_json::json!({"id": "pick", "type": "flow.label", "params": {"label": label}}),
            );
        }
        let json = serde_json::json!({"id": name, "name": name, "labels": labels, "nodes": nodes});
        FlowView {
            definition: Flow::decode(json.to_string().as_bytes()).unwrap(),
            invalid: None,
            active_run: None,
            runs: vec![],
        }
    }

    fn two_labels() -> FlowLabelCatalog {
        serde_json::from_value(serde_json::json!({
            "labels": [{"id": "ete", "name": "Été"}, {"id": "hiver", "name": "Hiver"}]
        }))
        .unwrap()
    }

    #[test]
    fn clear_stale_labels_keeps_known_ids_only() {
        let mut flow = picking("a", &["ete", "gone"], None).definition;
        clear_stale_labels(&mut flow, &two_labels());
        assert_eq!(flow.labels, ["ete"]);
        // A catalog emptied by deleting its last label drops that label too.
        clear_stale_labels(&mut flow, &FlowLabelCatalog::default());
        assert!(flow.labels.is_empty());
    }

    #[test]
    fn label_users_and_counts() {
        let flows = [
            picking("Zeta", &["ete"], Some("ete")),
            picking("Alpha", &["ete", "hiver"], Some("ete")),
            picking("Other", &[], Some("hiver")),
        ];
        assert_eq!(label_users(&flows, "ete"), ["Alpha", "Zeta"]);
        assert!(label_users(&flows, "vacances").is_empty());
        let counts = label_flow_counts(&flows);
        assert_eq!(counts.get("ete"), Some(&2));
        assert_eq!(counts.get("hiver"), Some(&1));
    }

    #[test]
    fn new_label_id_and_name_lookup() {
        let before = two_labels();
        let mut after = before.clone();
        after.labels.push(
            serde_json::from_value(serde_json::json!({"id": "vacances", "name": "Vacances"}))
                .unwrap(),
        );
        assert_eq!(new_label_id(&before, &after).as_deref(), Some("vacances"));
        assert_eq!(new_label_id(&before, &before), None);
        assert_eq!(
            label_by_name(&before, " été ").map(|l| l.id.as_str()),
            Some("ete")
        );
        assert_eq!(
            label_by_name(&before, "ETE").map(|l| l.id.as_str()),
            Some("ete")
        );
        assert!(label_by_name(&before, "Printemps").is_none());
    }

    #[test]
    fn toggled_labels_sorts_and_dedups() {
        let labels = ["hiver".to_owned(), "ete".to_owned()];
        assert_eq!(
            toggled_labels(&labels, "vacances"),
            ["ete", "hiver", "vacances"]
        );
        assert_eq!(toggled_labels(&labels, "hiver"), ["ete"]);
    }

    #[test]
    fn label_reach_counts_carried_flows_whatever_their_state() {
        let catalog: FlowLabelCatalog = serde_json::from_value(serde_json::json!({
            "labels": [
                {"id": "hiver", "name": "Hiver", "choice": "saison"},
                {"id": "ete", "name": "Été", "choice": "saison"},
                {"id": "vacances", "name": "Vacances"}
            ],
            "choices": [{"id": "saison", "name": "Saison"}]
        }))
        .unwrap();
        let mut off = picking("fan", &["ete"], None);
        off.definition.enabled = false;
        let mut broken = picking("broken", &["ete"], None);
        broken.invalid = Some("bad".into());
        let flows = [
            picking("heat", &["hiver"], None),
            picking("both", &["hiver", "ete"], None),
            off,
            broken,
        ];
        // Été: "both" and "fan" (off now) on; "heat" off; the invalid one is skipped.
        assert_eq!(
            label_reach(&catalog, &flows, "ete", LabelAction::Pick),
            Some((2, 1))
        );
        assert_eq!(
            label_reach(&catalog, &flows, "ete", LabelAction::Off),
            Some((0, 2))
        );
        assert_eq!(
            label_reach(&catalog, &flows, "hiver", LabelAction::On),
            Some((2, 0))
        );
        assert_eq!(
            label_reach(&catalog, &flows, "vacances", LabelAction::Pick),
            None
        );
        assert_eq!(label_reach(&catalog, &flows, "gone", LabelAction::On), None);
    }

    #[test]
    fn label_color_names_are_the_wire_names() {
        for color in nidoo_core::flow::LabelColor::ALL {
            let wire = serde_json::to_value(color).unwrap();
            assert_eq!(wire, label_color_name(color));
        }
    }

    #[test]
    fn grouping_label_decodes() {
        assert_eq!(
            decode_prefs(r#"{"grouping":"label"}"#).grouping,
            Grouping::Label
        );
        let p = FlowListPrefs {
            grouping: Grouping::Label,
            ..FlowListPrefs::default()
        };
        assert_eq!(decode_prefs(&encode_prefs(&p)), p);
    }

    #[test]
    fn rows_resolve_labels_in_catalog_order_and_drop_unknown_ids() {
        let catalog = seasons();
        let out = label_rows(&[labelled("a", &["vacances", "ghost", "hiver"])], &catalog);
        let ids: Vec<&str> = out[0].labels.iter().map(|l| l.id.as_str()).collect();
        assert_eq!(ids, ["hiver", "vacances"]);
    }

    #[test]
    fn label_sections_order_and_duplicates() {
        let catalog = seasons();
        let list = label_rows(
            &[
                labelled("both", &["hiver", "ete"]),
                labelled("winter_away", &["hiver", "vacances"]),
                labelled("plain", &[]),
                labelled("stale", &["ghost"]),
            ],
            &catalog,
        );
        let out = label_sections(Lang::Fr, list, &catalog);
        let keys: Vec<&str> = out.iter().map(|s| s.key.as_str()).collect();
        assert_eq!(
            keys,
            [
                "choice:saison",
                "choice:mode",
                "label:vacances",
                NO_LABEL_KEY
            ]
        );
        assert_eq!(ids(&out[0].rows), ["both", "winter_away"]);
        assert_eq!(ids(&out[2].rows), ["winter_away"]);
        assert_eq!(ids(&out[3].rows), ["plain", "stale"]);
        assert_eq!(out[3].title, "Sans étiquette");
        assert_eq!(out[0].title, "Saison");
        match &out[0].head {
            SectionHead::Choice { choice, members } => {
                assert_eq!(choice.id, "saison");
                let m: Vec<&str> = members.iter().map(|l| l.id.as_str()).collect();
                assert_eq!(m, ["hiver", "ete"]);
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(&out[2].head, SectionHead::Label(l) if l.id == "vacances"));
        assert_eq!(out[3].head, SectionHead::NoLabel);
    }

    #[test]
    fn empty_choice_section_is_kept() {
        let catalog = seasons();
        let out = label_sections(
            Lang::En,
            label_rows(&[labelled("x", &["hiver"])], &catalog),
            &catalog,
        );
        let keys: Vec<&str> = out.iter().map(|s| s.key.as_str()).collect();
        // `mode` and `vacances` carry no flow but keep their switch; "No label" is empty and dropped.
        assert_eq!(keys, ["choice:saison", "choice:mode", "label:vacances"]);
        assert!(out[1].rows.is_empty());
        assert!(matches!(&out[1].head, SectionHead::Choice { members, .. } if members.is_empty()));
    }

    #[test]
    fn room_sections_have_plain_heads() {
        let out = sections(Lang::En, vec![row(view("a", "A", &[]))], &[]);
        assert_eq!(out[0].head, SectionHead::Plain);
    }

    #[test]
    fn query_matches_label_names() {
        let catalog = seasons();
        let list = label_rows(&[labelled("a", &["ete"]), labelled("b", &[])], &catalog);
        let f = Filters {
            query: "ETE".into(),
            ..Filters::default()
        };
        assert_eq!(ids(&filter(&list, &f)), ["a"]);
    }

    #[test]
    fn row_key_changes_with_labels() {
        let catalog = seasons();
        let key = |labels: &[&str], catalog: &FlowLabelCatalog| {
            row_key(&label_rows(&[labelled("a", labels)], catalog)[0])
        };
        assert_ne!(key(&["hiver"], &catalog), key(&[], &catalog));
        assert_ne!(key(&["hiver"], &catalog), key(&["ete"], &catalog));
        let mut recoloured = catalog.clone();
        recoloured.labels[0].color = nidoo_core::flow::LabelColor::Red;
        assert_ne!(key(&["hiver"], &catalog), key(&["hiver"], &recoloured));
        let mut renamed = catalog.clone();
        renamed.labels[0].name = "Winter".into();
        assert_ne!(key(&["hiver"], &catalog), key(&["hiver"], &renamed));
    }

    fn row_in(v: FlowView, room: &str, room_name: &str) -> ListRow {
        let mut r = row(v);
        r.room = Some(room.into());
        r.room_name = Some(room_name.into());
        r
    }

    fn failing_sun(id: &str) -> FlowView {
        let mut v = view(id, id, &["trigger.sun"]);
        v.runs = vec![run(RunStatus::Failed, 1)];
        v
    }

    fn ids(rows: &[ListRow]) -> Vec<&str> {
        rows.iter().map(|r| r.view.definition.id.as_str()).collect()
    }

    #[test]
    fn attention_on_invalid_and_latest_finished_failure() {
        let with = |statuses: &[RunStatus]| {
            let mut v = view("f", "F", &[]);
            v.runs = statuses.iter().map(|s| run(*s, 1)).collect();
            v
        };
        assert!(needs_attention(&invalid("x")));
        assert!(needs_attention(&with(&[RunStatus::Failed])));
        assert!(needs_attention(&with(&[
            RunStatus::Skipped,
            RunStatus::Failed
        ])));
        assert!(!needs_attention(&with(&[
            RunStatus::Done,
            RunStatus::Failed
        ])));
        assert!(!needs_attention(&with(&[
            RunStatus::Running,
            RunStatus::Done
        ])));
        assert!(!needs_attention(&with(&[])));
    }

    #[test]
    fn kinds_per_trigger_type_and_invalid() {
        let v = view("f", "F", &["trigger.state", "trigger.schedule"]);
        assert_eq!(
            trigger_kinds(&v),
            BTreeSet::from([TriggerKind::State, TriggerKind::Schedule])
        );
        let all = view(
            "f",
            "F",
            &["trigger.sun", "trigger.signal", "trigger.state"],
        );
        assert_eq!(
            trigger_kinds(&all),
            BTreeSet::from([TriggerKind::Sun, TriggerKind::Signal, TriggerKind::State])
        );
        assert!(trigger_kinds(&invalid("x")).is_empty());
    }

    #[test]
    fn kinds_off_peak_is_its_own_kind() {
        let only = view("f", "F", &["trigger.off_peak"]);
        assert_eq!(trigger_kinds(&only), BTreeSet::from([TriggerKind::OffPeak]));
    }

    #[test]
    fn kinds_silent_is_its_own_kind() {
        let only = view("f", "F", &["trigger.silent"]);
        assert_eq!(trigger_kinds(&only), BTreeSet::from([TriggerKind::Silent]));
    }

    #[test]
    fn kinds_manual_card_is_manual() {
        let only = view("f", "F", &["trigger.manual"]);
        assert_eq!(trigger_kinds(&only), BTreeSet::from([TriggerKind::Manual]));
        let both = view("f", "F", &["trigger.state", "trigger.manual"]);
        assert_eq!(
            trigger_kinds(&both),
            BTreeSet::from([TriggerKind::State, TriggerKind::Manual])
        );
        assert!(trigger_kinds(&view("f", "F", &[])).is_empty());
    }

    #[test]
    fn rows_treat_unknown_area_as_no_room() {
        let areas = [area("salon", "Salon"), area(UNASSIGNED_AREA, "Unassigned")];
        let mut ghost = view("a", "A", &[]);
        ghost.definition.area = Some("ghost".into());
        let mut unassigned = view("b", "B", &[]);
        unassigned.definition.area = Some(UNASSIGNED_AREA.into());
        let mut known = view("c", "C", &[]);
        known.definition.area = Some("salon".into());
        let out = rows(
            Lang::En,
            &[ghost, unassigned, known],
            &areas,
            &BTreeMap::new(),
            &FlowLabelCatalog::default(),
        );
        assert_eq!(
            (out[0].room.clone(), out[0].room_name.clone()),
            (None, None)
        );
        assert_eq!(
            (out[1].room.clone(), out[1].room_name.clone()),
            (None, None)
        );
        assert_eq!(out[2].room.as_deref(), Some("salon"));
        assert_eq!(out[2].room_name.as_deref(), Some("Salon"));
        assert_eq!(out[2].trigger_text, "No trigger");
    }

    #[test]
    fn search_folds_case_and_accents() {
        let q = |query: &str| Filters {
            query: query.into(),
            ..Filters::default()
        };
        let by_room = row_in(view("a", "A", &[]), "entree", "Entrée");
        assert_eq!(filter(&[by_room], &q("ENTREE")).len(), 1);
        let by_name = row(view("b", "Lumière Salon", &[]));
        assert_eq!(filter(std::slice::from_ref(&by_name), &q("salon")).len(), 1);
        assert_eq!(
            filter(std::slice::from_ref(&by_name), &q("cuisine")).len(),
            0
        );
        let mut by_trigger = row(view("c", "C", &["trigger.sun"]));
        by_trigger.trigger_text = "At sunset".into();
        assert_eq!(filter(&[by_trigger], &q("SUNSET")).len(), 1);
        let mut v = view("d", "D", &[]);
        v.definition.description = Some("Ferme les volets".into());
        assert_eq!(filter(&[row(v)], &q("volets")).len(), 1);
    }

    #[test]
    fn filters_combine_status_and_triggers() {
        let mut off = view("off", "Off", &["trigger.sun"]);
        off.definition.enabled = false;
        let all = vec![
            row(failing_sun("fail_sun")),
            row(view("ok_sun", "Ok", &["trigger.sun"])),
            row(view("state", "State", &["trigger.state"])),
            row(invalid("bad")),
            row(off),
        ];
        let f = Filters {
            status: StatusFilter::Attention,
            triggers: BTreeSet::from([TriggerKind::Sun]),
            ..Filters::default()
        };
        assert_eq!(ids(&filter(&all, &f)), ["fail_sun"]);
        let attention = Filters {
            status: StatusFilter::Attention,
            ..Filters::default()
        };
        assert_eq!(ids(&filter(&all, &attention)), ["fail_sun", "bad"]);
        let on = Filters {
            status: StatusFilter::On,
            ..Filters::default()
        };
        assert_eq!(ids(&filter(&all, &on)), ["fail_sun", "ok_sun", "state"]);
        let off_only = Filters {
            status: StatusFilter::Off,
            ..Filters::default()
        };
        assert_eq!(ids(&filter(&all, &off_only)), ["off"]);
    }

    #[test]
    fn counts_follow_query_and_triggers_not_status() {
        let mut off = view("off", "Off", &["trigger.sun"]);
        off.definition.enabled = false;
        let all = vec![
            row(failing_sun("fail_sun")),
            row(view("state", "State", &["trigger.state"])),
            row(invalid("bad")),
            row(off),
        ];
        let c = status_counts(&all, "", &BTreeSet::new());
        assert_eq!((c.all, c.on, c.off, c.attention), (4, 2, 1, 2), "{c:?}");
        let sun = BTreeSet::from([TriggerKind::Sun]);
        let c = status_counts(&all, "", &sun);
        assert_eq!((c.all, c.on, c.off, c.attention), (2, 1, 1, 1));
        let c = status_counts(&all, "state", &BTreeSet::new());
        assert_eq!((c.all, c.on, c.off, c.attention), (1, 1, 0, 0));
    }

    #[test]
    fn sort_last_run_newest_first_never_run_last() {
        let mut a = view("a", "Alpha", &[]);
        a.runs = vec![run(RunStatus::Done, 10)];
        let mut b = view("b", "Bravo", &[]);
        b.runs = vec![run(RunStatus::Done, 50), run(RunStatus::Done, 5)];
        let never_z = view("z", "Zulu", &[]);
        let never_c = view("c", "Charlie", &[]);
        let mut tie = view("t", "Able", &[]);
        tie.runs = vec![run(RunStatus::Done, 10)];
        let mut list: Vec<ListRow> = [a, never_z, b, never_c, tie].into_iter().map(row).collect();
        sort_rows(&mut list, Sort::LastRun);
        assert_eq!(ids(&list), ["b", "t", "a", "c", "z"]);
        sort_rows(&mut list, Sort::Name);
        assert_eq!(ids(&list), ["t", "a", "b", "c", "z"]);
    }

    #[test]
    fn sections_follow_area_order_no_room_last() {
        let areas = [
            area("b", "Bedroom"),
            area("a", "Attic"),
            area("empty", "Empty"),
            area(UNASSIGNED_AREA, "Unassigned"),
        ];
        let list = vec![
            row_in(view("in_a", "A", &[]), "a", "Attic"),
            row(view("none", "N", &[])),
            row_in(view("in_b", "B", &[]), "b", "Bedroom"),
            row_in(view("in_b2", "B2", &[]), "b", "Bedroom"),
        ];
        let out = sections(Lang::En, list, &areas);
        let keys: Vec<&str> = out.iter().map(|s| s.key.as_str()).collect();
        assert_eq!(keys, ["b", "a", NO_ROOM_KEY]);
        assert_eq!(out[0].title, "Bedroom");
        assert_eq!(out[2].title, "No room");
        assert_eq!(ids(&out[0].rows), ["in_b", "in_b2"]);
    }

    #[test]
    fn section_flags_attention() {
        let list = vec![
            row_in(view("ok", "Ok", &[]), "a", "Attic"),
            row_in(invalid("bad"), "a", "Attic"),
            row_in(view("ok2", "Ok2", &[]), "b", "Bedroom"),
        ];
        let out = sections(Lang::En, list, &[area("a", "Attic"), area("b", "Bedroom")]);
        assert!(out[0].attention);
        assert!(!out[1].attention);
    }

    #[test]
    fn prefs_round_trip_and_decode_garbage_is_default() {
        assert_eq!(decode_prefs("nope"), FlowListPrefs::default());
        assert_eq!(decode_prefs(""), FlowListPrefs::default());
        let cards = decode_prefs(r#"{"view":"cards"}"#);
        assert_eq!(cards.view, ViewMode::Cards);
        assert_eq!(
            FlowListPrefs {
                view: ViewMode::List,
                ..cards.clone()
            },
            FlowListPrefs::default()
        );
        let p = FlowListPrefs {
            view: ViewMode::Cards,
            grouping: Grouping::None,
            sort: Sort::LastRun,
            status: StatusFilter::Attention,
            triggers: BTreeSet::from([TriggerKind::Sun, TriggerKind::Manual]),
            folded: BTreeSet::from(["salon".to_owned()]),
        };
        assert_eq!(decode_prefs(&encode_prefs(&p)), p);
    }
}
