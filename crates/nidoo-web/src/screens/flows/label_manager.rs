//! The label manager: a dialog listing the choices with their labels, then the standalone
//! labels, each with its colour, name and flow count, edit (✎: rename, recolour, move into or
//! out of a choice) and delete (🗑, asked first; a label a flow's label action names lists those
//! flows). Every change is one catalog edit on the hub; the dialog follows the reply.

use leptos::prelude::*;
use leptos::task::spawn_local;
use nidoo_api::LabelEdit;
use nidoo_core::flow::{FlowChoice, FlowLabel, LabelColor};

use crate::api::flow_labels::edit_flow_labels;
use crate::components::popup::Popup;
use crate::components::toast::Toasts;
use crate::flow_list::{label_color_name, label_flow_counts, label_users};
use crate::house::House;
use crate::i18n::flows::{
    FlowText, delete_flow_prompt, flow_text, label_color_word, label_flow_count, label_used_by,
};
use crate::i18n::{Key, Lang, text, use_lang};

/// The `focus` that opens the manager on a new standalone label's name field.
pub const FOCUS_NEW_LABEL: &str = "new_label";

/// The one form open in the dialog, if any.
#[derive(Clone, Debug, PartialEq)]
enum Form {
    None,
    /// A new label, in this choice or standalone.
    NewLabel(Option<String>),
    NewChoice,
    EditLabel(String),
    EditChoice(String),
    DeleteLabel(String),
    DeleteChoice(String),
}

/// What every part of the dialog shares: the open form, its fields, and the edit runner.
#[derive(Clone, Copy)]
struct Ctx {
    form: RwSignal<Form>,
    /// The name being typed in the open form.
    name: RwSignal<String>,
    color: RwSignal<LabelColor>,
    /// The choice picked in a label's edit form ("" for none).
    choice: RwSignal<String>,
    busy: RwSignal<bool>,
    /// Sends the edits in order; the catalog follows each reply, and the form closes when all
    /// went through.
    run: Callback<Vec<LabelEdit>>,
    /// `choice:<id>` or `label:<id>`: the item the dialog was opened on.
    focus: StoredValue<Option<String>>,
}

impl Ctx {
    /// Opens `form`, its name field holding `name`.
    fn open(self, form: Form, name: &str) {
        self.name.set(name.to_owned());
        self.form.set(form);
        focus_field();
    }

    fn focused(self, key: &str) -> bool {
        self.focus.with_value(|f| f.as_deref() == Some(key))
    }
}

/// Puts the keyboard in the open form's name field, once it is drawn.
fn focus_field() {
    request_animation_frame(|| {
        let field = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.query_selector(".label-manager__field").ok().flatten());
        if let Some(el) = field
            && let Ok(input) = wasm_bindgen::JsCast::dyn_into::<web_sys::HtmlElement>(el)
        {
            let _ = input.focus();
        }
    });
}

/// A ✎ or 🗑 icon button.
fn icon_button(
    class: &'static str,
    label: impl Fn() -> &'static str + Copy + Send + Sync + 'static,
    glyph: &'static str,
    on_click: impl Fn() + 'static,
) -> impl IntoView {
    view! {
        <button
            type="button"
            class=format!("btn btn--ghost btn--icon {class}")
            aria-label=label
            title=label
            on:click=move |_| on_click()
        >
            {glyph}
        </button>
    }
}

/// The name field of a form: Enter submits, the text lives in `ctx.name`.
fn name_field(ctx: Ctx, lang: Lang, submit: impl Fn() + Copy + 'static) -> impl IntoView {
    view! {
        <input
            type="text"
            class="input label-manager__field"
            aria-label=flow_text(lang, FlowText::LabelNameField)
            placeholder=flow_text(lang, FlowText::LabelNameField)
            autocomplete="off"
            prop:value=move || ctx.name.get()
            on:input=move |ev| ctx.name.set(event_target_value(&ev))
            on:keydown=move |ev: web_sys::KeyboardEvent| {
                if ev.key() == "Enter" {
                    ev.prevent_default();
                    submit();
                }
            }
        />
    }
}

