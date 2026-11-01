//! The action list of a device and the inputs of each action, as core's
//! [`nidoo_core::vocabulary`] lists them from discovery: the flow Command card and the
//! ambiance device action draw the same choices. Only the wording is the web's.

use leptos::prelude::*;
use nidoo_api::EntityView;
use nidoo_core::command::command_name;
use nidoo_core::flow::Scalar;
use nidoo_core::model::{Command, Hs};
use nidoo_core::vocabulary::{Action, Param, actions};
use serde_json::Value;

use crate::cards::vacuum::action_label as vacuum_action_label;
use crate::format::word;
use crate::i18n::Lang;
use crate::i18n::flows::{
    FlowText, arm_action, control_action, fan_mode_action, fan_speed_action, flow_text,
};
use crate::i18n::format::parse_decimal;
use crate::i18n::settings::{SettingsText, select_option, set_mode, settings_text};
use crate::i18n::states::{StateCtx, on_off, state_word};

/// The actions of `e`; a number entity's value starts at its current reading when it has one.
pub fn entity_actions(e: &EntityView) -> Vec<Action> {
    let mut list = actions(e.kind, &e.component, &e.capabilities);
    if let Some(Scalar::Number(now)) = Scalar::of_state(&e.state.value) {
        for action in &mut list {
            if let Command::SetNumber { value } = &mut action.command {
                *value = now;
            }
        }
    }
    list
}

/// Two commands with the same key are the same choice, possibly with another input value.
pub fn command_key(c: &Command) -> String {
    match c {
        Command::SelectOption { option } => format!("select:{option}"),
        Command::SetHvacMode { mode } => format!("mode:{mode}"),
        Command::SetFanMode { mode } => format!("fan:{mode}"),
        Command::SetFanSpeed { speed } => format!("speed:{speed}"),
        Command::VacuumAction { action } => format!("vacuum:{action}"),
        Command::Arm { mode, .. } => format!("arm:{mode:?}"),
        // An on/off or choice control is one choice per value; a number or text control is
        // one choice whose value is an input.
        Command::Control { control, value } => match value {
            Some(v @ (Value::Bool(_) | Value::String(_))) if !is_text_input(v) => {
                format!("control:{control}:{v}")
            }
            _ => format!("control:{control}"),
        },
        other => command_name(other).to_owned(),
    }
}

/// A text control starts empty: its value is typed, not a choice.
fn is_text_input(v: &Value) -> bool {
    v.as_str() == Some("")
}

/// The label of a command in an action list, in `lang`.
pub fn command_label(lang: Lang, c: &Command) -> String {
    let flow = |k| flow_text(lang, k).to_owned();
    let setting = |k| settings_text(lang, k).to_owned();
    match c {
        Command::TurnOn { .. } => flow(FlowText::TurnOn),
        Command::TurnOff => flow(FlowText::TurnOff),
        Command::Toggle => flow(FlowText::Toggle),
        Command::Open => flow(FlowText::Open),
        Command::Close => flow(FlowText::Close),
        Command::Stop => flow(FlowText::Stop),
        Command::Disarm { .. } => flow(FlowText::Disarm),
        Command::Press => setting(SettingsText::Press),
        Command::SetPosition { .. } => setting(SettingsText::SetPosition),
        Command::SetTemperature { .. } => setting(SettingsText::SetTemperature),
        Command::SetNumber { .. } => setting(SettingsText::SetValue),
        Command::SelectOption { option } => select_option(lang, option),
        Command::SetHvacMode { mode } => set_mode(
            lang,
            &lower_first(lang, &state_word(lang, StateCtx::Hvac, mode)),
        ),
        Command::SetFanMode { mode } => fan_mode_action(lang, &word(mode)),
        Command::SetFanSpeed { speed } => fan_speed_action(lang, &word(speed)),
        Command::VacuumAction { action } => vacuum_action_label(lang, action),
        Command::Arm { mode, .. } => arm_action(lang, *mode),
        Command::Control { control, value } => match (control.as_str(), value) {
            ("locked", Some(Value::Bool(true))) => flow(FlowText::Lock),
            ("locked", Some(Value::Bool(false))) => flow(FlowText::Unlock),
            (_, Some(Value::Bool(on))) => {
                control_action(lang, &word(control), on_off(lang, false, *on))
            }
            (_, Some(v @ Value::String(s))) if !is_text_input(v) => {
                control_action(lang, &word(control), &word(s))
            }
            _ => word(control),
        },
    }
}

