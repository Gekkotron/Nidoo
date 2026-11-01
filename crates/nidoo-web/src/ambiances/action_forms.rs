//! The forms of the ambiance editor: one per action type, and the "Add an action" chooser. Every
//! form reads and writes its row in the editor's `Rows` by key, so a row keeps its inputs (and
//! their focus) while the list around it changes.

use std::collections::BTreeMap;
use std::sync::Arc;

use leptos::prelude::*;
use nidoo_api::{
    AmbianceAction, Category, CategoryAction, ClimateAction, DeviceAction, EntityView, FlowAction,
    FlowDo, Target, category_members,
};
use nidoo_core::model::{Command, UNASSIGNED_AREA};
use serde_json::{Value, json};

use crate::command_form::{CommandFields, FieldStyle, entity_actions, retarget};
use crate::entity_picker::{EntityPickField, Keep};
use crate::format::list_label;
use crate::house::House;
use crate::i18n::devices::{DeviceText, device_text};
use crate::i18n::format::parse_decimal;
use crate::i18n::{self, Key, Lang, use_text};
use crate::screens::group_wizard::group_by_room;

/// One action of the list under its stable key.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub key: u64,
    pub action: AmbianceAction,
}

pub type Rows = RwSignal<Vec<Row>>;

fn edit(rows: Rows, key: u64, f: impl FnOnce(&mut AmbianceAction)) {
    rows.update(|list| {
        if let Some(row) = list.iter_mut().find(|r| r.key == key) {
            f(&mut row.action);
        }
    });
}

fn lang() -> RwSignal<Lang> {
    use_context::<RwSignal<Lang>>().unwrap_or_else(|| RwSignal::new(Lang::En))
}

// ---- device commands -------------------------------------------------------------------------

// ---- the list of actions ---------------------------------------------------------------------

fn default_category() -> CategoryAction {
    CategoryAction {
        what: Category::Lights,
        to: Target::Off,
        brightness: None,
        area: None,
        except: Vec::new(),
    }
}

fn default_device() -> DeviceAction {
    DeviceAction {
        entity: String::new(),
        command: Command::TurnOff,
    }
}

fn default_climate() -> ClimateAction {
    ClimateAction {
        setpoint: Some(json!(20.0)),
        hours: Some(4),
        ..Default::default()
    }
}

/// The action a new row of this type starts as.
fn new_action(kind: ActionType, first_flow: Option<String>) -> AmbianceAction {
    match kind {
        ActionType::Category => AmbianceAction::Category(default_category()),
        ActionType::Device => AmbianceAction::Device(default_device()),
        ActionType::Heating => AmbianceAction::Climate(default_climate()),
        ActionType::Automation => AmbianceAction::Flow(FlowAction {
            flow: first_flow.unwrap_or_default(),
            action: FlowDo::Run,
        }),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ActionType {
    Category,
    Device,
    Heating,
    Automation,
}

impl ActionType {
    fn key(self) -> Key {
        match self {
            Self::Category => Key::AmbCategoryType,
            Self::Device => Key::AmbDeviceType,
            Self::Heating => Key::AmbHeatingType,
            Self::Automation => Key::AmbAutomationType,
        }
    }
}

/// "Add an action": the button opens the four types, heating only while `heating` is true.
#[component]
pub fn AddAction(heating: Signal<bool>, on_add: Callback<AmbianceAction>) -> impl IntoView {
    let t = use_text();
    let house = expect_context::<House>();
    let open = RwSignal::new(false);
    let pick = move |kind: ActionType| {
        let first = house.flows.with_untracked(|f| {
            f.iter()
                .find(|f| f.invalid.is_none())
                .map(|f| f.definition.id.clone())
        });
        on_add.run(new_action(kind, first));
        open.set(false);
    };
    view! {
        <div class="ambiance-add">
            <button
                type="button"
                class="btn ambiance-add__toggle"
                aria-expanded=move || open.get().to_string()
                on:click=move |_| open.update(|o| *o = !*o)
            >
                {move || t(Key::AmbAddAction)}
            </button>
            <Show when=move || open.get()>
                <ul class="ambiance-add__types">
                    {move || {
                        [
                            ActionType::Category,
                            ActionType::Device,
                            ActionType::Heating,
                            ActionType::Automation,
                        ]
                            .into_iter()
                            .filter(|k| *k != ActionType::Heating || heating.get())
                            .map(|kind| {
                                view! {
                                    <li>
                                        <button
                                            type="button"
                                            class="btn ambiance-add__type"
                                            on:click=move |_| pick(kind)
                                        >
                                            {move || t(kind.key())}
                                        </button>
                                    </li>
                                }
                            })
                            .collect_view()
                    }}
                </ul>
            </Show>
        </div>
    }
}

// ---- category --------------------------------------------------------------------------------

fn what_value(w: Category) -> &'static str {
    match w {
        Category::Lights => "lights",
        Category::Plugs => "plugs",
        Category::Covers => "covers",
    }
}

fn target_value(t: Target) -> &'static str {
    match t {
        Target::On => "on",
        Target::Off => "off",
        Target::Open => "open",
        Target::Close => "close",
    }
}