/// Cancel and the submit button of a form.
fn form_actions(
    ctx: Ctx,
    lang: Lang,
    submit_text: String,
    submit: impl Fn() + 'static,
) -> impl IntoView {
    view! {
        <div class="label-manager__form-actions">
            <button type="button" class="btn btn--ghost" on:click=move |_| ctx.form.set(Form::None)>
                {flow_text(lang, FlowText::Cancel)}
            </button>
            <button
                type="button"
                class="btn"
                disabled=move || ctx.busy.get()
                on:click=move |_| submit()
            >
                {submit_text}
            </button>
        </div>
    }
}

/// The form creating a label (in `choice`, or standalone) or a choice.
fn new_form(ctx: Ctx, lang: Lang, form: Form) -> AnyView {
    let submit = move || {
        let name = ctx.name.get_untracked().trim().to_owned();
        if name.is_empty() {
            return;
        }
        let edit = match ctx.form.get_untracked() {
            Form::NewLabel(choice) => LabelEdit::CreateLabel { name, choice },
            Form::NewChoice => LabelEdit::CreateChoice { name },
            _ => return,
        };
        ctx.run.run(vec![edit]);
    };
    let kind = match form {
        Form::NewChoice => "choice",
        _ => "label",
    };
    view! {
        <div class="label-manager__new label-manager__form" data-new=kind>
            {name_field(ctx, lang, submit)}
            {form_actions(ctx, lang, flow_text(lang, FlowText::CreateLabelButton).to_owned(), submit)}
        </div>
    }
    .into_any()
}

/// A label's edit form: name, colour and choice; Save sends only what changed.
fn label_form(ctx: Ctx, lang: Lang, label: &FlowLabel, choices: &[FlowChoice]) -> AnyView {
    let original = label.clone();
    let swatches = LabelColor::ALL
        .into_iter()
        .map(|color| {
            let class = format!(
                "label-manager__swatch label-chip--{}",
                label_color_name(color)
            );
            view! {
                <button
                    type="button"
                    role="radio"
                    class=class
                    aria-checked=move || (ctx.color.get() == color).to_string()
                    aria-label=label_color_word(lang, color)
                    title=label_color_word(lang, color)
                    on:click=move |_| ctx.color.set(color)
                ></button>
            }
        })
        .collect_view();
    let options = choices
        .iter()
        .map(|c| {
            let id = c.id.clone();
            let selected = original.choice.as_deref() == Some(c.id.as_str());
            view! { <option value=id selected=selected>{c.name.clone()}</option> }
        })
        .collect_view();
    let submit = {
        let original = original.clone();
        move || {
            let name = ctx.name.get_untracked().trim().to_owned();
            let color = ctx.color.get_untracked();
            let choice = Some(ctx.choice.get_untracked()).filter(|c| !c.is_empty());
            let id = original.id.clone();
            let mut edits = Vec::new();
            if !name.is_empty() && name != original.name {
                edits.push(LabelEdit::RenameLabel {
                    id: id.clone(),
                    name,
                });
            }
            if color != original.color {
                edits.push(LabelEdit::RecolorLabel {
                    id: id.clone(),
                    color,
                });
            }
            if choice != original.choice {
                edits.push(LabelEdit::MoveLabel { id, choice });
            }
            if edits.is_empty() {
                ctx.form.set(Form::None);
            } else {
                ctx.run.run(edits);
            }
        }
    };
    let submit = StoredValue::new(submit);
    let submit = move || submit.with_value(|f| f());
    view! {
        <div class="label-manager__form" data-label=original.id.clone()>
            {name_field(ctx, lang, submit)}
            <div
                class="label-manager__swatches"
                role="radiogroup"
                aria-label=flow_text(lang, FlowText::LabelColorField)
            >
                {swatches}
            </div>
            <select
                class="select label-manager__choice"
                aria-label=flow_text(lang, FlowText::NewChoice)
                on:change=move |ev| ctx.choice.set(event_target_value(&ev))
            >
                <option value="" selected=original.choice.is_none()>
                    {flow_text(lang, FlowText::NoChoice)}
                </option>
                {options}
            </select>
            {form_actions(ctx, lang, flow_text(lang, FlowText::Save).to_owned(), submit)}
        </div>
    }
    .into_any()
}

