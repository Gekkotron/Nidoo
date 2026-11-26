//! The widget library in the side panel while editing a dashboard: a search box over the
//! widgets that can be added, grouped by kind, each row showing its icon, title, room and
//! device. Clicking a row builds a `Widget` with the smallest allowed span for its kind (a
//! big tile for several cameras) and calls `on_add`.

use std::collections::BTreeMap;

use leptos::ev;
use leptos::prelude::*;
use nidoo_api::{AlertChannel, AreaView, CameraInfo, EntityView};
use nidoo_core::dashboard::{
    DashboardHistoryRange, MAX_CAMERAS, Span, Widget, WidgetConfig, WidgetKind, allowed_spans,
};
use nidoo_core::model::Kind;

use super::energy_stat::ENERGY_METRICS;
use crate::cameras::{CameraList, load_cameras};
use crate::flow_edit::fold_accents;
use crate::format::{entity_title, kind_search_words, room_name};
use crate::house::House;
use crate::i18n::{DashText, Lang, dash_text, widget_text};
use crate::icons::{BOLT, HOUSE, PLAY, X_MARK, channel_icon, entity_icon};

/// A timestamp is unique enough: widgets are added by hand, one click at a time.
#[cfg(target_arch = "wasm32")]
fn new_widget_id() -> String {
    let n = js_sys::Date::now() as u64;
    format!("w_{:x}", n)
}

/// The span of a fresh widget: the smallest allowed one keeps it from crowding the grid (the
/// user resizes later), except a Camera widget with several cameras, which gets the big tile
/// that shows them side by side.
fn initial_span(kind: WidgetKind, config: &WidgetConfig) -> Span {
    match config {
        WidgetConfig::Cameras { cameras, .. } if cameras.len() >= 2 => Span::X2x2,
        _ => allowed_spans(kind).first().copied().unwrap_or(Span::X1x1),
    }
}

fn make_widget_with_id(id: String, kind: WidgetKind, config: WidgetConfig) -> Widget {
    let span = initial_span(kind, &config);
    Widget {
        id,
        kind,
        span,
        config,
        pos: None,
    }
}

#[cfg(target_arch = "wasm32")]
fn make_widget(kind: WidgetKind, config: WidgetConfig) -> Widget {
    make_widget_with_id(new_widget_id(), kind, config)
}

#[cfg(not(target_arch = "wasm32"))]
fn make_widget(kind: WidgetKind, config: WidgetConfig) -> Widget {
    // js_sys::Date panics off wasm, so native unit tests use a fixed id.
    make_widget_with_id("w_test".into(), kind, config)
}

/// The section of the library a row sits in, in display order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Section {
    Lights,
    Covers,
    Climates,
    Sensors,
    WaterHeaters,
    MediaPlayers,
    Locks,
    Cameras,
    Rooms,
    FlowButtons,
    Energy,
    HistoryCharts,
}

impl Section {
    fn header(self, lang: Lang) -> &'static str {
        dash_text(
            lang,
            match self {
                Self::Lights => DashText::SectionLights,
                Self::Covers => DashText::SectionCovers,
                Self::Climates => DashText::SectionClimate,
                Self::Sensors => DashText::SectionSensors,
                Self::WaterHeaters => DashText::SectionWaterHeaters,
                Self::MediaPlayers => DashText::SectionMediaPlayers,
                Self::Locks => DashText::SectionLocks,
                Self::Cameras => DashText::SectionCameras,
                Self::Rooms => DashText::SectionRoomGroups,
                Self::FlowButtons => DashText::SectionFlowButtons,
                Self::Energy => DashText::SectionEnergy,
                Self::HistoryCharts => DashText::SectionHistory,
            },
        )
    }

    /// The suffix of the section's colour modifier classes.
    fn suffix(self) -> &'static str {
        match self {
            Self::Lights => "lights",
            Self::Covers => "covers",
            Self::Climates => "climates",
            Self::Sensors => "sensors",
            Self::WaterHeaters => "water-heaters",
            Self::MediaPlayers => "media",
            Self::Locks => "locks",
            Self::Cameras => "cameras",
            Self::Rooms => "rooms",
            Self::FlowButtons => "flows",
            Self::Energy => "energy",
            Self::HistoryCharts => "history",
        }
    }
}

/// One widget the library can add.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub section: Section,
    pub title: String,
    /// The row's second line: an entity's room and device, or what the row does.
    pub sub: Option<String>,
    pub icon: &'static str,
    pub kind: WidgetKind,
    pub config: WidgetConfig,
}