/// The two targets a category takes.
fn targets(w: Category) -> [Target; 2] {
    match w {
        Category::Covers => [Target::Open, Target::Close],
        _ => [Target::On, Target::Off],
    }
}

fn target_key(w: Category, t: Target) -> Key {
    match (w, t) {
        (Category::Lights, Target::On) => Key::LightOn,
        (Category::Lights, _) => Key::LightOff,
        (_, Target::On) => Key::SwitchOn,
        (_, Target::Open) => Key::AmbOpen,
        (_, Target::Close) => Key::AmbClose,
        _ => Key::SwitchOff,
    }
}

/// Brightness only makes sense for lights turned on.
fn fix_brightness(c: &mut CategoryAction) {
    if !(c.what == Category::Lights && c.to == Target::On) {
        c.brightness = None;
    }
}

fn set_what(c: &mut CategoryAction, what: Category) {
    if c.what == what {
        return;
    }
    c.what = what;
    c.except.clear();
    if !targets(what).contains(&c.to) {
        // Off and close are the same intent, as are on and open.
        c.to = match c.to {
            Target::Off => targets(what)[1],
            Target::Close => targets(what)[1],
            _ => targets(what)[0],
        };
    }
    fix_brightness(c);
}

/// The ids in `c.except` that no entity of the house has any more: the hub refuses to save an
/// action that still names one, so the form must show them to be cleared.
pub fn stale_excepts(c: &CategoryAction, entities: &BTreeMap<String, EntityView>) -> Vec<String> {
    c.except
        .iter()
        .filter(|id| !entities.contains_key(*id))
        .cloned()
        .collect()
}

/// One checkbox of the "See which" list: ticked unless `id` is in `except`.
fn member_item(
    rows: Rows,
    key: u64,
    cat: Memo<CategoryAction>,
    id: String,
    label: String,
) -> impl IntoView {
    let checked_id = id.clone();
    view! {
        <li>
            <label class="ambiance-member">
                <input
                    type="checkbox"
                    prop:checked=move || cat.with(|c| !c.except.contains(&checked_id))
                    on:change=move |ev| {
                        let on = event_target_checked(&ev);
                        let id = id.clone();
                        edit_category(rows, key, |c| {
                            c.except.retain(|e| e != &id);
                            if !on {
                                c.except.push(id);
                            }
                        });
                    }
                />
                <span>{label}</span>
            </label>
        </li>
    }
}

/// The entities a category action targets as `(id, label, area)`.
fn members(
    lang: Lang,
    c: &CategoryAction,
    entities: &BTreeMap<String, EntityView>,
) -> Vec<(String, String, String)> {
    let all: Vec<EntityView> = entities.values().cloned().collect();
    category_members(c, &all)
        .into_iter()
        .map(|e| (e.id.clone(), list_label(lang, e), e.area.clone()))
        .collect()
}