/// The inline "Delete …?" with Cancel and Delete; `note` explains what follows.
fn confirm(ctx: Ctx, lang: Lang, name: &str, note: Option<String>, edit: LabelEdit) -> AnyView {
    let edit = StoredValue::new(edit);
    view! {
        <div class="label-manager__confirm" role="alertdialog">
            <p class="label-manager__prompt">
                <strong>{delete_flow_prompt(lang, name)}</strong>
                {note.map(|n| view! { " " {n} })}
            </p>
            <div class="label-manager__form-actions">
                <button type="button" class="btn btn--ghost" on:click=move |_| ctx.form.set(Form::None)>
                    {flow_text(lang, FlowText::Cancel)}
                </button>
                <button
                    type="button"
                    class="btn btn--danger"
                    disabled=move || ctx.busy.get()
                    on:click=move |_| ctx.run.run(vec![edit.get_value()])
                >
                    {text(lang, Key::Delete)}
                </button>
            </div>
        </div>
    }
    .into_any()
}

/// One label: its line, or its edit form, or its line over the delete question.
fn label_line(
    ctx: Ctx,
    lang: Lang,
    form: &Form,
    label: &FlowLabel,
    count: usize,
    choices: &[FlowChoice],
    users: Vec<String>,
) -> AnyView {
    let id = label.id.clone();
    if *form == Form::EditLabel(id.clone()) {
        return label_form(ctx, lang, label, choices);
    }
    let dot = format!(
        "label-manager__dot label-chip--{}",
        label_color_name(label.color)
    );
    let edit = {
        let label = label.clone();
        move || {
            ctx.color.set(label.color);
            ctx.choice.set(label.choice.clone().unwrap_or_default());
            ctx.open(Form::EditLabel(label.id.clone()), &label.name);
        }
    };
    let delete = {
        let id = id.clone();
        move || ctx.form.set(Form::DeleteLabel(id.clone()))
    };
    let asking = (*form == Form::DeleteLabel(id.clone())).then(|| {
        let note = (!users.is_empty()).then(|| label_used_by(lang, &users));
        confirm(
            ctx,
            lang,
            &label.name,
            note,
            LabelEdit::DeleteLabel { id: id.clone() },
        )
    });
    let focused = ctx.focused(&format!("label:{id}"));
    view! {
        <div class="label-manager__line" class:label-manager__line--focus=focused data-label=id>
            <span class=dot aria-hidden="true"></span>
            <span class="label-manager__name">{label.name.clone()}</span>
            <span class="label-manager__count">{label_flow_count(lang, count)}</span>
            {icon_button("label-manager__edit", move || flow_text(lang, FlowText::EditLabel), "✎", edit)}
            {icon_button("label-manager__delete", move || text(lang, Key::Delete), "🗑", delete)}
        </div>
        {asking}
    }
    .into_any()
}

/// What the dialog draws from: the catalog, the flow counts and the label-action users.
struct Snapshot {
    labels: Vec<FlowLabel>,
    choices: Vec<FlowChoice>,
    counts: std::collections::BTreeMap<String, usize>,
    flows: Vec<nidoo_api::FlowView>,
}

