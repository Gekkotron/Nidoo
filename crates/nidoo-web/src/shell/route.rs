//! Which sidebar entry is active for a path, and the breadcrumb trail of a page.

use crate::i18n::{AssistantText, Key, Lang, assistant_text, text};
use crate::screens::settings::SettingsPage;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Section {
    Overview,
    Room(String),
    Automations,
    Groups,
    Ambiances,
    Energy,
    Security,
    WakeUp,
    Climate,
    Alerts,
    Assistant,
    Settings,
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lookup<'a> {
    Dashboard(&'a str),
    Room(&'a str),
    Flow(&'a str),
    Group(&'a str),
    Ambiance(&'a str),
    Entity(&'a str),
    Alert(&'a str),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Crumb {
    pub label: String,
    pub href: Option<String>,
}

/// The path's segments, percent-decoded, ignoring any query, fragment and empty segments. Ids
/// are names ("Salle de bain"), and the address bar keeps them encoded.
fn segments(path: &str) -> Vec<String> {
    let path = path.split(['?', '#']).next().unwrap_or("");
    path.split('/')
        .filter(|s| !s.is_empty())
        .map(percent_decode)
        .collect()
}

/// `%XX` escapes back to their bytes; a segment that does not decode to UTF-8 stays as it is,
/// and so does a "%" that starts no escape.
fn percent_decode(segment: &str) -> String {
    let raw = segment.as_bytes();
    let hex = |b: u8| char::from(b).to_digit(16);
    let mut bytes = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        let escape = match raw.get(i..i + 3) {
            Some([b'%', hi, lo]) => hex(*hi).zip(hex(*lo)).map(|(hi, lo)| hi * 16 + lo),
            _ => None,
        };
        match escape {
            Some(byte) => {
                // Two hex digits: at most 255.
                bytes.push(u8::try_from(byte).unwrap_or(b'%'));
                i += 3;
            }
            None => {
                bytes.push(raw[i]);
                i += 1;
            }
        }
    }
    String::from_utf8(bytes).unwrap_or_else(|_| segment.to_owned())
}

fn is_detail_page(path: &str) -> bool {
    matches!(
        segments(path).first().map(String::as_str),
        Some("entity" | "history")
    )
}

/// The sidebar entry to highlight. Entity and history pages inherit the section of the page the
/// user came from (the latest non-entity/history entry of `back`).
pub fn section_for(path: &str, back: &[String]) -> Section {
    let owned = segments(path);
    let seg: Vec<&str> = owned.iter().map(String::as_str).collect();
    match seg.as_slice() {
        [] | ["dashboards", ..] => Section::Overview,
        ["rooms", id, ..] => Section::Room((*id).to_string()),
        ["flows", ..] => Section::Automations,
        ["groups", ..] | ["group", ..] => Section::Groups,
        ["ambiances", ..] => Section::Ambiances,
        ["energy", ..] => Section::Energy,
        ["security", ..] => Section::Security,
        ["wake-up", ..] => Section::WakeUp,
        ["climate", ..] => Section::Climate,
        ["weather", ..] | ["solar", ..] => Section::Overview,
        ["alerts", ..] => Section::Alerts,
        ["search", ..] => Section::Overview,
        ["assistant", ..] => Section::Assistant,
        ["settings", ..] | ["devices", ..] => Section::Settings,
        ["entity", ..] | ["history", ..] => back
            .iter()
            .rev()
            .find(|p| !is_detail_page(p))
            .map_or(Section::None, |p| section_for(p, &[])),
        _ => Section::None,
    }
}

/// The breadcrumb trail: always starts at "My home" and the last crumb has no link.
pub fn crumbs(path: &str, lang: Lang, name_of: impl Fn(Lookup) -> Option<String>) -> Vec<Crumb> {
    let t = |k| text(lang, k).to_string();
    let named = |l: Lookup, id: &str| name_of(l).unwrap_or_else(|| id.to_string());
    let link = |label: String, href: &str| Crumb {
        label,
        href: Some(href.to_string()),
    };
    let mut out = vec![link(t(Key::MyHome), "/")];
    let owned = segments(path);
    let seg: Vec<&str> = owned.iter().map(String::as_str).collect();
    let mut push = |label: String, href: String| out.push(link(label, &href));
    match seg.as_slice() {
        ["dashboards", "new"] => push(t(Key::NewDashboard), String::new()),
        ["dashboards", id, ..] => push(named(Lookup::Dashboard(id), id), String::new()),
        ["rooms", id, ..] => push(named(Lookup::Room(id), id), String::new()),
        ["flows"] => push(t(Key::Automations), String::new()),
        ["flows", rest @ ..] => {
            push(t(Key::Automations), "/flows".into());
            match rest {
                ["new"] => push(t(Key::NewFlow), String::new()),
                [id, ..] => push(named(Lookup::Flow(id), id), String::new()),
                [] => {}
            }
        }
        ["groups"] => push(t(Key::Groups), String::new()),
        ["groups", "new"] => {
            push(t(Key::Groups), "/groups".into());
            push(t(Key::NewGroup), String::new());
        }
        ["groups", id, "edit"] | ["group", id, "edit"] => {
            push(t(Key::Groups), "/groups".into());
            push(named(Lookup::Group(id), id), format!("/group/{id}"));
            push(t(Key::EditGroup), String::new());
        }
        ["groups", id, ..] | ["group", id, ..] => {
            push(t(Key::Groups), "/groups".into());
            push(named(Lookup::Group(id), id), String::new());
        }
        ["ambiances"] => push(t(Key::Ambiances), String::new()),
        ["ambiances", rest @ ..] => {
            push(t(Key::Ambiances), "/ambiances".into());
            match rest {
                ["new"] => push(t(Key::NewAmbiance), String::new()),
                ["about"] => push(t(Key::AmbiancesHowItWorks), String::new()),
                [id, "report", ..] => {
                    push(named(Lookup::Ambiance(id), id), format!("/ambiances/{id}"));
                    push(t(Key::AmbianceLastRun), String::new());
                }
                [id, ..] => push(named(Lookup::Ambiance(id), id), String::new()),
                [] => {}
            }
        }
        ["energy", ..] => push(t(Key::Energy), String::new()),
        ["security", ..] => push(t(Key::Security), String::new()),
        ["wake-up", ..] => push(t(Key::WakeUp), String::new()),
        ["climate", ..] => push(t(Key::Climate), String::new()),
        ["weather", ..] => push(t(Key::Weather), String::new()),
        ["solar", ..] => push(t(Key::Solar), String::new()),
        ["search"] => push(t(Key::Search), String::new()),
        ["assistant", ..] => push(
            assistant_text(lang, AssistantText::Title).to_string(),
            String::new(),
        ),
        ["alerts"] => push(t(Key::Alerts), String::new()),
        ["alerts", id, ..] => {
            push(t(Key::Alerts), "/alerts".into());
            push(
                name_of(Lookup::Alert(id)).unwrap_or_else(|| t(Key::Alert)),
                String::new(),
            );
        }
        ["settings"] => push(t(Key::Settings), String::new()),
        ["settings", slug, ..] => {
            push(t(Key::Settings), "/settings".into());
            if let Some(page) = SettingsPage::from_slug(slug) {
                push(t(page.key()), String::new());
            }
        }
        ["devices", ..] => push(t(Key::Devices), String::new()),
        ["entity", id, ..] => push(named(Lookup::Entity(id), id), String::new()),
        ["history", id, ..] => {
            push(named(Lookup::Entity(id), id), format!("/entity/{id}"));
            push(t(Key::History), String::new());
        }
        _ => {}
    }
    if let Some(last) = out.last_mut() {
        last.href = None;
    }
    out
}

#[cfg(test)]
mod tests;
