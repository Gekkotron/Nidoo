//! The Security tab's alarm card: colour and text by state, arm modes, Disarm, and an inline
//! code field when the panel asks for one.

use std::collections::BTreeSet;

use leptos::prelude::*;
use nidoo_core::model::{AlarmMode, Capabilities, CodeFormat, Command};

use crate::cards::Sender;
use crate::cards::alarm::mode_label;
use crate::components::now::unix_now;
use crate::i18n::security::{SecurityText, security_text};
use crate::i18n::{AlarmText, Lang, alarm_text, use_lang};
use crate::icons::SHIELD;
use crate::screens::alarm_settings::countdown_text;
use crate::security::{PanelUi, panel_state_text, panel_tone};

/// Which buttons the hero draws for a state. `current` is marked, not re-sent.
#[derive(Clone, Debug, PartialEq)]
struct HeroActions {
    arm: Vec<AlarmMode>,
    current: Option<AlarmMode>,
    disarm: bool,
    enabled: bool,
}

fn armed_mode(state: &str) -> Option<AlarmMode> {
    Some(match state {
        "armed_home" => AlarmMode::Home,
        "armed_away" => AlarmMode::Away,
        "armed_night" => AlarmMode::Night,
        "armed_vacation" => AlarmMode::Vacation,
        "armed_custom_bypass" => AlarmMode::CustomBypass,
        _ => return None,
    })
}

fn hero_actions(state: Option<&str>, available: bool, modes: &BTreeSet<AlarmMode>) -> HeroActions {
    let all: Vec<AlarmMode> = modes.iter().copied().collect();
    let enabled = available && state.is_some();
    match state {
        None => HeroActions {
            arm: all,
            current: None,
            disarm: true,
            enabled: false,
        },
        Some("disarmed") => HeroActions {
            arm: all,
            current: None,
            disarm: false,
            enabled,
        },
        Some("arming" | "pending" | "triggered") => HeroActions {
            arm: vec![],
            current: None,
            disarm: true,
            enabled,
        },
        Some("disarming") => HeroActions {
            arm: vec![],
            current: None,
            disarm: false,
            enabled,
        },
        Some(s) => HeroActions {
            arm: all,
            current: armed_mode(s),
            disarm: true,
            enabled,
        },
    }
}

/// The code to send: the typed one when `required` and not blank, else none.
fn code_for(required: bool, typed: &str) -> Option<String> {
    let typed = typed.trim();
    (required && !typed.is_empty()).then(|| typed.to_owned())
}

/// "Triggered by Front door, Hall": what set the alarm off. A `sos:<remote>` entry reads "SOS".
fn cause_line(lang: Lang, cause: &[String]) -> Option<String> {
    let mut names: Vec<String> = Vec::new();
    for c in cause {
        let name = if c.starts_with("sos:") {
            alarm_text(lang, AlarmText::Sos).to_owned()
        } else {
            c.clone()
        };
        if !names.contains(&name) {
            names.push(name);
        }
    }
    (!names.is_empty()).then(|| {
        format!(
            "{} {}",
            alarm_text(lang, AlarmText::TriggeredBy),
            names.join(", ")
        )
    })
}

