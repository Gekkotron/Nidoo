//! Side sheets for edit mode: per-widget config and per-dashboard settings.

use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::hooks::use_navigate;
use nidoo_api::HOUSE_ID;
use nidoo_core::dashboard::{
    Dashboard, DashboardHistoryRange, MAX_CAMERAS, Span, Widget, WidgetConfig, WidgetKind,
    allowed_spans,
};

use crate::api;
use crate::cameras::CameraList;
use crate::components::confirm::{ConfirmBar, Confirming};
use crate::components::toast::Toasts;
use crate::dashboards::camera_widget::camera_name;
use crate::dashboards::edit::EditState;
use crate::dashboards::history_widget::chart_range;
use crate::dashboards::library::{flow_names, retarget, target_id, targets};
use crate::house::House;
use crate::i18n::history::range_name;
use crate::i18n::{DashText, Key, dash_text, delete_question, text, use_lang, widget_sheet_title};

/// Per-widget sheet: what the widget shows, title override, size, a history chart's range, a
/// Camera widget's ordered camera list and delete.
#[component]
pub fn WidgetConfigSheet(state: EditState, index: usize, on_close: Callback<()>) -> impl IntoView {
    let lang = use_lang();
    // Kind and span are tracked separately so typing in the title does not rebuild the
    // inputs (which would drop focus on every keystroke).
    let kind = Memo::new(move |_| state.draft.with(|d| d.widgets.get(index).map(|w| w.kind)));
    let span = Memo::new(move |_| state.draft.with(|d| d.widgets.get(index).map(|w| w.span)));
    let range = Memo::new(move |_| {
        state
            .draft
            .with(|d| match d.widgets.get(index).map(|w| &w.config) {
                Some(WidgetConfig::History { range, .. }) => Some(*range),
                _ => None,
            })
    });
    let cameras = Memo::new(move |_| {
        state
            .draft
            .with(|d| match d.widgets.get(index).map(|w| &w.config) {
                Some(WidgetConfig::Cameras { cameras, .. }) => Some(cameras.clone()),
                _ => None,
            })
    });
    let initial_title = state
        .draft
        .with_untracked(|d| d.widgets.get(index).and_then(title_of))
        .unwrap_or_default();

    // The index goes stale once the widget is removed; close rather than render nothing.
    Effect::new(move |_| {
        if kind.get().is_none() {
            on_close.run(());
        }
    });

    view! {
        <aside class="widget-config-sheet">
            <h3>{move || widget_sheet_title(lang(), kind.get().unwrap_or(WidgetKind::Light))}</h3>
            <TargetField state=state index=index kind=kind />
            <label class="sheet-form__field">
                <span class="sheet-form__label">{move || dash_text(lang(), DashText::Title)}</span>
                <input
                    type="text"
                    class="field"
                    value=initial_title
                    on:input=move |ev| {
                        let v = event_target_value(&ev);
                        state.draft.update(|d| {
                            if let Some(w) = d.widgets.get_mut(index) {
                                set_title(w, if v.is_empty() { None } else { Some(v) });
                            }
                        });
                    }
                />
            </label>
            <label class="sheet-form__field">
                <span class="sheet-form__label">{move || dash_text(lang(), DashText::Size)}</span>
                <select
                    class="select"
                    on:change=move |ev| {
                        if let Some(s) = parse_span(&event_target_value(&ev)) {
                            state.resize(index, s);
                        }
                    }
                >
                    {move || {
                        let cur = span.get();
                        let k = kind.get().unwrap_or(WidgetKind::Light);
                        allowed_spans(k)
                            .iter()
                            .map(|s| {
                                let label = s.as_str();
                                view! { <option value=label selected={Some(*s) == cur}>{label}</option> }
                            })
                            .collect::<Vec<_>>()
                    }}
                </select>
            </label>
            <Show when=move || range.get().is_some()>
                <label class="sheet-form__field">
                    <span class="sheet-form__label">{move || dash_text(lang(), DashText::Range)}</span>
                    <select
                        class="select"
                        on:change=move |ev| {
                            let v = event_target_value(&ev);
                            state.draft.update(|d| {
                                if let (Some(w), Some(r)) = (d.widgets.get_mut(index), parse_range(&v)) {
                                    set_range(w, r);
                                }
                            });
                        }
                    >
                        {move || {
                            let cur = range.get();
                            HISTORY_RANGES
                                .iter()
                                .map(|r| {
                                    let r = *r;
                                    view! {
                                        <option value=range_wire(r) selected={Some(r) == cur}>
                                            {range_name(lang(), chart_range(r))}
                                        </option>
                                    }
                                })
                                .collect::<Vec<_>>()
                        }}
                    </select>
                </label>
            </Show>
            <Show when=move || cameras.with(Option::is_some)>
                <CameraListField state=state index=index cameras=cameras />
            </Show>
            <button
                type="button"
                class="btn btn--danger"
                on:click=move |_| {
                    state.draft.update(|d| remove_widget(d, index));
                    on_close.run(());
                }
            >
                {move || dash_text(lang(), DashText::DeleteWidget)}
            </button>
            <button type="button" class="btn" on:click=move |_| on_close.run(())>
                {move || dash_text(lang(), DashText::Close)}
            </button>
        </aside>
    }
}