/// French mode words are capitalised for display; inside "Régler le mode …" they read lower.
fn lower_first(lang: Lang, s: &str) -> String {
    match lang {
        Lang::En => s.to_owned(),
        Lang::Fr => {
            let mut chars = s.chars();
            chars
                .next()
                .map(|c| c.to_lowercase().chain(chars).collect())
                .unwrap_or_default()
        }
    }
}

/// `actions`, with `current` first when none of them is the same choice, so a saved command
/// the device does not list stays selected rather than replaced.
pub fn command_options(current: &Command, actions: &[Action]) -> Vec<Action> {
    let mut list = actions.to_vec();
    if matching_action(current, actions).is_none() {
        list.insert(
            0,
            Action {
                command: current.clone(),
                params: Vec::new(),
            },
        );
    }
    list
}

/// `current` when the entity offers it, else the entity's first action, else `current`.
/// The action `current` is: the same choice, or, for a control whose value is typed, the
/// action of that control.
pub fn matching_action<'a>(current: &Command, actions: &'a [Action]) -> Option<&'a Action> {
    let key = command_key(current);
    actions
        .iter()
        .find(|a| command_key(&a.command) == key)
        .or_else(|| match current {
            Command::Control { control, .. } => actions.iter().find(|a| {
                matches!(&a.command, Command::Control { control: c, .. } if c == control)
                    && a.params.iter().any(|p| p.key() == "value")
            }),
            _ => None,
        })
}

/// `current` when the entity offers it, else the entity's first action, else `current`.
pub fn retarget(current: &Command, actions: &[Action]) -> Command {
    // The new entity's own template, carrying over only the inputs it takes.
    if let Some(action) = matching_action(current, actions) {
        return action.params.iter().fold(action.command.clone(), |c, p| {
            match param_value(current, p.key()) {
                Some(v) => with_param(&c, p.key(), Some(v)),
                None => c,
            }
        });
    }
    actions
        .first()
        .map_or_else(|| current.clone(), |a| a.command.clone())
}

/// The value of the field `key` in the command's JSON.
pub fn param_value(c: &Command, key: &str) -> Option<Value> {
    serde_json::to_value(c)
        .ok()?
        .get(key)
        .filter(|v| !v.is_null())
        .cloned()
}

/// `c` with the field `key` set to `value` (`None` removes it). A result that is not a valid
/// command leaves `c` unchanged.
pub fn with_param(c: &Command, key: &str, value: Option<Value>) -> Command {
    let Ok(Value::Object(mut fields)) = serde_json::to_value(c) else {
        return c.clone();
    };
    match value {
        Some(v) => fields.insert(key.to_owned(), v),
        None => fields.remove(key),
    };
    serde_json::from_value(Value::Object(fields)).unwrap_or_else(|_| c.clone())
}

/// The stored JSON of a number shown as `shown`: scaled by `factor` and rounded when scaled,
/// and an integer when `step` and the result are whole (whole-number fields refuse `40.0`).
pub fn number_json(shown: f64, factor: f64, step: f64) -> Value {
    let shown = if step > 0.0 {
        (shown / step).round() * step
    } else {
        shown
    };
    let stored = if factor == 1.0 {
        shown
    } else {
        (shown * factor).round()
    };
    if step.fract() == 0.0 && stored.fract() == 0.0 {
        Value::from(stored as i64)
    } else {
        Value::from(stored)
    }
}

/// Which form the fields sit in: the flow card sheet or the ambiance editor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldStyle {
    Node,
    Ambiance,
}

impl FieldStyle {
    /// `(label wrapper, caption, text input, select)` classes.
    fn classes(self) -> (&'static str, &'static str, &'static str, &'static str) {
        match self {
            FieldStyle::Node => ("node-editor__field", "node-editor__label", "field", "field"),
            FieldStyle::Ambiance => (
                "field",
                "field__caption",
                "field__input",
                "field__input field__select",
            ),
        }
    }
}