fn category_memo(rows: Rows, key: u64) -> Memo<CategoryAction> {
    Memo::new(move |_| {
        rows.with(|list| {
            list.iter()
                .find(|r| r.key == key)
                .and_then(|r| match &r.action {
                    AmbianceAction::Category(c) => Some(c.clone()),
                    _ => None,
                })
        })
        .unwrap_or_else(default_category)
    })
}

fn edit_category(rows: Rows, key: u64, f: impl FnOnce(&mut CategoryAction)) {
    edit(rows, key, |a| {
        if let AmbianceAction::Category(c) = a {
            f(c);
        }
    });
}

#[component]
pub fn CategoryForm(rows: Rows, key: u64) -> impl IntoView {
    let t = use_text();
    let lang = lang();
    let house = expect_context::<House>();
    let cat = category_memo(rows, key);
    let listing = RwSignal::new(false);
    let count = Memo::new(move |_| {
        let c = cat.get();
        house.entities.with(|m| members(lang.get(), &c, m).len())
    });
    // Every member, the ones left alone included, so they can be ticked again.
    let rooms = Memo::new(move |_| {
        let mut c = cat.get();
        c.except.clear();
        let list = house.entities.with(|m| members(lang.get(), &c, m));
        let lang = lang.get();
        house.areas.with(|areas| group_by_room(lang, list, areas))
    });

    view! {
        <div class="ambiance-form">
            <label class="field">
                <span class="field__caption">{move || t(Key::AmbWhat)}</span>
                <select
                    class="field__input field__select ambiance-what"
                    on:change=move |ev| {
                        let what = match event_target_value(&ev).as_str() {
                            "plugs" => Category::Plugs,
                            "covers" => Category::Covers,
                            _ => Category::Lights,
                        };
                        edit_category(rows, key, |c| set_what(c, what));
                    }
                >
                    {[Category::Lights, Category::Plugs, Category::Covers]
                        .into_iter()
                        .map(|w| {
                            let k = match w {
                                Category::Lights => Key::AmbLights,
                                Category::Plugs => Key::AmbPlugs,
                                Category::Covers => Key::AmbCovers,
                            };
                            view! {
                                <option
                                    value=what_value(w)
                                    prop:selected=move || cat.with(|c| c.what == w)
                                >
                                    {move || t(k)}
                                </option>
                            }
                        })
                        .collect_view()}
                </select>
            </label>
            <label class="field">
                <span class="field__caption">{move || t(Key::AmbTo)}</span>
                <select
                    class="field__input field__select ambiance-to"
                    on:change=move |ev| {
                        let value = event_target_value(&ev);
                        edit_category(rows, key, |c| {
                            if let Some(to) = targets(c.what)
                                .into_iter()
                                .find(|t| target_value(*t) == value)
                            {
                                c.to = to;
                                fix_brightness(c);
                            }
                        });
                    }
                >
                    {move || {
                        let what = cat.with(|c| c.what);
                        targets(what)
                            .into_iter()
                            .map(|to| {
                                view! {
                                    <option
                                        value=target_value(to)
                                        prop:selected=move || cat.with(|c| c.to == to)
                                    >
                                        {move || t(target_key(what, to))}
                                    </option>
                                }
                            })
                            .collect_view()
                    }}
                </select>
            </label>
            <Show when=move || cat.with(|c| c.what == Category::Lights && c.to == Target::On)>
                <label class="field">
                    <span class="field__caption">{move || format!("{} %", t(Key::Brightness))}</span>
                    <input
                        class="field__input ambiance-brightness"
                        type="number"
                        min="1"
                        max="100"
                        step="1"
                        prop:value=move || {
                            cat.with(|c| c.brightness.map(|b| b.to_string()).unwrap_or_default())
                        }
                        on:change=move |ev| {
                            let value = event_target_value(&ev).trim().parse::<u8>().ok();
                            edit_category(rows, key, |c| c.brightness = value.map(|b| b.clamp(1, 100)));
                        }
                    />
                </label>
            </Show>
            <label class="field">
                <span class="field__caption">{move || t(Key::AmbWhere)}</span>
                <select
                    class="field__input field__select ambiance-where"
                    on:change=move |ev| {
                        let value = event_target_value(&ev);
                        edit_category(rows, key, |c| {
                            c.area = (!value.is_empty()).then_some(value);
                            c.except.clear();
                        });
                    }
                >
                    <option value="" prop:selected=move || cat.with(|c| c.area.is_none())>
                        {move || t(Key::AmbWholeHouse)}
                    </option>
                    {move || {
                        let areas = house.areas.get();
                        let current = cat.with(|c| c.area.clone());
                        let gone = current
                            .clone()
                            .filter(|id| !areas.iter().any(|a| &a.id == id))
                            .map(|id| {
                                let label = format!("{id} {}", t(Key::AmbianceMissing));
                                view! { <option value=id.clone() selected>{label}</option> }
                            });
                        let known = areas
                            .into_iter()
                            .filter(|a| a.id != UNASSIGNED_AREA)
                            .map(|a| {
                                let selected = current.as_deref() == Some(a.id.as_str());
                                view! { <option value=a.id.clone() selected=selected>{a.name.clone()}</option> }
                            })
                            .collect_view();
                        view! { {gone}{known} }
                    }}
                </select>
            </label>
            <div class="ambiance-count">
                <span class="ambiance-count__n">
                    {move || i18n::category_count(lang.get(), count.get())}
                </span>
                <button
                    type="button"
                    class="btn btn--compact ambiance-count__toggle"
                    aria-expanded=move || listing.get().to_string()
                    on:click=move |_| listing.update(|l| *l = !*l)
                >
                    {move || t(Key::AmbSeeWhich)}
                </button>
            </div>
            <Show when=move || listing.get()>
                <div class="ambiance-members">
                    {move || {
                        rooms
                            .get()
                            .into_iter()
                            .map(|(room, items)| {
                                view! {
                                    <h4 class="ambiance-members__room">{room}</h4>
                                    <ul class="ambiance-members__list">
                                        {items
                                            .into_iter()
                                            .map(|(id, label)| member_item(rows, key, cat, id, label))
                                            .collect_view()}
                                    </ul>
                                }
                            })
                            .collect_view()
                    }}
                    {move || {
                        let gone = cat.with(|c| house.entities.with(|m| stale_excepts(c, m)));
                        (!gone.is_empty())
                            .then(|| {
                                view! {
                                    <ul class="ambiance-members__list">
                                        {gone
                                            .into_iter()
                                            .map(|id| {
                                                let label = format!("{id} {}", t(Key::AmbianceMissing));
                                                member_item(rows, key, cat, id, label)
                                            })
                                            .collect_view()}
                                    </ul>
                                }
                            })
                    }}
                </div>
            </Show>
        </div>
    }
}

