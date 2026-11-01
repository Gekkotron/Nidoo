//! The generic card: any table-described entity, drawn from its `TypeDef`, caps and state.

use leptos::prelude::*;
use nidoo_api::EntityView;
use nidoo_core::model::{Command, GenericState, GenericValue, StateValue};
use nidoo_core::types::{self, ControlDef, GenericCaps, Shape, TypeDef};

use super::{ModeSelect, PowerToggle, Sender, Stepper};
use crate::components::card_frame::{CardFrame, open_entity};
use crate::format::{generic_value, value_class, word};
use crate::i18n::states::on_off;
use crate::i18n::{Key, use_text};

/// A control name as button text: `activate` becomes "Activate".
pub fn control_label(name: &str) -> String {
    word(name)
}

/// Whether an on/off control's field currently reads "on".
fn is_on(ctl: &ControlDef, state: &GenericState) -> bool {
    let Some(field) = ctl.field.as_deref() else {
        return false;
    };
    match state.fields.get(field) {
        Some(GenericValue::Bool(b)) => *b,
        Some(GenericValue::Text(s)) => ctl.on_word().is_some_and(|on| on == s),
        _ => false,
    }
}

trait OnWord {
    fn on_word(&self) -> Option<&str>;
}

impl OnWord for ControlDef {
    fn on_word(&self) -> Option<&str> {
        self.payload_on.as_ref().map(|w| w.value.as_str())
    }
}

/// The picture of a picture-primary entity, falling back to the type's glyph when it fails.
fn picture(entity: &EntityView) -> AnyView {
    let version = entity.state.last_update.unwrap_or(0);
    let src = format!(
        "/api/v1/entities/{}/picture?v={version}",
        crate::api::encode_id(&entity.id)
    );
    let broken = RwSignal::new(false);
    let glyph = crate::icons::entity_icon(entity);
    view! {
        {move || {
            if broken.get() {
                view! { <span class="generic__glyph" inner_html=glyph></span> }.into_any()
            } else {
                view! {
                    <img
                        class="generic__picture"
                        src=src.clone()
                        loading="lazy"
                        alt=""
                        on:error=move |_| broken.set(true)
                    />
                }
                    .into_any()
            }
        }}
    }
    .into_any()
}

/// The main control widget, chosen by the control's shape.
pub(crate) fn control_view(
    entity: &EntityView,
    def: &'static TypeDef,
    caps: &GenericCaps,
    name: &str,
    sender: Sender,
) -> AnyView {
    let Some(ctl) = def.controls.get(name) else {
        return ().into_any();
    };
    let state = match &entity.state.value {
        StateValue::Generic(g) => g.clone(),
        _ => GenericState::default(),
    };
    let lang = crate::i18n::use_lang();
    let t = use_text();
    let id = entity.id.clone();
    let control = name.to_owned();
    let send = move |value: Option<serde_json::Value>| {
        sender.send(
            &id,
            Command::Control {
                control: control.clone(),
                value,
            },
            None,
        );
    };
    if ctl.press {
        return view! {
            <button type="button" class="btn" on:click=move |_| send(None)>
                {control_label(name)}
            </button>
        }
        .into_any();
    }
    match ctl.shape {
        Shape::OnOff => {
            let on = is_on(ctl, &state);
            let toggle = Callback::new(move |()| send(Some(serde_json::Value::Bool(!on))));
            match &ctl.labels {
                Some(labels) => {
                    let text = if on { &labels.off } else { &labels.on }.clone();
                    view! {
                        <button type="button" class="btn" on:click=move |_| toggle.run(())>
                            {text}
                        </button>
                    }
                    .into_any()
                }
                None => view! {
                    <PowerToggle
                        on=on
                        label=Signal::derive(move || on_off(lang(), false, on))
                        on_toggle=toggle
                        aria_label=control_label(name)
                    />
                }
                .into_any(),
            }
        }
        Shape::Number => {
            let field = ctl.field.clone().unwrap_or_default();
            let range = caps.ranges.get(&field).cloned().unwrap_or_default();
            let step = if range.step > 0.0 { range.step } else { 1.0 };
            let current = match state.fields.get(&field) {
                Some(GenericValue::Number(n)) => Some(*n),
                _ => None,
            };
            let label = Signal::derive(move || {
                current.map_or_else(
                    || "—".to_owned(),
                    |n| crate::i18n::format::number(lang(), n),
                )
            });
            let on_step = Callback::new(move |dir: f64| {
                if let Some(n) = current {
                    let next = (n + dir * step).clamp(range.min, range.max.max(range.min));
                    send(serde_json::Number::from_f64(next).map(serde_json::Value::Number));
                }
            });
            view! { <Stepper label=label can_step=current.is_some() on_step=on_step /> }.into_any()
        }
        Shape::Choice => {
            let field = ctl.field.clone().unwrap_or_default();
            let options = caps.options.get(&field).cloned().unwrap_or_default();
            let selected = match state.fields.get(&field) {
                Some(GenericValue::Text(s)) => Some(s.clone()),
                _ => None,
            };
            let on_select =
                Callback::new(move |o: String| send(Some(serde_json::Value::String(o))));
            view! {
                <ModeSelect
                    label=control_label(name)
                    options=options
                    selected=selected
                    on_select=on_select
                />
            }
            .into_any()
        }
        Shape::Text => {
            let open = open_entity(&entity.id);
            view! {
                <button type="button" class="btn" on:click=move |_| open.run(())>
                    {move || t(Key::Edit)}
                </button>
            }
            .into_any()
        }
        Shape::Picture | Shape::Version => ().into_any(),
    }
}

#[component]
pub fn GenericCard(entity: EntityView) -> impl IntoView {
    let sender = Sender::from_context();
    let def = types::type_def(&entity.component);
    let caps = entity.capabilities.generic.clone();
    let lang = crate::i18n::use_lang();
    let value = {
        let caps = caps.clone();
        let state = entity.state.clone();
        Signal::derive(move || generic_value(lang(), def, caps.as_ref(), &state))
    };
    let is_alert = match (def, &entity.state.value) {
        (Some(d), StateValue::Generic(g)) => types::alerting(d, g),
        _ => false,
    };
    let primary_is_picture = def.is_some_and(|d| {
        d.primary
            .as_deref()
            .and_then(|p| d.fields.get(p))
            .is_some_and(|f| f.shape == Shape::Picture)
    });
    let control = match (def, caps.as_ref()) {
        (Some(d), Some(c)) => types::main_control(d, c, entity.ui.main_control.as_deref())
            .map(|name| control_view(&entity, d, c, name, sender)),
        _ => None,
    };
    let media = primary_is_picture.then(|| picture(&entity));
    let pending = sender.pending(&entity.id);
    let on_click = open_entity(&entity.id);
    view! {
        <CardFrame entity=entity on_click=on_click pending=pending alert=is_alert>
            {media}
            {move || {
                let text = value.get();
                (!text.is_empty())
                    .then(|| view! { <span class=value_class(&text)>{text.clone()}</span> })
            }}
            {control.map(|c| view! { <div class="card__controls">{c}</div> })}
        </CardFrame>
    }
}
