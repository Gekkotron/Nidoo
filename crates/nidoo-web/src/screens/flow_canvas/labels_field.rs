//! The flow's labels in the editor's title bar: chips with ✕ and an "add" box that offers the
//! existing labels and creates a new one from a typed name on Enter. Edits go to the draft (one
//! undo step each); read mode shows the chips alone.

use leptos::prelude::*;
use leptos::task::spawn_local;
use nidoo_api::LabelEdit;
use nidoo_core::flow::FlowLabelCatalog;

use super::Editor;
use crate::api::flow_labels::edit_flow_labels;
use crate::components::toast::Toasts;
use crate::flow_list::{label_by_name, label_color_name, new_label_id};
use crate::house::House;
use crate::i18n::flows::{FlowText, remove_label, use_flow_text};
use crate::i18n::use_lang;

/// The id of the editor's label suggestions.
const OPTIONS_ID: &str = "flow-label-options";

/// The chips of `labels` the catalog knows, in catalog order; with `remove`, each gets a ✕.
fn chips(
    catalog: &FlowLabelCatalog,
    labels: &[String],
    remove: Option<Callback<String>>,
) -> impl IntoView + use<> {
    let lang = use_lang();
    catalog
        .labels
        .iter()
        .filter(|l| labels.contains(&l.id))
        .map(|label| {
            let class = format!(
                "label-chip label-chip--{} label-chip--removable",
                label_color_name(label.color)
            );
            let id = label.id.clone();
            let name = label.name.clone();
            let cross = remove.map(|remove| {
                let id = id.clone();
                let name = StoredValue::new(name.clone());
                let word = move || name.with_value(|n| remove_label(lang(), n));
                view! {
                    <button
                        type="button"
                        class="label-chip__remove"
                        aria-label=word
                        title=word
                        on:click=move |_| remove.run(id.clone())
                    >
                        "✕"
                    </button>
                }
            });
            view! {
                <span class=class data-label=id>
                    <span class="label-chip__name">{label.name.clone()}</span>
                    {cross}
                </span>
            }
        })
        .collect_view()
}

/// The labels of the flow shown in read mode, from its saved definition.
#[component]
pub fn LabelChips(#[prop(into)] labels: Signal<Vec<String>>) -> impl IntoView {
    let house = expect_context::<House>();
    move || {
        let labels = labels.get();
        (!labels.is_empty()).then(|| {
            let chips = house.flow_labels.with(|v| chips(&v.catalog, &labels, None));
            view! { <span class="flow-labels flow-labels--read">{chips}</span> }
        })
    }
}

/// The editing field over `editor.draft.labels`.
#[component]
pub fn LabelsField(editor: Editor) -> impl IntoView {
    let house = expect_context::<House>();
    let toasts = expect_context::<Toasts>();
    let t = use_flow_text();
    let typed = RwSignal::new(String::new());
    // One undo step per change; the hub stores labels sorted, so the draft does too.
    let set_labels = move |mut labels: Vec<String>| {
        labels.sort();
        labels.dedup();
        if editor.draft.with_untracked(|f| f.labels == labels) {
            return;
        }
        editor
            .history
            .update_value(|h| h.record(editor.draft.get_untracked()));
        editor.draft.update(|f| f.labels = labels);
    };
    let add_id = move |id: String| {
        let mut labels = editor.draft.with_untracked(|f| f.labels.clone());
        labels.push(id);
        set_labels(labels);
    };
    let remove = Callback::new(move |id: String| {
        let mut labels = editor.draft.with_untracked(|f| f.labels.clone());
        labels.retain(|l| *l != id);
        set_labels(labels);
    });
    // A known name adds that label; with `create`, a new name makes the label first.
    let add = move |create: bool| {
        let name = typed.get_untracked().trim().to_owned();
        if name.is_empty() {
            return;
        }
        let known = house
            .flow_labels
            .with_untracked(|v| label_by_name(&v.catalog, &name).map(|l| l.id.clone()));
        if let Some(id) = known {
            typed.set(String::new());
            add_id(id);
            return;
        }
        if !create {
            return;
        }
        typed.set(String::new());
        let before = house.flow_labels.with_untracked(|v| v.catalog.clone());
        spawn_local(async move {
            match edit_flow_labels(&LabelEdit::CreateLabel { name, choice: None }).await {
                Ok(view) => {
                    let id = new_label_id(&before, &view.catalog);
                    house.flow_labels.set(view);
                    if let Some(id) = id {
                        add_id(id);
                    }
                }
                Err(e) => toasts.push(e.error),
            }
        });
    };
    let shown = move || {
        house.flow_labels.with(|v| {
            editor
                .draft
                .with(|f| chips(&v.catalog, &f.labels, Some(remove)))
        })
    };
    // The suggestions leave out the labels the flow already carries.
    let options = move || {
        house.flow_labels.with(|v| {
            editor.draft.with(|f| {
                v.catalog
                    .labels
                    .iter()
                    .filter(|l| !f.labels.contains(&l.id))
                    .map(|l| view! { <option value=l.name.clone()></option> })
                    .collect_view()
            })
        })
    };
    view! {
        <span class="flow-labels">
            {shown}
            <input
                type="text"
                class="input flow-labels__add"
                list=OPTIONS_ID
                aria-label=move || t(FlowText::Labels)
                placeholder=move || t(FlowText::AddLabelPlaceholder)
                autocomplete="off"
                prop:value=move || typed.get()
                on:input=move |ev| typed.set(event_target_value(&ev))
                // Picking a suggestion fires `change`; a typed name waits for Enter.
                on:change=move |_| add(false)
                on:keydown=move |ev: web_sys::KeyboardEvent| {
                    if ev.key() == "Enter" {
                        ev.prevent_default();
                        add(true);
                    }
                }
            />
            <datalist id=OPTIONS_ID>{options}</datalist>
        </span>
    }
}
