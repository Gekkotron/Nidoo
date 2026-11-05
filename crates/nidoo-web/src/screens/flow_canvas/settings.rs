//! The settings of one card: its name and the fields of its kind (the schedule picker, the
//! flow switch, raw JSON for values and commands), and a button that deletes the card. Edits
//! write into the draft, one undo step per field change.

use leptos::prelude::*;
use nidoo_core::flow::{
    Flow, IfParams, MAX_SWITCH_RULES, Missing, Node, NodeKind, OffPeakEdge, Op, RuleOp, Scalar,
    SwitchParams, SwitchRule, TimeBetweenParams, command_with_number, math_names,
};
use nidoo_core::model::{Command, Kind};

use super::Editor;
use super::graph::Selection;
use super::math::math_fields;
use super::sun::{FallbackBadge, sun_sheet};
use super::template_field::{NotifyPreview, TemplateField};
use super::value_field::IfValueField;
use crate::command_form::{CommandFields, FieldStyle, entity_actions, retarget};
use crate::entity_picker::needs::{is_commandable, is_signal, keep};
use crate::entity_picker::{EntityPickField, Suggest};
use crate::flow_edit::{add_rule, delete_rule, move_rule, remove_node, set_otherwise};
use crate::house::House;
use crate::i18n::flows::{
    FlowText, flow_text, invalid_json, removed_wires, signal_chip_label, use_flow_text,
};
use crate::i18n::format::parse_decimal;
use crate::i18n::picker::PickerText;
use crate::i18n::states::{kind_word, on_off};
use crate::i18n::{Lang, use_lang};
use crate::if_value::{
    ValueInput, ValueOf, after_entity_change, average_allowed, compared_input, manual_value_input,
    upstream_manual_value, upstream_trigger_entity, upstream_trigger_field, value_input_of,
    value_of_choices,
};
use crate::screens::flow_schedule::ScheduleField;

/// A fixed word as a signal that follows the language, for a field's label or placeholder.
pub(super) fn word(k: FlowText) -> Signal<String> {
    let lang = use_lang();
    Signal::derive(move || flow_text(lang(), k).to_owned())
}

/// A text that is an id or an example, the same in both languages.
fn plain(s: &str) -> Signal<String> {
    Signal::stored(s.to_owned())
}

/// How the form writes into the draft. The first write after the user starts a change (focus
/// on a field, a click, a committed `change`) records an undo step; the keystrokes that follow
/// in the same field join it.
#[derive(Clone, Copy)]
pub(super) struct Form {
    pub(super) editor: Editor,
    armed: StoredValue<bool>,
}

impl Form {
    /// Applies `mutate` to node `id`; a write that changes nothing is dropped.
    pub(super) fn update(self, id: &str, mutate: impl FnOnce(&mut Node)) {
        let draft = self.editor.draft;
        let Some(mut node) = draft.with_untracked(|f| f.node(id).cloned()) else {
            return;
        };
        mutate(&mut node);
        if draft.with_untracked(|f| f.node(id) == Some(&node)) {
            return;
        }
        if self.armed.get_value() {
            self.armed.set_value(false);
            self.editor
                .history
                .update_value(|h| h.record(draft.get_untracked()));
        }
        draft.update(|f| {
            if let Some(n) = f.nodes.iter_mut().find(|n| n.id == node.id) {
                *n = node;
            }
        });
    }

    /// The next write starts a new undo step.
    fn arm(self) {
        self.armed.set_value(true);
    }
}

fn op_str(op: Op) -> &'static str {
    match op {
        Op::Eq => "eq",
        Op::Ne => "ne",
        Op::Gt => "gt",
        Op::Lt => "lt",
        Op::Gte => "gte",
        Op::Lte => "lte",
        Op::IsDark => "is_dark",
        Op::IsOffPeak => "is_off_peak",
    }
}

/// What a comparison reads as in the Compare list: a sign, or words for `is_dark` and
/// `is_off_peak`.
fn op_label(lang: Lang, op: &str) -> &'static str {
    match op {
        "eq" => "=",
        "ne" => "≠",
        "gt" => ">",
        "lt" => "<",
        "gte" => "≥",
        "lte" => "≤",
        "is_off_peak" => flow_text(lang, FlowText::OpIsOffPeak),
        _ => flow_text(lang, FlowText::OpIsDark),
    }
}

/// Whether the card shows the value, entity, average and unknown fields for `op`.
fn takes_operands(op: Op) -> bool {
    !matches!(op, Op::IsDark | Op::IsOffPeak)
}

fn op_from(s: &str) -> Option<Op> {
    match s {
        "eq" => Some(Op::Eq),
        "ne" => Some(Op::Ne),
        "gt" => Some(Op::Gt),
        "lt" => Some(Op::Lt),
        "gte" => Some(Op::Gte),
        "lte" => Some(Op::Lte),
        "is_dark" => Some(Op::IsDark),
        "is_off_peak" => Some(Op::IsOffPeak),
        _ => None,
    }
}

/// A text field whose value is a JSON scalar. The user types raw JSON (`true`, `21.5`, `"ON"`);
/// `None` means absent (an empty textbox clears an optional field). On a parse error the field
/// shows a hint and does not touch `write`.
fn json_text(initial: &serde_json::Value) -> String {
    match initial {
        serde_json::Value::String(s) => format!("\"{s}\""),
        other => other.to_string(),
    }
}

/// What is wrong with a JSON field: empty while required (our text) or the parser's message
/// (shown as received).
#[derive(Clone, Debug, PartialEq, Eq)]
enum JsonProblem {
    Required,
    Parse(String),
}

/// What a JSON field's text means: JSON, or a number typed with a decimal comma ("21,5"). The
/// parser's message comes back as the error.
fn parse_json_input(text: &str) -> Result<serde_json::Value, String> {
    serde_json::from_str::<serde_json::Value>(text).or_else(|e| {
        parse_decimal(text)
            .map(serde_json::Value::from)
            .ok_or_else(|| e.to_string())
    })
}

#[component]
fn JsonField(
    label: Signal<String>,
    initial: serde_json::Value,
    optional: bool,
    write: Callback<Option<serde_json::Value>>,
) -> impl IntoView {
    let lang = use_lang();
    let raw = RwSignal::new(json_text(&initial));
    let error = RwSignal::new(None::<JsonProblem>);
    let on_input = move |ev: web_sys::Event| {
        let text = event_target_value(&ev);
        raw.set(text.clone());
        let trimmed = text.trim();
        if trimmed.is_empty() {
            if optional {
                error.set(None);
                write.run(None);
            } else {
                error.set(Some(JsonProblem::Required));
            }
            return;
        }
        match parse_json_input(trimmed) {
            Ok(value) => {
                error.set(None);
                write.run(Some(value));
            }
            Err(e) => error.set(Some(JsonProblem::Parse(e))),
        }
    };
    view! {
        <label class="node-editor__field">
            <span class="node-editor__label">{move || label.get()}</span>
            <input
                type="text"
                class="field"
                spellcheck="false"
                autocomplete="off"
                prop:value=move || raw.get()
                on:input=on_input
                aria-invalid=move || error.with(|e| e.is_some()).then_some("true")
            />
            {move || error.get().map(|e| view! {
                <span class="node-editor__error">
                    {match e {
                        JsonProblem::Required => flow_text(lang(), FlowText::Required).to_owned(),
                        JsonProblem::Parse(message) => invalid_json(lang(), &message),
                    }}
                </span>
            })}
        </label>
    }
}

#[component]
fn TextField(
    label: Signal<String>,
    initial: String,
    placeholder: Signal<String>,
    optional: bool,
    write: Callback<Option<String>>,
) -> impl IntoView {
    let raw = RwSignal::new(initial);
    let on_input = move |ev: web_sys::Event| {
        let text = event_target_value(&ev);
        raw.set(text.clone());
        let trimmed = text.trim();
        if trimmed.is_empty() {
            if optional {
                write.run(None);
            }
        } else {
            write.run(Some(trimmed.to_owned()));
        }
    };
    view! {
        <label class="node-editor__field">
            <span class="node-editor__label">{move || label.get()}</span>
            <input
                type="text"
                class="field"
                placeholder=move || placeholder.get()
                spellcheck="false"
                autocomplete="off"
                prop:value=move || raw.get()
                on:input=on_input
            />
        </label>
    }
}

/// The settings of the selected card, or nothing when no card is selected. The form is built
/// from the draft when the selection changes or undo/redo replace the draft.
#[component]
pub fn NodeEditor(editor: Editor, selected: RwSignal<Option<Selection>>) -> impl IntoView {
    let node_id = move || match selected.get() {
        Some(Selection::Node(id)) => Some(id),
        _ => None,
    };
    view! {
        <Show when=move || node_id().is_some()>
            {move || {
                editor.restored.track();
                let Some(id) = node_id() else { return ().into_any(); };
                let Some(node) = editor.draft.with_untracked(|f| f.node(&id).cloned()) else {
                    return ().into_any();
                };
                let form = Form {
                    editor,
                    armed: StoredValue::new(false),
                };
                node_editor_body(form, selected, id, node).into_any()
            }}
        </Show>
    }
}

/// What an `if` held before it switched to `is_dark` (and the `is_dark` shift before it switched
/// away), put back when it switches again so the fields on screen and the draft agree.
#[derive(Default)]
struct IfStash {
    value: Option<serde_json::Value>,
    entity: Option<String>,
    missing: Option<Missing>,
    average: Option<String>,
    offset_minutes: Option<i32>,
}

/// Changes an `if`'s op. `is_dark` reads the sun and `is_off_peak` the tariff, so the
/// validator rejects any operand: they are set aside on the way in and restored on the way out.
/// The `is_dark` shift is set aside the same way when leaving `is_dark`.
fn switch_op(i: &mut IfParams, op: Op, stash: &mut IfStash) {
    let bare = |op: Op| matches!(op, Op::IsDark | Op::IsOffPeak);
    if bare(op) && !bare(i.op) {
        stash.value = i.value.take();
        stash.entity = i.entity.take();
        stash.missing = i.missing.take();
        stash.average = i.average.take();
    } else if !bare(op) && bare(i.op) {
        i.value = stash.value.take();
        i.entity = stash.entity.take();
        i.missing = stash.missing.take();
        i.average = stash.average.take();
    }
    if i.op == Op::IsDark && op != Op::IsDark {
        stash.offset_minutes = i.offset_minutes.take();
    } else if op == Op::IsDark && i.op != Op::IsDark {
        i.offset_minutes = stash.offset_minutes.take();
    }
    i.op = op;
}