#[component]
pub fn AlarmHero(panel: PanelUi) -> impl IntoView {
    let sender = Sender::from_context();
    let lang = use_lang();
    let state = panel.state.as_deref();
    let tone = panel_tone(state, panel.available);
    let triggered = tone == "triggered";
    let available = panel.available;
    let state_owned = panel.state.clone();
    let text = move || panel_state_text(lang(), state_owned.as_deref(), available);
    let caps: &Capabilities = &panel.capabilities;
    let actions = hero_actions(state, panel.available, &caps.alarm_modes);
    let (arm_required, disarm_required) = (caps.code_arm_required, caps.code_disarm_required);
    let numeric = caps.code_format == CodeFormat::Number;
    let typed = RwSignal::new(String::new());
    let blank = move || typed.with(|t| t.trim().is_empty());
    let enabled = actions.enabled;

    // The exit or entry delay counts down to the hub's deadline, once a second while it lasts.
    let countdown = match (state, panel.deadline) {
        (Some(s @ ("arming" | "pending")), Some(deadline)) => {
            let now = RwSignal::new(unix_now());
            #[cfg(target_arch = "wasm32")]
            leptos::task::spawn_local(async move {
                // `try_set` hands the value back once the hero is gone: stop then.
                while now.try_set(unix_now()).is_none() {
                    gloo_timers::future::TimeoutFuture::new(1000).await;
                }
            });
            let word = if s == "arming" {
                AlarmText::ArmsIn
            } else {
                AlarmText::SirenIn
            };
            Some(view! {
                <span class="hero__detail">
                    {move || format!("{} {}", alarm_text(lang(), word), countdown_text(deadline, now.get()))}
                </span>
            })
        }
        _ => None,
    };
    let cause = triggered
        .then(|| {
            let cause = panel.cause.clone();
            move || cause_line(lang(), &cause)
        })
        .map(|line| view! { <span class="hero__detail">{line}</span> });

    let arm_buttons = actions
        .arm
        .iter()
        .copied()
        .map(|mode| {
            let id = panel.id.clone();
            let selected = actions.current == Some(mode);
            view! {
                <button
                    type="button"
                    class="btn btn--compact"
                    class:btn--active=selected
                    aria-pressed=selected.to_string()
                    disabled=move || !enabled || (arm_required && blank())
                    on:click=move |_| {
                        if selected {
                            return;
                        }
                        let code = code_for(arm_required, &typed.get_untracked());
                        typed.set(String::new());
                        sender.send(&id, Command::Arm { mode, code }, None);
                    }
                >
                    {move || mode_label(lang(), mode)}
                </button>
            }
        })
        .collect_view();
    let modes =
        (!actions.arm.is_empty()).then(|| view! { <div class="hero__modes">{arm_buttons}</div> });

    let disarm = actions.disarm.then(|| {
        let id = panel.id.clone();
        view! {
            <button
                type="button"
                class="btn hero__disarm"
                disabled=move || !enabled || (disarm_required && blank())
                on:click=move |_| {
                    let code = code_for(disarm_required, &typed.get_untracked());
                    typed.set(String::new());
                    sender.send(&id, Command::Disarm { code }, None);
                }
            >
                {move || security_text(lang(), SecurityText::Disarm)}
            </button>
        }
    });

    let code_input = (actions.enabled && (arm_required || disarm_required)).then(|| {
        view! {
            <input
                class="field hero__code"
                type=if numeric { "number" } else { "text" }
                inputmode=if numeric { "numeric" } else { "text" }
                autocomplete="off"
                placeholder=move || security_text(lang(), SecurityText::Code)
                aria-label=move || security_text(lang(), SecurityText::Code)
                prop:value=move || typed.get()
                on:input=move |ev| typed.set(event_target_value(&ev))
            />
        }
    });

    let pending = sender.pending(&panel.id);
    view! {
        <section class=format!("hero hero--{tone}")>
            <div
                class="hero__head"
                aria-live=if triggered { "assertive" } else { "polite" }
            >
                <span class="hero__icon" inner_html=SHIELD></span>
                <span class="hero__text">
                    <span class="hero__name">{panel.name.clone()}</span>
                    <span class="hero__state">{text}</span>
                    {countdown}
                    {cause}
                </span>
                <span class="dot dot--pending" class:dot--active=move || pending.get()></span>
            </div>
            {code_input}
            {modes}
            {disarm}
        </section>
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use nidoo_core::model::AlarmMode;

    use super::{cause_line, code_for, hero_actions};
    use crate::i18n::Lang;

    fn modes() -> BTreeSet<AlarmMode> {
        [AlarmMode::Away, AlarmMode::Home, AlarmMode::Night]
            .into_iter()
            .collect()
    }

    #[test]
    fn disarmed_offers_modes_only() {
        let a = hero_actions(Some("disarmed"), true, &modes());
        assert_eq!(a.arm, [AlarmMode::Home, AlarmMode::Away, AlarmMode::Night]);
        assert!(!a.disarm && a.enabled && a.current.is_none());
    }

    #[test]
    fn armed_marks_the_current_mode_and_offers_disarm() {
        let a = hero_actions(Some("armed_away"), true, &modes());
        assert_eq!(a.current, Some(AlarmMode::Away));
        assert!(a.disarm);
    }

    #[test]
    fn delays_and_triggered_offer_disarm_only() {
        for s in ["arming", "pending", "triggered"] {
            let a = hero_actions(Some(s), true, &modes());
            assert!(a.arm.is_empty() && a.disarm, "{s}");
        }
        let a = hero_actions(Some("disarming"), true, &modes());
        assert!(a.arm.is_empty() && !a.disarm);
    }

    #[test]
    fn unknown_or_unavailable_is_disabled() {
        assert!(!hero_actions(None, true, &modes()).enabled);
        assert!(!hero_actions(Some("disarmed"), false, &modes()).enabled);
    }

    #[test]
    fn code_is_sent_only_when_required_and_typed() {
        assert_eq!(code_for(true, " 1234 "), Some("1234".into()));
        assert_eq!(code_for(true, "  "), None);
        assert_eq!(code_for(false, "1234"), None);
    }

    #[test]
    fn cause_line_names_the_sensors_and_words_sos() {
        let c = |v: &[&str]| v.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        assert_eq!(
            cause_line(Lang::En, &c(&["Front door", "Hall"])).as_deref(),
            Some("Triggered by Front door, Hall")
        );
        assert_eq!(
            cause_line(Lang::Fr, &c(&["sos:remote.x"])).as_deref(),
            Some("Déclenchée par SOS")
        );
        assert_eq!(cause_line(Lang::En, &[]), None);
    }
}
