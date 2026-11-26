//! Ambiances on the web side: pure helpers the screens build on (Home row order, ids for new
//! moods, the four starter examples and one readable sentence per action). Tested natively.

use std::collections::{BTreeMap, BTreeSet};

pub mod about;
pub mod action_forms;
pub mod actions;
pub mod editor;
pub mod page;
pub mod report;
pub mod row;

use nidoo_api::{
    Ambiance, AmbianceAction, AmbianceView, ApplyReport, AreaView, Category, CategoryAction,
    EntityView, FlowDo, Target,
};
use nidoo_core::flow::{CommandParams, Node, NodeKind};

use crate::flow_edit::fold_accents;
use crate::flow_text::{Ctx, sentence};
use crate::i18n::{self, Lang};

const ID_MAX: usize = 48;
/// The Home row shows at most this many ambiances.
const HOME_MAX: usize = 4;

/// The house an action sentence is read against.
pub struct HouseView<'a> {
    pub entities: &'a BTreeMap<String, EntityView>,
    pub areas: &'a [AreaView],
    /// `(id, name)` of every flow.
    pub flows: &'a [(String, String)],
    /// `(id, name)` of every climate zone.
    pub zones: &'a [(String, String)],
}

/// The ambiances the Home row shows: valid and pinned, by `(order, id)`, at most four.
pub fn pinned_for_home(list: &[AmbianceView]) -> Vec<Ambiance> {
    let mut pinned: Vec<Ambiance> = list
        .iter()
        .filter_map(|v| v.ambiance.clone())
        .filter(|a| a.pinned)
        .collect();
    pinned.sort_by(|a, b| (a.order, &a.id).cmp(&(b.order, &b.id)));
    pinned.truncate(HOME_MAX);
    pinned
}

/// An id from a name: lowercase, accents folded, `-`, `.` and whitespace as `_`, anything else
/// outside `[a-z0-9_]` dropped, at most 48 characters, `fallback` when nothing is left, then
/// `_2`, `_3`, … appended on collision with `taken`.
pub fn slug_id(name: &str, taken: &BTreeSet<String>, fallback: &str) -> String {
    let mut slug: String = fold_accents(name)
        .chars()
        .filter_map(|c| match c {
            '-' | '.' => Some('_'),
            c if c.is_whitespace() => Some('_'),
            c if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' => Some(c),
            _ => None,
        })
        .collect();
    slug.truncate(ID_MAX);
    if slug.is_empty() {
        slug = fallback.to_owned();
    }
    if !taken.contains(&slug) {
        return slug;
    }
    for n in 2.. {
        let suffix = format!("_{n}");
        let mut base = slug.clone();
        base.truncate(ID_MAX - suffix.len());
        let candidate = format!("{base}{suffix}");
        if !taken.contains(&candidate) {
            return candidate;
        }
    }
    unreachable!("an unbounded range always yields a free id")
}

/// The order a new ambiance takes: after every readable one.
pub fn next_order(list: &[AmbianceView]) -> u32 {
    list.iter()
        .filter_map(|v| v.ambiance.as_ref())
        .map(|a| a.order)
        .max()
        .map_or(1, |m| m + 1)
}

/// How many entities across the report's actions failed.
pub fn failed_count(report: &ApplyReport) -> usize {
    report.actions.iter().map(|a| a.failed.len()).sum()
}

fn category(what: Category, to: Target, brightness: Option<u8>) -> AmbianceAction {
    AmbianceAction::Category(CategoryAction {
        what,
        to,
        brightness,
        area: None,
        except: Vec::new(),
    })
}

/// The four starter ambiances, pinned, with consecutive orders from `first_order` and ids free
/// against `taken` (and against each other).
pub fn examples(lang: Lang, taken: &BTreeSet<String>, first_order: u32) -> Vec<Ambiance> {
    let actions: [(&str, Vec<AmbianceAction>); 4] = [
        (
            "moon",
            vec![
                category(Category::Lights, Target::Off, None),
                category(Category::Plugs, Target::Off, None),
                category(Category::Covers, Target::Close, None),
            ],
        ),
        (
            "door_out",
            vec![
                category(Category::Lights, Target::Off, None),
                category(Category::Plugs, Target::Off, None),
            ],
        ),
        (
            "sofa",
            vec![category(Category::Lights, Target::On, Some(30))],
        ),
        ("book", Vec::new()),
    ];
    let mut used = taken.clone();
    actions
        .into_iter()
        .enumerate()
        .map(|(i, (icon, actions))| {
            let (name, subtitle) = i18n::ambiance_example(lang, i);
            let id = slug_id(name, &used, "ambiance");
            used.insert(id.clone());
            Ambiance {
                id,
                name: name.to_owned(),
                subtitle: Some(subtitle.to_owned()),
                icon: Some(icon.to_owned()),
                pinned: true,
                order: first_order + i as u32,
                actions,
                extra: Default::default(),
            }
        })
        .collect()
}