/// The names of the draft's math cards, in order, for the lists that offer them. A memo, so a
/// field that holds focus is not rebuilt by an unrelated edit.
fn result_names(draft: RwSignal<Flow>) -> Memo<Vec<String>> {
    Memo::new(move |_| {
        draft.with(|f| {
            math_names(f)
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
    })
}

/// `value` when it is a number, else 0: what a math result is compared with.
fn number_or_zero(value: Option<&serde_json::Value>) -> serde_json::Value {
    after_entity_change(value, &ValueInput::Number { unit: None }).unwrap_or_else(|| 0.into())
}

fn node_editor_body(
    form: Form,
    selected: RwSignal<Option<Selection>>,
    id: String,
    node: Node,
) -> impl IntoView {
    let t = use_flow_text();
    let kind_label = match &node.kind {
        NodeKind::TriggerState(_) => FlowText::NodeTrigger,
        NodeKind::TriggerSchedule(_) => FlowText::Schedule,
        NodeKind::TriggerSun(_) => FlowText::NodeSun,
        NodeKind::TriggerOffPeak(_) => FlowText::NodeOffPeak,
        NodeKind::TriggerSilent(_) => FlowText::NodeSilent,
        NodeKind::Limit(_) => FlowText::NodeLimit,
        NodeKind::TriggerManual(_) => FlowText::NodeManual,
        NodeKind::TriggerSignal(_) => FlowText::NodeSignal,
        NodeKind::If(_) => FlowText::NodeIf,
        NodeKind::Delay(_) => FlowText::NodeDelay,
        NodeKind::Command(_) => FlowText::NodeCommand,
        NodeKind::FlowEnable(_) => FlowText::NodeFlowSwitch,
        NodeKind::Notify(_) => FlowText::Notify,
        NodeKind::Speak(_) | NodeKind::Ask(_) => FlowText::Speak,
        NodeKind::Switch(_) => FlowText::SwitchCard,
        NodeKind::TimeBetween(_) => FlowText::TimeCard,
        NodeKind::Math(_) => FlowText::MathCard,
    };
    let id_display = id.clone();
    let close = move |_: web_sys::MouseEvent| selected.set(None);
    let initial_name = node.name.clone().unwrap_or_default();
    let name_id = id.clone();
    let name_write = Callback::new(move |value: Option<String>| {
        form.update(&name_id, |n| n.name = value);
    });
    let delete_id = id.clone();
    let delete = move |_: web_sys::MouseEvent| {
        form.editor.edit(|f| remove_node(f, &delete_id));
        selected.set(None);
    };
    let kind_fields = kind_fields(form, id.clone(), node.kind);
    view! {
        <section
            class="node-editor"
            aria-label=move || t(FlowText::NodeEditor)
            on:focusin=move |_| form.arm()
            on:click=move |_| form.arm()
            on:change=move |_| form.arm()
        >
            <header class="node-editor__header">
                <div class="node-editor__title">
                    <span class="chip">{move || t(kind_label)}</span>
                    <code class="node-editor__id">{id_display}</code>
                </div>
                <button
                    type="button"
                    class="btn btn--ghost"
                    aria-label=move || t(FlowText::CloseEditor)
                    on:click=close
                >
                    {move || t(FlowText::Close)}
                </button>
            </header>
            <TextField
                label=word(FlowText::Name)
                initial=initial_name
                placeholder=word(FlowText::Auto)
                optional=true
                write=name_write
            />
            {kind_fields}
            <footer class="node-editor__footer">
                <button type="button" class="btn btn--ghost node-editor__delete" on:click=delete>
                    {move || t(FlowText::DeleteCard)}
                </button>
                <span class="node-editor__keys">
                    <kbd>"⌫"</kbd>
                    {move || format!(" {} · ", t(FlowText::HintDelete))}
                    <kbd>"⌘D"</kbd>
                    {move || format!(" {}", t(FlowText::HintDuplicate))}
                </span>
            </footer>
        </section>
    }
}

/// The select value of an `if`'s missing-value choice; absent reads as `fail`.
fn missing_str(missing: Option<Missing>) -> &'static str {
    match missing {
        None | Some(Missing::Fail) => "fail",
        Some(Missing::True) => "true",
        Some(Missing::False) => "false",
    }
}

/// The choice behind a select value; `fail` is the default so it is stored as absent.
fn missing_from(value: &str) -> Option<Missing> {
    match value {
        "true" => Some(Missing::True),
        "false" => Some(Missing::False),
        _ => None,
    }
}

/// The `flow.enable` editor: which flow it switches ("This flow" or another by name) and
/// whether it enables or disables it.
fn flow_enable_fields(
    form: Form,
    id: String,
    p: nidoo_core::flow::FlowEnableParams,
) -> impl IntoView {
    let house = expect_context::<House>();
    let t = use_flow_text();
    let own_id = form.editor.draft.with_untracked(|f| f.id.clone());
    let others: Vec<(String, String)> = house.flows.with_untracked(|flows| {
        flows
            .iter()
            .filter(|f| f.definition.id != own_id)
            .map(|f| (f.definition.id.clone(), f.definition.name.clone()))
            .collect()
    });
    let target = p.flow.clone();
    let known = target
        .as_ref()
        .is_none_or(|t| others.iter().any(|(id, _)| id == t));
    let target_id = id.clone();
    let on_target = move |ev: web_sys::Event| {
        let value = event_target_value(&ev);
        form.update(&target_id, |n| {
            if let NodeKind::FlowEnable(ref mut f) = n.kind {
                f.flow = (!value.is_empty()).then_some(value);
            }
        });
    };
    let state_id = id;
    let on_state = move |ev: web_sys::Event| {
        let enabled = event_target_value(&ev) == "enable";
        form.update(&state_id, |n| {
            if let NodeKind::FlowEnable(ref mut f) = n.kind {
                f.enabled = enabled;
            }
        });
    };
    let enabled = p.enabled;
    view! {
        <label class="node-editor__field">
            <span class="node-editor__label">{move || t(FlowText::NodeFlow)}</span>
            <select class="field" on:change=on_target>
                <option value="" selected=target.is_none()>
                    {move || t(FlowText::ThisFlowCap)}
                </option>
                {(!known)
                    .then(|| {
                        let gone = target.clone().unwrap_or_default();
                        view! { <option value=gone.clone() selected=true>{gone.clone()}</option> }
                    })}
                {others
                    .into_iter()
                    .map(|(id, name)| {
                        let selected = target.as_deref() == Some(id.as_str());
                        view! { <option value=id selected=selected>{name}</option> }
                    })
                    .collect_view()}
            </select>
        </label>
        <label class="node-editor__field">
            <span class="node-editor__label">{move || t(FlowText::NodeAction)}</span>
            <select class="field" on:change=on_state>
                <option value="enable" selected=enabled>
                    {move || t(FlowText::Enable)}
                </option>
                <option value="disable" selected=!enabled>
                    {move || t(FlowText::Disable)}
                </option>
            </select>
        </label>
    }
}

/// The `notify` editor: title and body text, the channel and severity of the alert, and
/// whether it arrives silently.
/// A severity as a word; one the table does not know stays as the hub names it.
fn severity_word(lang: Lang, severity: &str) -> String {
    let word = match severity {
        "info" => FlowText::SevInfo,
        "warning" => FlowText::SevWarning,
        "critical" => FlowText::SevCritical,
        other => return other.to_owned(),
    };
    flow_text(lang, word).to_owned()
}

/// A `speak` card: the sentence (with placeholders, as a notification title) and the room whose
/// satellites say it: every room, or one of the rooms that hold a satellite.
fn speak_fields(form: Form, id: String, p: nidoo_core::flow::SpeakParams) -> impl IntoView {
    let t = use_flow_text();
    let house = expect_context::<House>();
    let text_id = id.clone();
    let text_write = Callback::new(move |value: Option<String>| {
        form.update(&text_id, |n| {
            if let NodeKind::Speak(ref mut f) = n.kind {
                f.text = value.unwrap_or_default();
            }
        });
    });
    let on_room = move |ev: web_sys::Event| {
        let value = event_target_value(&ev);
        form.update(&id, |n| {
            if let NodeKind::Speak(ref mut f) = n.kind {
                f.room = (!value.is_empty()).then_some(value);
            }
        });
    };
    let results = Signal::from(result_names(form.editor.draft));
    let chosen = p.room.clone().unwrap_or_default();
    let rooms = move || {
        let mut rooms = house.entities.with(|m| {
            let found = crate::screens::sound_settings::satellites(m);
            house
                .areas
                .with(|areas| crate::screens::sound_settings::satellite_rooms(&found, areas))
        });
        // A room chosen earlier stays listed even once its satellites are gone.
        if !chosen.is_empty() && !rooms.iter().any(|(id, _)| *id == chosen) {
            let name = house.areas.with(|areas| {
                areas
                    .iter()
                    .find(|a| a.id == chosen)
                    .map_or_else(|| chosen.clone(), |a| a.name.clone())
            });
            rooms.push((chosen.clone(), name));
        }
        rooms
    };
    let current = p.room.clone().unwrap_or_default();
    view! {
        <TemplateField
            label=word(FlowText::SpeakTextLabel)
            results=results
            initial=p.text
            placeholder=word(FlowText::SpeakTextExample)
            optional=false
            write=text_write
        />
        <label class="node-editor__field">
            <span class="node-editor__label">{move || t(FlowText::SpeakRoomLabel)}</span>
            <select class="field speak-room" on:change=on_room>
                <option value="" selected=current.is_empty()>
                    {move || t(FlowText::SpeakAllRooms)}
                </option>
                {move || {
                    rooms()
                        .into_iter()
                        .map(|(id, name)| {
                            let selected = id == current;
                            view! { <option value=id selected=selected>{name}</option> }
                        })
                        .collect_view()
                }}
            </select>
        </label>
    }
}

fn notify_fields(form: Form, id: String, p: nidoo_core::flow::NotifyParams) -> impl IntoView {
    let t = use_flow_text();
    let lang = use_lang();
    let title_id = id.clone();
    let title_write = Callback::new(move |value: Option<String>| {
        form.update(&title_id, |n| {
            if let NodeKind::Notify(ref mut f) = n.kind {
                f.title = value.unwrap_or_default();
            }
        });
    });
    let body_id = id.clone();
    let body_write = Callback::new(move |value: Option<String>| {
        form.update(&body_id, |n| {
            if let NodeKind::Notify(ref mut f) = n.kind {
                f.body = value;
            }
        });
    });
    let channel_id = id.clone();
    let on_channel = move |ev: web_sys::Event| {
        let value = event_target_value(&ev);
        form.update(&channel_id, |n| {
            if let NodeKind::Notify(ref mut f) = n.kind {
                f.channel = (value != "misc").then_some(value);
            }
        });
    };
    let severity_id = id.clone();
    let on_severity = move |ev: web_sys::Event| {
        let value = event_target_value(&ev);
        form.update(&severity_id, |n| {
            if let NodeKind::Notify(ref mut f) = n.kind {
                f.severity = (value != "info").then_some(value);
            }
        });
    };
    let draft = form.editor.draft;
    let results = Signal::from(result_names(draft));
    let notify_of = move |id: &str, read: fn(&nidoo_core::flow::NotifyParams) -> String| {
        draft.with(|f| match f.node(id).map(|n| &n.kind) {
            Some(NodeKind::Notify(p)) => read(p),
            _ => String::new(),
        })
    };
    let preview_id = id.clone();
    let preview_title = Signal::derive(move || notify_of(&preview_id, |p| p.title.clone()));
    let preview_id = id.clone();
    let preview_body = Signal::derive(move || {
        Some(notify_of(&preview_id, |p| {
            p.body.clone().unwrap_or_default()
        }))
        .filter(|b| !b.is_empty())
    });
    let trigger_entity = Signal::derive(move || {
        draft.with(|f| {
            f.nodes.iter().find_map(|n| match &n.kind {
                NodeKind::TriggerState(t) => Some(t.entity.clone()),
                _ => None,
            })
        })
    });
    let on_silent = move |ev: web_sys::Event| {
        let silent = event_target_checked(&ev);
        form.update(&id, |n| {
            if let NodeKind::Notify(ref mut f) = n.kind {
                f.silent = silent.then_some(true);
            }
        });
    };
    let channel = p.channel.clone().unwrap_or_else(|| "misc".to_owned());
    let severity = p.severity.clone().unwrap_or_else(|| "info".to_owned());
    let silent = p.silent.unwrap_or(false);
    view! {
        <TemplateField
            label=word(FlowText::TitleLabel)
            results=results
            initial=p.title
            placeholder=word(FlowText::TitleExample)
            optional=false
            write=title_write
        />
        <TemplateField
            label=word(FlowText::BodyLabel)
            results=results
            initial=p.body.unwrap_or_default()
            placeholder=word(FlowText::NoneParen)
            optional=true
            write=body_write
        />
        <NotifyPreview title=preview_title body=preview_body trigger_entity=trigger_entity />
        <label class="node-editor__field">
            <span class="node-editor__label">{move || t(FlowText::Channel)}</span>
            <select class="field" on:change=on_channel>
                {nidoo_core::flow::NOTIFY_CHANNELS
                    .iter()
                    .map(|c| {
                        let chosen = *c == channel;
                        view! { <option value=*c selected=chosen>{*c}</option> }
                    })
                    .collect_view()}
            </select>
        </label>
        <label class="node-editor__field">
            <span class="node-editor__label">{move || t(FlowText::Severity)}</span>
            <select class="field" on:change=on_severity>
                {nidoo_core::flow::NOTIFY_SEVERITIES
                    .iter()
                    .map(|s| {
                        let chosen = *s == severity;
                        view! {
                            <option value=*s selected=chosen>
                                {move || severity_word(lang(), s)}
                            </option>
                        }
                    })
                    .collect_view()}
            </select>
        </label>
        <label class="node-editor__field">
            <span class="node-editor__label">{move || t(FlowText::Silent)}</span>
            <input
                type="checkbox"
                aria-label=move || t(FlowText::Silent)
                prop:checked=silent
                on:change=on_silent
            />
        </label>
    }
}

/// The entity a node's settings point at, read from the draft so it follows edits; an empty
/// id is `None`.
fn node_entity(
    form: Form,
    id: String,
    read: fn(&NodeKind) -> Option<String>,
) -> Signal<Option<String>> {
    Signal::derive(move || {
        form.editor.draft.with(|f| {
            f.node(&id)
                .and_then(|n| read(&n.kind))
                .filter(|e| !e.is_empty())
        })
    })
}

/// The option value of a "Value of" entry.
fn value_of_key(of: &ValueOf) -> String {
    match of {
        ValueOf::Headline => String::new(),
        ValueOf::Field(name) => format!("field:{name}"),
        ValueOf::Sibling(id) => format!("sibling:{id}"),
    }
}

/// "Value of": which value of the card's entity it reads, its headline value, a field of its
/// state or another entity of the same device. Shown once the entity reports more than one.
#[component]
fn ValueOfField(
    entity: Signal<Option<String>>,
    field: Signal<Option<String>>,
    on_pick: Callback<ValueOf>,
) -> impl IntoView {
    let t = use_flow_text();
    let lang = use_lang();
    let house = expect_context::<House>();
    let choices = Memo::new(move |_| {
        let Some(id) = entity.get() else {
            return Vec::new();
        };
        house.entities.with(|m| {
            m.get(&id)
                .map(|e| value_of_choices(lang(), m, e))
                .unwrap_or_default()
        })
    });
    let on_change = move |ev: web_sys::Event| {
        let picked = event_target_value(&ev);
        let of = choices.with_untracked(|list| {
            list.iter()
                .find(|c| value_of_key(&c.of) == picked)
                .map(|c| c.of.clone())
        });
        if let Some(of) = of {
            on_pick.run(of);
        }
    };
    view! {
        <Show when=move || choices.with(|list| list.len() > 1)>
            <label class="node-editor__field">
                <span class="node-editor__label">{move || t(FlowText::ValueOf)}</span>
                <select class="field" aria-label=move || t(FlowText::ValueOf) on:change=on_change>
                    {move || {
                        let current = value_of_key(
                            &field.get().map_or(ValueOf::Headline, ValueOf::Field),
                        );
                        choices
                            .get()
                            .into_iter()
                            .map(|c| {
                                let key = value_of_key(&c.of);
                                let selected = key == current;
                                view! { <option value=key selected=selected>{c.label}</option> }
                            })
                            .collect_view()
                    }}
                </select>
            </label>
        </Show>
    }
}

fn kind_fields(form: Form, id: String, kind: NodeKind) -> impl IntoView {
    let t = use_flow_text();
    let lang = use_lang();
    match kind {
        NodeKind::Math(_) => math_fields(form, id).into_any(),
        NodeKind::TriggerState(_) => {
            let entity_value = node_entity(form, id.clone(), |k| match k {
                NodeKind::TriggerState(t) => Some(t.entity.clone()),
                _ => None,
            });
            let entity_id = id.clone();
            let entity_write = Callback::new(move |value: Option<String>| {
                form.update(&entity_id, |n| {
                    if let NodeKind::TriggerState(ref mut t) = n.kind {
                        let next = value.unwrap_or_default();
                        // What the card compares changed: the old value filters no longer fit.
                        if t.entity != next {
                            t.entity = next;
                            t.field = None;
                            t.to = None;
                            t.from = None;
                        }
                    }
                });
            });
            let field_value = Signal::derive({
                let id = id.clone();
                move || {
                    form.editor
                        .draft
                        .with(|f| match f.node(&id).map(|n| &n.kind) {
                            Some(NodeKind::TriggerState(t)) => t.field.clone(),
                            _ => None,
                        })
                }
            });
            let pick_id = id.clone();
            let pick = Callback::new(move |of: ValueOf| {
                form.update(&pick_id, |n| {
                    if let NodeKind::TriggerState(ref mut t) = n.kind {
                        match of {
                            ValueOf::Headline => t.field = None,
                            ValueOf::Field(name) => t.field = Some(name),
                            ValueOf::Sibling(id) => {
                                t.entity = id;
                                t.field = None;
                            }
                        }
                        t.to = None;
                        t.from = None;
                    }
                });
            });
            let to_id = id.clone();
            let to_write = Callback::new(move |value: Option<serde_json::Value>| {
                form.update(&to_id, |n| {
                    if let NodeKind::TriggerState(ref mut t) = n.kind {
                        t.to = value;
                    }
                });
            });
            let from_id = id.clone();
            let from_write = Callback::new(move |value: Option<serde_json::Value>| {
                form.update(&from_id, |n| {
                    if let NodeKind::TriggerState(ref mut t) = n.kind {
                        t.from = value;
                    }
                });
            });
            // The filters re-mount when what the card reads changes, showing the cleared values.
            let reads = Memo::new(move |_| (entity_value.get(), field_value.get()));
            let filters_id = id;
            let filters = move || {
                reads.track();
                let (to, from) = form.editor.draft.with_untracked(|f| {
                    match f.node(&filters_id).map(|n| &n.kind) {
                        Some(NodeKind::TriggerState(t)) => (t.to.clone(), t.from.clone()),
                        _ => (None, None),
                    }
                });
                view! {
                    <JsonField
                        label=word(FlowText::FiresWhen)
                        initial=to.unwrap_or(serde_json::Value::Null)
                        optional=true
                        write=to_write
                    />
                    <JsonField
                        label=word(FlowText::OnlyAfter)
                        initial=from.unwrap_or(serde_json::Value::Null)
                        optional=true
                        write=from_write
                    />
                }
            };
            view! {
                <EntityPickField
                    label=word(FlowText::Entity)
                    value=entity_value
                    on_change=entity_write
                    pool=Some(keep(|e| !is_signal(e)))
                    compact=true
                    class=Some("node-editor__field".to_owned())
                />
                <ValueOfField entity=entity_value field=field_value on_pick=pick />
                {filters}
            }
            .into_any()
        }
        NodeKind::TriggerSilent(p) => {
            let entity_value = node_entity(form, id.clone(), |k| match k {
                NodeKind::TriggerSilent(t) => Some(t.entity.clone()),
                _ => None,
            });
            let entity_id = id.clone();
            let entity_write = Callback::new(move |value: Option<String>| {
                form.update(&entity_id, |n| {
                    if let NodeKind::TriggerSilent(ref mut t) = n.kind {
                        t.entity = value.unwrap_or_default();
                    }
                });
            });
            let for_write = Callback::new(move |value: Option<String>| {
                form.update(&id, |n| {
                    if let NodeKind::TriggerSilent(ref mut t) = n.kind {
                        t.for_ = value.unwrap_or_default();
                    }
                });
            });
            view! {
                <EntityPickField
                    label=word(FlowText::Entity)
                    value=entity_value
                    on_change=entity_write
                    pool=Some(keep(|e| !is_signal(e)))
                    compact=true
                    class=Some("node-editor__field".to_owned())
                />
                <TextField
                    label=word(FlowText::NodeSilent)
                    initial=p.for_
                    placeholder=plain("50m, 1h30m")
                    optional=false
                    write=for_write
                />
            }
            .into_any()
        }
        NodeKind::Limit(p) => {
            let max_id = id.clone();
            let on_max = move |ev: web_sys::Event| {
                let Some(m) = parse_decimal(&event_target_value(&ev)) else {
                    return;
                };
                let m = (m as u32).clamp(1, nidoo_core::flow::MAX_LIMIT);
                // Show the value that was kept, so a typed 500 reads 100 in the field.
                let input = event_target::<web_sys::HtmlInputElement>(&ev);
                if input.value() != m.to_string() {
                    input.set_value(&m.to_string());
                }
                form.update(&max_id, |n| {
                    if let NodeKind::Limit(ref mut l) = n.kind {
                        l.max = m;
                    }
                });
            };
            let per_write = Callback::new(move |value: Option<String>| {
                form.update(&id, |n| {
                    if let NodeKind::Limit(ref mut l) = n.kind {
                        l.per = value.unwrap_or_default();
                    }
                });
            });
            view! {
                <label class="node-editor__field">
                    <span class="node-editor__label">{move || t(FlowText::LimitMax)}</span>
                    <input
                        type="number"
                        class="field"
                        min="1"
                        max=nidoo_core::flow::MAX_LIMIT.to_string()
                        step="1"
                        prop:value=p.max.to_string()
                        on:input=on_max
                    />
                </label>
                <TextField
                    label=word(FlowText::LimitPer)
                    initial=p.per
                    placeholder=plain("24h, 1h30m")
                    optional=false
                    write=per_write
                />
            }
            .into_any()
        }
        NodeKind::TriggerSignal(_) => {
            let entity_value = node_entity(form, id.clone(), |k| match k {
                NodeKind::TriggerSignal(t) => Some(t.entity.clone()),
                _ => None,
            });
            let entity_id = id.clone();
            let entity_write = Callback::new(move |value: Option<String>| {
                form.update(&entity_id, |n| {
                    if let NodeKind::TriggerSignal(ref mut t) = n.kind {
                        t.entity = value.unwrap_or_default();
                    }
                });
            });
            let house = expect_context::<House>();
            let signals_of = move |entity: &str| -> Vec<String> {
                house.entities.with(|m| {
                    m.get(entity)
                        .and_then(|e| e.capabilities.generic.as_ref())
                        .map(|g| g.signals.clone())
                        .unwrap_or_default()
                })
            };
            let draft = form.editor.draft;
            let node_id = id.clone();
            let current = move || {
                draft.with(|f| match f.node(&node_id).map(|n| &n.kind) {
                    Some(NodeKind::TriggerSignal(t)) => (t.entity.clone(), t.signal.clone()),
                    _ => (String::new(), None),
                })
            };
            let chip_id = id;
            let lang = use_lang();
            let chips = move || {
                let (entity, selected) = current();
                let options: Vec<Option<String>> = std::iter::once(None)
                    .chain(signals_of(&entity).into_iter().map(Some))
                    .collect();
                options
                    .into_iter()
                    .map(|option| {
                        let is_selected = selected == option;
                        let label = option.clone();
                        let chip_id = chip_id.clone();
                        view! {
                            <button
                                type="button"
                                class="chip"
                                class:chip--selected=is_selected
                                aria-pressed=is_selected.to_string()
                                on:click=move |_| {
                                    form.arm();
                                    let option = option.clone();
                                    form.update(&chip_id, |n| {
                                        if let NodeKind::TriggerSignal(ref mut t) = n.kind {
                                            t.signal = option;
                                        }
                                    });
                                }
                            >
                                {move || {
                                    label
                                        .as_deref()
                                        .map(|s| signal_chip_label(lang(), s))
                                        .unwrap_or_else(|| t(FlowText::AnySignal).to_owned())
                                }}
                            </button>
                        }
                    })
                    .collect_view()
            };
            view! {
                <EntityPickField
                    label=word(FlowText::Entity)
                    value=entity_value
                    on_change=entity_write
                    pool=Some(keep(is_signal))
                    compact=true
                    class=Some("node-editor__field".to_owned())
                />
                <div
                    class="override__field"
                    role="group"
                    aria-label=move || t(FlowText::NodeSignal)
                >
                    <span class="override__label">{move || t(FlowText::NodeSignal)}</span>
                    <div class="override__chips signal-chips">{chips}</div>
                </div>
            }
            .into_any()
        }
        NodeKind::If(p) => {
            let op_id = id.clone();
            let current_op = p.op;
            let op_signal = RwSignal::new(current_op);
            let stash = StoredValue::new(IfStash::default());
            let on_op = move |ev: web_sys::Event| {
                if let Some(op) = op_from(&event_target_value(&ev)) {
                    op_signal.set(op);
                    form.update(&op_id, |n| {
                        if let NodeKind::If(ref mut i) = n.kind {
                            stash.update_value(|s| switch_op(i, op, s));
                        }
                    });
                }
            };
            let not_dark = move || takes_operands(op_signal.get());
            let value_id = id.clone();
            let value_write = Callback::new(move |value: Option<serde_json::Value>| {
                if let Some(v) = value {
                    form.update(&value_id, |n| {
                        if let NodeKind::If(ref mut i) = n.kind {
                            i.value = Some(v);
                        }
                    });
                }
            });
            let missing_id = id.clone();
            let on_missing = Callback::new(move |ev: web_sys::Event| {
                let missing = missing_from(&event_target_value(&ev));
                form.update(&missing_id, |n| {
                    if let NodeKind::If(ref mut i) = n.kind {
                        i.missing = missing;
                    }
                });
            });
            let live_id = id.clone();
            // The fields re-mount when leaving `is_dark`: they read the draft as it is then.
            let live = Callback::new(move |()| {
                form.editor
                    .draft
                    .with_untracked(|f| match f.node(&live_id).map(|n| &n.kind) {
                        Some(NodeKind::If(i)) => Some(i.clone()),
                        _ => None,
                    })
            });
            let entity_value = Signal::derive({
                let id = id.clone();
                move || {
                    form.editor
                        .draft
                        .with(|f| match f.node(&id).map(|n| &n.kind) {
                            Some(NodeKind::If(i)) => i.entity.clone().filter(|e| !e.is_empty()),
                            _ => None,
                        })
                }
            });
            let house = expect_context::<House>();
            let draft = form.editor.draft;
            let names = result_names(draft);
            // The math result the card compares, when it compares one.
            let result_value = Signal::derive({
                let id = id.clone();
                move || {
                    draft.with(|f| match f.node(&id).map(|n| &n.kind) {
                        Some(NodeKind::If(i)) => i.result.clone().filter(|r| !r.is_empty()),
                        _ => None,
                    })
                }
            });
            let source_value =
                Signal::derive(move || result_value.get().or_else(|| entity_value.get()));
            // The entity the value is compared with: the card's own, else its state trigger's.
            let resolved = {
                let id = id.clone();
                move |own: Option<String>| {
                    let target = own.or_else(|| draft.with(|f| upstream_trigger_entity(f, &id)))?;
                    house.entities.with(|m| m.get(&target).cloned())
                }
            };
            // A result is a number, whatever the trigger is.
            let compared = Memo::new({
                let resolved = resolved.clone();
                move |_| {
                    result_value
                        .with(Option::is_none)
                        .then(|| resolved(entity_value.get()))
                        .flatten()
                }
            });
            let field_value = Signal::derive({
                let id = id.clone();
                move || {
                    draft.with(|f| match f.node(&id).map(|n| &n.kind) {
                        Some(NodeKind::If(i)) => i.field.clone(),
                        _ => None,
                    })
                }
            });
            // The field compared: the card's own with its own entity, else its trigger's.
            let compared_field = Memo::new({
                let id = id.clone();
                move |_| match entity_value.get() {
                    Some(_) => field_value.get(),
                    None => draft.with(|f| upstream_trigger_field(f, &id)),
                }
            });
            let manual = {
                let manual = manual_input(draft, id.clone());
                Signal::derive(move || {
                    if result_value.with(Option::is_some) {
                        Some(ValueInput::Number { unit: None })
                    } else {
                        manual.get()
                    }
                })
            };
            let entity_id = id.clone();
            let entity_write = Callback::new(move |value: Option<String>| {
                if untrack(|| source_value.get()) == value {
                    return;
                }
                if let Some(name) = value
                    .as_ref()
                    .filter(|v| names.with_untracked(|n| n.contains(v)))
                {
                    // A result stands in for the entity, so what only an entity has goes.
                    form.update(&entity_id, |n| {
                        if let NodeKind::If(ref mut i) = n.kind {
                            i.result = Some(name.clone());
                            // A result is a number: text or on/off values do not compare.
                            i.value = Some(number_or_zero(i.value.as_ref()));
                            i.entity = None;
                            i.field = None;
                            i.average = None;
                            i.missing = None;
                        }
                    });
                    return;
                }
                let next = untrack(|| resolved(value.clone()));
                let input = untrack(|| {
                    compared_input(
                        lang(),
                        next.as_ref(),
                        draft
                            .with(|f| upstream_manual_value(f, &entity_id))
                            .as_ref(),
                    )
                });
                form.update(&entity_id, |n| {
                    if let NodeKind::If(ref mut i) = n.kind {
                        i.value = after_entity_change(i.value.as_ref(), &input);
                        i.entity = value;
                        i.result = None;
                        i.field = None;
                    }
                });
            });
            let pick_id = id.clone();
            let pick = Callback::new(move |of: ValueOf| {
                let field = match of {
                    ValueOf::Sibling(sibling) => return entity_write.run(Some(sibling)),
                    ValueOf::Headline => None,
                    ValueOf::Field(name) => Some(name),
                };
                let e = untrack(|| compared.get());
                let input = untrack(|| value_input_of(lang(), e.as_ref(), field.as_deref()));
                let averages = average_allowed(e.as_ref(), field.as_deref());
                form.update(&pick_id, |n| {
                    if let NodeKind::If(ref mut i) = n.kind {
                        i.value = after_entity_change(i.value.as_ref(), &input);
                        if !averages {
                            i.average = None;
                        }
                        i.field = field;
                    }
                });
            });
            // An average the value cannot have stays on screen until cleared; the hub refuses it.
            let average_set = Signal::derive({
                let id = id.clone();
                move || {
                    draft.with(|f| {
                        matches!(f.node(&id).map(|n| &n.kind), Some(NodeKind::If(i)) if i.average.is_some())
                    })
                }
            });
            let averages = move || {
                average_set.get()
                    || compared.with(|e| {
                        compared_field.with(|f| average_allowed(e.as_ref(), f.as_deref()))
                    })
            };
            let value_id = id.clone();
            let stored_value = Signal::derive(move || {
                draft.with(|f| match f.node(&value_id).map(|n| &n.kind) {
                    Some(NodeKind::If(i)) => i.value.clone(),
                    _ => None,
                })
            });
            let shift_id = id.clone();
            let shift_write = Callback::new(move |ev: web_sys::Event| {
                let Some(m) = parse_decimal(&event_target_value(&ev)) else {
                    return;
                };
                let m = (m as i32).clamp(-720, 720);
                form.update(&shift_id, |n| {
                    if let NodeKind::If(ref mut i) = n.kind {
                        i.offset_minutes = (m != 0).then_some(m);
                    }
                });
            });
            let average_id = id;
            let average_write = Callback::new(move |value: Option<String>| {
                form.update(&average_id, |n| {
                    if let NodeKind::If(ref mut i) = n.kind {
                        i.average = value;
                    }
                });
            });
            let cfg = Signal::from(expect_context::<House>().flows_config);
            view! {
                <label class="node-editor__field">
                    <span class="node-editor__label">{move || t(FlowText::NodeCompare)}</span>
                    <select class="field" on:change=on_op>
                        {["eq", "ne", "gt", "lt", "gte", "lte", "is_dark", "is_off_peak"]
                            .into_iter()
                            .map(|o| {
                                let selected = op_str(current_op) == o;
                                view! {
                                    <option value=o selected=selected>
                                        {move || op_label(lang(), o)}
                                    </option>
                                }
                            })
                            .collect_view()}
                    </select>
                </label>
                <Show
                    when=not_dark
                    fallback=move || if op_signal.get() == Op::IsOffPeak {
                        view! {
                            <p class="node-editor__hint">{move || t(FlowText::IsOffPeakSub)}</p>
                        }
                        .into_any()
                    } else { view! {
                        <p class="node-editor__hint">{move || t(FlowText::IsDark)}</p>
                        <label class="node-editor__field">
                            <span class="node-editor__label">{move || t(FlowText::DarkShift)}</span>
                            <input
                                type="number"
                                class="field"
                                min="-720"
                                max="720"
                                step="5"
                                prop:value=live
                                    .run(())
                                    .and_then(|i| i.offset_minutes)
                                    .unwrap_or(0)
                                    .to_string()
                                on:input=move |ev| shift_write.run(ev)
                            />
                        </label>
                        <p class="node-editor__hint">{move || t(FlowText::DarkShiftHint)}</p>
                        <FallbackBadge cfg=cfg />
                    }
                    .into_any() }
                >
                    <IfValueField
                        value=stored_value
                        entity=Signal::from(compared)
                        write=Callback::new(move |v| value_write.run(Some(v)))
                        fallback=manual
                        field=compared_field
                    />
                    <EntityPickField
                        label=word(FlowText::EntityDefaults)
                        value=source_value
                        on_change=entity_write
                        pinned=Some(word(FlowText::TriggerValue))
                        results=Some(Signal::from(names))
                        compact=true
                        class=Some("node-editor__field".to_owned())
                    />
                    <Show when=move || result_value.with(Option::is_none)>
                    <ValueOfField entity=entity_value field=field_value on_pick=pick />
                    <Show when=averages>
                        <TextField
                            label=word(FlowText::AverageOver)
                            initial=live.run(()).and_then(|i| i.average).unwrap_or_default()
                            placeholder=word(FlowText::AverageHint)
                            optional=true
                            write=average_write
                        />
                    </Show>
                    <label class="node-editor__field">
                        <span class="node-editor__label">{move || t(FlowText::WhenUnknown)}</span>
                        <select class="field" on:change=move |ev| on_missing.run(ev)>
                            {[
                                ("fail", FlowText::MissingFail),
                                ("true", FlowText::MissingTrue),
                                ("false", FlowText::MissingFalse),
                            ]
                                .into_iter()
                                .map(|(value, label)| {
                                    let selected = missing_str(live.run(()).and_then(|i| i.missing)) == value;
                                    view! {
                                        <option value=value selected=selected>
                                            {move || t(label)}
                                        </option>
                                    }
                                })
                                .collect_view()}
                        </select>
                    </label>
                    </Show>
                </Show>
            }
            .into_any()
        }
        NodeKind::TriggerSchedule(p) => {
            let write = Callback::new(move |cron: String| {
                form.update(&id, |n| {
                    if let NodeKind::TriggerSchedule(ref mut s) = n.kind {
                        s.cron = cron;
                    }
                });
            });
            view! { <ScheduleField initial=p.cron write=write /> }.into_any()
        }
        NodeKind::TriggerSun(params) => {
            let node = RwSignal::new(nidoo_core::flow::Node {
                id: id.clone(),
                name: None,
                position: None,
                kind: NodeKind::TriggerSun(params),
                extra: Default::default(),
            });
            // The sheet edits its own copy of the node; each change goes into the draft.
            Effect::new(move |prev: Option<()>| {
                let kind = node.with(|n| n.kind.clone());
                if prev.is_some() {
                    form.update(&id, |n| n.kind = kind);
                }
            });
            let cfg = Signal::from(expect_context::<House>().flows_config);
            sun_sheet(node, cfg).into_any()
        }
        NodeKind::TriggerOffPeak(p) => off_peak_fields(form, id, p.edge).into_any(),
        NodeKind::TriggerManual(p) => manual_fields(form, id, p.value).into_any(),
        NodeKind::FlowEnable(p) => flow_enable_fields(form, id, p).into_any(),
        NodeKind::Notify(p) => notify_fields(form, id, p).into_any(),
        NodeKind::Speak(p) => speak_fields(form, id, p).into_any(),
        NodeKind::Ask(_) => ().into_any(),
        NodeKind::Switch(p) => switch_fields(form, id, p).into_any(),
        NodeKind::TimeBetween(p) => time_fields(form, id, p).into_any(),
        NodeKind::Delay(p) => {
            let for_id = id.clone();
            let for_write = Callback::new(move |value: Option<String>| {
                form.update(&for_id, |n| {
                    if let NodeKind::Delay(ref mut d) = n.kind {
                        d.for_ = value.unwrap_or_default();
                    }
                });
            });
            let up_to_id = id;
            let up_to_write = Callback::new(move |value: Option<String>| {
                form.update(&up_to_id, |n| {
                    if let NodeKind::Delay(ref mut d) = n.kind {
                        d.up_to = value;
                    }
                });
            });
            view! {
                <TextField
                    label=word(FlowText::Duration)
                    initial=p.for_
                    placeholder=plain("5m, 1h30m, 45s")
                    optional=false
                    write=for_write
                />
                <TextField
                    label=word(FlowText::UpToRandom)
                    initial=p.up_to.unwrap_or_default()
                    placeholder=word(FlowText::ExactDuration)
                    optional=true
                    write=up_to_write
                />
            }
            .into_any()
        }
        NodeKind::Command(p) => {
            let entity_value = node_entity(form, id.clone(), |k| match k {
                NodeKind::Command(t) => Some(t.entity.clone()),
                _ => None,
            });
            let house = expect_context::<House>();
            let entity = Memo::new(move |_| {
                let id = entity_value.get()?;
                house.entities.with(|m| m.get(&id).cloned())
            });
            let entity_id = id.clone();
            let entity_write = Callback::new(move |value: Option<String>| {
                let actions = value.as_ref().and_then(|v| {
                    house
                        .entities
                        .with_untracked(|m| m.get(v).map(entity_actions))
                });
                form.update(&entity_id, |n| {
                    if let NodeKind::Command(ref mut c) = n.kind {
                        c.entity = value.unwrap_or_default();
                        if let Some(actions) = &actions {
                            c.command = retarget(&c.command, actions);
                            if command_with_number(&c.command, 0.0).is_none() {
                                c.value_from = None;
                            }
                        }
                    }
                });
            });
            let draft = form.editor.draft;
            let command_id = id.clone();
            let command = Signal::derive(move || {
                draft.with(|f| match f.node(&command_id).map(|n| &n.kind) {
                    Some(NodeKind::Command(c)) => c.command.clone(),
                    _ => p.command.clone(),
                })
            });
            let value_from_id = id.clone();
            let cmd_id = id;
            let command_write = Callback::new(move |cmd: Command| {
                form.update(&cmd_id, |n| {
                    if let NodeKind::Command(ref mut c) = n.kind {
                        // A link to a result only fits a command that has a number.
                        if command_with_number(&cmd, 0.0).is_none() {
                            c.value_from = None;
                        }
                        c.command = cmd;
                    }
                });
            });
            let json_write = Callback::new(move |value: Option<serde_json::Value>| {
                if let Some(cmd) = value.and_then(|v| serde_json::from_value(v).ok()) {
                    command_write.run(cmd);
                }
            });
            let json_box = move || {
                let initial = command
                    .with_untracked(|c| serde_json::to_value(c))
                    .unwrap_or(serde_json::Value::Null);
                view! {
                    <JsonField
                        label=word(FlowText::CommandJson)
                        initial=initial
                        optional=false
                        write=json_write
                    />
                }
            };
            let names = result_names(draft);
            let from_id = value_from_id;
            let value_from = Signal::derive({
                let from_id = from_id.clone();
                move || {
                    draft.with(|f| match f.node(&from_id).map(|n| &n.kind) {
                        Some(NodeKind::Command(c)) => {
                            c.value_from.clone().filter(|v| !v.is_empty())
                        }
                        _ => None,
                    })
                }
            });
            // Only a command with a number can take one from a result.
            let takes_number =
                Memo::new(move |_| command.with(|c| command_with_number(c, 0.0).is_some()));
            let on_value_from = move |ev: web_sys::Event| {
                let value = Some(event_target_value(&ev)).filter(|v| !v.is_empty());
                form.update(&from_id, |n| {
                    if let NodeKind::Command(ref mut c) = n.kind {
                        c.value_from = value;
                    }
                });
            };
            let replaced =
                Signal::derive(move || value_from.with(Option::is_some) && takes_number.get());
            view! {
                <EntityPickField
                    label=word(FlowText::Entity)
                    value=entity_value
                    on_change=entity_write
                    suggest=Some(Suggest {
                        text: PickerText::Commandable,
                        keep: keep(is_commandable),
                    })
                    compact=true
                    class=Some("node-editor__field".to_owned())
                />
                <Show when=move || takes_number.get() && (names.with(|n| !n.is_empty()) || value_from.with(Option::is_some))>
                    <label class="node-editor__field">
                        <span class="node-editor__label">{move || t(FlowText::ValueFromResult)}</span>
                        <select class="field value-from" on:change=on_value_from.clone()>
                            <option value="" selected=move || value_from.with(Option::is_none)>
                                "—"
                            </option>
                            // A link to a card that is gone stays visible, so it can be cleared.
                            {move || {
                                let mut all = names.get();
                                if let Some(v) = value_from.get().filter(|v| !all.contains(v)) {
                                    all.push(v);
                                }
                                all.into_iter()
                                    .map(|n| {
                                        let is = n.clone();
                                        let text = n.clone();
                                        view! {
                                            <option
                                                value=n
                                                selected=move || value_from.get().as_deref() == Some(is.as_str())
                                            >
                                                {text}
                                            </option>
                                        }
                                    })
                                    .collect_view()
                            }}
                        </select>
                    </label>
                </Show>
                <Show when=move || entity.with(Option::is_some) fallback=json_box>
                    <CommandFields
                        command=command
                        entity=Signal::from(entity)
                        write=command_write
                        style=FieldStyle::Node
                        advanced=ViewFn::from(json_box)
                        numbers_replaced=replaced
                    />
                </Show>
            }
            .into_any()
        }
    }
}

const RULE_OPS: [RuleOp; 7] = [
    RuleOp::Eq,
    RuleOp::Ne,
    RuleOp::Gt,
    RuleOp::Lt,
    RuleOp::Gte,
    RuleOp::Lte,
    RuleOp::Between,
];

fn rule_op_label(lang: Lang, op: RuleOp) -> &'static str {
    match op {
        RuleOp::Between => flow_text(lang, FlowText::OpBetween),
        other => op_label(lang, other.wire_name()),
    }
}