/// What the widget shows: the entities, rooms, flows or energy metrics a widget of its kind can
/// show, as the library lists them. A target that is gone (a deleted device) stays listed under
/// its id so the select does not jump to another one. Camera widgets have their own list.
#[component]
fn TargetField(state: EditState, index: usize, kind: Memo<Option<WidgetKind>>) -> impl IntoView {
    let lang = use_lang();
    let house = expect_context::<House>();
    let current = Memo::new(move |_| {
        state.draft.with(|d| {
            d.widgets
                .get(index)
                .and_then(|w| target_id(&w.config).map(str::to_owned))
        })
    });
    let options = Memo::new(move |_| {
        let Some(kind) = kind.get() else {
            return Vec::new();
        };
        let flows = flow_names(house);
        let mut list = house.entities.with(|entities| {
            house
                .areas
                .with(|areas| targets(lang(), kind, entities, areas, &flows))
        });
        if let Some(cur) = current.get()
            && !list.iter().any(|(id, _)| *id == cur)
        {
            list.insert(0, (cur.clone(), cur));
        }
        list
    });
    view! {
        <Show when=move || current.with(Option::is_some)>
            <label class="sheet-form__field">
                <span class="sheet-form__label">{move || dash_text(lang(), DashText::Shows)}</span>
                <select
                    class="select"
                    on:change=move |ev| {
                        let id = event_target_value(&ev);
                        state.draft.update(|d| {
                            if let Some(w) = d.widgets.get_mut(index) {
                                retarget(&mut w.config, id);
                            }
                        });
                    }
                >
                    {move || {
                        let cur = current.get();
                        options
                            .get()
                            .into_iter()
                            .map(|(id, label)| {
                                let selected = cur.as_deref() == Some(id.as_str());
                                view! { <option value=id selected=selected>{label}</option> }
                            })
                            .collect_view()
                    }}
                </select>
            </label>
        </Show>
    }
}

