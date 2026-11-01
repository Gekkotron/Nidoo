//! The ambiance editor: name, subtitle, icon, Show on Home and the list of actions, with Save,
//! Try it and Delete. `/ambiances/new` starts blank, `/ambiances/:id` edits the stored one; the
//! edit starts from the stored `Ambiance` and replaces only the fields the form owns, so fields
//! this page does not know survive.

use std::collections::{BTreeMap, BTreeSet};

use leptos::prelude::*;
use leptos_router::NavigateOptions;
use leptos_router::hooks::{use_navigate, use_params_map};
use nidoo_api::{AMBIANCE_ICONS, Ambiance, AmbianceAction, AreaView, EntityView};

use super::action_forms::{
    AddAction, AutomationForm, CategoryForm, DeviceForm, HeatingForm, Row, Rows, stale_excepts,
};
use super::actions::{apply_and_toast, upsert};
use super::{HouseView, action_sentence, next_order, slug_id};
use crate::api::ambiances::{delete_ambiance, put_ambiance};
use crate::api::climate::get_climate;
use crate::components::confirm::ConfirmBar;
use crate::components::toast::Toasts;
use crate::house::House;
use crate::i18n::{self, Key, Lang, use_text};
use crate::icons::{CHEVRON_DOWN, CHEVRON_UP, ambiance_icon};
use crate::nav::BackLink;

/// Routes under `/ambiances/` that an id must not equal, or the mood could not be opened.
const STATIC_SEGMENTS: [&str; 2] = ["new", "about"];

/// New moods are pinned while fewer than this many are.
const HOME_MAX: usize = 4;

/// The id of a new ambiance: its name as a slug, free against `taken` and the static routes.
pub fn new_ambiance_id(name: &str, taken: &BTreeSet<String>) -> String {
    let mut taken = taken.clone();
    taken.extend(STATIC_SEGMENTS.map(str::to_owned));
    slug_id(name, &taken, "ambiance")
}

/// True when `action` points at something the house no longer has. A climate zone counts only
/// once the zones are known (`zones` is `Some`).
fn is_missing(
    action: &AmbianceAction,
    entities: &BTreeMap<String, EntityView>,
    areas: &[AreaView],
    flows: &[(String, String)],
    zones: Option<&[(String, String)]>,
) -> bool {
    match action {
        AmbianceAction::Category(c) => {
            c.area
                .as_deref()
                .is_some_and(|id| !areas.iter().any(|a| a.id == id))
                || !stale_excepts(c, entities).is_empty()
        }
        AmbianceAction::Device(d) => !entities.contains_key(&d.entity),
        AmbianceAction::Climate(c) => match (c.zone.as_deref(), zones) {
            (Some(id), Some(zones)) => !zones.iter().any(|(z, _)| z == id),
            _ => false,
        },
        AmbianceAction::Flow(f) => !flows.iter().any(|(id, _)| id == &f.flow),
    }
}

/// Leaves the editor for the list without leaving the editor in the history: Back from the list
/// must not reopen a form that would send the user straight back.
fn leave(navigate: &impl Fn(&str, NavigateOptions)) {
    navigate(
        "/ambiances",
        NavigateOptions {
            replace: true,
            ..Default::default()
        },
    );
}

fn blank() -> Ambiance {
    Ambiance {
        id: String::new(),
        name: String::new(),
        subtitle: None,
        icon: None,
        pinned: false,
        order: 0,
        actions: Vec::new(),
        extra: Default::default(),
    }
}

