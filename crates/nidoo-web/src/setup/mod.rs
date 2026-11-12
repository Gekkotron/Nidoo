//! The onboarding wizard: shared state and the rules for when to send the browser to it.

pub mod connection;
pub mod devices;
pub mod done;
pub mod restore;
pub mod services;
pub mod welcome;
pub mod wizard;

use gloo_timers::future::TimeoutFuture;
use leptos::prelude::*;
use leptos_router::NavigateOptions;
use leptos_router::components::{Route, Router, Routes};
use leptos_router::hooks::use_navigate;
use leptos_router::path;
use nidoo_api::{SetupStatus, SetupStep};

use crate::api::setup::get_setup;
use crate::components::toast::{ToastQueue, Toasts};
use crate::i18n::Lang;
use crate::i18n::setup::{SetupText, use_setup_text};
use crate::theme::install_theme;
use wizard::SetupScreen;

/// What `GET /api/v1/setup` last said. `None` means the hub has no setup API (an older hub) or
/// the call failed; nothing redirects then.
#[derive(Clone, Copy)]
pub struct SetupState(pub RwSignal<Option<SetupStatus>>);

/// The "Applying…" overlay's state: a save that restarts the hub calls `start` with the path to
/// reload into once the hub answers again.
#[derive(Clone, Copy)]
pub struct Applying {
    /// The path to reload into; `Some` while the overlay shows.
    pub target: RwSignal<Option<String>>,
    /// The hub did not come back in time.
    pub failed: RwSignal<bool>,
    reload: fn(&str),
}

impl Applying {
    /// `reload` navigates the page (a function so a test can record it instead).
    pub fn new(reload: fn(&str)) -> Self {
        Self {
            target: RwSignal::new(None),
            failed: RwSignal::new(false),
            reload,
        }
    }

    /// Shows the overlay and reloads into `target` once the hub is back.
    pub fn start(&self, target: &str) {
        // A second start (a double click, a 503 after the first save) must not spawn a second wait.
        if self.target.with_untracked(Option::is_some) {
            return;
        }
        self.failed.set(false);
        self.target.set(Some(target.to_owned()));
    }
}

/// Reloads the page at `target`, which re-reads the hub's mode.
pub fn reload_to(target: &str) {
    if let Some(window) = web_sys::window() {
        let _ = window.location().assign(target);
    }
}

/// Waits for a restarting hub. It sleeps `interval_ms` first so a hub about to restart is not
/// caught still answering, then polls `GET /api/v1/setup` every `interval_ms`, ignoring errors,
/// and returns the first reply that is not `restarting`. `None` after `attempts` polls.
pub async fn wait_for_hub(interval_ms: u32, attempts: u32) -> Option<SetupStatus> {
    TimeoutFuture::new(interval_ms).await;
    for attempt in 0..attempts {
        if let Ok(status) = get_setup().await
            && !status.restarting
        {
            return Some(status);
        }
        if attempt + 1 < attempts {
            TimeoutFuture::new(interval_ms).await;
        }
    }
    None
}

/// Poll interval and count for the overlay: about 90 s, longer than a hub restart.
const OVERLAY_INTERVAL_MS: u32 = 1_000;
const OVERLAY_ATTEMPTS: u32 = 90;

/// Waits for the hub, then reloads into the target; on a timeout sets `failed`.
fn wait_then_reload(applying: Applying, target: String) {
    leptos::task::spawn_local(async move {
        match wait_for_hub(OVERLAY_INTERVAL_MS, OVERLAY_ATTEMPTS).await {
            Some(_) => (applying.reload)(&target),
            None => {
                applying.failed.try_set(true);
            }
        }
    });
}

/// A full-screen layer while the hub restarts. Needs [`Applying`] in context.
#[component]
pub fn ApplyingOverlay() -> impl IntoView {
    let applying = expect_context::<Applying>();
    let t = use_setup_text();
    Effect::new(move |_| {
        if let Some(target) = applying.target.get() {
            wait_then_reload(applying, target);
        }
    });
    let retry = move |_| {
        if let Some(target) = applying.target.get_untracked() {
            applying.failed.set(false);
            wait_then_reload(applying, target);
        }
    };
    view! {
        <Show when=move || applying.target.with(Option::is_some)>
            <div class="applying" role="status" aria-live="polite">
                <Show
                    when=move || applying.failed.get()
                    fallback=move || {
                        view! {
                            <div class="boot__spinner"></div>
                            <p class="applying__text">{move || t(SetupText::Applying)}</p>
                        }
                    }
                >
                    <p class="applying__text">{move || t(SetupText::ApplyFailed)}</p>
                    <button type="button" class="btn" on:click=retry>
                        {move || t(SetupText::Retry)}
                    </button>
                </Show>
            </div>
        </Show>
    }
}

/// Sends any path but `/setup` to `/setup`.
#[component]
fn ToSetup() -> impl IntoView {
    let navigate = use_navigate();
    Effect::new(move |_| {
        navigate(
            "/setup",
            NavigateOptions {
                replace: true,
                ..Default::default()
            },
        );
    });
}

