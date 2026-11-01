//! The notify card's title and body fields: a text input with a **＋ Value** button that inserts
//! an `{entity}` placeholder at the caret.

use leptos::prelude::*;

use nidoo_core::flow::template::{Reading, Template, TemplateError, parse, placeholder_for};

use crate::entity_picker::needs::{is_signal, keep};
use crate::entity_picker::{EntityPicker, Mode, Pinned};
use crate::house::House;
use crate::i18n::flows::{FlowText, use_flow_text};
use crate::i18n::use_lang;

/// `text` with `piece` inserted at `caret_utf16` (a browser caret, in UTF-16 units), and the
/// caret after `piece`, in UTF-16 units too. No caret, or one past the end, appends.
pub fn insert_at(text: &str, caret_utf16: Option<u32>, piece: &str) -> (String, u32) {
    let mut at = text.len();
    if let Some(caret) = caret_utf16 {
        let mut units = 0;
        for (i, c) in text.char_indices() {
            if units >= caret as usize {
                at = i;
                break;
            }
            units += c.len_utf16();
        }
    }
    let mut out = String::with_capacity(text.len() + piece.len());
    out.push_str(&text[..at]);
    out.push_str(piece);
    let caret = out.encode_utf16().count() as u32;
    out.push_str(&text[at..]);
    (out, caret)
}

/// A text field whose value takes placeholders; the button below it opens the entity picker
/// (with a pinned "Trigger value" row) and inserts the pick where the caret last was.
#[component]
pub fn TemplateField(
    label: Signal<String>,
    initial: String,
    placeholder: Signal<String>,
    optional: bool,
    write: Callback<Option<String>>,
    /// The flow's math results, offered first in the picker.
    #[prop(optional)]
    results: Option<Signal<Vec<String>>>,
) -> impl IntoView {
    let t = use_flow_text();
    let raw = RwSignal::new(initial);
    let caret = StoredValue::new(None::<u32>);
    let input = NodeRef::<leptos::html::Input>::new();
    let open = RwSignal::new(false);
    let send = move |text: &str| {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            if optional {
                write.run(None);
            }
        } else {
            write.run(Some(trimmed.to_owned()));
        }
    };
    let remember = move |_| {
        if let Some(el) = input.get_untracked() {
            caret.set_value(el.selection_start().ok().flatten());
        }
    };
    let on_input = move |ev: web_sys::Event| {
        let text = event_target_value(&ev);
        raw.set(text.clone());
        send(&text);
        remember(());
    };
    let on_pick = Callback::new(move |id: Option<String>| {
        // The pool only offers ids that read back, so the fallback never shows.
        let piece = match id {
            Some(id) => placeholder_for(&id).unwrap_or_else(|| format!("{{{id}}}")),
            None => "{trigger}".to_owned(),
        };
        let (text, at) = insert_at(&raw.get_untracked(), caret.get_value(), &piece);
        raw.set(text.clone());
        send(&text);
        caret.set_value(Some(at));
        // After the dialog has closed and handed focus back to the button.
        request_animation_frame(move || {
            if let Some(el) = input.get_untracked() {
                let _ = el.focus();
                let _ = el.set_selection_range(at, at);
            }
        });
    });
    view! {
        <label class="node-editor__field">
            <span class="node-editor__label">{move || label.get()}</span>
            <input
                type="text"
                class="field"
                node_ref=input
                placeholder=move || placeholder.get()
                spellcheck="false"
                autocomplete="off"
                prop:value=move || raw.get()
                on:input=on_input
                on:keyup=move |_| remember(())
                on:click=move |_| remember(())
                on:select=move |_| remember(())
            />
        </label>
        <button
            type="button"
            class="chip chip--add node-editor__add-value"
            aria-haspopup="dialog"
            on:click=move |_| open.set(true)
        >
            {move || t(FlowText::AddValue)}
        </button>
        <EntityPicker
            open=open
            title=label
            mode=Mode::Single
            current=Signal::derive(Vec::new)
            pool=Some(keep(|e| !is_signal(e) && placeholder_for(&e.id).is_some()))
            pinned=Some(Pinned {
                text: Signal::derive(move || t(FlowText::TriggerValueRow).to_owned()),
            })
            results=results
            on_pick=on_pick
        />
    }
}

/// The notify text as the phone would get it now: the house's live values, in the web app's
/// language. `{trigger}` stands for the current value of `trigger_entity` (the flow's first
/// state trigger). A text the hub would refuse shows its reason instead.
#[component]
pub fn NotifyPreview(
    title: Signal<String>,
    body: Signal<Option<String>>,
    trigger_entity: Signal<Option<String>>,
) -> impl IntoView {
    let t = use_flow_text();
    let lang = use_lang();
    let house = expect_context::<House>();
    let parsed = Memo::new(
        move |_| -> Result<(Template, Option<Template>), TemplateError> {
            let field = |name: &str, text: &str| {
                parse(text).map_err(|e| TemplateError {
                    reason: format!("{name}: {}", e.reason),
                })
            };
            let title = title.with(|text| field("title", text))?;
            let body =
                body.with(|text| text.as_deref().map(|text| field("body", text)).transpose())?;
            Ok((title, body))
        },
    );
    let filled = move || {
        let (title, body) = parsed.get()?;
        let lang = lang().core();
        Ok::<_, TemplateError>(house.entities.with(|entities| {
            let lookup = |id: &str| {
                let e = entities.get(id)?;
                Reading::of(
                    &e.component,
                    e.device_class.as_deref(),
                    e.unit.as_deref(),
                    &e.state.value,
                )
            };
            let trigger = trigger_entity.with(|id| id.as_deref().and_then(lookup));
            let fill = |t: &Template| t.render(lang, &lookup, trigger.as_ref());
            (fill(&title), body.as_ref().map(fill))
        }))
    };
    view! {
        <div class="node-editor__field node-editor__preview">
            <span class="node-editor__label">{move || t(FlowText::Preview)}</span>
            {move || match filled() {
                Ok((title, body)) => view! {
                    <p class="node-editor__preview-text">
                        <strong>{title}</strong>
                        {body.map(|body| view! { <br/>{body} })}
                    </p>
                }
                .into_any(),
                Err(err) => view! { <p class="node-editor__error">{err.reason}</p> }.into_any(),
            }}
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_at_counts_utf16_units() {
        assert_eq!(insert_at("Temp ", Some(5), "{t}"), ("Temp {t}".into(), 8));
        assert_eq!(insert_at("é!", Some(1), "{t}"), ("é{t}!".into(), 4));
        assert_eq!(insert_at("🌧 x", Some(2), "{r}"), ("🌧{r} x".into(), 5));
        assert_eq!(insert_at("ab", None, "{t}"), ("ab{t}".into(), 5));
        assert_eq!(insert_at("ab", Some(99), "{t}"), ("ab{t}".into(), 5));
    }
}
