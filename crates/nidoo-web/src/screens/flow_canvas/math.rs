//! The settings of a math card: its inputs (a name, an entity and an optional average window
//! each), the formula with its live error or preview, the result's name and the number entity
//! the result is saved to. While the card is invalid Save is blocked, with the reason under the
//! formula.

use leptos::prelude::*;
use nidoo_core::flow::{
    FUNCTIONS, Flow, MAX_INPUTS, MathInput, MathParams, NodeKind, Scalar, is_valid_name, math_names,
};
use nidoo_core::model::Kind;

use super::settings::{Form, word};
use crate::entity_picker::EntityPickField;
use crate::entity_picker::needs::keep;
use crate::flow_math::{
    Preview, accept, char_offset, function_snippet, input_name_for, insert_at, is_placeholder_name,
    preview, suggestions, utf16_offset,
};
use crate::format::entity_title;
use crate::house::House;
use crate::i18n::flows::{
    FlowText, function_help, math_averaged_note, math_fails, math_unknown, math_value,
    use_flow_text,
};
use crate::i18n::use_lang;
use crate::screens::flow_schedule::SaveBlocked;

/// The average windows the select offers; any other window the card holds is added to the list.
const WINDOWS: [&str; 4] = ["1m", "5m", "15m", "1h"];

/// The math params of node `id` in the draft.
fn params_of(form: Form, id: &str) -> Option<MathParams> {
    form.editor
        .draft
        .with(|f| match f.node(id).map(|n| &n.kind) {
            Some(NodeKind::Math(p)) => Some(p.clone()),
            _ => None,
        })
}

/// The math params of node `id` in `flow`.
fn math_mut<'a>(flow: &'a mut Flow, id: &str) -> Option<&'a mut MathParams> {
    flow.nodes
        .iter_mut()
        .find(|n| n.id == id)
        .and_then(|n| match &mut n.kind {
            NodeKind::Math(p) => Some(p),
            _ => None,
        })
}

/// Applies `mutate` to the math params of node `id`.
fn update(form: Form, id: &str, mutate: impl FnOnce(&mut MathParams)) {
    form.update(id, |n| {
        if let NodeKind::Math(ref mut p) = n.kind {
            mutate(p);
        }
    });
}

