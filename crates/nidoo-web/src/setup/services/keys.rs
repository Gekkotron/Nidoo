//! The Solcast, Frigate and Push forms of the Services step. Each is a hub settings section, so
//! a save restarts the hub. A form shows a few fields and sends the section's other keys back
//! exactly as the hub returned them, and a secret is always a string: `""` keeps the stored one.

use leptos::prelude::*;
use nidoo_api::{FrigateView, PushView, SolcastView, TestResult};
use serde_json::{Value, json};
use wasm_bindgen_futures::JsFuture;

use super::{EnabledRow, NotifyRow, load_section, save_section, with_fields};
use crate::api::setup::test_service;
use crate::i18n::setup::{SetupText, use_setup_text};
use crate::setup::connection::TextField;
use web_sys::HtmlInputElement;

/// The alert labels Frigate reports; a stored label outside this list still gets a chip.
const LABELS: [&str; 4] = ["person", "car", "dog", "cat"];

/// Splits on commas and whitespace, drops empty parts and keeps the first of each duplicate.
pub fn parse_list(s: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for part in s.split(|c: char| c == ',' || c.is_whitespace()) {
        if !part.is_empty() && !out.iter().any(|p| p == part) {
            out.push(part.to_owned());
        }
    }
    out
}

/// The `project_id` of a service-account JSON; `None` when the text is not JSON or has none.
pub fn project_id_of(json: &str) -> Option<String> {
    serde_json::from_str::<Value>(json)
        .ok()?
        .get("project_id")?
        .as_str()
        .filter(|p| !p.is_empty())
        .map(str::to_owned)
}

/// A whole number, 1 or more.
fn parse_calls(text: &str) -> Option<u32> {
    text.trim().parse::<u32>().ok().filter(|n| *n >= 1)
}

type Tested = RwSignal<Option<Result<TestResult, String>>>;

/// Runs a service test on `body` and stores the outcome; the hub's text is kept as received.
fn run_test(kind: &'static str, body: Value, out: Tested, testing: RwSignal<bool>) {
    testing.set(true);
    out.set(None);
    leptos::task::spawn_local(async move {
        let result = test_service(kind, &body).await.map_err(|e| e.error);
        out.try_set(Some(result));
        testing.try_set(false);
    });
}

/// A test's outcome line, plus `extra` text under it when the test passed.
fn outcome(
    out: Tested,
    extra: impl Fn(&TestResult) -> Option<String> + Send + Sync + 'static,
) -> impl IntoView {
    move || {
        out.get().map(|r| match r {
            Ok(r) => {
                let class = if r.ok {
                    "conn__line conn__line--ok"
                } else {
                    "conn__line conn__line--bad"
                };
                let more = r.ok.then(|| extra(&r)).flatten();
                view! {
                    <p class=class>{r.message.clone()}</p>
                    {more.map(|m| view! { <p class="settings__hint">{m}</p> })}
                }
                .into_any()
            }
            Err(e) => view! { <p class="conn__line conn__line--bad">{e}</p> }.into_any(),
        })
    }
}

/// What a form shows until its section has loaded, or when it could not.
fn loaded_view(
    loaded: RwSignal<Option<Result<Value, String>>>,
    form: impl Fn(Value) -> AnyView + Send + Sync + 'static,
) -> impl IntoView {
    let t = use_setup_text();
    move || {
        loaded.get().map(|result| match result {
            Ok(base) => form(base),
            Err(e) => view! {
                <p class="settings__empty" role="alert">
                    {format!("{} {e}", t(SetupText::LoadFailed))}
                </p>
            }
            .into_any(),
        })
    }
}

/// A switch-sized chip that toggles `name` in `list`.
#[component]
fn ToggleChip(name: String, list: RwSignal<Vec<String>>) -> impl IntoView {
    let on = {
        let name = name.clone();
        move || list.with(|l| l.contains(&name))
    };
    let toggle = {
        let name = name.clone();
        move |_| {
            list.update(|l| match l.iter().position(|n| *n == name) {
                Some(i) => {
                    l.remove(i);
                }
                None => l.push(name.clone()),
            })
        }
    };
    view! {
        <button
            type="button"
            class="chip"
            class:chip--selected=on.clone()
            aria-pressed=move || on().to_string()
            on:click=toggle
        >
            {name}
        </button>
    }
}

/// The Save button and the failure line under it.
#[component]
fn SaveRow(
    saving: RwSignal<bool>,
    failure: RwSignal<Option<String>>,
    #[prop(into)] blocked: Signal<bool>,
    on_save: Callback<()>,
) -> impl IntoView {
    let t = use_setup_text();
    view! {
        <p class="loc__problem" role="alert">{move || failure.get()}</p>
        <div class="loc__actions">
            <button
                type="button"
                class="btn btn--primary svc__save"
                prop:disabled=move || blocked.get() || saving.get()
                on:click=move |_| on_save.run(())
            >
                {move || t(SetupText::Save)}
            </button>
        </div>
    }
}