/// The caption of an input, by the command field it writes; an unknown key is prettified.
fn param_caption(lang: Lang, key: &str) -> String {
    let k = match key {
        "position" => FlowText::ParamPosition,
        "temperature" => FlowText::ParamTemperature,
        "value" => FlowText::ParamValue,
        "brightness" => FlowText::ParamBrightness,
        "color" => FlowText::ParamColour,
        "effect" => FlowText::ParamEffect,
        "code" => FlowText::ParamCode,
        other => return word(other),
    };
    flow_text(lang, k).to_owned()
}

/// The number shown for a stored value: divided by `factor` and kept on `step` when scaled.
fn shown_number(stored: f64, factor: f64, step: f64) -> f64 {
    if factor == 1.0 {
        stored
    } else {
        ((stored / factor) / step).round() * step
    }
}

/// The value the select shows for `Action` index `i` / the advanced entry.
const ADVANCED: &str = "advanced";

/// The Action dropdown of a device and one input per input of the chosen action, drawn from
/// the input's type alone. `advanced`, when given, adds a last "Advanced (JSON)" action that
/// shows that view instead.
#[component]
pub fn CommandFields(
    command: Signal<Command>,
    entity: Signal<Option<EntityView>>,
    write: Callback<Command>,
    style: FieldStyle,
    #[prop(optional)] advanced: Option<ViewFn>,
    /// An extra class on the Action select (the ambiance form's tests find it by it).
    #[prop(optional)]
    action_class: &'static str,
    /// While true the number inputs are left out: a math result supplies the number.
    #[prop(optional)]
    numbers_replaced: Option<Signal<bool>>,
) -> impl IntoView {
    let lang = crate::i18n::use_lang();
    let (wrap, caption, input, select) = style.classes();
    let options = Memo::new(move |_| {
        let list = entity.with(|e| e.as_ref().map(entity_actions).unwrap_or_default());
        command.with(|c| command_options(c, &list))
    });
    let has_advanced = advanced.is_some();
    let advanced_open = RwSignal::new(false);
    let current_params = Memo::new(move |_| {
        command.with(|c| {
            options.with(|list| {
                matching_action(c, list)
                    .map(|a| a.params.clone())
                    .unwrap_or_default()
            })
        })
    });
    let on_action = move |ev: web_sys::Event| {
        let value = event_target_value(&ev);
        if value == ADVANCED {
            advanced_open.set(true);
            return;
        }
        advanced_open.set(false);
        let picked = value.parse::<usize>().ok();
        if let Some(a) = picked.and_then(|i| options.with(|o| o.get(i).cloned())) {
            write.run(a.command);
        }
    };
    let param_view = move |param: Param| {
        let key = param.key().to_owned();
        let label = {
            let key = key.clone();
            move || param_caption(lang(), &key)
        };
        let stored = {
            let key = key.clone();
            Signal::derive(move || command.with(|c| param_value(c, &key)))
        };
        let set = {
            let key = key.clone();
            move |value: Option<Value>| {
                write.run(command.with_untracked(|c| with_param(c, &key, value)));
            }
        };
        match param {
            Param::Number {
                min,
                max,
                step,
                unit,
                factor,
                optional,
                ..
            } => {
                let unit = unit.or_else(|| entity.with_untracked(|e| e.as_ref()?.unit.clone()));
                let value = move || {
                    stored
                        .get()
                        .and_then(|v| v.as_f64())
                        .map(|v| shown_number(v, factor, step).to_string())
                        .unwrap_or_default()
                };
                view! {
                    <label class=wrap>
                        <span class=caption>{label}</span>
                        <span class="command-field__number">
                            <input
                                type="number"
                                class=format!("{input} command-field__input")
                                min=min.to_string()
                                max=max.to_string()
                                step=step.to_string()
                                prop:value=value
                                on:change=move |ev| {
                                    let text = event_target_value(&ev);
                                    match parse_decimal(text.trim()) {
                                        Some(v) => set(Some(number_json(v.clamp(min, max), factor, step))),
                                        None if optional && text.trim().is_empty() => set(None),
                                        None => {}
                                    }
                                }
                            />
                            {unit.map(|u| view! { <span class="command-field__unit">{u}</span> })}
                        </span>
                    </label>
                }
                .into_any()
            }
            Param::Choice {
                options: choices,
                optional,
                ..
            } => {
                let current = move || stored.get().and_then(|v| v.as_str().map(str::to_owned));
                view! {
                    <label class=wrap>
                        <span class=caption>{label}</span>
                        <select
                            class=select
                            on:change=move |ev| {
                                let v = event_target_value(&ev);
                                set((!v.is_empty()).then_some(Value::String(v)));
                            }
                        >
                            {optional.then(|| view! {
                                <option value="" selected=move || current().is_none()>
                                    {move || flow_text(lang(), FlowText::NoneChoice)}
                                </option>
                            })}
                            {choices
                                .into_iter()
                                .map(|c| {
                                    let shown = word(&c);
                                    let is = c.clone();
                                    view! {
                                        <option value=c selected=move || current().as_deref() == Some(is.as_str())>
                                            {shown}
                                        </option>
                                    }
                                })
                                .collect_view()}
                        </select>
                    </label>
                }
                .into_any()
            }
            Param::Colour { .. } => {
                let current = move || {
                    stored
                        .get()
                        .and_then(|v| serde_json::from_value::<Hs>(v).ok())
                };
                let picked = move || match current() {
                    None => String::new(),
                    Some(hs) => {
                        swatch_of(hs).map_or_else(|| "custom".to_owned(), |i| i.to_string())
                    }
                };
                view! {
                    <label class=wrap>
                        <span class=caption>{label}</span>
                        <select
                            class=select
                            prop:value=picked
                            on:change=move |ev| {
                                let v = event_target_value(&ev);
                                if v == "custom" {
                                    return;
                                }
                                let hs = v.parse::<usize>().ok().and_then(|i| SWATCHES.get(i)).map(|(_, hs)| *hs);
                                set(hs.and_then(|hs| serde_json::to_value(hs).ok()));
                            }
                        >
                            <option value="">{move || flow_text(lang(), FlowText::NoneChoice)}</option>
                            {SWATCHES
                                .iter()
                                .enumerate()
                                .map(|(i, (name, _))| {
                                    let name = *name;
                                    view! { <option value=i.to_string()>{move || word(flow_text(lang(), name))}</option> }
                                })
                                .collect_view()}
                            {move || (picked() == "custom").then(|| view! {
                                <option value="custom">{move || flow_text(lang(), FlowText::ColourCustom)}</option>
                            })}
                        </select>
                    </label>
                }
                .into_any()
            }
            Param::Text {
                secret, optional, ..
            } => {
                let value = move || {
                    stored
                        .get()
                        .and_then(|v| v.as_str().map(str::to_owned))
                        .unwrap_or_default()
                };
                view! {
                    <label class=wrap>
                        <span class=caption>{label}</span>
                        <input
                            type=if secret { "password" } else { "text" }
                            class=input
                            autocomplete="off"
                            prop:value=value
                            on:input=move |ev| {
                                let v = event_target_value(&ev);
                                if v.is_empty() && optional {
                                    set(None);
                                } else {
                                    set(Some(Value::String(v)));
                                }
                            }
                        />
                    </label>
                }
                .into_any()
            }
        }
    };
    view! {
        <label class=wrap>
            <span class=caption>{move || flow_text(lang(), FlowText::Action)}</span>
            <select class=format!("{select} command-field__action {action_class}") on:change=on_action>
                {move || {
                    let key = command.with(|c| {
                        options.with(|l| matching_action(c, l).map(|a| command_key(&a.command)))
                    });
                    let open = advanced_open.get();
                    options
                        .get()
                        .into_iter()
                        .enumerate()
                        .map(|(i, a)| {
                            let selected = !open && key.as_deref() == Some(command_key(&a.command).as_str());
                            view! {
                                <option value=i.to_string() selected=selected>
                                    {command_label(lang(), &a.command)}
                                </option>
                            }
                        })
                        .collect_view()
                }}
                {has_advanced.then(|| view! {
                    <option value=ADVANCED selected=move || advanced_open.get()>
                        {move || flow_text(lang(), FlowText::AdvancedJson)}
                    </option>
                })}
            </select>
        </label>
        {move || {
            if advanced_open.get() {
                return advanced.as_ref().map(|v| v.run()).into_any();
            }
            let replaced = numbers_replaced.is_some_and(|r| r.get());
            current_params
                .get()
                .into_iter()
                .filter(|p| !(replaced && matches!(p, Param::Number { .. })))
                .map(param_view)
                .collect_view()
                .into_any()
        }}
    }
}