pub(super) fn math_fields(form: Form, id: String) -> impl IntoView {
    let t = use_flow_text();
    let lang = use_lang();
    let draft = form.editor.draft;
    let house = expect_context::<House>();

    // The preview follows the draft and the house's readings.
    let shown = Memo::new({
        let id = id.clone();
        move |_| {
            house.entities.with(|entities| {
                let value = |entity: &str| {
                    entities
                        .get(entity)
                        .and_then(|e| Scalar::of_state(&e.state.value))
                };
                draft.with(|f| match f.node(&id).map(|n| &n.kind) {
                    Some(NodeKind::Math(p)) => Some(preview(&id, p, f, &value)),
                    _ => None,
                })
            })
        }
    });
    // Save waits for a formula the hub would accept; a failure with today's readings
    // (`Preview::Fails`) does not block it.
    if let Some(blocked) = use_context::<SaveBlocked>() {
        Effect::new(move |_| {
            blocked
                .0
                .set(matches!(shown.get(), Some(Preview::Error(_))));
        });
        on_cleanup(move || blocked.0.set(false));
    }

    // Rows are rebuilt when the draft's number of inputs changes (add, remove, undo, redo), not
    // on every edit; each row reads its fields from the draft.
    // A memo, so the rows are rebuilt only when the number changes, not on every draft edit.
    let count = Memo::new({
        let id = id.clone();
        move |_| params_of(form, &id).map_or(0, |p| p.inputs.len())
    });
    let rows = {
        let id = id.clone();
        move || {
            let n = count.get();
            untrack(|| {
                (0..n)
                    .map(|i| input_row(form, id.clone(), i))
                    .collect_view()
            })
        }
    };
    let add_id = id.clone();
    let add = move |_: web_sys::MouseEvent| {
        form.editor.edit(|f| {
            if let Some(p) = math_mut(f, &add_id) {
                let taken: Vec<&str> = p.inputs.iter().map(|i| i.name.as_str()).collect();
                let name = input_name_for("input", &taken);
                p.inputs.push(MathInput {
                    name,
                    entity: String::new(),
                    average: None,
                    extra: Default::default(),
                });
            }
        });
    };

    // Picking functions and names: the formula's caret (in chars), the suggestion list and the
    // Functions menu. None of it touches the input rows.
    let formula_ref = NodeRef::<leptos::html::Input>::new();
    let caret = RwSignal::new(None::<usize>);
    let dismissed = RwSignal::new(false);
    let active = RwSignal::new(0usize);
    let menu_open = RwSignal::new(false);
    let formula = Signal::derive({
        let id = id.clone();
        move || params_of(form, &id).map(|p| p.formula).unwrap_or_default()
    });
    let remember_caret = move || {
        if let Some(el) = formula_ref.get_untracked() {
            let at = el
                .selection_start()
                .ok()
                .flatten()
                .map(|units| char_offset(&el.value(), units));
            caret.set(at);
        }
    };
    // The names the formula can read: the card's inputs (`chips`), then the other math cards'
    // results (`names`); `trigger` ends both.
    let chips = Memo::new({
        let id = id.clone();
        move |_| {
            let mut names: Vec<String> = params_of(form, &id)
                .map(|p| p.inputs.into_iter().map(|i| i.name).collect())
                .unwrap_or_default();
            names.retain(|n| !n.is_empty());
            names.push("trigger".to_owned());
            names
        }
    });
    let names = Memo::new({
        let id = id.clone();
        move |_| {
            let mut names = chips.get();
            let own = params_of(form, &id).map(|p| p.name).unwrap_or_default();
            draft.with(|f| {
                names.extend(
                    math_names(f)
                        .into_iter()
                        .filter(|n| *n != own)
                        .map(str::to_owned),
                );
            });
            names
        }
    });
    let proposals = Memo::new(move |_| {
        let at = caret.get();
        if dismissed.get() {
            return Vec::new();
        }
        let Some(at) = at else { return Vec::new() };
        let names = names.get();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        suggestions(&formula.get(), at, &refs)
    });
    Effect::new(move |_| {
        proposals.track();
        active.set(0);
    });
    // Writes the edited formula and puts the selection where the helper says, once the field
    // has taken the new value.
    let apply = {
        let id = id.clone();
        Callback::new(move |(text, from, to): (String, usize, usize)| {
            let (a, b) = (utf16_offset(&text, from), utf16_offset(&text, to));
            update(form, &id, |p| p.formula = text);
            caret.set(Some(to));
            dismissed.set(true);
            menu_open.set(false);
            request_animation_frame(move || {
                if let Some(el) = formula_ref.get_untracked() {
                    let _ = el.focus();
                    let _ = el.set_selection_range(a, b);
                }
            });
        })
    };
    let insert = Callback::new(move |snippet: String| {
        let text = formula.get_untracked();
        let at = caret
            .get_untracked()
            .unwrap_or_else(|| text.chars().count());
        apply.run(insert_at(&text, at, &snippet));
    });
    let accept_at = Callback::new(move |index: usize| {
        let text = formula.get_untracked();
        let at = caret
            .get_untracked()
            .unwrap_or_else(|| text.chars().count());
        if let Some(s) = proposals.get_untracked().get(index) {
            apply.run(accept(&text, at, s));
        }
    });
    let on_key = move |ev: web_sys::KeyboardEvent| {
        let len = proposals.get_untracked().len();
        if len == 0 {
            return;
        }
        match ev.key().as_str() {
            "ArrowDown" => {
                ev.prevent_default();
                active.update(|a| *a = (*a + 1) % len);
            }
            "ArrowUp" => {
                ev.prevent_default();
                active.update(|a| *a = (*a + len - 1) % len);
            }
            _ if matches!(ev.key().to_lowercase().as_str(), "tab" | "enter") => {
                ev.prevent_default();
                accept_at.run(active.get_untracked());
            }
            "Escape" => {
                // Closes the list, not the drawer.
                ev.prevent_default();
                ev.stop_propagation();
                dismissed.set(true);
            }
            _ => {}
        }
    };

    let formula_id = id.clone();
    let on_formula = move |ev: web_sys::Event| {
        let text = event_target_value(&ev);
        update(form, &formula_id, |p| p.formula = text);
        // Typing reopens the suggestions Escape or an insertion had closed.
        dismissed.set(false);
        remember_caret();
    };
    let window_note = {
        let id = id.clone();
        move || {
            params_of(form, &id)
                .and_then(|p| p.inputs.into_iter().find_map(|i| i.average))
                .unwrap_or_default()
        }
    };
    let line = move || {
        shown.get().map(|preview| match preview {
            Preview::Error(reason) => {
                view! { <p class="node-editor__error math-line" role="alert">{reason}</p> }
                    .into_any()
            }
            Preview::Fails(reason) => view! {
                <p class="math-line math-line--fails" role="status">
                    {math_fails(lang(), &reason)}
                </p>
            }
            .into_any(),
            Preview::Unknown(name) => view! {
                <p class="node-editor__hint math-line">{math_unknown(lang(), &name)}</p>
            }
            .into_any(),
            Preview::Value {
                value,
                averaged_now,
            } => view! {
                <p class="math-line math-line--value" role="status">
                    <span class="math-line__value">"= " {math_value(lang(), value)}</span>
                    {averaged_now
                        .then(|| {
                            view! {
                                <span class="node-editor__hint">
                                    {math_averaged_note(lang(), &window_note())}
                                </span>
                            }
                        })}
                </p>
            }
            .into_any(),
        })
    };

    let name_id = id.clone();
    let on_name = move |ev: web_sys::Event| {
        let text = event_target_value(&ev);
        update(form, &name_id, |p| p.name = text);
    };
    let result_name = Signal::derive({
        let id = id.clone();
        move || params_of(form, &id).map(|p| p.name).unwrap_or_default()
    });
    let save_to = Signal::derive({
        let id = id.clone();
        move || {
            params_of(form, &id)
                .and_then(|p| p.save_to)
                .filter(|e| !e.is_empty())
        }
    });
    let save_id = id.clone();
    let on_save_to = Callback::new(move |value: Option<String>| {
        update(form, &save_id, |p| p.save_to = value);
    });
    let name_hint = move || {
        let name = result_name.get();
        if is_valid_name(&name) {
            view! {
                <span class="node-editor__hint">
                    {t(FlowText::MathResultHint).replace("{name}", &format!("{{{name}}}"))}
                </span>
            }
            .into_any()
        } else {
            view! { <span class="node-editor__error">{t(FlowText::MathNameInvalid)}</span> }
                .into_any()
        }
    };

    view! {
        <p class="node-editor__hint">{move || t(FlowText::MathSub)}"."</p>
        <div class="node-editor__field math-inputs">
            <span class="node-editor__label">{move || t(FlowText::MathInputs)}</span>
            {rows}
            <button
                type="button"
                class="btn btn--ghost math-inputs__add"
                disabled=move || { count.get() >= MAX_INPUTS }
                on:click=add
            >
                {move || t(FlowText::MathAddInput)}
            </button>
            <span class="node-editor__hint">{move || t(FlowText::MathAverageNote)}</span>
        </div>
        <div class="node-editor__field math-formula-box">
            <label class="math-formula-label">
                <span class="node-editor__label">{move || t(FlowText::MathFormula)}</span>
                <input
                    type="text"
                    class="field math-formula"
                    node_ref=formula_ref
                    spellcheck="false"
                    autocomplete="off"
                    prop:value=move || formula.get()
                    on:input=on_formula
                    on:keydown=on_key
                    on:keyup=move |_| remember_caret()
                    on:click=move |_| remember_caret()
                    on:select=move |_| remember_caret()
                    on:blur=move |_| dismissed.set(true)
                />
            </label>
            <Show when=move || !proposals.with(Vec::is_empty)>
                <ul
                    class="math-suggestions"
                    role="listbox"
                    aria-label=move || t(FlowText::MathSuggestions)
                >
                    {move || {
                        proposals
                            .get()
                            .into_iter()
                            .enumerate()
                            .map(|(i, s)| {
                                view! {
                                    <li
                                        class="math-suggestions__item"
                                        class:math-suggestions__item--function=s.is_function
                                        class:math-suggestions__item--active=move || active.get() == i
                                        role="option"
                                        aria-selected=move || (active.get() == i).to_string()
                                        on:mousedown=|ev| ev.prevent_default()
                                        on:click=move |_| accept_at.run(i)
                                    >
                                        {s.label}
                                    </li>
                                }
                            })
                            .collect_view()
                    }}
                </ul>
            </Show>
            <div class="math-chips chip-row" aria-label=move || t(FlowText::MathNames)>
                {move || {
                    chips
                        .get()
                        .into_iter()
                        .map(|name| {
                            let piece = name.clone();
                            view! {
                                <button
                                    type="button"
                                    class="chip math-chips__name"
                                    on:mousedown=|ev| ev.prevent_default()
                                    on:click=move |_| insert.run(piece.clone())
                                >
                                    {name}
                                </button>
                            }
                        })
                        .collect_view()
                }}
                <button
                    type="button"
                    class="chip math-functions__toggle"
                    aria-haspopup="true"
                    aria-expanded=move || menu_open.get().to_string()
                    on:mousedown=|ev| ev.prevent_default()
                    on:click=move |_| menu_open.update(|o| *o = !*o)
                >
                    {move || t(FlowText::MathFunctions)}
                </button>
            </div>
            <Show when=move || menu_open.get()>
                <ul class="math-functions" role="menu">
                    {FUNCTIONS
                        .iter()
                        .map(|f| {
                            view! {
                                <li role="none">
                                    <button
                                        type="button"
                                        role="menuitem"
                                        class="math-functions__item"
                                        on:mousedown=|ev| ev.prevent_default()
                                        on:click=move |_| insert.run(function_snippet(f))
                                    >
                                        <code class="math-functions__signature">{f.signature}</code>
                                        <span class="node-editor__hint">
                                            {move || function_help(lang(), f.name)}
                                        </span>
                                    </button>
                                </li>
                            }
                        })
                        .collect_view()}
                </ul>
            </Show>
            {line}
        </div>
        <label class="node-editor__field">
            <span class="node-editor__label">{move || t(FlowText::MathResultName)}</span>
            <input
                type="text"
                class="field math-name"
                spellcheck="false"
                autocomplete="off"
                prop:value=move || result_name.get()
                aria-invalid=move || (!is_valid_name(&result_name.get())).then_some("true")
                on:input=on_name
            />
            {name_hint}
        </label>
        <EntityPickField
            label=word(FlowText::MathSaveTo)
            value=save_to
            on_change=on_save_to
            pool=Some(keep(|e| e.kind == Kind::Number))
            pinned=Some(word(FlowText::MathNotSaved))
            compact=true
            class=Some("node-editor__field".to_owned())
        />
    }
}