fn rule_op_from(s: &str) -> Option<RuleOp> {
    RULE_OPS.into_iter().find(|o| o.wire_name() == s)
}

/// Gives `rule` a new operator: `between` starts with an upper bound equal to the lower one,
/// every other operator has none.
fn set_rule_op(rule: &mut SwitchRule, op: RuleOp) {
    rule.op = op;
    if op == RuleOp::Between {
        if rule.to.is_none() {
            rule.to = Some(rule.value.clone());
        }
    } else {
        rule.to = None;
    }
}

/// The rule "Add rule" appends: equal to the value the last rule tests, or `true`.
fn next_rule(last: Option<&SwitchRule>) -> SwitchRule {
    SwitchRule {
        op: RuleOp::Eq,
        value: last.map_or(serde_json::Value::Bool(true), |r| r.value.clone()),
        to: None,
        extra: Default::default(),
    }
}

/// The rule at `index` of switch `id` in the draft.
fn rule_at(form: Form, id: &str, index: usize) -> Option<SwitchRule> {
    form.editor
        .draft
        .with_untracked(|f| match f.node(id).map(|n| &n.kind) {
            Some(NodeKind::Switch(p)) => p.rules.get(index).cloned(),
            _ => None,
        })
}

/// Writes `mutate` into rule `index` of switch `id`.
fn update_rule(form: Form, id: &str, index: usize, mutate: impl FnOnce(&mut SwitchRule)) {
    form.update(id, |n| {
        if let NodeKind::Switch(ref mut p) = n.kind
            && let Some(r) = p.rules.get_mut(index)
        {
            mutate(r);
        }
    });
}