/// Saves `body` as `section`; the failure text lands in `failure`.
fn save(
    section: &'static str,
    body: Value,
    target: &str,
    saving: RwSignal<bool>,
    failure: RwSignal<Option<String>>,
) {
    if saving.get_untracked() {
        return;
    }
    saving.set(true);
    failure.set(None);
    let done = save_section(section, body, target);
    leptos::task::spawn_local(async move {
        if let Err(message) = done.await {
            failure.try_set(Some(message));
        }
        saving.try_set(false);
    });
}

#[component]
fn SolcastFields(base: Value, applying_target: String) -> impl IntoView {
    let t = use_setup_text();
    let view = serde_json::from_value::<SolcastView>(base.clone()).ok();
    let key_saved = view.as_ref().is_some_and(|v| v.api_key.set);
    let on = RwSignal::new(view.as_ref().is_some_and(|v| v.enabled));
    let notify = RwSignal::new(base["notify_unreachable"].as_bool().unwrap_or(false));
    let key = RwSignal::new(String::new());
    let sites = RwSignal::new(
        view.as_ref()
            .map(|v| v.sites.join(", "))
            .unwrap_or_default(),
    );
    let calls = RwSignal::new(
        view.as_ref()
            .map(|v| v.daily_limit.to_string())
            .unwrap_or_default(),
    );
    let saving = RwSignal::new(false);
    let testing = RwSignal::new(false);
    let failure = RwSignal::new(None::<String>);
    let tested: Tested = RwSignal::new(None);
    let body = move || {
        let n = parse_calls(&calls.get_untracked())?;
        Some(with_fields(
            base.clone(),
            &[
                ("enabled", json!(on.get_untracked())),
                ("api_key", json!(key.get_untracked())),
                ("sites", json!(parse_list(&sites.get_untracked()))),
                ("daily_limit", json!(n)),
                ("notify_unreachable", json!(notify.get_untracked())),
            ],
        ))
    };
    let invalid = move || parse_calls(&calls.get()).is_none();
    let placeholder = Signal::derive(move || key_saved.then(|| t(SetupText::SecretSaved)));
    let run = {
        let body = body.clone();
        move |_| {
            if testing.get_untracked() {
                return;
            }
            if let Some(b) = body() {
                run_test("solcast", b, tested, testing);
            }
        }
    };
    let on_save = Callback::new(move |()| {
        if let Some(b) = body() {
            save("solcast", b, &applying_target, saving, failure);
        }
    });
    view! {
        <div class="svc__form">
            <EnabledRow on=on />
            <NotifyRow on=notify />
            <div class="loc__grid">
                <TextField
                    id="solcast-key"
                    label=SetupText::SolcastKey
                    value=key
                    password=true
                    placeholder=placeholder
                />
                <TextField id="solcast-sites" label=SetupText::SolcastSites value=sites />
                <TextField id="solcast-calls" label=SetupText::CallsPerDay value=calls />
            </div>
            <p class="loc__problem" role="status">
                {move || invalid().then(|| t(SetupText::CallsInvalid))}
            </p>
            <div class="conn__result" role="status">
                {outcome(
                    tested,
                    |r| {
                        let names: Vec<String> = r
                            .detail
                            .as_ref()
                            .and_then(|d| d["sites"].as_array())
                            .map(|a| {
                                a.iter()
                                    .filter_map(|s| s["name"].as_str().map(str::to_owned))
                                    .collect()
                            })
                            .unwrap_or_default();
                        (!names.is_empty()).then(|| names.join(", "))
                    },
                )}
            </div>
            <div class="loc__actions">
                <button
                    type="button"
                    class="btn"
                    prop:disabled=move || invalid() || testing.get() || saving.get()
                    on:click=run
                >
                    {move || t(if testing.get() { SetupText::Testing } else { SetupText::TestSolcast })}
                </button>
            </div>
            <SaveRow saving=saving failure=failure blocked=invalid on_save=on_save />
        </div>
    }
}

/// Solcast's switch, API key, rooftop sites and daily call budget, with a one-call test.
#[component]
pub fn SolcastForm(applying_target: String) -> impl IntoView {
    let loaded = load_section("solcast");
    loaded_view(loaded, move |base| {
        view! { <SolcastFields base=base applying_target=applying_target.clone() /> }.into_any()
    })
}

