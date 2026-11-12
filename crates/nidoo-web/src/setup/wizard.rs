//! The wizard frame: the progress line and the current step's body.

use leptos::prelude::*;
use leptos_router::hooks::use_query_map;
use nidoo_api::SetupStep;
use serde_json::Value;

use super::connection::{DatabaseStep, MqttStep};
use super::devices::DevicesStep;
use super::done::DoneStep;
use super::services::ServicesStep;
use super::welcome::WelcomeStep;
use super::{SetupState, opening_step};
use crate::i18n::setup::{SetupText, use_setup_text};

/// Every step, in order; the progress line has one dot each.
const STEPS: [SetupStep; 6] = [
    SetupStep::Welcome,
    SetupStep::Mqtt,
    SetupStep::Database,
    SetupStep::Devices,
    SetupStep::Services,
    SetupStep::Done,
];

/// The step the steps' Back and Next buttons move; provided as context by [`SetupScreen`].
#[derive(Clone, Copy)]
pub struct WizardNav {
    pub step: RwSignal<SetupStep>,
}

fn step_name(step: SetupStep) -> SetupText {
    match step {
        SetupStep::Welcome => SetupText::StepWelcome,
        SetupStep::Mqtt => SetupText::StepMqtt,
        SetupStep::Database => SetupText::StepDatabase,
        SetupStep::Restore => SetupText::StepRestore,
        SetupStep::Devices => SetupText::StepDevices,
        SetupStep::Services => SetupText::StepServices,
        SetupStep::Done => SetupText::StepDone,
    }
}

/// The dot for `s` while the wizard is on `current`.
fn dot_class(current: SetupStep, s: SetupStep) -> &'static str {
    match current.cmp(&s) {
        std::cmp::Ordering::Equal => "setup__dot setup__dot--current",
        std::cmp::Ordering::Greater => "setup__dot setup__dot--done",
        std::cmp::Ordering::Less => "setup__dot",
    }
}

/// A `step` query value read as a [`SetupStep`]; anything unparsable counts as absent.
fn parse_step(raw: Option<String>) -> Option<SetupStep> {
    serde_json::from_value(Value::String(raw?)).ok()
}

/// The `/setup` page. It opens where the hub (or the `step` query) says, then the buttons of
/// each step move [`WizardNav::step`].
#[component]
pub fn SetupScreen() -> impl IntoView {
    let t = use_setup_text();
    let query_step = parse_step(use_query_map().get_untracked().get("step"));
    let step = RwSignal::new(SetupStep::Welcome);
    let ready = RwSignal::new(false);
    provide_context(WizardNav { step });
    let state = use_context::<SetupState>();
    let known = state.and_then(|s| s.0.get_untracked());
    if let Some(status) = known {
        step.set(opening_step(&status, query_step));
        ready.set(true);
    } else {
        leptos::task::spawn_local(async move {
            if let Ok(status) = crate::api::setup::get_setup().await {
                step.try_set(opening_step(&status, query_step));
                if let Some(state) = state {
                    state.0.try_set(Some(status));
                }
            }
            ready.try_set(true);
        });
    }
    view! {
        <Show when=move || ready.get()>
            <div class="setup">
                <ol class="setup__progress" aria-label=move || t(SetupText::Progress)>
                    {STEPS
                        .into_iter()
                        .map(|s| {
                            view! {
                                <li
                                    class=move || dot_class(step.get(), s)
                                    aria-current=move || (step.get() == s).then_some("step")
                                    title=move || t(step_name(s))
                                ></li>
                            }
                        })
                        .collect_view()}
                </ol>
                {move || match step.get() {
                    SetupStep::Welcome => view! { <WelcomeStep /> }.into_any(),
                    SetupStep::Mqtt => view! { <MqttStep /> }.into_any(),
                    SetupStep::Database => view! { <DatabaseStep /> }.into_any(),
                    // The restore screen comes with a later change.
                    SetupStep::Restore => ().into_any(),
                    SetupStep::Devices => {
                        let z2m_base = state
                            .and_then(|s| s.0.get_untracked())
                            .and_then(|s| s.mqtt)
                            .and_then(|m| m.z2m_base)
                            .unwrap_or_default();
                        view! { <DevicesStep z2m_base=z2m_base /> }.into_any()
                    }
                    SetupStep::Services => view! { <ServicesStep /> }.into_any(),
                    SetupStep::Done => view! { <DoneStep /> }.into_any(),
                }}
            </div>
        </Show>
    }
}