impl Snapshot {
    fn line(&self, ctx: Ctx, lang: Lang, form: &Form, label: &FlowLabel) -> AnyView {
        let count = self.counts.get(&label.id).copied().unwrap_or(0);
        // Only the label being deleted needs its users.
        let users = if *form == Form::DeleteLabel(label.id.clone()) {
            label_users(&self.flows, &label.id)
        } else {
            Vec::new()
        };
        label_line(ctx, lang, form, label, count, &self.choices, users)
    }
}

/// A choice: its title line (or rename form, or delete question), its labels, and "＋ Label in
/// this choice".
fn choice_group(
    ctx: Ctx,
    lang: Lang,
    form: &Form,
    snap: &Snapshot,
    choice: &FlowChoice,
) -> AnyView {
    let id = choice.id.clone();
    let members: Vec<&FlowLabel> = snap
        .labels
        .iter()
        .filter(|l| l.choice.as_deref() == Some(id.as_str()))
        .collect();
    let head = if *form == Form::EditChoice(id.clone()) {
        let submit = {
            let id = id.clone();
            let original = choice.name.clone();
            move || {
                let name = ctx.name.get_untracked().trim().to_owned();
                if name.is_empty() || name == original {
                    ctx.form.set(Form::None);
                    return;
                }
                ctx.run.run(vec![LabelEdit::RenameChoice {
                    id: id.clone(),
                    name,
                }]);
            }
        };
        let submit = StoredValue::new(submit);
        let submit = move || submit.with_value(|f| f());
        view! {
            <div class="label-manager__form">
                {name_field(ctx, lang, submit)}
                {form_actions(ctx, lang, flow_text(lang, FlowText::Save).to_owned(), submit)}
            </div>
        }
        .into_any()
    } else {
        let rename = {
            let id = id.clone();
            let name = choice.name.clone();
            move || ctx.open(Form::EditChoice(id.clone()), &name)
        };
        let delete = {
            let id = id.clone();
            move || ctx.form.set(Form::DeleteChoice(id.clone()))
        };
        let hint = if members.len() < 2 {
            flow_text(lang, FlowText::ChoiceIncomplete)
        } else {
            flow_text(lang, FlowText::ChoiceOneActive)
        };
        let asking = (*form == Form::DeleteChoice(id.clone())).then(|| {
            confirm(
                ctx,
                lang,
                &choice.name,
                Some(flow_text(lang, FlowText::ChoiceDeleteNote).to_owned()),
                LabelEdit::DeleteChoice { id: id.clone() },
            )
        });
        view! {
            <div class="label-manager__line label-manager__line--choice">
                <span class="label-manager__choice-name">{choice.name.clone()}</span>
                <span class="label-manager__hint">{hint}</span>
                {icon_button("label-manager__edit", move || flow_text(lang, FlowText::EditLabel), "✎", rename)}
                {icon_button("label-manager__delete-choice", move || text(lang, Key::Delete), "🗑", delete)}
            </div>
            {asking}
        }
        .into_any()
    };
    let lines = members
        .iter()
        .map(|label| snap.line(ctx, lang, form, label))
        .collect_view();
    let adding = *form == Form::NewLabel(Some(id.clone()));
    let add = {
        let id = id.clone();
        move |_| ctx.open(Form::NewLabel(Some(id.clone())), "")
    };
    let focused = ctx.focused(&format!("choice:{id}"));
    view! {
        <section class="label-manager__group" class:label-manager__group--focus=focused data-choice=id>
            {head}
            {lines}
            {if adding {
                new_form(ctx, lang, form.clone())
            } else {
                view! {
                    <button type="button" class="btn btn--ghost label-manager__add-in-choice" on:click=add>
                        "＋ "
                        {flow_text(lang, FlowText::AddLabelInChoice)}
                    </button>
                }
                .into_any()
            }}
        </section>
    }
    .into_any()
}