/// One rule of a switch: operator, value (and upper bound for `between`), move and delete.
/// What a switch's rules compare with: the entity, its field and, without an entity, the input
/// a Manual card upstream gives.
#[derive(Clone, Copy)]
struct Compared {
    entity: Memo<Option<nidoo_api::EntityView>>,
    field: Memo<Option<String>>,
    manual: Signal<Option<ValueInput>>,
}

fn rule_row(
    form: Form,
    id: String,
    index: usize,
    count: usize,
    compared: Compared,
    bump: Callback<usize>,
) -> impl IntoView {
    let lang = use_lang();
    let t = use_flow_text();
    let rule = rule_at(form, &id, index).unwrap_or_else(|| next_rule(None));
    let is_between = RwSignal::new(rule.op == RuleOp::Between);
    let current_op = rule.op;
    let draft = form.editor.draft;
    let read = {
        let id = id.clone();
        move |pick: fn(&SwitchRule) -> Option<serde_json::Value>| {
            draft.with(|f| match f.node(&id).map(|n| &n.kind) {
                Some(NodeKind::Switch(p)) => p.rules.get(index).and_then(pick),
                _ => None,
            })
        }
    };
    let value = Signal::derive({
        let read = read.clone();
        move || read(|r| Some(r.value.clone()))
    });
    let upper = {
        let read = read.clone();
        move || {
            read(|r| r.to.clone())
                .as_ref()
                .and_then(serde_json::Value::as_f64)
                .map(|n| n.to_string())
                .unwrap_or_default()
        }
    };
    let op_id = id.clone();
    let on_op = move |ev: web_sys::Event| {
        if let Some(op) = rule_op_from(&event_target_value(&ev)) {
            is_between.set(op == RuleOp::Between);
            update_rule(form, &op_id, index, |r| set_rule_op(r, op));
        }
    };
    let value_id = id.clone();
    let value_write = Callback::new(move |v: serde_json::Value| {
        update_rule(form, &value_id, index, |r| r.value = v);
    });
    let to_id = id.clone();
    let on_to = move |ev: web_sys::Event| {
        if let Some(n) = parse_decimal(event_target_value(&ev).trim()) {
            update_rule(form, &to_id, index, |r| {
                r.to = Some(Scalar::Number(n).to_json())
            });
        }
    };
    let up_id = id.clone();
    let up = move |_: web_sys::MouseEvent| {
        form.editor.edit(|f| move_rule(f, &up_id, index, index - 1));
        bump.run(0);
    };
    let down_id = id.clone();
    let down = move |_: web_sys::MouseEvent| {
        form.editor
            .edit(|f| move_rule(f, &down_id, index, index + 1));
        bump.run(0);
    };
    let delete_id = id;
    let delete = move |_: web_sys::MouseEvent| {
        let removed = form.editor.edit(|f| delete_rule(f, &delete_id, index));
        bump.run(removed);
    };
    view! {
        <fieldset class="rule-row">
            <legend class="node-editor__label">
                {move || format!("{} {}", t(FlowText::RuleLabel), index + 1)}
            </legend>
            <select class="field" on:change=on_op>
                {RULE_OPS
                    .into_iter()
                    .map(|o| {
                        view! {
                            <option value=o.wire_name() selected=o == current_op>
                                {move || rule_op_label(lang(), o)}
                            </option>
                        }
                    })
                    .collect_view()}
            </select>
            <IfValueField
                value=value
                entity=Signal::from(compared.entity)
                write=value_write
                fallback=compared.manual
                field=compared.field
            />
            <Show when=move || is_between.get()>
                <span class="command-field__number">
                    <span class="command-field__unit">{move || t(FlowText::RuleAnd)}</span>
                    <input
                        type="number"
                        class="field command-field__input"
                        step="any"
                        prop:value=upper.clone()
                        on:change=on_to.clone()
                    />
                </span>
            </Show>
            <span class="rule-row__buttons">
                <button
                    type="button"
                    class="btn btn--ghost"
                    aria-label=move || t(FlowText::MoveRuleUp)
                    disabled=index == 0
                    on:click=up
                >
                    "↑"
                </button>
                <button
                    type="button"
                    class="btn btn--ghost"
                    aria-label=move || t(FlowText::MoveRuleDown)
                    disabled={index + 1 >= count}
                    on:click=down
                >
                    "↓"
                </button>
                <button
                    type="button"
                    class="btn btn--ghost"
                    aria-label=move || t(FlowText::DeleteRule)
                    title=move || t(FlowText::DeleteRule)
                    disabled={count <= 1}
                    on:click=delete
                >
                    "✕"
                </button>
            </span>
        </fieldset>
    }
}