// ---- device ----------------------------------------------------------------------------------

fn device_memo(rows: Rows, key: u64) -> Memo<DeviceAction> {
    Memo::new(move |_| {
        rows.with(|list| {
            list.iter()
                .find(|r| r.key == key)
                .and_then(|r| match &r.action {
                    AmbianceAction::Device(d) => Some(d.clone()),
                    _ => None,
                })
        })
        .unwrap_or_else(default_device)
    })
}

fn edit_device(rows: Rows, key: u64, f: impl FnOnce(&mut DeviceAction)) {
    edit(rows, key, |a| {
        if let AmbianceAction::Device(d) = a {
            f(d);
        }
    });
}

#[component]
pub fn DeviceForm(rows: Rows, key: u64) -> impl IntoView {
    let house = expect_context::<House>();
    let dev = device_memo(rows, key);
    let entity = Memo::new(move |_| {
        let id = dev.with(|d| d.entity.clone());
        house.entities.with(|m| m.get(&id).cloned())
    });
    let pick = move |id: String| {
        let actions = house
            .entities
            .with_untracked(|m| m.get(&id).map(entity_actions))
            .unwrap_or_default();
        edit_device(rows, key, |d| {
            d.entity = id;
            d.command = retarget(&d.command, &actions);
        });
    };
    let pool: Keep = Arc::new(|e| e.is_primary() && !e.id.starts_with("group_"));

    view! {
        <div class="ambiance-form">
            <EntityPickField
                label=Signal::derive(move || device_text(i18n::use_lang()(), DeviceText::Device).to_owned())
                value=Signal::derive(move || dev.with(|d| Some(d.entity.clone()).filter(|e| !e.is_empty())))
                on_change=Callback::new(move |v: Option<String>| {
                    if let Some(id) = v {
                        pick(id);
                    }
                })
                pool=Some(pool)
            />
            <CommandFields
                command=Signal::derive(move || dev.with(|d| d.command.clone()))
                entity=Signal::from(entity)
                write=Callback::new(move |c: Command| edit_device(rows, key, |d| d.command = c))
                style=FieldStyle::Ambiance
                action_class="ambiance-device__command"
            />
        </div>
    }
}