#[component]
pub fn LabelManager(focus: Option<String>, on_close: Callback<()>) -> impl IntoView {
    let house = expect_context::<House>();
    let toasts = expect_context::<Toasts>();
    let lang = use_lang();
    let open = RwSignal::new(true);
    Effect::new(move |_| {
        if !open.get() {
            on_close.run(());
        }
    });
    let start = if focus.as_deref() == Some(FOCUS_NEW_LABEL) {
        Form::NewLabel(None)
    } else {
        Form::None
    };
    let form = RwSignal::new(start.clone());
    let busy = RwSignal::new(false);
    let run = Callback::new(move |edits: Vec<LabelEdit>| {
        if busy.get_untracked() {
            return;
        }
        busy.set(true);
        spawn_local(async move {
            let mut ok = true;
            for edit in edits {
                match edit_flow_labels(&edit).await {
                    Ok(view) => house.set_flow_labels(view),
                    Err(e) => {
                        toasts.push(e.error);
                        ok = false;
                        break;
                    }
                }
            }
            busy.try_set(false);
            if ok {
                form.try_set(Form::None);
            }
        });
    });
    let ctx = Ctx {
        form,
        name: RwSignal::new(String::new()),
        color: RwSignal::new(LabelColor::default()),
        choice: RwSignal::new(String::new()),
        busy,
        run,
        focus: StoredValue::new(focus),
    };
    if start != Form::None {
        focus_field();
    }
    // The item the dialog opened on, scrolled into sight once drawn.
    let root = NodeRef::<leptos::html::Div>::new();
    Effect::new(move |done: Option<bool>| {
        if done == Some(true) {
            return true;
        }
        let Some(el) = root.get() else { return false };
        if let Ok(Some(target)) =
            el.query_selector(".label-manager__group--focus, .label-manager__line--focus")
        {
            target.scroll_into_view();
        }
        true
    });

    let body = move || {
        let lang = lang();
        let form = form.get();
        let snap = Snapshot {
            labels: house.flow_labels.with(|v| v.catalog.labels.clone()),
            choices: house.flow_labels.with(|v| v.catalog.choices.clone()),
            counts: house.flows.with(|f| label_flow_counts(f)),
            flows: house.flows.get(),
        };
        let groups = snap
            .choices
            .iter()
            .map(|c| choice_group(ctx, lang, &form, &snap, c))
            .collect_view();
        // A label whose choice is gone counts as standalone.
        let standalone: Vec<&FlowLabel> = snap
            .labels
            .iter()
            .filter(|l| {
                l.choice
                    .as_deref()
                    .is_none_or(|c| snap.choices.iter().all(|x| x.id != c))
            })
            .collect();
        let lines = standalone
            .iter()
            .map(|l| snap.line(ctx, lang, &form, l))
            .collect_view();
        let bottom_form = matches!(form, Form::NewLabel(None) | Form::NewChoice)
            .then(|| new_form(ctx, lang, form.clone()));
        view! {
            {groups}
            <section class="label-manager__group label-manager__group--standalone">
                <h3 class="label-manager__heading">
                    {flow_text(lang, FlowText::StandaloneLabels)}
                </h3>
                {lines}
            </section>
            {bottom_form}
            <div class="label-manager__actions">
                <button
                    type="button"
                    class="btn btn--ghost"
                    on:click=move |_| ctx.open(Form::NewLabel(None), "")
                >
                    "＋ "
                    {flow_text(lang, FlowText::GroupLabel)}
                </button>
                <button
                    type="button"
                    class="btn btn--ghost"
                    on:click=move |_| ctx.open(Form::NewChoice, "")
                >
                    "＋ "
                    {flow_text(lang, FlowText::NewChoice)}
                </button>
            </div>
        }
    };
    let title = Signal::derive(move || flow_text(lang(), FlowText::Labels).to_owned());
    view! {
        <Popup open=open title=title>
            <div class="label-manager" node_ref=root>
                {body}
            </div>
        </Popup>
    }
}
