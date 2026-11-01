//! The If card's value: a list of what the entity can report, a number box for a measurement,
//! or a text box, with "Other value…" for anything the list lacks.

use leptos::prelude::*;
use nidoo_api::EntityView;
use nidoo_core::flow::Scalar;
use serde_json::Value;

use crate::i18n::flows::{FlowText, flow_text};
use crate::i18n::format::parse_decimal;
use crate::i18n::use_lang;
use crate::if_value::{ValueInput, parse_other, value_input, value_input_of, with_current};

/// The option value of "Other value…" in the list.
const OTHER: &str = "other";

/// Which input is on screen; the field re-mounts only when this changes.
#[derive(Clone, Debug, PartialEq)]
enum Mode {
    List,
    Typed { back: bool },
    Number { unit: Option<String> },
}

/// A value as the user would type it: text bare, numbers and booleans as written.
fn typed_text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

#[component]
pub fn IfValueField(
    value: Signal<Option<Value>>,
    entity: Signal<Option<EntityView>>,
    write: Callback<Value>,
    /// The input to use when there is no entity: what a Manual card upstream carries.
    #[prop(optional, into)]
    fallback: Option<Signal<Option<ValueInput>>>,
    /// The value of `entity` compared: a field of its state, its headline value when absent.
    #[prop(optional, into)]
    field: Option<Signal<Option<String>>>,
) -> impl IntoView {
    let lang = use_lang();
    let other = RwSignal::new(false);
    let input = Memo::new(move |_| {
        let field = field.and_then(|f| f.get());
        entity.with(|e| match e {
            Some(_) => value_input_of(lang(), e.as_ref(), field.as_deref()),
            None => fallback
                .and_then(|f| f.get())
                .unwrap_or_else(|| value_input(lang(), None)),
        })
    });
    let mode = Memo::new(move |_| match input.get() {
        ValueInput::List(_) if other.get() => Mode::Typed { back: true },
        ValueInput::List(_) => Mode::List,
        ValueInput::Number { unit } => Mode::Number { unit },
        ValueInput::Text => Mode::Typed { back: false },
    });
    let choices = Memo::new(move |_| {
        let listed = value
            .with(|v| entity.with(|e| with_current(input.get(), v.as_ref(), lang(), e.as_ref())));
        match listed {
            ValueInput::List(list) => list,
            _ => Vec::new(),
        }
    });
    let field = move || {
        match mode.get() {
        Mode::List => view! {
            <select
                class="field"
                on:change=move |ev| {
                    let picked = event_target_value(&ev);
                    if picked == OTHER {
                        other.set(true);
                        return;
                    }
                    let chosen = picked
                        .parse::<usize>()
                        .ok()
                        .and_then(|i| choices.with(|l| l.get(i).map(|c| c.value.clone())));
                    if let Some(v) = chosen {
                        write.run(v);
                    }
                }
            >
                {move || value.with(Option::is_none).then(|| view! {
                    <option value="" disabled=true selected=true>
                        {move || flow_text(lang(), FlowText::ChooseValue)}
                    </option>
                })}
                {move || {
                    let current = value.get();
                    choices
                        .get()
                        .into_iter()
                        .enumerate()
                        .map(|(i, c)| {
                            let selected = current.as_ref() == Some(&c.value);
                            view! { <option value=i.to_string() selected=selected>{c.label}</option> }
                        })
                        .collect_view()
                }}
                <option value=OTHER>{move || flow_text(lang(), FlowText::OtherValue)}</option>
            </select>
        }
        .into_any(),
        Mode::Typed { back } => {
            let raw = RwSignal::new(value.with_untracked(|v| typed_text(v.as_ref())));
            view! {
                <input
                    type="text"
                    class="field"
                    spellcheck="false"
                    autocomplete="off"
                    prop:value=move || raw.get()
                    on:input=move |ev| {
                        let text = event_target_value(&ev);
                        raw.set(text.clone());
                        if let Some(v) = parse_other(&text) {
                            write.run(v);
                        }
                    }
                />
                {back.then(|| view! {
                    <button
                        type="button"
                        class="btn btn--ghost node-editor__back-to-list"
                        on:click=move |_| other.set(false)
                    >
                        {move || flow_text(lang(), FlowText::BackToList)}
                    </button>
                })}
            }
            .into_any()
        }
        Mode::Number { unit } => view! {
            <span class="command-field__number">
                <input
                    type="number"
                    class="field command-field__input"
                    step="any"
                    prop:value=move || {
                        value.with(|v| v.as_ref().and_then(Value::as_f64).map(|n| n.to_string()))
                            .unwrap_or_default()
                    }
                    on:change=move |ev| {
                        if let Some(n) = parse_decimal(event_target_value(&ev).trim()) {
                            write.run(Scalar::Number(n).to_json());
                        }
                    }
                />
                {unit.map(|u| view! { <span class="command-field__unit">{u}</span> })}
            </span>
        }
        .into_any(),
    }
    };
    view! {
        <label class="node-editor__field">
            <span class="node-editor__label">{move || flow_text(lang(), FlowText::ParamValue)}</span>
            {field}
        </label>
    }
}