/// The input a Manual card upstream of `id` calls for, read from the draft as it changes.
fn manual_input(draft: RwSignal<nidoo_core::flow::Flow>, id: String) -> Signal<Option<ValueInput>> {
    let lang = use_lang();
    Signal::derive(move || {
        draft
            .with(|f| upstream_manual_value(f, &id))
            .map(|v| manual_value_input(lang(), &v))
    })
}

/// The `switch` editor: what it compares, then one row per rule, then the exits.
fn switch_fields(form: Form, id: String, p: SwitchParams) -> impl IntoView {
    let t = use_flow_text();
    let lang = use_lang();
    let draft = form.editor.draft;
    let house = expect_context::<House>();
    let entity_value = Signal::derive({
        let id = id.clone();
        move || {
            draft.with(|f| match f.node(&id).map(|n| &n.kind) {
                Some(NodeKind::Switch(s)) => s.entity.clone().filter(|e| !e.is_empty()),
                _ => None,
            })
        }
    });
    let resolved = {
        let id = id.clone();
        move |own: Option<String>| {
            let target = own.or_else(|| draft.with(|f| upstream_trigger_entity(f, &id)))?;
            house.entities.with(|m| m.get(&target).cloned())
        }
    };
    let names = result_names(draft);
    // The math result the card switches on, when it does.
    let result_value = Signal::derive({
        let id = id.clone();
        move || {
            draft.with(|f| match f.node(&id).map(|n| &n.kind) {
                Some(NodeKind::Switch(s)) => s.result.clone().filter(|r| !r.is_empty()),
                _ => None,
            })
        }
    });
    let source_value = Signal::derive(move || result_value.get().or_else(|| entity_value.get()));
    // A result is a number, whatever the trigger is.
    let compared = Memo::new({
        let resolved = resolved.clone();
        move |_| {
            result_value
                .with(Option::is_none)
                .then(|| resolved(entity_value.get()))
                .flatten()
        }
    });
    let field_value = Signal::derive({
        let id = id.clone();
        move || {
            draft.with(|f| match f.node(&id).map(|n| &n.kind) {
                Some(NodeKind::Switch(s)) => s.field.clone(),
                _ => None,
            })
        }
    });
    // The field compared: the card's own with its own entity, else its trigger's.
    let compared_field = Memo::new({
        let id = id.clone();
        move |_| match entity_value.get() {
            Some(_) => field_value.get(),
            None => draft.with(|f| upstream_trigger_field(f, &id)),
        }
    });
    let manual = {
        let manual = manual_input(draft, id.clone());
        Signal::derive(move || {
            if result_value.with(Option::is_some) {
                Some(ValueInput::Number { unit: None })
            } else {
                manual.get()
            }
        })
    };
    let entity_id = id.clone();
    let entity_write = Callback::new(move |value: Option<String>| {
        if untrack(|| source_value.get()) == value {
            return;
        }
        if let Some(name) = value
            .as_ref()
            .filter(|v| names.with_untracked(|n| n.contains(v)))
        {
            // A result stands in for the entity, so the field and the average go.
            form.update(&entity_id, |n| {
                if let NodeKind::Switch(ref mut s) = n.kind {
                    s.result = Some(name.clone());
                    for r in &mut s.rules {
                        r.value = number_or_zero(Some(&r.value));
                    }
                    s.entity = None;
                    s.field = None;
                    s.average = None;
                }
            });
            return;
        }
        let next = untrack(|| resolved(value.clone()));
        let input = untrack(|| {
            compared_input(
                lang(),
                next.as_ref(),
                draft
                    .with(|f| upstream_manual_value(f, &entity_id))
                    .as_ref(),
            )
        });
        form.update(&entity_id, |n| {
            if let NodeKind::Switch(ref mut s) = n.kind {
                for r in &mut s.rules {
                    if let Some(v) = after_entity_change(Some(&r.value), &input) {
                        r.value = v;
                    }
                }
                s.entity = value;
                s.result = None;
                s.field = None;
            }
        });
    });
    let pick_id = id.clone();
    let pick = Callback::new(move |of: ValueOf| {
        let field = match of {
            ValueOf::Sibling(sibling) => return entity_write.run(Some(sibling)),
            ValueOf::Headline => None,
            ValueOf::Field(name) => Some(name),
        };
        let e = untrack(|| compared.get());
        let input = untrack(|| value_input_of(lang(), e.as_ref(), field.as_deref()));
        let averages = average_allowed(e.as_ref(), field.as_deref());
        form.update(&pick_id, |n| {
            if let NodeKind::Switch(ref mut s) = n.kind {
                for r in &mut s.rules {
                    if let Some(v) = after_entity_change(Some(&r.value), &input) {
                        r.value = v;
                    }
                }
                if !averages {
                    s.average = None;
                }
                s.field = field;
            }
        });
    });
    // An average the value cannot have stays on screen until cleared; the hub refuses it.
    let average_set = Signal::derive({
        let id = id.clone();
        move || {
            draft.with(|f| {
                matches!(f.node(&id).map(|n| &n.kind), Some(NodeKind::Switch(s)) if s.average.is_some())
            })
        }
    });
    let averages = move || {
        average_set.get()
            || compared.with(|e| compared_field.with(|f| average_allowed(e.as_ref(), f.as_deref())))
    };
    let average_initial = {
        let id = id.clone();
        move || {
            draft.with_untracked(|f| match f.node(&id).map(|n| &n.kind) {
                Some(NodeKind::Switch(s)) => s.average.clone().unwrap_or_default(),
                _ => String::new(),
            })
        }
    };
    let average_id = id.clone();
    let average_write = Callback::new(move |value: Option<String>| {
        form.update(&average_id, |n| {
            if let NodeKind::Switch(ref mut s) = n.kind {
                s.average = value;
            }
        });
    });
    let rules_compare = Compared {
        entity: compared,
        field: compared_field,
        manual,
    };
    // Rows are rebuilt when the rules change in number or order, not on every value edit.
    let version = RwSignal::new(0_u32);
    let removed = RwSignal::new(0_usize);
    let bump = Callback::new(move |n: usize| {
        removed.set(n);
        version.update(|v| *v += 1);
    });
    let count = Signal::derive({
        let id = id.clone();
        move || {
            draft.with(|f| match f.node(&id).map(|n| &n.kind) {
                Some(NodeKind::Switch(s)) => s.rules.len(),
                _ => 0,
            })
        }
    });
    let rows = {
        let id = id.clone();
        move || {
            version.track();
            let n = count.get_untracked();
            (0..n)
                .map(|i| rule_row(form, id.clone(), i, n, rules_compare, bump))
                .collect_view()
        }
    };
    let add_id = id.clone();
    let add = move |_: web_sys::MouseEvent| {
        let last = rule_at(form, &add_id, count.get_untracked().saturating_sub(1));
        form.editor
            .edit(|f| add_rule(f, &add_id, next_rule(last.as_ref())));
        bump.run(0);
    };
    let otherwise_id = id.clone();
    let on_otherwise = move |ev: web_sys::Event| {
        let on = event_target_checked(&ev);
        let removed_now = form.editor.edit(|f| set_otherwise(f, &otherwise_id, on));
        removed.set(removed_now);
    };
    let all_id = id;
    let on_all = move |ev: web_sys::Event| {
        let on = event_target_checked(&ev);
        form.update(&all_id, |n| {
            if let NodeKind::Switch(ref mut s) = n.kind {
                s.all = on;
            }
        });
    };
    let has_removed = move || removed.get() > 0;
    view! {
        <EntityPickField
            label=word(FlowText::EntityDefaults)
            value=source_value
            on_change=entity_write
            pinned=Some(word(FlowText::TriggerValue))
            results=Some(Signal::from(names))
            compact=true
            class=Some("node-editor__field".to_owned())
        />
        <ValueOfField entity=entity_value field=field_value on_pick=pick />
        <Show when=averages>
            <TextField
                label=word(FlowText::AverageOver)
                initial=average_initial()
                placeholder=word(FlowText::AverageHint)
                optional=true
                write=average_write
            />
        </Show>
        <div class="rule-list">{rows}</div>
        <button
            type="button"
            class="btn btn--ghost"
            disabled=move || { count.get() >= MAX_SWITCH_RULES }
            on:click=add
        >
            {move || t(FlowText::AddRule)}
        </button>
        <label class="node-editor__check">
            <input type="checkbox" prop:checked=p.otherwise on:change=on_otherwise />
            <span>{move || t(FlowText::OtherwiseExit)}</span>
        </label>
        <label class="node-editor__check">
            <input type="checkbox" prop:checked=p.all on:change=on_all />
            <span>{move || t(FlowText::FireAll)}</span>
        </label>
        <Show when=has_removed>
            <p class="node-editor__hint" role="status">
                {move || removed_wires(lang(), removed.get())}
            </p>
        </Show>
    }
}