#[component]
pub fn AmbianceEditorScreen() -> impl IntoView {
    let t = use_text();
    let house = expect_context::<House>();
    let toasts = expect_context::<Toasts>();
    let lang = use_context::<RwSignal<Lang>>().unwrap_or_else(|| RwSignal::new(Lang::En));
    let navigate = use_navigate();
    let params = use_params_map();
    let edit_id: Option<String> = params.read_untracked().get("id").filter(|s| !s.is_empty());
    let is_new = edit_id.is_none();

    let name = RwSignal::new(String::new());
    let subtitle = RwSignal::new(String::new());
    let icon = RwSignal::new(None::<String>);
    let pinned = RwSignal::new(false);
    let pin_touched = RwSignal::new(false);
    // A new mood is pinned while the Home row has room; that needs the list, so it waits for it
    // (unless the user already chose).
    if is_new {
        Effect::new(move |_| {
            if house.ambiances_loaded.get() && !pin_touched.get_untracked() {
                pinned.set(house.ambiances.with_untracked(|list| {
                    list.iter()
                        .filter_map(|v| v.ambiance.as_ref())
                        .filter(|a| a.pinned)
                        .count()
                        < HOME_MAX
                }));
            }
        });
    }
    let rows: Rows = RwSignal::new(Vec::new());
    let counter = RwSignal::new(0u64);
    let expanded = RwSignal::new(None::<u64>);
    let saving = RwSignal::new(false);
    let trying = RwSignal::new(false);
    let confirming = RwSignal::new(false);
    // True once an existing mood has filled the form; a new one starts filled.
    let inited = RwSignal::new(is_new);
    let climate_on = RwSignal::new(false);
    let zones = RwSignal::new(Vec::<(String, String)>::new());
    let zones_known = RwSignal::new(false);

    let fresh_key = move || {
        let key = counter.get_untracked();
        counter.set(key + 1);
        key
    };

    leptos::task::spawn_local(async move {
        if let Ok(climate) = get_climate().await
            && climate.enabled
        {
            zones.try_set(
                climate
                    .zones
                    .iter()
                    .map(|z| (z.zone.clone(), z.name.clone()))
                    .collect(),
            );
            zones_known.try_set(true);
            climate_on.try_set(true);
        }
    });

    // The stored version, live: the hub may push while the form is open.
    let stored = {
        let id = edit_id.clone();
        Memo::new(move |_| {
            let id = id.as_ref()?;
            house.ambiances.with(|list| {
                list.iter()
                    .find(|v| &v.id == id)
                    .and_then(|v| v.ambiance.clone())
            })
        })
    };

    // Fills the form from the stored mood once it is there; leaves when the list has loaded and
    // the mood is not in it (or is unreadable).
    if !is_new {
        let navigate = navigate.clone();
        Effect::new(move |_| {
            let found = stored.get();
            let loaded = house.ambiances_loaded.get();
            if inited.get_untracked() {
                return;
            }
            match found {
                Some(a) => {
                    name.set(a.name.clone());
                    subtitle.set(a.subtitle.clone().unwrap_or_default());
                    icon.set(a.icon.clone());
                    pinned.set(a.pinned);
                    rows.set(
                        a.actions
                            .iter()
                            .map(|action| Row {
                                key: fresh_key(),
                                action: action.clone(),
                            })
                            .collect(),
                    );
                    inited.set(true);
                }
                None if loaded => leave(&navigate),
                None => {}
            }
        });
    }

    // The mood the form describes: `base` with the edited fields replaced.
    let build = move |mut base: Ambiance| {
        base.name = name.get();
        let sub = subtitle.get();
        base.subtitle = (!sub.trim().is_empty()).then_some(sub);
        base.icon = icon.get();
        base.pinned = pinned.get();
        base.actions = rows.with(|list| list.iter().map(|r| r.action.clone()).collect());
        base
    };
    let dirty = Memo::new(move |_| {
        if !inited.get() {
            return true;
        }
        match stored.get() {
            Some(base) => build(base.clone()) != base,
            None => true,
        }
    });

    let save = {
        let navigate = navigate.clone();
        let edit_id = edit_id.clone();
        move |_| {
            let mut ambiance = build(stored.get_untracked().unwrap_or_else(|| Ambiance {
                id: edit_id.clone().unwrap_or_default(),
                ..blank()
            }));
            ambiance.name = ambiance.name.trim().to_owned();
            if is_new {
                house.ambiances.with_untracked(|list| {
                    let taken = list.iter().map(|v| v.id.clone()).collect();
                    ambiance.id = new_ambiance_id(&ambiance.name, &taken);
                    ambiance.order = next_order(list);
                });
            }
            saving.set(true);
            let navigate = navigate.clone();
            leptos::task::spawn_local(async move {
                match put_ambiance(&ambiance).await {
                    Ok(view) => {
                        upsert(house, view);
                        leave(&navigate);
                    }
                    Err(e) => toasts.push(e.error),
                }
                saving.try_set(false);
            });
        }
    };
    let try_it = move |_| {
        let Some(a) = stored.get_untracked() else {
            return;
        };
        trying.set(true);
        leptos::task::spawn_local(async move {
            apply_and_toast(house, a.id, a.name, toasts, lang.get_untracked()).await;
            trying.try_set(false);
        });
    };
    let delete = {
        let navigate = navigate.clone();
        let id = edit_id.clone().unwrap_or_default();
        move |()| {
            confirming.set(false);
            let (navigate, id) = (navigate.clone(), id.clone());
            leptos::task::spawn_local(async move {
                match delete_ambiance(&id).await {
                    Ok(()) => {
                        house.ambiances.update(|list| list.retain(|v| v.id != id));
                        leave(&navigate);
                    }
                    Err(e) => toasts.push(e.error),
                }
            });
        }
    };

    let add = Callback::new(move |action: AmbianceAction| {
        let key = fresh_key();
        rows.update(|list| list.push(Row { key, action }));
        expanded.set(Some(key));
    });
    let step = move |key: u64, up: bool| {
        rows.update(|list| {
            let Some(i) = list.iter().position(|r| r.key == key) else {
                return;
            };
            let j = if up { i.checked_sub(1) } else { Some(i + 1) };
            if let Some(j) = j.filter(|j| *j < list.len()) {
                list.swap(i, j);
            }
        });
    };
    let remove = move |key: u64| {
        rows.update(|list| list.retain(|r| r.key != key));
        if expanded.get_untracked() == Some(key) {
            expanded.set(None);
        }
    };

    let row_view = move |row: Row| {
        let key = row.key;
        let action_of =
            move || rows.with(|list| list.iter().find(|r| r.key == key).map(|r| r.action.clone()));
        // The sentence and the missing flag are read against the live house.
        let read = move |f: &dyn Fn(&AmbianceAction, &HouseView, Lang) -> String| {
            let action = action_of()?;
            let flows: Vec<(String, String)> = house.flows.with(|l| {
                l.iter()
                    .map(|f| (f.definition.id.clone(), f.definition.name.clone()))
                    .collect()
            });
            let zone_list = zones.get();
            Some(house.entities.with(|entities| {
                house.areas.with(|areas| {
                    let view = HouseView {
                        entities,
                        areas,
                        flows: &flows,
                        zones: &zone_list,
                    };
                    f(&action, &view, lang.get())
                })
            }))
        };
        let sentence = move || read(&|a, v, l| action_sentence(a, v, l)).unwrap_or_default();
        let missing = move || {
            let Some(action) = action_of() else {
                return false;
            };
            let flows: Vec<(String, String)> = house.flows.with(|l| {
                l.iter()
                    .map(|f| (f.definition.id.clone(), f.definition.name.clone()))
                    .collect()
            });
            let zone_list = zones.get();
            house.entities.with(|entities| {
                house.areas.with(|areas| {
                    is_missing(
                        &action,
                        entities,
                        areas,
                        &flows,
                        zones_known.get().then_some(zone_list.as_slice()),
                    )
                })
            })
        };
        let open = move || expanded.get() == Some(key);
        let first = move || rows.with(|l| l.first().is_some_and(|r| r.key == key));
        let last = move || rows.with(|l| l.last().is_some_and(|r| r.key == key));
        let form = match row.action {
            AmbianceAction::Category(_) => view! { <CategoryForm rows=rows key=key /> }.into_any(),
            AmbianceAction::Device(_) => view! { <DeviceForm rows=rows key=key /> }.into_any(),
            AmbianceAction::Climate(_) => {
                view! { <HeatingForm rows=rows key=key zones=zones /> }.into_any()
            }
            AmbianceAction::Flow(_) => view! { <AutomationForm rows=rows key=key /> }.into_any(),
        };
        view! {
            <li class="ambiance-action" class:ambiance-action--missing=missing>
                <div class="ambiance-action__head">
                    <span class="ambiance-action__sentence">{sentence}</span>
                    <button
                        type="button"
                        class="btn btn--round ambiance-action__up"
                        aria-label=move || t(Key::MoveUp)
                        title=move || t(Key::MoveUp)
                        disabled=first
                        inner_html=CHEVRON_UP
                        on:click=move |_| step(key, true)
                    ></button>
                    <button
                        type="button"
                        class="btn btn--round ambiance-action__down"
                        aria-label=move || t(Key::MoveDown)
                        title=move || t(Key::MoveDown)
                        disabled=last
                        inner_html=CHEVRON_DOWN
                        on:click=move |_| step(key, false)
                    ></button>
                    <button
                        type="button"
                        class="btn btn--round ambiance-action__remove"
                        aria-label=move || t(Key::Delete)
                        title=move || t(Key::Delete)
                        on:click=move |_| remove(key)
                    >
                        "✕"
                    </button>
                    <button
                        type="button"
                        class="btn btn--round ambiance-action__toggle"
                        aria-label=move || t(Key::Edit)
                        title=move || t(Key::Edit)
                        aria-expanded=move || open().to_string()
                        on:click=move |_| expanded.set(if open() { None } else { Some(key) })
                    >
                        {move || if open() { "−" } else { "+" }}
                    </button>
                </div>
                <div class="ambiance-action__form" hidden=move || !open()>
                    {form}
                </div>
            </li>
        }
    };

    let ask_delete = move || {
        let question = i18n::ambiance_delete_question(lang.get(), &name.get_untracked());
        view! {
            <ConfirmBar
                question=question
                yes=i18n::text(lang.get_untracked(), Key::Delete)
                on_yes=Callback::new(delete.clone())
                on_cancel=Callback::new(move |()| confirming.set(false))
            />
        }
    };

    view! {
        <div class="sheet ambiances ambiance-editor">
            <header class="sheet__header">
                <BackLink />
                <h1 class="sheet__title">
                    {move || t(if is_new { Key::NewAmbiance } else { Key::Ambiances })}
                </h1>
            </header>
            <section class="sheet__panel">
                <label class="field">
                    <span class="field__caption">{move || t(Key::AmbName)}</span>
                    <input
                        class="field__input ambiance-name"
                        type="text"
                        maxlength="60"
                        required
                        prop:value=move || name.get()
                        on:input=move |ev| name.set(event_target_value(&ev))
                    />
                </label>
                <label class="field">
                    <span class="field__caption">{move || t(Key::AmbSubtitle)}</span>
                    <input
                        class="field__input ambiance-subtitle"
                        type="text"
                        maxlength="80"
                        prop:value=move || subtitle.get()
                        on:input=move |ev| subtitle.set(event_target_value(&ev))
                    />
                </label>
                <fieldset class="ambiance-icons">
                    <legend class="field__caption">{move || t(Key::AmbIcon)}</legend>
                    {AMBIANCE_ICONS
                        .iter()
                        .map(|&icon_name| {
                            view! {
                                <label class="ambiance-icon">
                                    <input
                                        type="radio"
                                        name="ambiance-icon"
                                        aria-label=icon_name
                                        prop:checked=move || {
                                            icon.with(|i| i.as_deref() == Some(icon_name))
                                        }
                                        on:change=move |_| icon.set(Some(icon_name.to_owned()))
                                    />
                                    <span
                                        class="ambiance-icon__glyph"
                                        inner_html=ambiance_icon(Some(icon_name))
                                    ></span>
                                </label>
                            }
                        })
                        .collect_view()}
                </fieldset>
                <div class="ambiance-pin">
                    <span>{move || t(Key::AmbianceShowOnHome)}</span>
                    <button
                        type="button"
                        role="switch"
                        class="switch ambiance-pin__switch"
                        class:switch--on=move || pinned.get()
                        aria-checked=move || pinned.get().to_string()
                        aria-label=move || t(Key::AmbianceShowOnHome)
                        on:click=move |_| {
                            pin_touched.set(true);
                            pinned.update(|p| *p = !*p);
                        }
                    >
                        <span class="switch__knob"></span>
                    </button>
                </div>
            </section>
            <section class="sheet__panel">
                <h2 class="sheet__heading">{move || t(Key::AmbActions)}</h2>
                <ul class="ambiance-actions">
                    <For each=move || rows.get() key=|r| r.key children=row_view />
                </ul>
                <AddAction heating=Signal::from(climate_on) on_add=add />
            </section>
            <div class="sheet__actions ambiance-editor__actions">
                <button
                    type="button"
                    class="btn btn--primary ambiance-save"
                    disabled=move || {
                        saving.get() || name.with(|n| n.trim().is_empty())
                            || (is_new && !house.ambiances_loaded.get())
                    }
                    on:click=save
                >
                    {move || t(Key::AmbSave)}
                </button>
                <button
                    type="button"
                    class="btn ambiance-try"
                    aria-busy=move || trying.get().then_some("true")
                    disabled=move || dirty.get() || trying.get()
                    title=move || if dirty.get() { t(Key::AmbSaveFirst) } else { "" }
                    on:click=try_it
                >
                    {move || t(Key::AmbTryIt)}
                </button>
                {(!is_new)
                    .then(|| {
                        view! {
                            {move || if confirming.get() {
                                ask_delete().into_any()
                            } else {
                                view! {
                                    <button
                                        type="button"
                                        class="btn btn--danger ambiance-delete"
                                        on:click=move |_| confirming.set(true)
                                    >
                                        {move || t(Key::Delete)}
                                    </button>
                                }
                                    .into_any()
                            }}
                        }
                    })}
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nidoo_api::{CategoryAction, ClimateAction, DeviceAction, FlowAction, Target};
    use nidoo_core::model::Command;

    fn set(ids: &[&str]) -> BTreeSet<String> {
        ids.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn a_new_id_never_equals_a_static_route() {
        assert_eq!(new_ambiance_id("New", &set(&[])), "new_2");
        assert_eq!(new_ambiance_id("About", &set(&[])), "about_2");
        assert_eq!(new_ambiance_id("About", &set(&["about_2"])), "about_3");
        assert_eq!(
            new_ambiance_id("Bonne nuit", &set(&["bonne_nuit"])),
            "bonne_nuit_2"
        );
    }

    #[test]
    fn is_missing_checks_each_reference() {
        let entities = BTreeMap::new();
        let areas = vec![AreaView {
            id: "salon".into(),
            name: "Salon".into(),
            icon: None,
            order: None,
            entity_count: 0,
            primary_count: 0,
            group_names: Default::default(),
            group_favorites: Default::default(),
        }];
        let flows = vec![("f".to_owned(), "F".to_owned())];
        let zones = vec![("z".to_owned(), "Z".to_owned())];
        let missing = |a: &AmbianceAction, z: Option<&[(String, String)]>| {
            is_missing(a, &entities, &areas, &flows, z)
        };
        let cat = |area: Option<&str>| {
            AmbianceAction::Category(CategoryAction {
                what: nidoo_api::Category::Lights,
                to: Target::Off,
                brightness: None,
                area: area.map(str::to_owned),
                except: Vec::new(),
            })
        };
        assert!(!missing(&cat(None), None));
        assert!(!missing(&cat(Some("salon")), None));
        assert!(missing(&cat(Some("gone")), None));
        let device = AmbianceAction::Device(DeviceAction {
            entity: "gone".into(),
            command: Command::TurnOff,
        });
        assert!(missing(&device, None));
        let flow = |id: &str| {
            AmbianceAction::Flow(FlowAction {
                flow: id.into(),
                action: nidoo_api::FlowDo::Run,
            })
        };
        assert!(!missing(&flow("f"), None));
        assert!(missing(&flow("gone"), None));
        let zone = |id: &str| {
            AmbianceAction::Climate(ClimateAction {
                zone: Some(id.into()),
                ..Default::default()
            })
        };
        assert!(!missing(&zone("gone"), None), "zones unknown: not missing");
        assert!(missing(&zone("gone"), Some(&zones)));
        assert!(!missing(&zone("z"), Some(&zones)));
    }
}