/// The colours a light can be set to, each reading back as its own name in card titles.
pub const SWATCHES: [(FlowText, Hs); 10] = [
    (FlowText::ColourWhite, Hs { h: 0.0, s: 0.0 }),
    (FlowText::ColourRed, Hs { h: 0.0, s: 100.0 }),
    (FlowText::ColourOrange, Hs { h: 30.0, s: 100.0 }),
    (FlowText::ColourYellow, Hs { h: 55.0, s: 100.0 }),
    (FlowText::ColourGreen, Hs { h: 120.0, s: 100.0 }),
    (FlowText::ColourCyan, Hs { h: 180.0, s: 100.0 }),
    (FlowText::ColourBlue, Hs { h: 225.0, s: 100.0 }),
    (FlowText::ColourPurple, Hs { h: 270.0, s: 100.0 }),
    (FlowText::ColourMagenta, Hs { h: 300.0, s: 100.0 }),
    (FlowText::ColourPink, Hs { h: 335.0, s: 100.0 }),
];

/// The swatch `hs` is, or `None` for a colour set some other way.
pub fn swatch_of(hs: Hs) -> Option<usize> {
    SWATCHES.iter().position(|(_, s)| *s == hs)
}

#[cfg(test)]
mod tests {
    use nidoo_core::model::{
        AlarmMode, Capabilities, EntityCategory, EntityState, Kind, StateValue, UiOverride,
    };
    use serde_json::json;