// ---- heating ---------------------------------------------------------------------------------

fn climate_memo(rows: Rows, key: u64) -> Memo<ClimateAction> {
    Memo::new(move |_| {
        rows.with(|list| {
            list.iter()
                .find(|r| r.key == key)
                .and_then(|r| match &r.action {
                    AmbianceAction::Climate(c) => Some(c.clone()),
                    _ => None,
                })
        })
        .unwrap_or_else(default_climate)
    })
}

fn edit_climate(rows: Rows, key: u64, f: impl FnOnce(&mut ClimateAction)) {
    edit(rows, key, |a| {
        if let AmbianceAction::Climate(c) = a {
            f(c);
        }
    });
}

/// A held setpoint: 5 to 30 degrees, in steps of 0.5.
fn clamp_setpoint(v: f64) -> f64 {
    (v.clamp(5.0, 30.0) * 2.0).round() / 2.0
}

fn is_off(c: &ClimateAction) -> bool {
    c.setpoint.as_ref().and_then(Value::as_str) == Some("off")
}

#[component]
pub fn HeatingForm(rows: Rows, key: u64, zones: RwSignal<Vec<(String, String)>>) -> impl IntoView {
    let t = use_text();
    let c = climate_memo(rows, key);
    view! {
        <div class="ambiance-form">
            <label class="field">
                <span class="field__caption">{move || t(Key::AmbZone)}</span>
                <select
                    class="field__input field__select ambiance-zone"
                    on:change=move |ev| {
                        let value = event_target_value(&ev);
                        edit_climate(rows, key, |c| c.zone = (!value.is_empty()).then_some(value));
                    }
                >
                    <option value="" prop:selected=move || c.with(|c| c.zone.is_none())>
                        {move || t(Key::AmbEveryZone)}
                    </option>
                    {move || {
                        let zones = zones.get();
                        let current = c.with(|c| c.zone.clone());
                        let gone = current
                            .clone()
                            .filter(|id| !zones.iter().any(|(z, _)| z == id))
                            .map(|id| {
                                let label = format!("{id} {}", t(Key::AmbianceMissing));
                                view! { <option value=id.clone() selected>{label}</option> }
                            });
                        let known = zones
                            .into_iter()
                            .map(|(id, name)| {
                                let selected = current.as_deref() == Some(id.as_str());
                                view! { <option value=id.clone() selected=selected>{name}</option> }
                            })
                            .collect_view();
                        view! { {gone}{known} }
                    }}
                </select>
            </label>
            <label class="field">
                <select
                    class="field__input field__select ambiance-hold"
                    aria-label=move || t(Key::AmbHeatingType)
                    on:change=move |ev| {
                        let clear = event_target_value(&ev) == "clear";
                        edit_climate(rows, key, |c| {
                            c.clear = clear;
                            if clear {
                                c.setpoint = None;
                                c.mode = None;
                                c.hours = None;
                            } else {
                                *c = ClimateAction { zone: c.zone.take(), ..default_climate() };
                            }
                        });
                    }
                >
                    <option value="hold" prop:selected=move || c.with(|c| !c.clear)>
                        {move || t(Key::AmbHold)}
                    </option>
                    <option value="clear" prop:selected=move || c.with(|c| c.clear)>
                        {move || t(Key::AmbBackToSchedule)}
                    </option>
                </select>
            </label>
            <Show when=move || c.with(|c| !c.clear)>
                <label class="ambiance-check">
                    <input
                        class="ambiance-off"
                        type="checkbox"
                        prop:checked=move || c.with(is_off)
                        on:change=move |ev| {
                            let off = event_target_checked(&ev);
                            edit_climate(rows, key, |c| {
                                c.setpoint = Some(if off { json!("off") } else { json!(20.0) });
                            });
                        }
                    />
                    <span>{move || t(Key::ModeOff)}</span>
                </label>
                <Show when=move || c.with(|c| !is_off(c))>
                    <label class="field">
                        <span class="field__caption">{move || t(Key::AmbTemperature)}</span>
                        <input
                            class="field__input ambiance-setpoint"
                            type="number"
                            min="5"
                            max="30"
                            step="0.5"
                            prop:value=move || {
                                c.with(|c| {
                                    c.setpoint
                                        .as_ref()
                                        .and_then(Value::as_f64)
                                        .map(|v| v.to_string())
                                        .unwrap_or_default()
                                })
                            }
                            on:change=move |ev| {
                                let value = parse_decimal(&event_target_value(&ev));
                                edit_climate(rows, key, |c| c.setpoint = value.map(|v| json!(clamp_setpoint(v))));
                            }
                        />
                    </label>
                </Show>
                <label class="field">
                    <span class="field__caption">{move || t(Key::Mode)}</span>
                    <select
                        class="field__input field__select ambiance-mode"
                        on:change=move |ev| {
                            let value = event_target_value(&ev);
                            edit_climate(rows, key, |c| c.mode = (!value.is_empty()).then_some(value));
                        }
                    >
                        <option value="" prop:selected=move || c.with(|c| c.mode.is_none())>
                            "—"
                        </option>
                        <option value="heat" prop:selected=move || c.with(|c| c.mode.as_deref() == Some("heat"))>
                            {move || t(Key::ModeHeat)}
                        </option>
                        <option value="cool" prop:selected=move || c.with(|c| c.mode.as_deref() == Some("cool"))>
                            {move || t(Key::ModeCool)}
                        </option>
                    </select>
                </label>
                <label class="field">
                    <span class="field__caption">{move || t(Key::AmbHours)}</span>
                    <input
                        class="field__input ambiance-hours"
                        type="number"
                        min="1"
                        max="720"
                        step="1"
                        prop:value=move || c.with(|c| c.hours.map(|h| h.to_string()).unwrap_or_default())
                        on:change=move |ev| {
                            let value = event_target_value(&ev).trim().parse::<u32>().ok();
                            edit_climate(rows, key, |c| c.hours = value.map(|h| h.clamp(1, 720)));
                        }
                    />
                </label>
            </Show>
        </div>
    }
}