/// A Camera widget's cameras in display order: each row moves up or down or goes (not the last
/// one), and a select adds a camera the hub lists that is not there yet, up to [`MAX_CAMERAS`].
#[component]
fn CameraListField(
    state: EditState,
    index: usize,
    cameras: Memo<Option<Vec<String>>>,
) -> impl IntoView {
    let lang = use_lang();
    let list = use_context::<CameraList>().unwrap_or_default();
    let edit = move |change: &dyn Fn(&mut Vec<String>)| {
        state.draft.update(|d| {
            if let Some(Widget {
                config: WidgetConfig::Cameras { cameras, .. },
                ..
            }) = d.widgets.get_mut(index)
            {
                change(cameras);
            }
        });
    };
    let ids = move || cameras.get().unwrap_or_default();
    let rows = move || {
        let ids = ids();
        let last = ids.len() == 1;
        ids.iter()
            .map(|id| {
                let name = camera_name(list, id);
                let (up, down, gone) = (id.clone(), id.clone(), id.clone());
                view! {
                    <li class="camera-list__row">
                        <span class="camera-list__name">{name}</span>
                        <button
                            type="button"
                            class="btn btn--compact"
                            aria-label=move || dash_text(lang(), DashText::MoveUp)
                            on:click=move |_| edit(&|c| *c = shifted_order(c.clone(), &up, -1))
                        >
                            "▲"
                        </button>
                        <button
                            type="button"
                            class="btn btn--compact"
                            aria-label=move || dash_text(lang(), DashText::MoveDown)
                            on:click=move |_| edit(&|c| *c = shifted_order(c.clone(), &down, 1))
                        >
                            "▼"
                        </button>
                        <button
                            type="button"
                            class="btn btn--compact"
                            aria-label=move || dash_text(lang(), DashText::RemoveCamera)
                            disabled=last
                            on:click=move |_| edit(&|c| {
                                remove_camera(c, &gone);
                            })
                        >
                            "✕"
                        </button>
                    </li>
                }
            })
            .collect_view()
    };
    let addable = Memo::new(move |_| {
        let ids = cameras.get().unwrap_or_default();
        if ids.len() >= MAX_CAMERAS {
            return Vec::new();
        }
        list.0.with(|v| {
            v.cameras
                .iter()
                .filter(|c| !ids.contains(&c.id))
                .map(|c| (c.id.clone(), c.name.clone()))
                .collect::<Vec<_>>()
        })
    });
    view! {
        <div class="sheet-form__field">
            <span class="sheet-form__label">{move || dash_text(lang(), DashText::CamerasLabel)}</span>
            <ul class="camera-list">{rows}</ul>
            <Show when=move || addable.with(|a| !a.is_empty())>
                <select
                    class="select"
                    aria-label=move || dash_text(lang(), DashText::AddCamera)
                    prop:value=""
                    on:change=move |ev| {
                        let id = event_target_value(&ev);
                        if !id.is_empty() {
                            edit(&|c| {
                                if c.len() < MAX_CAMERAS && !c.contains(&id) {
                                    c.push(id.clone());
                                }
                            });
                        }
                        // Back to the prompt, or the next camera would show as already picked
                        // and choosing it would fire no change.
                        event_target::<web_sys::HtmlSelectElement>(&ev).set_value("");
                    }
                >
                    <option value="" selected=true disabled=true>
                        {move || dash_text(lang(), DashText::AddCamera)}
                    </option>
                    {move || {
                        addable
                            .get()
                            .into_iter()
                            .map(|(id, name)| view! { <option value=id>{name}</option> })
                            .collect_view()
                    }}
                </select>
            </Show>
        </div>
    }
}

/// Removes camera `id` from `ids` unless it is the last one; whether it went.
fn remove_camera(ids: &mut Vec<String>, id: &str) -> bool {
    if ids.len() <= 1 {
        return false;
    }
    let before = ids.len();
    ids.retain(|i| i != id);
    ids.len() != before
}

/// History widgets carry no title in the model, so the override is a no-op for them.
pub(super) fn title_of(w: &Widget) -> Option<String> {
    match &w.config {
        WidgetConfig::Entity { title, .. }
        | WidgetConfig::Area { title, .. }
        | WidgetConfig::Flow { title, .. }
        | WidgetConfig::Energy { title, .. }
        | WidgetConfig::Cameras { title, .. } => title.clone(),
        WidgetConfig::History { .. } => None,
    }
}

fn set_title(w: &mut Widget, new: Option<String>) {
    match &mut w.config {
        WidgetConfig::Entity { title, .. }
        | WidgetConfig::Area { title, .. }
        | WidgetConfig::Flow { title, .. }
        | WidgetConfig::Energy { title, .. }
        | WidgetConfig::Cameras { title, .. } => *title = new,
        WidgetConfig::History { .. } => {}
    }
}

fn parse_span(s: &str) -> Option<Span> {
    match s {
        "1x1" => Some(Span::X1x1),
        "2x1" => Some(Span::X2x1),
        "2x2" => Some(Span::X2x2),
        "1x2" => Some(Span::X1x2),
        "4x2" => Some(Span::X4x2),
        "6x3" => Some(Span::X6x3),
        _ => None,
    }
}

/// The ranges a history widget offers, shortest first.
const HISTORY_RANGES: [DashboardHistoryRange; 5] = [
    DashboardHistoryRange::H1,
    DashboardHistoryRange::H6,
    DashboardHistoryRange::H24,
    DashboardHistoryRange::D7,
    DashboardHistoryRange::D30,
];