/// Which kind of value a Manual card carries, as the select offers it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ManualKind {
    Nothing,
    On,
    Off,
    Number,
    Text,
}

const MANUAL_KINDS: [ManualKind; 5] = [
    ManualKind::Nothing,
    ManualKind::On,
    ManualKind::Off,
    ManualKind::Number,
    ManualKind::Text,
];

impl ManualKind {
    fn of(value: Option<&Scalar>) -> Self {
        match value {
            None => Self::Nothing,
            Some(Scalar::Bool(true)) => Self::On,
            Some(Scalar::Bool(false)) => Self::Off,
            Some(Scalar::Number(_)) => Self::Number,
            Some(Scalar::Text(_)) => Self::Text,
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::Nothing => "none",
            Self::On => "on",
            Self::Off => "off",
            Self::Number => "number",
            Self::Text => "text",
        }
    }

    fn from_key(key: &str) -> Option<Self> {
        MANUAL_KINDS.into_iter().find(|k| k.key() == key)
    }

    fn has_box(self) -> bool {
        matches!(self, Self::Number | Self::Text)
    }

    /// The value for this kind with `raw` typed in the box; a number box that does not read as
    /// a number gives no value, so the draft always matches the kind shown.
    fn value(self, raw: &str) -> Option<Option<Scalar>> {
        match self {
            Self::Nothing => Some(None),
            Self::On => Some(Some(Scalar::Bool(true))),
            Self::Off => Some(Some(Scalar::Bool(false))),
            Self::Number => Some(parse_decimal(raw.trim()).map(Scalar::Number)),
            Self::Text => Some(Some(Scalar::Text(raw.to_owned()))),
        }
    }
}