// ---- automation ------------------------------------------------------------------------------

#[component]
pub fn AutomationForm(rows: Rows, key: u64) -> impl IntoView {
    let t = use_text();
    let house = expect_context::<House>();
    let flow = Memo::new(move |_| {
        rows.with(|list| {
            list.iter()
                .find(|r| r.key == key)
                .and_then(|r| match &r.action {
                    AmbianceAction::Flow(f) => Some(f.clone()),
                    _ => None,
                })
        })
        .unwrap_or(FlowAction {
            flow: String::new(),
            action: FlowDo::Run,
        })
    });
    let edit_flow = move |f: Box<dyn FnOnce(&mut FlowAction)>| {
        edit(rows, key, |a| {
            if let AmbianceAction::Flow(x) = a {
                f(x);
            }
        });
    };
    view! {
        <div class="ambiance-form">
            <label class="field">
                <span class="field__caption">{move || t(Key::AmbAutomationType)}</span>
                <select
                    class="field__input field__select ambiance-flow"
                    on:change=move |ev| {
                        let id = event_target_value(&ev);
                        edit_flow(Box::new(|f| f.flow = id));
                    }
                >
                    {move || {
                        let current = flow.with(|f| f.flow.clone());
                        let flows: Vec<(String, String)> = house
                            .flows
                            .with(|l| {
                                l.iter()
                                    .map(|f| (f.definition.id.clone(), f.definition.name.clone()))
                                    .collect()
                            });
                        let gone = (!flows.iter().any(|(id, _)| id == &current))
                            .then(|| {
                                let label = format!("{current} {}", t(Key::AmbianceMissing));
                                view! { <option value=current.clone() selected>{label}</option> }
                            });
                        let known = flows
                            .into_iter()
                            .map(|(id, name)| {
                                let selected = id == current;
                                view! { <option value=id selected=selected>{name}</option> }
                            })
                            .collect_view();
                        view! { {gone}{known} }
                    }}
                </select>
            </label>
            <label class="field">
                <select
                    class="field__input field__select ambiance-flow-do"
                    aria-label=move || t(Key::AmbAutomationType)
                    on:change=move |ev| {
                        let to = match event_target_value(&ev).as_str() {
                            "enable" => FlowDo::Enable,
                            "disable" => FlowDo::Disable,
                            _ => FlowDo::Run,
                        };
                        edit_flow(Box::new(move |f| f.action = to));
                    }
                >
                    <option value="enable" prop:selected=move || flow.with(|f| f.action == FlowDo::Enable)>
                        {move || t(Key::AmbSwitchOn)}
                    </option>
                    <option value="disable" prop:selected=move || flow.with(|f| f.action == FlowDo::Disable)>
                        {move || t(Key::AmbSwitchOff)}
                    </option>
                    <option value="run" prop:selected=move || flow.with(|f| f.action == FlowDo::Run)>
                        {move || t(Key::AmbRun)}
                    </option>
                </select>
            </label>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nidoo_core::model::{Capabilities, EntityCategory, EntityState, Kind, UiOverride};

    fn entity(kind: Kind, caps: Capabilities) -> EntityView {
        EntityView {
            id: "e".into(),
            device_id: None,
            name: "E".into(),
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
            area: "a".into(),
            part_of: None,
            capabilities: caps,
            ui: UiOverride::default(),
            state: EntityState::default(),
        }
    }

    #[test]
    fn setpoint_is_clamped_and_stepped() {
        assert_eq!(clamp_setpoint(2.0), 5.0);
        assert_eq!(clamp_setpoint(99.0), 30.0);
        assert_eq!(clamp_setpoint(20.3), 20.5);
        assert_eq!(clamp_setpoint(20.2), 20.0);
        assert_eq!(clamp_setpoint(21.0), 21.0);
    }

    #[test]
    fn changing_what_keeps_a_valid_target_and_drops_the_rest() {
        let mut c = CategoryAction {
            what: Category::Lights,
            to: Target::On,
            brightness: Some(40),
            area: None,
            except: vec!["x".into()],
        };
        set_what(&mut c, Category::Covers);
        assert_eq!(c.to, Target::Open);
        assert_eq!(c.brightness, None);
        assert!(c.except.is_empty());
        set_what(&mut c, Category::Plugs);
        assert_eq!(c.to, Target::On, "open maps back to on");
    }

    #[test]
    fn changing_what_maps_the_intent_across_kinds() {
        let mut c = CategoryAction {
            what: Category::Lights,
            to: Target::Off,
            brightness: None,
            area: None,
            except: Vec::new(),
        };
        set_what(&mut c, Category::Covers);
        assert_eq!(c.to, Target::Close);
        set_what(&mut c, Category::Lights);
        assert_eq!(c.to, Target::Off);
        c.to = Target::On;
        set_what(&mut c, Category::Covers);
        assert_eq!(c.to, Target::Open);
        set_what(&mut c, Category::Plugs);
        assert_eq!(c.to, Target::On);
    }

    #[test]
    fn stale_excepts_lists_only_unknown_entities() {
        let known = entity(Kind::Light, Capabilities::default());
        let entities = BTreeMap::from([("e".to_owned(), known)]);
        let c = CategoryAction {
            what: Category::Lights,
            to: Target::Off,
            brightness: None,
            area: None,
            except: vec!["e".into(), "gone".into()],
        };
        assert_eq!(stale_excepts(&c, &entities), vec!["gone".to_owned()]);
    }
}