/// The serde wire name of a widget range, used as the option value.
fn range_wire(r: DashboardHistoryRange) -> &'static str {
    match r {
        DashboardHistoryRange::H1 => "1h",
        DashboardHistoryRange::H6 => "6h",
        DashboardHistoryRange::H24 => "24h",
        DashboardHistoryRange::D7 => "7d",
        DashboardHistoryRange::D30 => "30d",
    }
}

fn parse_range(s: &str) -> Option<DashboardHistoryRange> {
    HISTORY_RANGES.into_iter().find(|r| range_wire(*r) == s)
}

fn set_range(w: &mut Widget, new: DashboardHistoryRange) {
    if let WidgetConfig::History { range, .. } = &mut w.config {
        *range = new;
    }
}

fn remove_widget(d: &mut Dashboard, index: usize) {
    if index < d.widgets.len() {
        d.widgets.remove(index);
    }
}

/// Swaps `id` with its neighbour (`delta` -1 up, +1 down); unchanged at the ends or when
/// `id` is absent.
fn shifted_order(mut ids: Vec<String>, id: &str, delta: isize) -> Vec<String> {
    if let Some(pos) = ids.iter().position(|i| i == id) {
        let target = pos as isize + delta;
        if target >= 0 && (target as usize) < ids.len() {
            ids.swap(pos, target as usize);
        }
    }
    ids
}

/// Persists a reorder and mirrors it into the in-memory list, since the hub does not
/// push dashboard structure. House stays first; it is not part of the reorder payload.
fn move_dashboard(house: House, toasts: Toasts, id: String, delta: isize) {
    let ids: Vec<String> = house
        .dashboards
        .get_untracked()
        .iter()
        .map(|dv| dv.dashboard.id.clone())
        .filter(|i| i != HOUSE_ID)
        .collect();
    let ids = shifted_order(ids, &id, delta);
    spawn_local(async move {
        match api::reorder_dashboards(&ids).await {
            Ok(_) => house.dashboards.update(|v| {
                v.sort_by_key(|dv| {
                    ids.iter()
                        .position(|i| *i == dv.dashboard.id)
                        .map_or(0, |p| p + 1)
                });
            }),
            Err(e) => toasts.push(e.error),
        }
    });
}