    use super::*;
    use nidoo_core::vocabulary::Param;

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

    fn bare(command: Command) -> Action {
        Action {
            command,
            params: vec![],
        }
    }

    #[test]
    fn command_labels_follow_the_language() {
        let on = Command::TurnOn {
            brightness: None,
            color: None,
            effect: None,
        };
        assert_eq!(command_label(Lang::En, &on), "Turn on");
        assert_eq!(command_label(Lang::Fr, &on), "Allumer");
        assert_eq!(command_label(Lang::Fr, &Command::Stop), "Arrêter");
        assert_eq!(command_label(Lang::Fr, &Command::Press), "Appuyer");
        let pos = Command::SetPosition { position: 50 };
        assert_eq!(command_label(Lang::Fr, &pos), "Régler la position");
        let pick = Command::SelectOption {
            option: "low".into(),
        };
        assert_eq!(command_label(Lang::En, &pick), "Select low");
        assert_eq!(command_label(Lang::Fr, &pick), "Choisir low");
        let mode = Command::SetHvacMode {
            mode: "heat".into(),
        };
        assert_eq!(command_label(Lang::Fr, &mode), "Régler le mode chauffage");
    }

    #[test]
    fn new_command_labels_follow_the_language() {
        let dock = Command::VacuumAction {
            action: "return_to_base".into(),
        };
        assert_eq!(command_label(Lang::Fr, &dock), "Retour à la base");
        let fan = Command::SetFanMode { mode: "low".into() };
        assert_eq!(command_label(Lang::En, &fan), "Fan: Low");
        let arm = Command::Arm {
            mode: AlarmMode::Away,
            code: None,
        };
        assert_eq!(command_label(Lang::Fr, &arm), "Activer (absent)");
        let lock = Command::Control {
            control: "locked".into(),
            value: Some(json!(true)),
        };
        assert_eq!(command_label(Lang::En, &lock), "Lock");
    }

    #[test]
    fn setting_one_param_keeps_the_others() {
        let on = Command::TurnOn {
            brightness: Some(100),
            color: Some(SWATCHES[1].1),
            effect: None,
        };
        assert_eq!(
            with_param(&on, "effect", Some(json!("blink"))),
            Command::TurnOn {
                brightness: Some(100),
                color: Some(SWATCHES[1].1),
                effect: Some("blink".into())
            }
        );
        assert_eq!(
            with_param(&on, "color", None),
            Command::TurnOn {
                brightness: Some(100),
                color: None,
                effect: None
            }
        );
        assert_eq!(param_value(&on, "brightness"), Some(json!(100)));
    }

    #[test]
    fn a_param_that_does_not_parse_is_ignored() {
        let pos = Command::SetPosition { position: 10 };
        assert_eq!(with_param(&pos, "position", Some(json!("abc"))), pos);
    }

