//! The sheet panel of a generic entity: one row per announced control, drawn by shape, plus the
//! signal, update and picture subpanels its placement calls for.

use leptos::prelude::*;
use nidoo_api::EntityView;
use nidoo_core::model::{Command, GenericState, GenericValue, Kind, StateValue};
use nidoo_core::types::{self, Placement, Shape, TypeDef};

use crate::cards::Sender;
use crate::cards::generic::{control_label, control_view};
use crate::components::now::{Now, unix_now};
use crate::format::word;
use crate::i18n::devices::{DeviceText, firmware_versions, last_signal, use_device_text};
use crate::i18n::format::ago;
use crate::i18n::use_lang;

fn generic_state(entity: &EntityView) -> GenericState {
    match &entity.state.value {
        StateValue::Generic(g) => g.clone(),
        _ => GenericState::default(),
    }
}

fn text_field(
    entity: &EntityView,
    def: &TypeDef,
    name: &str,
    secret: bool,
    sender: Sender,
) -> AnyView {
    let id = entity.id.clone();
    let control = name.to_owned();
    let current = match (
        secret,
        def.controls
            .get(name)
            .and_then(|c| c.field.as_deref())
            .and_then(|f| generic_state(entity).fields.get(f).cloned()),
    ) {
        (false, Some(GenericValue::Text(s))) => s,
        _ => String::new(),
    };
    view! {
        <input
            class="input"
            type=if secret { "password" } else { "text" }
            aria-label=control_label(name)
            value=current
            on:change=move |ev| {
                sender
                    .send(
                        &id,
                        Command::Control {
                            control: control.clone(),
                            value: Some(serde_json::Value::String(event_target_value(&ev))),
                        },
                        None,
                    );
            }
        />
    }
    .into_any()
}

fn signal_line(entity: &EntityView, def: &TypeDef, now: u64) -> Option<AnyView> {
    let lang = use_lang();
    let g = generic_state(entity);
    let last = match g.fields.get(def.primary.as_deref().unwrap_or("last"))? {
        GenericValue::Text(s) => word(s),
        _ => return None,
    };
    let at = match g.fields.get("at") {
        Some(GenericValue::Number(ms)) if *ms >= 0.0 => Some((*ms / 1000.0) as u64),
        _ => entity.state.last_update,
    };
    let text = move || {
        last_signal(
            lang(),
            &last,
            at.map(|t| ago(lang(), now.saturating_sub(t))).as_deref(),
        )
    };
    Some(view! { <p class="sheet__generic__last">{text}</p> }.into_any())
}

fn update_line(entity: &EntityView) -> Option<AnyView> {
    let t = use_device_text();
    let g = generic_state(entity);
    let text = |k: &str| match g.fields.get(k) {
        Some(GenericValue::Text(s)) if !s.is_empty() => Some(s.clone()),
        _ => None,
    };
    let installed = text("installed")?;
    let latest = text("latest").unwrap_or_else(|| installed.clone());
    let available = installed != latest;
    Some(
        view! {
            <div class="sheet__generic__row">
                <span>{firmware_versions(&installed, &latest)}</span>
                {available.then(|| view! { <span class="chip chip--selected">{move || t(DeviceText::UpdateAvailable)}</span> })}
            </div>
        }
        .into_any(),
    )
}

fn picture_primary(def: &TypeDef) -> bool {
    def.primary
        .as_deref()
        .and_then(|p| def.fields.get(p))
        .is_some_and(|f| f.shape == Shape::Picture)
}

#[component]
pub fn GenericPanel(entity: Memo<Option<EntityView>>) -> impl IntoView {
    let sender = Sender::from_context();
    let now = use_context::<Now>();
    let t = use_device_text();
    let applies = Memo::new(move |_| {
        entity.with(|e| {
            e.as_ref()
                .is_some_and(|e| e.kind == Kind::Generic && types::type_def(&e.component).is_some())
        })
    });
    view! {
        <Show when=move || applies.get()>
            {move || {
                let e = entity.get()?;
                let def = types::type_def(&e.component)?;
                let caps = e.capabilities.generic.clone().unwrap_or_default();
                let now_s = now.map_or_else(unix_now, |n| n.0.get());
                let mut sections: Vec<AnyView> = Vec::new();
                match def.placement {
                    Placement::Card => {
                        for name in &caps.controls {
                            // The lock card above asks before unlocking; a second, one-tap
                            // Unlock here would undo that.
                            if e.component == "lock" && name == "locked" {
                                continue;
                            }
                            let Some(ctl) = def.controls.get(name) else { continue };
                            let widget = if ctl.shape == Shape::Text && !ctl.press {
                                text_field(&e, def, name, caps.secret, sender)
                            } else {
                                control_view(&e, def, &caps, name, sender)
                            };
                            sections
                                .push(
                                    view! {
                                        <div class="sheet__generic__row">
                                            <span class="sheet__generic__label">{control_label(name)}</span>
                                            {widget}
                                        </div>
                                    }
                                        .into_any(),
                                );
                        }
                    }
                    Placement::Signal => sections.extend(signal_line(&e, def, now_s)),
                    Placement::Update => sections.extend(update_line(&e)),
                    Placement::Target => {}
                }
                if picture_primary(def) {
                    let src = format!(
                        "/api/v1/entities/{}/picture?v={}",
                        crate::api::encode_id(&e.id),
                        e.state.last_update.unwrap_or(0),
                    );
                    sections
                        .push(
                            view! { <img class="sheet__generic__picture" src=src loading="lazy" alt="" /> }
                                .into_any(),
                        );
                }
                (!sections.is_empty())
                    .then(|| {
                        view! {
                            <section class="sheet__panel sheet__generic">
                                <h2 class="sheet__heading">{move || t(DeviceText::Details)}</h2>
                                {sections}
                            </section>
                        }
                    })
            }}
        </Show>
    }
}