/// `name` for a known thing, otherwise its id followed by "(missing)".
fn or_missing(lang: Lang, found: Option<&str>, id: &str) -> String {
    match found {
        Some(name) => name.to_owned(),
        None => format!("{id} {}", i18n::text(lang, i18n::Key::AmbianceMissing)),
    }
}

fn area_name(lang: Lang, house: &HouseView, id: &str) -> String {
    let found = house
        .areas
        .iter()
        .find(|a| a.id == id)
        .map(|a| a.name.as_str());
    or_missing(lang, found, id)
}

fn lookup<'a>(list: &'a [(String, String)], id: &str) -> Option<&'a str> {
    list.iter()
        .find(|(i, _)| i == id)
        .map(|(_, name)| name.as_str())
}

/// One readable line for an action: "Every light in the house: off", "Desk lamp: turn off".
/// Device commands reuse the flow card sentence, in the same language.
pub fn action_sentence(action: &AmbianceAction, house: &HouseView, lang: Lang) -> String {
    match action {
        AmbianceAction::Category(c) => {
            let what = match c.what {
                Category::Lights => "lights",
                Category::Plugs => "plugs",
                Category::Covers => "covers",
            };
            let to = match c.to {
                Target::On => "on",
                Target::Off => "off",
                Target::Open => "open",
                Target::Close => "close",
            };
            let area = c.area.as_deref().map(|id| area_name(lang, house, id));
            i18n::category_sentence(lang, what, to, c.brightness, area.as_deref())
        }
        AmbianceAction::Device(d) => device_sentence(&d.entity, &d.command, house, lang),
        AmbianceAction::Climate(c) => {
            let zone = c
                .zone
                .as_deref()
                .map(|id| or_missing(lang, lookup(house.zones, id), id));
            let setpoint = c.setpoint.as_ref().map(|v| match v {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Number(n) => n.to_string(),
                other => other.to_string(),
            });
            i18n::climate_sentence(lang, zone.as_deref(), c.clear, setpoint.as_deref(), c.hours)
        }
        AmbianceAction::Flow(f) => {
            let name = or_missing(lang, lookup(house.flows, &f.flow), &f.flow);
            let do_ = match f.action {
                FlowDo::Enable => "enable",
                FlowDo::Disable => "disable",
                FlowDo::Run => "run",
            };
            i18n::flow_sentence(lang, &name, do_)
        }
    }
}

/// "Desk lamp: turn off": the flow card sentence with the entity lifted to the front.
fn device_sentence(
    entity: &str,
    command: &nidoo_core::model::Command,
    house: &HouseView,
    lang: Lang,
) -> String {
    let node = Node {
        id: "a".into(),
        name: None,
        position: None,
        kind: NodeKind::Command(CommandParams {
            entity: entity.to_owned(),
            command: command.clone(),
            value_from: None,
            extra: Default::default(),
        }),
        extra: Default::default(),
    };
    let ctx = Ctx {
        entities: house.entities,
        areas: house.areas,
        flows: house.flows,
        labels: &crate::flow_text::NO_LABELS,
        lang,
    };
    let full = sentence(&node, &ctx);
    let (title, label) = match house.entities.get(entity) {
        Some(e) => {
            let t = crate::format::entity_title(lang, e);
            (t.clone(), t)
        }
        None => (entity.to_owned(), or_missing(lang, None, entity)),
    };
    let rest = full.replacen(&title, " ", 1);
    // Split on plain spaces only: a no-break space in "21 °C" must survive.
    let rest = rest
        .split(' ')
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let mut chars = rest.chars();
    let rest = match chars.next() {
        Some(c) => c.to_lowercase().chain(chars).collect(),
        None => rest,
    };
    let colon = if lang == Lang::Fr { " : " } else { ": " };
    format!("{label}{colon}{rest}")
}

#[cfg(test)]
mod tests;