    #[test]
    fn whole_numbers_are_written_as_integers() {
        let pos = Command::SetPosition { position: 10 };
        assert_eq!(
            with_param(&pos, "position", Some(number_json(40.0, 1.0, 1.0))),
            Command::SetPosition { position: 40 }
        );
        assert_eq!(number_json(60.0, 2.55, 1.0), json!(153));
        assert_eq!(number_json(19.5, 1.0, 0.5), json!(19.5));
    }

    #[test]
    fn unlisted_command_is_offered_first() {
        let list = vec![bare(Command::TurnOff)];
        let odd = Command::SetPosition { position: 10 };
        assert_eq!(command_options(&odd, &list)[0].command, odd);
        assert_eq!(command_options(&Command::TurnOff, &list).len(), 1);
    }

    #[test]
    fn retarget_keeps_offered_command_else_first() {
        let cover = vec![bare(Command::Open), bare(Command::Close)];
        assert_eq!(retarget(&Command::Close, &cover), Command::Close);
        assert_eq!(retarget(&Command::TurnOff, &cover), Command::Open);
        assert_eq!(retarget(&Command::TurnOff, &[]), Command::TurnOff);
    }

    fn text_action() -> Action {
        Action {
            command: Command::Control {
                control: "value".into(),
                value: Some(json!("")),
            },
            params: vec![Param::Text {
                key: "value".into(),
                secret: false,
                optional: false,
            }],
        }
    }

    #[test]
    fn a_typed_text_control_keeps_its_input() {
        let actions = vec![text_action()];
        let typed = Command::Control {
            control: "value".into(),
            value: Some(json!("h")),
        };
        assert_eq!(command_options(&typed, &actions).len(), 1);
        assert_eq!(
            matching_action(&typed, &actions).map(|a| a.params.len()),
            Some(1)
        );
        assert_eq!(retarget(&typed, &actions), typed);
    }

    #[test]
    fn retarget_drops_inputs_the_new_entity_lacks() {
        let on = Command::TurnOn {
            brightness: Some(100),
            color: Some(SWATCHES[1].1),
            effect: Some("blink".into()),
        };
        let dimmer = vec![Action {
            command: Command::TurnOn {
                brightness: None,
                color: None,
                effect: None,
            },
            params: vec![Param::Number {
                key: "brightness".into(),
                min: 1.0,
                max: 100.0,
                step: 1.0,
                unit: Some("%".into()),
                factor: 2.55,
                optional: true,
            }],
        }];
        assert_eq!(
            retarget(&on, &dimmer),
            Command::TurnOn {
                brightness: Some(100),
                color: None,
                effect: None
            }
        );
    }

    #[test]
    fn a_number_is_kept_on_its_step() {
        assert_eq!(number_json(40.4, 1.0, 1.0), json!(40));
        assert_eq!(number_json(19.3, 1.0, 0.5), json!(19.5));
    }

    #[test]
    fn swatches_read_back_as_their_own_colour_name() {
        for (name, hs) in SWATCHES {
            assert_eq!(crate::flow_text::colour_name(hs), name);
        }
        assert_eq!(swatch_of(SWATCHES[2].1), Some(2));
        assert_eq!(swatch_of(Hs { h: 12.0, s: 80.0 }), None);
    }

    #[test]
    fn lock_and_unlock_are_two_choices() {
        let lock = Command::Control {
            control: "locked".into(),
            value: Some(json!(true)),
        };
        let unlock = Command::Control {
            control: "locked".into(),
            value: Some(json!(false)),
        };
        assert_ne!(command_key(&lock), command_key(&unlock));
    }

    #[test]
    fn a_number_entity_starts_at_its_current_value() {
        let mut n = entity(Kind::Number, Capabilities::default());
        n.state.value = StateValue::Number(42.0);
        assert_eq!(
            entity_actions(&n)[0].command,
            Command::SetNumber { value: 42.0 }
        );
    }

    #[test]
    fn select_options_are_listed_from_core() {
        let caps = Capabilities {
            options: vec!["low".into(), "high".into()],
            ..Default::default()
        };
        let list = entity_actions(&entity(Kind::Select, caps));
        assert_eq!(
            list.into_iter().map(|a| a.command).collect::<Vec<_>>(),
            vec![
                Command::SelectOption {
                    option: "low".into()
                },
                Command::SelectOption {
                    option: "high".into()
                }
            ]
        );
    }
}