/// The word of a manual value kind in the select.
fn manual_kind_label(lang: Lang, kind: ManualKind) -> String {
    match kind {
        ManualKind::Nothing => flow_text(lang, FlowText::ManualNoValue).to_owned(),
        ManualKind::On => on_off(lang, false, true).to_owned(),
        ManualKind::Off => on_off(lang, false, false).to_owned(),
        ManualKind::Number => kind_word(lang, Kind::Number).to_owned(),
        ManualKind::Text => flow_text(lang, FlowText::ManualText).to_owned(),
    }
}

/// The Manual card editor: one optional value, none, on/off, a number or text.
/// The off-peak trigger's sheet: fire when off-peak starts or when it ends.
fn off_peak_fields(form: Form, id: String, initial: OffPeakEdge) -> impl IntoView {
    let lang = use_lang();
    let edge = RwSignal::new(initial);
    let choose = move |next: OffPeakEdge| {
        edge.set(next);
        form.update(&id, |n| {
            if let NodeKind::TriggerOffPeak(ref mut p) = n.kind {
                p.edge = next;
            }
        });
    };
    let chip = move |value: OffPeakEdge| {
        let choose = choose.clone();
        view! {
            <button
                type="button"
                class="chip"
                class:chip--selected=move || edge.get() == value
                aria-pressed=move || if edge.get() == value { "true" } else { "false" }
                on:click=move |_| choose(value)
            >
                {move || flow_text(lang(), crate::flow_text::off_peak_edge_text(value))}
            </button>
        }
    };
    view! {
        <div class="node-editor__field" role="group">
            <span class="node-editor__label">{move || flow_text(lang(), FlowText::OffPeakEdge)}</span>
            <div class="chip-row">
                {chip(OffPeakEdge::Start)}
                {chip(OffPeakEdge::End)}
            </div>
        </div>
    }
}