/// Per-dashboard sheet: name, header, delete and move up/down in the sidebar list.
#[component]
pub fn DashboardSettingsSheet(state: EditState, on_close: Callback<()>) -> impl IntoView {
    let house = expect_context::<House>();
    let toasts = expect_context::<Toasts>();
    let confirming = expect_context::<Confirming>();
    let lang = use_lang();
    let navigate = use_navigate();
    let initial_name = state.draft.with_untracked(|d| d.name.clone());
    let show_header = move || state.draft.with(|d| d.show_header);

    view! {
        <aside class="dashboard-settings-sheet">
            <h3>{move || dash_text(lang(), DashText::DashboardSettings)}</h3>
            <label class="sheet-form__field">
                <span class="sheet-form__label">{move || dash_text(lang(), DashText::Name)}</span>
                <input
                    type="text"
                    class="field"
                    value=initial_name
                    on:input=move |ev| {
                        let v = event_target_value(&ev);
                        state.draft.update(|d| d.name = v);
                    }
                />
            </label>
            <label class="sheet-form__flag">
                <span class="sheet-form__label">{move || dash_text(lang(), DashText::ShowHeader)}</span>
                <button
                    type="button"
                    role="switch"
                    class="switch"
                    class:switch--on=show_header
                    aria-checked=move || show_header().to_string()
                    on:click=move |_| state.draft.update(|d| d.show_header = !d.show_header)
                >
                    <span class="switch__knob"></span>
                </button>
            </label>
            <div class="dashboard-settings-sheet__reorder">
                <button
                    type="button"
                    class="btn"
                    on:click=move |_| {
                        move_dashboard(house, toasts, state.draft.get_untracked().id, -1)
                    }
                >
                    {move || text(lang(), Key::MoveUp)}
                </button>
                <button
                    type="button"
                    class="btn"
                    on:click=move |_| {
                        move_dashboard(house, toasts, state.draft.get_untracked().id, 1)
                    }
                >
                    {move || text(lang(), Key::MoveDown)}
                </button>
            </div>
            <Show
                when=move || confirming.is_asking(&state.draft.with(|d| d.id.clone()))
                fallback=move || {
                    view! {
                        <button
                            type="button"
                            class="btn btn--danger"
                            on:click=move |_| confirming.ask(&state.draft.get_untracked().id)
                        >
                            {move || dash_text(lang(), DashText::DeleteDashboard)}
                        </button>
                    }
                }
            >
                <ConfirmBar
                    question=Signal::derive(move || {
                        delete_question(lang(), &state.draft.with(|d| d.name.clone()))
                    })
                    yes=Signal::derive(move || text(lang(), Key::Delete).to_owned())
                    on_yes=Callback::new({
                        let navigate = navigate.clone();
                        move |()| {
                            confirming.clear();
                            let id = state.draft.get_untracked().id;
                            let navigate = navigate.clone();
                            spawn_local(async move {
                                match api::delete_dashboard(&id).await {
                                    Ok(_) => {
                                        house
                                            .dashboards
                                            .update(|v| v.retain(|dv| dv.dashboard.id != id));
                                        navigate(
                                            &format!("/dashboards/{HOUSE_ID}"),
                                            Default::default(),
                                        );
                                    }
                                    Err(e) => toasts.push(e.error),
                                }
                            });
                        }
                    })
                    on_cancel=Callback::new(move |()| confirming.clear())
                />
            </Show>
            <button type="button" class="btn" on:click=move |_| on_close.run(())>
                {move || dash_text(lang(), DashText::Close)}
            </button>
        </aside>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn widget(id: &str) -> Widget {
        Widget {
            id: id.into(),
            kind: WidgetKind::Light,
            span: Span::X1x1,
            config: WidgetConfig::Entity {
                entity_id: "light.a".into(),
                title: None,
            },
            pos: None,
        }
    }

    #[test]
    fn a_camera_moves_down_the_list() {
        let ids = vec!["front_door".to_owned(), "garden".to_owned()];
        assert_eq!(
            shifted_order(ids, "front_door", 1),
            ["garden", "front_door"]
        );
    }

    #[test]
    fn the_last_camera_cannot_be_removed() {
        let mut ids = vec!["front_door".to_owned(), "garden".to_owned()];
        assert!(remove_camera(&mut ids, "front_door"));
        assert_eq!(ids, ["garden"]);
        assert!(!remove_camera(&mut ids, "garden"));
        assert_eq!(ids, ["garden"]);
        assert!(!remove_camera(&mut ids, "nope"));
    }

    #[test]
    fn config_sheet_delete_removes_widget_from_draft() {
        let s = EditState::new(
            Dashboard {
                id: "d".into(),
                name: "D".into(),
                order: 0,
                widgets: vec![widget("a"), widget("b")],
                show_header: false,
            },
            true,
        );
        s.draft.update(|d| remove_widget(d, 0));
        let d = s.draft.get();
        assert_eq!(d.widgets.len(), 1);
        assert_eq!(d.widgets[0].id, "b");
    }

    #[test]
    fn range_wire_names_round_trip() {
        for r in HISTORY_RANGES {
            assert_eq!(parse_range(range_wire(r)), Some(r));
        }
        assert_eq!(parse_range("1y"), None);
        assert_eq!(parse_range(""), None);
    }

    #[test]
    fn set_range_changes_only_history_widgets() {
        let mut h = Widget {
            id: "h".into(),
            kind: WidgetKind::HistoryChart,
            span: Span::X2x1,
            config: WidgetConfig::History {
                entity_id: "sensor.a".into(),
                range: DashboardHistoryRange::H24,
            },
            pos: None,
        };
        set_range(&mut h, DashboardHistoryRange::H6);
        assert_eq!(
            h.config,
            WidgetConfig::History {
                entity_id: "sensor.a".into(),
                range: DashboardHistoryRange::H6,
            }
        );
        let mut l = widget("l");
        set_range(&mut l, DashboardHistoryRange::H6);
        assert_eq!(l.config, widget("l").config);
    }

    #[test]
    fn move_up_calculates_adjacent_swap() {
        let ids = vec!["a".to_string(), "b".into(), "c".into()];
        assert_eq!(shifted_order(ids.clone(), "b", -1), vec!["b", "a", "c"]);
        assert_eq!(shifted_order(ids.clone(), "a", -1), vec!["a", "b", "c"]);
        assert_eq!(shifted_order(ids, "b", 1), vec!["a", "c", "b"]);
    }
}