/// `"{room} · {device}"`, skipping a missing part and a device that only repeats the title.
fn subline(entity: &EntityView, areas: &[AreaView], title: &str) -> Option<String> {
    let room = room_name(areas, &entity.area);
    let device = entity
        .device_name
        .clone()
        .filter(|d| !d.eq_ignore_ascii_case(title));
    let parts: Vec<String> = [room, device]
        .into_iter()
        .flatten()
        .filter(|s| !s.is_empty())
        .collect();
    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// Whether the hub records a chartable number for this kind (`nidoo_core::history::num_of`):
/// on/off is a 0/1 step, a cover its position, a climate or water heater its reading. Text,
/// alarm, vacuum, select and button states have none, and a generic entity only when its type
/// says so, which the web does not know.
fn charts(kind: Kind) -> bool {
    matches!(
        kind,
        Kind::Light
            | Kind::Switch
            | Kind::Cover
            | Kind::Climate
            | Kind::Sensor
            | Kind::BinarySensor
            | Kind::Number
            | Kind::WaterHeater
    )
}

/// What a widget of `kind` can be pointed at, as `(target id, label)`: the library's rows of
/// that kind, labelled "title · room · device". Camera widgets edit their list instead.
pub fn targets(
    lang: Lang,
    kind: WidgetKind,
    entities: &BTreeMap<String, EntityView>,
    areas: &[AreaView],
    flows: &[(String, String)],
) -> Vec<(String, String)> {
    entries(lang, "", entities, areas, flows, &[])
        .into_iter()
        .filter(|e| e.kind == kind)
        .filter_map(|e| {
            let id = target_id(&e.config)?.to_owned();
            let label = match e.sub {
                Some(sub) => format!("{} · {sub}", e.title),
                None => e.title,
            };
            Some((id, label))
        })
        .collect()
}

/// The one thing a widget shows: its entity, room, flow or energy metric. `None` for a
/// Camera widget, which shows a list.
pub fn target_id(config: &WidgetConfig) -> Option<&str> {
    match config {
        WidgetConfig::Entity { entity_id, .. } | WidgetConfig::History { entity_id, .. } => {
            Some(entity_id)
        }
        WidgetConfig::Area { area_id, .. } => Some(area_id),
        WidgetConfig::Flow { flow_id, .. } => Some(flow_id),
        WidgetConfig::Energy { metric, .. } => Some(metric),
        WidgetConfig::Cameras { .. } => None,
    }
}

/// Points the widget at `id`, keeping its title and range.
pub fn retarget(config: &mut WidgetConfig, id: String) {
    match config {
        WidgetConfig::Entity { entity_id, .. } | WidgetConfig::History { entity_id, .. } => {
            *entity_id = id
        }
        WidgetConfig::Area { area_id, .. } => *area_id = id,
        WidgetConfig::Flow { flow_id, .. } => *flow_id = id,
        WidgetConfig::Energy { metric, .. } => *metric = id,
        WidgetConfig::Cameras { .. } => {}
    }
}

/// What `query` offers to put on a dashboard, ordered by section, then title. A row matches
/// when its title or second line contains the query, ignoring case and accents. Each Frigate
/// camera is a row, plus "All cameras" (the first [`MAX_CAMERAS`]) when there are several.
pub fn entries(
    lang: Lang,
    query: &str,
    entities: &BTreeMap<String, EntityView>,
    areas: &[AreaView],
    flows: &[(String, String)],
    cameras: &[CameraInfo],
) -> Vec<Entry> {
    // Each row with the kind words that also find it ("cover", "volet"), for entity rows.
    let mut rows: Vec<(String, Entry, Option<[String; 2]>)> = Vec::new();
    // A row's fixed text in both languages, so "lights" finds a room group on the French UI too.
    let both = |k: DashText| {
        [
            dash_text(Lang::En, k).to_owned(),
            dash_text(Lang::Fr, k).to_owned(),
        ]
    };
    for e in entities.values().filter(|e| e.is_primary()) {
        let card = match e.kind {
            Kind::Light => Some((Section::Lights, WidgetKind::Light)),
            Kind::Cover => Some((Section::Covers, WidgetKind::Cover)),
            Kind::Climate => Some((Section::Climates, WidgetKind::Climate)),
            Kind::Sensor => Some((Section::Sensors, WidgetKind::Sensor)),
            Kind::WaterHeater => Some((Section::WaterHeaters, WidgetKind::WaterHeater)),
            Kind::MediaPlayer => Some((Section::MediaPlayers, WidgetKind::MediaPlayer)),
            Kind::Generic if e.component == "lock" => Some((Section::Locks, WidgetKind::Lock)),
            _ => None,
        };
        if card.is_none() && !charts(e.kind) {
            continue;
        }
        let title = entity_title(lang, e);
        let sub = subline(e, areas, &title);
        let icon = entity_icon(e);
        let words = kind_search_words(e);
        let entity_id = e.id.clone();
        if let Some((section, kind)) = card {
            rows.push((
                entity_id.clone(),
                Entry {
                    section,
                    title: title.clone(),
                    sub: sub.clone(),
                    icon,
                    kind,
                    config: WidgetConfig::Entity {
                        entity_id: entity_id.clone(),
                        title: None,
                    },
                },
                Some(words.clone()),
            ));
        }
        if charts(e.kind) {
            rows.push((
                entity_id.clone(),
                Entry {
                    section: Section::HistoryCharts,
                    title,
                    sub,
                    icon,
                    kind: WidgetKind::HistoryChart,
                    config: WidgetConfig::History {
                        entity_id,
                        range: DashboardHistoryRange::H24,
                    },
                },
                Some(words),
            ));
        }
    }
    for area in areas.iter().filter(|a| room_name(areas, &a.id).is_some()) {
        rows.push((
            area.id.clone(),
            Entry {
                section: Section::Rooms,
                title: area.name.clone(),
                sub: Some(dash_text(lang, DashText::RoomGroupSub).into()),
                icon: HOUSE,
                kind: WidgetKind::RoomGroup,
                config: WidgetConfig::Area {
                    area_id: area.id.clone(),
                    title: None,
                },
            },
            Some(both(DashText::RoomGroupSub)),
        ));
    }
    for (id, name) in flows {
        rows.push((
            id.clone(),
            Entry {
                section: Section::FlowButtons,
                title: name.clone(),
                sub: Some(dash_text(lang, DashText::FlowButtonSub).into()),
                icon: PLAY,
                kind: WidgetKind::FlowButton,
                config: WidgetConfig::Flow {
                    flow_id: id.clone(),
                    title: None,
                },
            },
            Some(both(DashText::FlowButtonSub)),
        ));
    }
    let camera_icon = channel_icon(AlertChannel::Camera);
    for camera in cameras {
        rows.push((
            camera.id.clone(),
            Entry {
                section: Section::Cameras,
                title: camera.name.clone(),
                sub: None,
                icon: camera_icon,
                kind: WidgetKind::Camera,
                config: WidgetConfig::Cameras {
                    cameras: vec![camera.id.clone()],
                    title: None,
                },
            },
            Some(both(DashText::CamerasLabel)),
        ));
    }
    if cameras.len() >= 2 {
        rows.push((
            String::new(),
            Entry {
                section: Section::Cameras,
                title: dash_text(lang, DashText::AllCameras).to_owned(),
                sub: None,
                icon: camera_icon,
                kind: WidgetKind::Camera,
                config: WidgetConfig::Cameras {
                    cameras: cameras
                        .iter()
                        .take(MAX_CAMERAS)
                        .map(|c| c.id.clone())
                        .collect(),
                    title: None,
                },
            },
            Some(both(DashText::CamerasLabel)),
        ));
    }
    for (id, label) in ENERGY_METRICS {
        rows.push((
            id.to_owned(),
            Entry {
                section: Section::Energy,
                title: widget_text(lang, label).to_owned(),
                sub: None,
                icon: BOLT,
                kind: WidgetKind::EnergyStat,
                config: WidgetConfig::Energy {
                    metric: id.to_owned(),
                    title: None,
                },
            },
            Some([
                widget_text(Lang::En, label).to_owned(),
                widget_text(Lang::Fr, label).to_owned(),
            ]),
        ));
    }

    let needle = fold_accents(query.trim());
    rows.retain(|(_, e, words)| {
        needle.is_empty()
            || fold_accents(&e.title).contains(&needle)
            || words
                .iter()
                .flatten()
                .any(|w| fold_accents(w).contains(&needle))
            || e.sub
                .as_deref()
                .is_some_and(|s| fold_accents(s).contains(&needle))
    });
    rows.sort_by_cached_key(|(id, e, _)| (e.section, e.title.to_lowercase(), id.clone()));
    rows.into_iter().map(|(_, e, _)| e).collect()
}

/// Every flow as `(id, name)`, for flow-button rows.
pub(super) fn flow_names(house: House) -> Vec<(String, String)> {
    house.flows.with(|flows| {
        flows
            .iter()
            .map(|f| (f.definition.id.clone(), f.definition.name.clone()))
            .collect()
    })
}

/// The library. Clicking a row, or Enter on the highlighted one while the search box has the
/// focus, calls `on_add`; the library stays as it is so several widgets can go in a row. Esc
/// in the search box clears it.
#[component]
pub fn WidgetLibrary(
    on_add: Callback<Widget>,
    /// Closes the drawer the library sits in; `None` where it is always shown.
    #[prop(optional)]
    on_close: Option<Callback<()>>,
) -> impl IntoView {
    let house = expect_context::<House>();
    let lang = crate::i18n::use_lang();
    let query = RwSignal::new(String::new());
    let highlight = RwSignal::new(0_usize);
    // Refreshed on open, so a camera just added to Frigate shows up.
    let cameras = use_context::<CameraList>().unwrap_or_default();
    load_cameras(cameras);

    let results = Memo::new(move |_| {
        let query = query.get();
        let flows = flow_names(house);
        house.entities.with(|entities| {
            house.areas.with(|areas| {
                cameras
                    .0
                    .with(|list| entries(lang(), &query, entities, areas, &flows, &list.cameras))
            })
        })
    });

    // A new query starts again from the top.
    Effect::watch(move || query.get(), move |_, _, _| highlight.set(0), false);

    let add = move |entry: Entry| on_add.run(make_widget(entry.kind, entry.config));
    let on_key = move |ev: ev::KeyboardEvent| {
        let last = results.with_untracked(Vec::len).saturating_sub(1);
        match ev.key().to_ascii_lowercase().as_str() {
            "arrowdown" => {
                ev.prevent_default();
                highlight.update(|i| *i = (*i + 1).min(last));
            }
            "arrowup" => {
                ev.prevent_default();
                highlight.update(|i| *i = i.saturating_sub(1));
            }
            "enter" => {
                ev.prevent_default();
                if let Some(entry) =
                    results.with_untracked(|r| r.get(highlight.get_untracked()).cloned())
                {
                    add(entry);
                }
            }
            "escape" => query.set(String::new()),
            _ => {}
        }
    };

    let list = move || {
        let mut last = None;
        let lang = lang();
        results
            .get()
            .into_iter()
            .enumerate()
            .map(|(i, entry)| {
                let suffix = entry.section.suffix();
                let head = (last != Some(entry.section)).then(|| {
                    let class = format!("flow-library__header dashboard-library__header--{suffix}");
                    view! { <li class=class role="presentation">{entry.section.header(lang)}</li> }
                });
                last = Some(entry.section);
                let class = format!(
                    "flow-library__item dashboard-library__item dashboard-library__item--{suffix}"
                );
                let title = entry.title.clone();
                let sub = entry.sub.clone();
                let icon = entry.icon;
                view! {
                    {head}
                    <li
                        class=class
                        class:flow-library__item--on=move || highlight.get() == i
                        role="option"
                        aria-selected=move || (highlight.get() == i).to_string()
                        on:pointermove=move |_| highlight.set(i)
                        on:click=move |_| add(entry.clone())
                    >
                        <span class="flow-library__icon" inner_html=icon></span>
                        <span class="flow-library__text">
                            <span class="flow-library__title">{title}</span>
                            {sub.map(|s| view! { <span class="flow-library__sub">{s}</span> })}
                        </span>
                    </li>
                }
            })
            .collect_view()
    };

    view! {
        <section class="flow-library dashboard-library" aria-label=move || dash_text(lang(), DashText::AddWidget)>
            <header class="flow-drawer__header">
                <h2 class="flow-drawer__title">{move || dash_text(lang(), DashText::AddWidget)}</h2>
                {on_close.map(|close| view! {
                    <button
                        type="button"
                        class="btn btn--round flow-drawer__close dashboard-library__close"
                        aria-label=move || dash_text(lang(), DashText::Close)
                        on:click=move |_| close.run(())
                        inner_html=X_MARK
                    ></button>
                })}
            </header>
            <input
                class="field flow-library__search"
                type="search"
                placeholder=move || dash_text(lang(), DashText::SearchPlaceholder)
                aria-label=move || dash_text(lang(), DashText::SearchWidgets)
                prop:value=move || query.get()
                on:input=move |ev| query.set(event_target_value(&ev))
                on:keydown=on_key
            />
            <ul class="flow-library__list" role="listbox">
                {list}
                <Show when=move || results.with(Vec::is_empty)>
                    <li class="flow-library__empty">{move || dash_text(lang(), DashText::NothingMatches)}</li>
                </Show>
            </ul>
        </section>
    }
}

#[cfg(test)]
mod tests;
