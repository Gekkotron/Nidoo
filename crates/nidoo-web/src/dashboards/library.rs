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
use crate::icons::{BOLT, HOUSE, PLAY, channel_icon, entity_icon};

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
pub fn WidgetLibrary(on_add: Callback<Widget>) -> impl IntoView {
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
mod tests {
    use nidoo_core::model::{Capabilities, EntityCategory, EntityState, UiOverride};

    use super::*;

    fn ent(id: &str, name: &str, kind: Kind) -> EntityView {
        EntityView {
            id: id.into(),
            device_id: None,
            name: name.into(),
            device_name: None,
            label: None,
            kind,
            component: kind.component().to_string(),
            protocol_kind: kind,
            device_class: None,
            unit: None,
            icon: None,
            category: EntityCategory::Primary,
            enabled_by_default: true,
            area: "salon".into(),
            part_of: None,
            capabilities: Capabilities::default(),
            ui: UiOverride::default(),
            state: EntityState::default(),
        }
    }

    fn house(list: Vec<EntityView>) -> BTreeMap<String, EntityView> {
        list.into_iter().map(|e| (e.id.clone(), e)).collect()
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

    fn run(query: &str, entities: &BTreeMap<String, EntityView>) -> Vec<Entry> {
        entries(
            Lang::En,
            query,
            entities,
            &[area("salon", "Salon")],
            &[],
            &[],
        )
    }

    fn of(entries: &[Entry], section: Section) -> Vec<&Entry> {
        entries.iter().filter(|e| e.section == section).collect()
    }

    #[test]
    fn hidden_and_diagnostic_entities_are_left_out() {
        let mut hidden = ent("l2", "Hidden", Kind::Light);
        hidden.ui.hidden = true;
        let mut diag = ent("l3", "Diag", Kind::Light);
        diag.category = EntityCategory::Diagnostic;
        let entities = house(vec![ent("l1", "Lamp", Kind::Light), hidden, diag]);
        let got = run("", &entities);
        assert_eq!(of(&got, Section::Lights).len(), 1);
    }

    #[test]
    fn sub_line_is_room_then_device() {
        let mut light = ent("l1", "Lamp", Kind::Light);
        light.device_name = Some("Hue bulb".into());
        let mut twin = ent("l2", "Light", Kind::Light);
        twin.device_name = Some("light".into());
        let entities = house(vec![light, twin]);
        let got = run("", &entities);
        let subs: Vec<_> = of(&got, Section::Lights)
            .iter()
            .map(|e| e.sub.as_deref())
            .collect();
        assert_eq!(subs, [Some("Salon · Hue bulb"), Some("Salon")]);
    }

    #[test]
    fn search_ignores_accents_and_matches_the_room() {
        let entities = house(vec![
            ent("l1", "Lampe évier", Kind::Light),
            ent("l2", "Other", Kind::Light),
        ]);
        let got = run("EVIER", &entities);
        assert_eq!(of(&got, Section::Lights).len(), 1);
        let got = run("  salon ", &entities);
        assert_eq!(of(&got, Section::Lights).len(), 2);
    }

    #[test]
    fn the_kind_word_finds_entities_in_either_language() {
        let entities = house(vec![
            ent("c1", "Shutter", Kind::Cover),
            ent("l1", "Lamp", Kind::Light),
        ]);
        for query in ["volet", "cover"] {
            let got = run(query, &entities);
            assert_eq!(of(&got, Section::Covers).len(), 1, "{query}");
            assert!(of(&got, Section::Lights).is_empty(), "{query}");
        }
    }

    #[test]
    fn english_words_still_find_the_rows_on_the_french_ui() {
        let flows = [("f1".to_string(), "Night".to_string())];
        let areas = [area("salon", "Salon")];
        let entities = BTreeMap::new();
        let find = |q: &str| entries(Lang::Fr, q, &entities, &areas, &flows, &[]);
        for (query, section) in [
            ("lights", Section::Rooms),
            ("runs the flow", Section::FlowButtons),
            ("today", Section::Energy),
            ("éclairages", Section::Rooms),
            ("lance", Section::FlowButtons),
            ("aujourd", Section::Energy),
        ] {
            assert!(find(query).iter().any(|e| e.section == section), "{query}");
        }
    }

    #[test]
    fn sections_come_in_display_order() {
        let mut lock = ent("k1", "Front door", Kind::Generic);
        lock.component = "lock".into();
        let entities = house(vec![
            ent("s1", "Temp", Kind::Sensor),
            ent("c1", "Shutter", Kind::Cover),
            ent("l1", "Lamp", Kind::Light),
            lock,
        ]);
        let got = entries(
            Lang::En,
            "",
            &entities,
            &[area("salon", "Salon")],
            &[("f1".into(), "Night".into())],
            &[],
        );
        let mut sections: Vec<_> = got.iter().map(|e| e.section).collect();
        sections.dedup();
        assert_eq!(
            sections,
            [
                Section::Lights,
                Section::Covers,
                Section::Sensors,
                Section::Locks,
                Section::Rooms,
                Section::FlowButtons,
                Section::Energy,
                Section::HistoryCharts,
            ]
        );
    }

    fn cams(ids: &[&str]) -> Vec<CameraInfo> {
        ids.iter()
            .map(|id| CameraInfo {
                id: (*id).into(),
                name: nidoo_frigate::pretty(id),
            })
            .collect()
    }

    fn with_cameras(cameras: &[CameraInfo]) -> Vec<Entry> {
        entries(Lang::En, "", &BTreeMap::new(), &[], &[], cameras)
    }

    fn camera_ids(entry: &Entry) -> Vec<String> {
        match &entry.config {
            WidgetConfig::Cameras { cameras, .. } => cameras.clone(),
            other => panic!("not a camera config: {other:?}"),
        }
    }

    #[test]
    fn cameras_section_lists_each_camera_and_all() {
        let got = with_cameras(&cams(&["front_door", "garden"]));
        let rows = of(&got, Section::Cameras);
        assert_eq!(rows.len(), 3);
        assert!(rows.iter().all(|e| e.kind == WidgetKind::Camera));
        let all = rows
            .iter()
            .find(|e| e.title == "All cameras")
            .expect("an All cameras row");
        assert_eq!(camera_ids(all), ["front_door", "garden"]);
        let one = rows.iter().find(|e| e.title == "Garden").expect("Garden");
        assert_eq!(camera_ids(one), ["garden"]);
    }

    #[test]
    fn one_camera_has_no_all_row() {
        let got = with_cameras(&cams(&["front_door"]));
        let rows = of(&got, Section::Cameras);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, "Front Door");
    }

    #[test]
    fn no_cameras_no_section() {
        assert!(of(&with_cameras(&[]), Section::Cameras).is_empty());
    }

    #[test]
    fn all_cameras_keeps_the_first_four() {
        let got = with_cameras(&cams(&["a", "b", "c", "d", "e"]));
        let all = of(&got, Section::Cameras)
            .into_iter()
            .find(|e| e.title == "All cameras")
            .expect("an All cameras row");
        assert_eq!(camera_ids(all), ["a", "b", "c", "d"]);
    }

    #[test]
    fn initial_span_is_big_for_several_cameras() {
        let two = WidgetConfig::Cameras {
            cameras: vec!["a".into(), "b".into()],
            title: None,
        };
        let one = WidgetConfig::Cameras {
            cameras: vec!["a".into()],
            title: None,
        };
        let entity = WidgetConfig::Entity {
            entity_id: "light.a".into(),
            title: None,
        };
        assert_eq!(initial_span(WidgetKind::Camera, &two), Span::X2x2);
        assert_eq!(initial_span(WidgetKind::Camera, &one), Span::X2x1);
        assert_eq!(initial_span(WidgetKind::Light, &entity), Span::X1x1);
    }

    #[test]
    fn locks_get_their_own_section() {
        let mut lock = ent("lock.front", "Front door", Kind::Generic);
        lock.component = "lock".into();
        let mut siren = ent("siren.x", "Siren", Kind::Generic);
        siren.component = "siren".into();
        let got = run("", &house(vec![lock, siren]));
        let locks = of(&got, Section::Locks);
        assert_eq!(locks.len(), 1);
        assert_eq!(locks[0].kind, WidgetKind::Lock);
        assert!(matches!(
            &locks[0].config,
            WidgetConfig::Entity { entity_id, .. } if entity_id == "lock.front"
        ));
    }

    #[test]
    fn no_lock_no_lock_section() {
        let got = run("", &house(vec![ent("l1", "Lamp", Kind::Light)]));
        assert!(of(&got, Section::Locks).is_empty());
    }

    #[test]
    fn a_sensor_is_offered_as_a_sensor_and_as_a_chart() {
        let entities = house(vec![ent("s1", "Temp", Kind::Sensor)]);
        let got = run("", &entities);
        let sensor = of(&got, Section::Sensors);
        let chart = of(&got, Section::HistoryCharts);
        assert_eq!(sensor[0].kind, WidgetKind::Sensor);
        assert_eq!(
            sensor[0].config,
            WidgetConfig::Entity {
                entity_id: "s1".into(),
                title: None
            }
        );
        assert_eq!(chart[0].kind, WidgetKind::HistoryChart);
        assert_eq!(
            chart[0].config,
            WidgetConfig::History {
                entity_id: "s1".into(),
                range: DashboardHistoryRange::H24
            }
        );
    }

    #[test]
    fn any_entity_with_a_chartable_history_is_offered_as_a_chart() {
        let entities = house(vec![
            ent("l1", "Lamp", Kind::Light),
            ent("sw1", "Plug", Kind::Switch),
            ent("b1", "Door", Kind::BinarySensor),
            ent("n1", "Level", Kind::Number),
            ent("a1", "Alarm", Kind::AlarmPanel),
            ent("se1", "Mode", Kind::Select),
            ent("v1", "Robot", Kind::Vacuum),
        ]);
        let got = run("", &entities);
        let mut charted: Vec<_> = of(&got, Section::HistoryCharts)
            .iter()
            .filter_map(|e| target_id(&e.config).map(str::to_owned))
            .collect();
        charted.sort();
        assert_eq!(charted, ["b1", "l1", "n1", "sw1"]);
        // A switch has no card of its own, only the chart.
        assert!(got.iter().all(|e| e.section == Section::HistoryCharts
            || target_id(&e.config) != Some("sw1")));
    }

    #[test]
    fn targets_list_the_rows_of_one_kind() {
        let entities = house(vec![
            ent("l1", "Lamp", Kind::Light),
            ent("l2", "Desk", Kind::Light),
            ent("s1", "Temp", Kind::Sensor),
        ]);
        let areas = [area("salon", "Salon")];
        let flows = [("f1".to_owned(), "Night".to_owned())];
        let lights = targets(Lang::En, WidgetKind::Light, &entities, &areas, &flows);
        assert_eq!(
            lights,
            [
                ("l2".to_owned(), "Desk · Salon".to_owned()),
                ("l1".to_owned(), "Lamp · Salon".to_owned()),
            ]
        );
        let charts = targets(
            Lang::En,
            WidgetKind::HistoryChart,
            &entities,
            &areas,
            &flows,
        );
        assert_eq!(charts.len(), 3);
        let buttons = targets(Lang::En, WidgetKind::FlowButton, &entities, &areas, &flows);
        assert_eq!(buttons[0].0, "f1");
        let rooms = targets(Lang::En, WidgetKind::RoomGroup, &entities, &areas, &flows);
        assert_eq!(rooms[0].0, "salon");
        assert!(targets(Lang::En, WidgetKind::Camera, &entities, &areas, &flows).is_empty());
    }

    #[test]
    fn retarget_keeps_the_title_and_the_range() {
        let mut entity = WidgetConfig::Entity {
            entity_id: "l1".into(),
            title: Some("Hall".into()),
        };
        retarget(&mut entity, "l2".into());
        assert_eq!(
            entity,
            WidgetConfig::Entity {
                entity_id: "l2".into(),
                title: Some("Hall".into())
            }
        );
        let mut chart = WidgetConfig::History {
            entity_id: "s1".into(),
            range: DashboardHistoryRange::D7,
        };
        retarget(&mut chart, "s2".into());
        assert_eq!(
            chart,
            WidgetConfig::History {
                entity_id: "s2".into(),
                range: DashboardHistoryRange::D7
            }
        );
        let mut cams = WidgetConfig::Cameras {
            cameras: vec!["a".into()],
            title: None,
        };
        retarget(&mut cams, "b".into());
        assert_eq!(target_id(&cams), None);
        assert!(matches!(cams, WidgetConfig::Cameras { cameras, .. } if cameras == ["a"]));
    }

    #[test]
    fn empty_sections_have_no_rows() {
        let got = entries(Lang::En, "", &BTreeMap::new(), &[], &[], &[]);
        assert!(got.iter().all(|e| e.section == Section::Energy));
        assert_eq!(got.len(), ENERGY_METRICS.len());
    }

    #[test]
    fn adding_a_light_builds_entity_config_with_smallest_allowed_span() {
        let w = make_widget(
            WidgetKind::Light,
            WidgetConfig::Entity {
                entity_id: "light.a".into(),
                title: None,
            },
        );
        assert_eq!(w.kind, WidgetKind::Light);
        assert_eq!(w.span, Span::X1x1);
        assert_eq!(
            w.config,
            WidgetConfig::Entity {
                entity_id: "light.a".into(),
                title: None
            }
        );
    }

    #[test]
    fn adding_a_history_chart_defaults_to_2x1_and_24h_range() {
        let w = make_widget(
            WidgetKind::HistoryChart,
            WidgetConfig::History {
                entity_id: "x".into(),
                range: DashboardHistoryRange::H24,
            },
        );
        assert_eq!(w.span, Span::X2x1);
        assert!(matches!(
            w.config,
            WidgetConfig::History {
                range: DashboardHistoryRange::H24,
                ..
            }
        ));
    }
}