/// The app while the hub is in setup mode: only `/setup`, with no house, no socket and no shell.
#[component]
pub fn SetupApp() -> impl IntoView {
    let lang = use_context::<RwSignal<Lang>>().unwrap_or_else(crate::i18n::install_lang);
    provide_context(lang);
    let theme = install_theme();
    provide_context(theme);
    provide_context(Toasts::new());
    view! {
        <Router>
            <Routes fallback=ToSetup>
                <Route path=path!("/setup") view=SetupScreen />
            </Routes>
            <ToastQueue />
        </Router>
    }
}

/// Whether the hub serves only the wizard: it has no house to show yet, because it is in setup
/// mode or waiting for a backup to be restored.
pub fn setup_only(status: &SetupStatus) -> bool {
    status.mode == "setup" || status.mode == "restore"
}

/// Whether `path` must be replaced by `/setup`: any path while the hub serves only the wizard,
/// and the landing page until setup is finished.
pub fn redirects_to_setup(status: &SetupStatus, path: &str) -> bool {
    if path == "/setup" {
        return false;
    }
    setup_only(status) || (!status.finished && path == "/")
}

/// The step the wizard opens on. `query_step` (from `/setup?step=…`) wins when given, else a
/// finished hub starts over at `Welcome` and an unfinished one resumes at its stored step. In
/// setup mode only the steps up to `Database` work, so it never goes past that; in restore mode
/// only the restore works.
pub fn opening_step(status: &SetupStatus, query_step: Option<SetupStep>) -> SetupStep {
    if status.mode == "restore" {
        return SetupStep::Restore;
    }
    let step = query_step.unwrap_or(if status.finished {
        SetupStep::Welcome
    } else {
        status.step
    });
    if status.mode == "setup" {
        step.min(SetupStep::Database)
    } else {
        step
    }
}

/// The `/setup?step=…` address a wizard save reloads into, so a restart resumes at that step.
/// The name comes from serde, so it is always what the hub parses.
pub fn setup_href(step: SetupStep) -> String {
    let name = serde_json::to_string(&step).unwrap_or_default();
    format!("/setup?step={}", name.trim_matches('"'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(mode: &str, finished: bool, step: SetupStep) -> SetupStatus {
        SetupStatus {
            mode: mode.into(),
            step,
            finished,
            bundled: false,
            location: None,
            mqtt: None,
            database: None,
            restarting: false,
            restore_pending: false,
        }
    }

    #[test]
    fn setup_mode_sends_every_path_to_setup() {
        let s = status("setup", false, SetupStep::Welcome);
        assert!(redirects_to_setup(&s, "/"));
        assert!(redirects_to_setup(&s, "/dashboards/house"));
        assert!(!redirects_to_setup(&s, "/setup"));
    }

    #[test]
    fn restore_mode_sends_every_path_to_setup() {
        let s = status("restore", false, SetupStep::Restore);
        assert!(redirects_to_setup(&s, "/"));
        assert!(redirects_to_setup(&s, "/rooms/x"));
        assert!(!redirects_to_setup(&s, "/setup"));
        assert!(setup_only(&s));
        assert!(!setup_only(&status("running", false, SetupStep::Devices)));
    }

    #[test]
    fn opening_step_in_restore_mode() {
        let s = status("restore", false, SetupStep::Database);
        assert_eq!(opening_step(&s, None), SetupStep::Restore);
        assert_eq!(
            opening_step(&s, Some(SetupStep::Welcome)),
            SetupStep::Restore
        );
    }

    #[test]
    fn a_running_hub_redirects_only_landing_until_finished() {
        assert!(redirects_to_setup(
            &status("running", false, SetupStep::Devices),
            "/"
        ));
        assert!(!redirects_to_setup(
            &status("running", false, SetupStep::Devices),
            "/rooms/x"
        ));
        assert!(!redirects_to_setup(
            &status("running", true, SetupStep::Done),
            "/"
        ));
    }

    #[test]
    fn opening_step_resumes_but_never_past_database_in_setup_mode() {
        assert_eq!(
            opening_step(&status("running", true, SetupStep::Done), None),
            SetupStep::Welcome
        );
        assert_eq!(
            opening_step(&status("setup", false, SetupStep::Devices), None),
            SetupStep::Database
        );
        assert_eq!(
            opening_step(&status("running", false, SetupStep::Services), None),
            SetupStep::Services
        );
        // Run setup again on a finished hub: a save restarts it, and the reload must resume, not loop.
        assert_eq!(
            opening_step(
                &status("running", true, SetupStep::Done),
                Some(SetupStep::Mqtt)
            ),
            SetupStep::Mqtt
        );
        assert_eq!(
            opening_step(
                &status("setup", false, SetupStep::Welcome),
                Some(SetupStep::Services)
            ),
            SetupStep::Database
        );
        assert_eq!(setup_href(SetupStep::Mqtt), "/setup?step=mqtt");
    }
}