fn manual_fields(form: Form, id: String, initial: Option<Scalar>) -> impl IntoView {
    let lang = use_lang();
    let kind = RwSignal::new(ManualKind::of(initial.as_ref()));
    let raw = RwSignal::new(match &initial {
        Some(Scalar::Number(n)) => n.to_string(),
        Some(Scalar::Text(s)) => s.clone(),
        _ => String::new(),
    });
    let write = move |value: Option<Scalar>| {
        form.update(&id, |n| {
            if let NodeKind::TriggerManual(ref mut m) = n.kind {
                m.value = value;
            }
        });
    };
    let on_kind = {
        let write = write.clone();
        move |ev: web_sys::Event| {
            let Some(next) = ManualKind::from_key(&event_target_value(&ev)) else {
                return;
            };
            kind.set(next);
            if !next.has_box() {
                raw.set(String::new());
            }
            if let Some(v) = next.value(&raw.get_untracked()) {
                write(v);
            }
        }
    };
    let on_raw = move |ev: web_sys::Event| {
        let text = event_target_value(&ev);
        raw.set(text.clone());
        if let Some(v) = kind.get_untracked().value(&text) {
            write(v);
        }
    };
    let on_text = on_raw.clone();
    view! {
        <label class="node-editor__field">
            <span class="node-editor__label">{move || flow_text(lang(), FlowText::ManualValue)}</span>
            <select class="field" on:change=on_kind>
                {MANUAL_KINDS
                    .into_iter()
                    .map(|k| {
                        view! {
                            <option value=k.key() selected=k == kind.get_untracked()>
                                {move || manual_kind_label(lang(), k)}
                            </option>
                        }
                    })
                    .collect_view()}
            </select>
        </label>
        <Show when=move || kind.get() == ManualKind::Number>
            <input
                type="number"
                class="field"
                step="any"
                // Not reactive: mid-typing ("-", "2.") a number box reports "" and would be wiped.
                prop:value=raw.get_untracked()
                on:input=on_raw.clone()
            />
        </Show>
        <Show when=move || kind.get() == ManualKind::Text>
            <input
                type="text"
                class="field"
                spellcheck="false"
                autocomplete="off"
                prop:value=move || raw.get()
                on:input=on_text.clone()
            />
        </Show>
    }
}

/// The `time.between` editor: two `HH:MM` fields.
fn time_fields(form: Form, id: String, p: TimeBetweenParams) -> impl IntoView {
    let field = |label: FlowText, initial: String, to: bool| {
        let id = id.clone();
        let on_input = move |ev: web_sys::Event| {
            let text = event_target_value(&ev);
            if text.is_empty() {
                return;
            }
            form.update(&id, |n| {
                if let NodeKind::TimeBetween(ref mut t) = n.kind {
                    if to {
                        t.to = text;
                    } else {
                        t.from = text;
                    }
                }
            });
        };
        view! {
            <label class="node-editor__field">
                <span class="node-editor__label">{word(label)}</span>
                <input type="time" class="field" prop:value=initial on:input=on_input />
            </label>
        }
    };
    view! {
        {field(FlowText::TimeFrom, p.from, false)}
        {field(FlowText::TimeTo, p.to, true)}
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn manual_kind_round_trips_its_value() {
        for (value, kind) in [
            (None, ManualKind::Nothing),
            (Some(Scalar::Bool(true)), ManualKind::On),
            (Some(Scalar::Bool(false)), ManualKind::Off),
            (Some(Scalar::Number(21.5)), ManualKind::Number),
            (Some(Scalar::Text("hi".into())), ManualKind::Text),
        ] {
            assert_eq!(ManualKind::of(value.as_ref()), kind);
            assert_eq!(ManualKind::from_key(kind.key()), Some(kind));
        }
        assert_eq!(
            ManualKind::Number.value("21,5"),
            Some(Some(Scalar::Number(21.5)))
        );
        // An empty or unreadable number box holds no value, never the previous one.
        assert_eq!(ManualKind::Number.value("abc"), Some(None));
        assert_eq!(ManualKind::Number.value(""), Some(None));
        assert_eq!(
            ManualKind::On.value("ignored"),
            Some(Some(Scalar::Bool(true)))
        );
        assert_eq!(ManualKind::Nothing.value("x"), Some(None));
    }

    #[test]
    fn between_gains_an_upper_bound_and_others_lose_it() {
        let mut r = next_rule(None);
        r.value = json!(18);
        set_rule_op(&mut r, RuleOp::Between);
        assert_eq!(r.to, Some(json!(18)));
        r.to = Some(json!(22));
        set_rule_op(&mut r, RuleOp::Between);
        assert_eq!(r.to, Some(json!(22)));
        set_rule_op(&mut r, RuleOp::Gt);
        assert_eq!((r.op, r.to), (RuleOp::Gt, None));
    }

    #[test]
    fn a_new_rule_repeats_the_last_value() {
        assert_eq!(next_rule(None).value, json!(true));
        let mut last = next_rule(None);
        last.value = json!("ON");
        assert_eq!(next_rule(Some(&last)).value, json!("ON"));
    }

    #[test]
    fn rule_ops_round_trip_their_names() {
        for op in RULE_OPS {
            assert_eq!(rule_op_from(op.wire_name()), Some(op));
        }
        assert_eq!(rule_op_from("is_dark"), None);
    }

    #[test]
    fn json_fields_take_a_decimal_comma() {
        assert_eq!(parse_json_input("21,5"), Ok(json!(21.5)));
        assert_eq!(parse_json_input("21.5"), Ok(json!(21.5)));
        assert_eq!(parse_json_input("true"), Ok(json!(true)));
        assert_eq!(parse_json_input("\"ON\""), Ok(json!("ON")));
        assert!(parse_json_input("{oops").is_err());
    }

    #[test]
    fn flipping_to_is_dark_and_back_keeps_the_operands() {
        let mut i = IfParams {
            op: Op::Eq,
            value: Some(json!(true)),
            entity: Some("x".into()),
            field: None,
            missing: Some(Missing::True),
            average: Some("15m".into()),
            offset_minutes: None,
            result: None,
            extra: Default::default(),
        };
        let mut stash = IfStash::default();
        switch_op(&mut i, Op::IsDark, &mut stash);
        assert_eq!(
            (i.value.clone(), i.entity.clone(), i.missing),
            (None, None, None)
        );
        switch_op(&mut i, Op::Eq, &mut stash);
        assert_eq!(i.op, Op::Eq);
        assert_eq!(i.value, Some(json!(true)));
        assert_eq!(i.entity.as_deref(), Some("x"));
        assert_eq!(i.missing, Some(Missing::True));
    }

    #[test]
    fn flipping_to_is_off_peak_and_back_keeps_the_operands() {
        assert_eq!(op_from("is_off_peak"), Some(Op::IsOffPeak));
        assert_eq!(op_str(Op::IsOffPeak), "is_off_peak");
        let mut i = IfParams {
            op: Op::Eq,
            value: Some(json!(true)),
            entity: Some("x".into()),
            field: None,
            missing: Some(Missing::True),
            average: Some("15m".into()),
            offset_minutes: None,
            result: None,
            extra: Default::default(),
        };
        let mut stash = IfStash::default();
        switch_op(&mut i, Op::IsOffPeak, &mut stash);
        assert_eq!(
            (
                i.value.clone(),
                i.entity.clone(),
                i.missing,
                i.average.clone()
            ),
            (None, None, None, None)
        );
        switch_op(&mut i, Op::Eq, &mut stash);
        assert_eq!(i.value, Some(json!(true)));
        assert_eq!(i.entity.as_deref(), Some("x"));
        assert_eq!(i.average.as_deref(), Some("15m"));
    }

    #[test]
    fn only_compare_ops_show_operand_fields() {
        for op in [Op::Eq, Op::Ne, Op::Gt, Op::Lt, Op::Gte, Op::Lte] {
            assert!(takes_operands(op), "{op:?}");
        }
        assert!(!takes_operands(Op::IsDark));
        assert!(!takes_operands(Op::IsOffPeak));
    }

    #[test]
    fn dark_to_off_peak_sets_the_shift_aside() {
        let mut i = IfParams {
            op: Op::IsDark,
            value: None,
            entity: None,
            field: None,
            missing: None,
            average: None,
            offset_minutes: Some(-30),
            result: None,
            extra: Default::default(),
        };
        let mut stash = IfStash::default();
        switch_op(&mut i, Op::IsOffPeak, &mut stash);
        assert_eq!(i.offset_minutes, None);
        switch_op(&mut i, Op::IsDark, &mut stash);
        assert_eq!(i.offset_minutes, Some(-30));
    }

    #[test]
    fn leaving_is_dark_sets_the_shift_aside_and_restores_it() {
        let mut i = IfParams {
            op: Op::IsDark,
            value: None,
            entity: None,
            field: None,
            missing: None,
            average: None,
            offset_minutes: Some(-30),
            result: None,
            extra: Default::default(),
        };
        let mut stash = IfStash::default();
        switch_op(&mut i, Op::Gt, &mut stash);
        assert_eq!(i.offset_minutes, None);
        switch_op(&mut i, Op::IsDark, &mut stash);
        assert_eq!(i.offset_minutes, Some(-30));
    }

    #[test]
    fn switching_between_comparisons_leaves_operands_alone() {
        let mut i = IfParams {
            op: Op::Eq,
            value: Some(json!(1)),
            entity: None,
            field: None,
            missing: None,
            average: None,
            offset_minutes: None,
            result: None,
            extra: Default::default(),
        };
        switch_op(&mut i, Op::Gt, &mut IfStash::default());
        assert_eq!((i.op, i.value), (Op::Gt, Some(json!(1))));
    }
}