/// One input row: its name, the entity it reads, the window it averages over and a remove button.
fn input_row(form: Form, id: String, index: usize) -> impl IntoView {
    let t = use_flow_text();
    let lang = use_lang();
    let house = expect_context::<House>();
    let current = move |id: &str| params_of(form, id).and_then(|p| p.inputs.get(index).cloned());
    let window = Signal::derive({
        let id = id.clone();
        move || current(&id).and_then(|i| i.average)
    });
    // A window the list lacks (the hub accepts any up to 24h) is listed too.
    let extra = window
        .get_untracked()
        .filter(|w| !WINDOWS.contains(&w.as_str()))
        .into_iter();

    let name = Signal::derive({
        let id = id.clone();
        move || current(&id).map(|i| i.name).unwrap_or_default()
    });
    let entity = Signal::derive({
        let id = id.clone();
        move || current(&id).map(|i| i.entity).filter(|e| !e.is_empty())
    });
    let name_id = id.clone();
    let on_name = move |ev: web_sys::Event| {
        let text = event_target_value(&ev);
        update(form, &name_id, |p| {
            if let Some(input) = p.inputs.get_mut(index) {
                input.name = text;
            }
        });
    };
    // The first entity chosen for a row still named `input…` gives it a name drawn from the
    // entity (`Réseau` becomes `reseau`); later changes leave the name alone, since the
    // formula may use it.
    let entity_id = id.clone();
    let on_entity = Callback::new(move |value: Option<String>| {
        let title = value.as_ref().and_then(|e| {
            house
                .entities
                .with_untracked(|m| m.get(e).map(|e| entity_title(lang(), e)))
        });
        update(form, &entity_id, |p| {
            let others: Vec<String> = p
                .inputs
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != index)
                .map(|(_, input)| input.name.clone())
                .collect();
            let Some(input) = p.inputs.get_mut(index) else {
                return;
            };
            let first_pick = input.entity.is_empty();
            input.entity = value.unwrap_or_default();
            if let (true, true, Some(title)) = (first_pick, is_placeholder_name(&input.name), title)
            {
                let taken: Vec<&str> = others.iter().map(String::as_str).collect();
                input.name = input_name_for(&title, &taken);
            }
        });
    });
    let window_id = id.clone();
    let on_window = move |ev: web_sys::Event| {
        let value = event_target_value(&ev);
        update(form, &window_id, |p| {
            if let Some(input) = p.inputs.get_mut(index) {
                input.average = (value != "now").then_some(value);
            }
        });
    };
    let remove_id = id;
    let remove = move |_: web_sys::MouseEvent| {
        form.editor.edit(|f| {
            if let Some(p) = math_mut(f, &remove_id).filter(|p| index < p.inputs.len()) {
                p.inputs.remove(index);
            }
        });
    };

    let windows = WINDOWS.iter().map(|w| (*w).to_owned()).chain(extra);
    view! {
        <div class="math-input">
            <input
                type="text"
                class="field math-input__name"
                spellcheck="false"
                autocomplete="off"
                aria-label=move || t(FlowText::MathInputName)
                prop:value=name
                aria-invalid=move || (!is_valid_name(&name.get())).then_some("true")
                on:input=on_name
            />
            <EntityPickField
                label=word(FlowText::Entity)
                value=entity
                on_change=on_entity
                compact=true
                class=Some("math-input__entity".to_owned())
            />
            <select class="field math-input__window" on:change=on_window>
                <option value="now" selected=move || window.get().is_none()>
                    {move || t(FlowText::MathNow)}
                </option>
                {windows
                    .map(|w| {
                        let mine = w.clone();
                        let chosen = move || window.get().as_deref() == Some(mine.as_str());
                        view! { <option value=w.clone() selected=chosen>{w.clone()}</option> }
                    })
                    .collect_view()}
            </select>
            <button
                type="button"
                class="btn btn--ghost btn--icon math-input__remove"
                aria-label=move || t(FlowText::MathRemoveInput)
                on:click=remove
            >
                "✕"
            </button>
        </div>
    }
}