#[component]
fn FrigateFields(base: Value, applying_target: String) -> impl IntoView {
    let t = use_setup_text();
    let view = serde_json::from_value::<FrigateView>(base.clone()).ok();
    let password_saved = view.as_ref().is_some_and(|v| v.password.set);
    let text_of = |f: fn(&FrigateView) -> &Option<String>| {
        RwSignal::new(view.as_ref().and_then(|v| f(v).clone()).unwrap_or_default())
    };
    let url = text_of(|v| &v.url);
    let user = text_of(|v| &v.user);
    let media_url = text_of(|v| &v.media_url);
    let password = RwSignal::new(String::new());
    let bridge = RwSignal::new(view.as_ref().is_some_and(|v| v.bridge));
    let live = RwSignal::new(view.as_ref().is_some_and(|v| v.live));
    let labels = RwSignal::new(view.as_ref().map(|v| v.labels.clone()).unwrap_or_default());
    let cameras = RwSignal::new(view.as_ref().map(|v| v.cameras.clone()).unwrap_or_default());
    // The label chips are the known labels plus whatever else is stored.
    let label_chips: Vec<String> = LABELS
        .iter()
        .map(|l| (*l).to_owned())
        .chain(
            labels
                .get_untracked()
                .into_iter()
                .filter(|l| !LABELS.contains(&l.as_str())),
        )
        .collect();
    // Camera chips: the stored cameras plus any a test finds. Never derived from the live
    // selection, so a camera that is switched off keeps its chip and can be switched back on.
    let known = RwSignal::new(cameras.get_untracked());
    let saving = RwSignal::new(false);
    let testing = RwSignal::new(false);
    let failure = RwSignal::new(None::<String>);
    let tested: Tested = RwSignal::new(None);
    let body = move || {
        with_fields(
            base.clone(),
            &[
                ("url", json!(url.get_untracked())),
                ("user", json!(user.get_untracked())),
                ("password", json!(password.get_untracked())),
                ("media_url", json!(media_url.get_untracked())),
                ("bridge", json!(bridge.get_untracked())),
                ("labels", json!(labels.get_untracked())),
                ("cameras", json!(cameras.get_untracked())),
                ("live", json!(live.get_untracked())),
            ],
        )
    };
    let placeholder = Signal::derive(move || password_saved.then(|| t(SetupText::SecretSaved)));
    let run = {
        let body = body.clone();
        move |_| {
            if testing.get_untracked() {
                return;
            }
            run_test("frigate", body(), tested, testing);
        }
    };
    // A passing test lists the cameras Frigate knows.
    Effect::new(move |_| {
        if let Some(Ok(r)) = tested.get() {
            let names: Vec<String> = r
                .detail
                .as_ref()
                .and_then(|d| d["cameras"].as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|c| c.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default();
            known.try_update(|k| {
                for n in names {
                    if !k.contains(&n) {
                        k.push(n);
                    }
                }
            });
        }
    });
    let on_save = Callback::new(move |()| {
        save("frigate", body(), &applying_target, saving, failure);
    });
    let flag = |label: SetupText, on: RwSignal<bool>| {
        view! {
            <label class="override__flag svc__switch">
                <span class="override__label">{move || t(label)}</span>
                <button
                    type="button"
                    role="switch"
                    class="switch"
                    class:switch--on=move || on.get()
                    aria-checked=move || on.get().to_string()
                    on:click=move |_| on.update(|v| *v = !*v)
                >
                    <span class="switch__knob"></span>
                </button>
            </label>
        }
    };
    view! {
        <div class="svc__form">
            <div class="loc__grid">
                <TextField id="frigate-url" label=SetupText::FrigateUrl value=url wide=true />
                <TextField id="frigate-user" label=SetupText::User value=user />
                <TextField
                    id="frigate-password"
                    label=SetupText::Password
                    value=password
                    password=true
                    placeholder=placeholder
                />
                <TextField
                    id="frigate-media"
                    label=SetupText::MediaUrl
                    value=media_url
                    wide=true
                />
            </div>
            {flag(SetupText::AlertsBridge, bridge)}
            <p class="svc__label">{move || t(SetupText::AlertOn)}</p>
            <div class="svc__chips">
                {label_chips
                    .into_iter()
                    .map(|l| view! { <ToggleChip name=l list=labels /> })
                    .collect_view()}
            </div>
            {flag(SetupText::LiveView, live)}
            <div class="conn__result" role="status">{outcome(tested, |_| None)}</div>
            <Show when=move || known.with(|k| !k.is_empty())>
                <p class="svc__label">{move || t(SetupText::FrigateCameras)}</p>
                <p class="settings__hint">{move || t(SetupText::CamerasAllHint)}</p>
                <div class="svc__chips">
                    {move || {
                        known
                            .get()
                            .into_iter()
                            .map(|c| view! { <ToggleChip name=c list=cameras /> })
                            .collect_view()
                    }}
                </div>
            </Show>
            <div class="loc__actions">
                <button
                    type="button"
                    class="btn"
                    prop:disabled=move || testing.get() || saving.get()
                    on:click=run
                >
                    {move || t(if testing.get() { SetupText::Testing } else { SetupText::TestConnection })}
                </button>
            </div>
            <SaveRow saving=saving failure=failure blocked=false on_save=on_save />
        </div>
    }
}

/// Frigate's address, login, alert options and camera filter, with a connection test.
#[component]
pub fn FrigateForm(applying_target: String) -> impl IntoView {
    let loaded = load_section("frigate");
    loaded_view(loaded, move |base| {
        view! { <FrigateFields base=base applying_target=applying_target.clone() /> }.into_any()
    })
}

#[component]
fn PushFields(base: Value, applying_target: String) -> impl IntoView {
    let t = use_setup_text();
    let lang = crate::i18n::use_lang();
    let view = serde_json::from_value::<PushView>(base.clone()).ok();
    let project = view.and_then(|v| v.project_id);
    // The text of a chosen file that passed the project check; `None` until one does.
    let pending = RwSignal::new(None::<String>);
    let not_a_key = RwSignal::new(false);
    let read_failed = RwSignal::new(false);
    // Bumped on every selection so a slow read of an earlier file is ignored.
    let generation = RwSignal::new(0u32);
    let saving = RwSignal::new(false);
    let failure = RwSignal::new(None::<String>);
    let on_change = move |ev: leptos::ev::Event| {
        let input = event_target::<HtmlInputElement>(&ev);
        let mine = generation.get_untracked() + 1;
        generation.set(mine);
        pending.set(None);
        not_a_key.set(false);
        read_failed.set(false);
        let Some(file) = input.files().and_then(|f| f.get(0)) else {
            return;
        };
        leptos::task::spawn_local(async move {
            let read = JsFuture::from(file.text())
                .await
                .ok()
                .and_then(|v| v.as_string());
            if generation.try_get_untracked() != Some(mine) {
                return;
            }
            match read {
                None => {
                    read_failed.try_set(true);
                }
                Some(text) if project_id_of(&text).is_none() => {
                    not_a_key.try_set(true);
                }
                Some(text) => {
                    pending.try_set(Some(text));
                }
            }
        });
    };
    let on_save = Callback::new(move |()| {
        if let Some(text) = pending.get_untracked() {
            let body = with_fields(base.clone(), &[("service_account", json!(text))]);
            save("push", body, &applying_target, saving, failure);
        }
    });
    view! {
        <div class="svc__form">
            {project
                .map(|p| {
                    view! {
                        <p class="settings__hint">
                            {move || {
                                crate::i18n::setup::setup_text(lang(), SetupText::PushProject)
                                    .replace("{project}", &p)
                            }}
                        </p>
                    }
                })}
            <label class="field" for="push-file">
                <span class="field__caption">{move || t(SetupText::PushFile)}</span>
                <input
                    id="push-file"
                    class="field__input"
                    type="file"
                    accept=".json"
                    on:change=on_change
                />
            </label>
            <p class="loc__problem" role="status">
                {move || {
                    if read_failed.get() {
                        Some(t(SetupText::FileReadFailed))
                    } else {
                        not_a_key.get().then(|| t(SetupText::NotAServiceAccount))
                    }
                }}
            </p>
            <SaveRow
                saving=saving
                failure=failure
                blocked=Signal::derive(move || pending.with(Option::is_none))
                on_save=on_save
            />
        </div>
    }
}

/// The push service-account key upload.
#[component]
pub fn PushForm(applying_target: String) -> impl IntoView {
    let loaded = load_section("push");
    loaded_view(loaded, move |base| {
        view! { <PushFields base=base applying_target=applying_target.clone() /> }.into_any()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_split_on_commas_and_whitespace() {
        assert_eq!(parse_list("a, b  c,,a"), ["a", "b", "c"]);
        assert!(parse_list(" , ").is_empty());
    }

    #[test]
    fn the_project_id_comes_from_the_key_file() {
        assert_eq!(
            project_id_of(r#"{"project_id":"demo-123","type":"service_account"}"#),
            Some("demo-123".to_owned())
        );
        assert_eq!(project_id_of("nope"), None);
        assert_eq!(project_id_of(r#"{"type":"service_account"}"#), None);
        assert_eq!(project_id_of(r#"{"project_id":7}"#), None);
    }

    #[test]
    fn calls_per_day_is_a_whole_number_of_one_or_more() {
        assert_eq!(parse_calls("10"), Some(10));
        assert_eq!(parse_calls("0"), None);
        assert_eq!(parse_calls("x"), None);
    }
}
