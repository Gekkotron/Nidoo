//! `/assistant/settings`: the languages the hub speaks (tabs, in `assist_languages`), the
//! installed models, and adding a model with the Hugging Face token folded in.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use leptos::prelude::*;
use leptos::task::spawn_local;
use nidoo_api::{
    DEFAULT_VOICE, DownloadRequest, EngineState, JobState, ModelView, RepoListing, TtsStatus,
    VoiceView, VoicesView, WeightFile,
};
use wasm_bindgen::JsCast;

use super::assist_languages::LanguagesCard;
use crate::api::tts;
use crate::components::confirm::ConfirmBar;
use crate::components::toast::Toasts;
use crate::i18n::{
    AssistSettingsText as T, Lang, assist_settings_text, language_name, use_assist_settings_text,
    use_lang,
};

/// How often the status is fetched while a download runs or a model loads.
const POLL_MS: u32 = 1000;

/// The file names the hub accepts in an upload; anything else is skipped here (the hub would
/// refuse it anyway).
const UPLOAD_NAMES: &[&str] = &[
    "config.json",
    "generation_config.json",
    "processor_config.json",
    "tokenizer.json",
    "tokenizer.5.0.0.json",
    "tokenizer.model",
    "tokenizer_config.json",
    "chat_template.jinja",
    "default-voice.safetensors",
    "model.q8.gguf",
    "model.q4k.gguf",
    "model.safetensors",
];

/// A byte count in the largest decimal unit, one decimal: "74,6 Mo" or "74.6 MB".
pub fn size_label(bytes: u64, lang: Lang) -> String {
    let (div, unit) = match bytes {
        1_000_000_000.. => (1e9, T::UnitGb),
        1_000_000.. => (1e6, T::UnitMb),
        1_000.. => (1e3, T::UnitKb),
        _ => return format!("{bytes} {}", assist_settings_text(lang, T::UnitB)),
    };
    let n = format!("{:.1}", bytes as f64 / div);
    let n = if lang == Lang::Fr {
        n.replace('.', ",")
    } else {
        n
    };
    format!("{n} {}", assist_settings_text(lang, unit))
}

/// How far a transfer is, in whole percent: 0 when the total is unknown, never over 100.
pub fn progress_pct(done: u64, total: u64) -> u8 {
    if total == 0 {
        return 0;
    }
    (u128::from(done) * 100 / u128::from(total)).min(100) as u8
}

/// The weight file to offer first: the 8-bit one, else the 4-bit one, else the first listed.
pub fn preselect(weights: &[WeightFile]) -> Option<String> {
    ["model.q8.gguf", "model.q4k.gguf"]
        .iter()
        .find(|want| weights.iter().any(|w| w.name == **want))
        .map(|want| (*want).to_owned())
        .or_else(|| weights.first().map(|w| w.name.clone()))
}

/// Whether a picked file goes up: an allowed name, straight in the chosen folder (its relative
/// path is `folder/name`; empty when the browser gives none).
pub fn uploadable(name: &str, relative_path: &str) -> bool {
    UPLOAD_NAMES.contains(&name) && relative_path.matches('/').count() <= 1
}

/// The path the browser gives a file picked inside a folder (`folder/name`); empty when none.
fn relative_path(file: &web_sys::File) -> String {
    js_sys::Reflect::get(file, &wasm_bindgen::JsValue::from_str("webkitRelativePath"))
        .ok()
        .and_then(|v| v.as_string())
        .unwrap_or_default()
}

/// Whether the status is worth fetching again soon: a download runs or a model loads.
pub fn needs_polling(status: &TtsStatus) -> bool {
    status
        .languages
        .iter()
        .any(|l| matches!(l.engine, EngineState::Loading { .. }))
        || status
            .download
            .as_ref()
            .is_some_and(|d| d.state == JobState::Running)
}

/// The facts of a voice on one line: gender, age, description, the empty ones left out.
pub(crate) fn voice_detail(v: &VoiceView) -> String {
    [&v.gender, &v.age, &v.description]
        .into_iter()
        .flatten()
        .filter(|s| !s.is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join(" · ")
}

/// The voices with Default first, the rest in the hub's order.
pub(crate) fn voices_in_order(view: &VoicesView) -> Vec<VoiceView> {
    let mut list = view.voices.clone();
    list.sort_by_key(|v| v.name != DEFAULT_VOICE);
    list
}

/// The model's name: its repository, or "Uploaded".
pub(crate) fn model_name(m: &ModelView, lang: Lang) -> String {
    m.repo
        .clone()
        .unwrap_or_else(|| assist_settings_text(lang, T::Uploaded).to_owned())
}

/// The facts of a model on one line: weight file, size, short commit, licence.
fn model_detail(m: &ModelView, lang: Lang) -> String {
    let mut parts = vec![m.weights.clone(), size_label(m.size, lang)];
    if let Some(rev) = &m.revision {
        parts.push(rev.chars().take(7).collect());
    }
    if let Some(licence) = &m.licence {
        parts.push(licence.clone());
    }
    parts.join(" · ")
}

/// Language codes as names in the UI language, comma-separated.
fn language_names(codes: &[String], lang: Lang) -> String {
    codes
        .iter()
        .map(|c| language_name(c, lang))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The speech settings screen.
#[component]
pub fn AssistSettingsScreen() -> impl IntoView {
    let lang = use_lang();
    let t = use_assist_settings_text();
    let toasts = use_context::<Toasts>();
    let toast = move |text: String| {
        if let Some(toasts) = toasts {
            toasts.push(text);
        }
    };

    let status = RwSignal::new(None::<TtsStatus>);
    let models = RwSignal::new(Vec::<ModelView>::new());
    let loaded = RwSignal::new(false);
    let load_failed = RwSignal::new(false);

    // Everything the page shows, fetched again. `try_`: the page may be gone by the answer.
    let reload = move || {
        spawn_local(async move {
            match tts::get_status().await {
                Ok(s) => {
                    status.try_set(Some(s));
                    load_failed.try_set(false);
                }
                Err(_) => {
                    load_failed.try_set(true);
                }
            }
            if let Ok(list) = tts::get_models().await {
                models.try_set(list);
            }
            loaded.try_set(true);
        });
    };
    reload();
    let reload_cb = Callback::new(move |()| reload());

    // The status again every second while a download runs or a model loads; when that ends,
    // the lists too (a model was installed).
    let stop = Arc::new(AtomicBool::new(false));
    {
        let stop = stop.clone();
        spawn_local(async move {
            loop {
                gloo_timers::future::TimeoutFuture::new(POLL_MS).await;
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                let moving = status
                    .try_with_untracked(|s| s.as_ref().is_some_and(needs_polling))
                    .unwrap_or(false);
                if !moving {
                    continue;
                }
                if let Ok(s) = tts::get_status().await {
                    let still = needs_polling(&s);
                    if status.try_set(Some(s)).is_some() || stop.load(Ordering::Relaxed) {
                        break;
                    }
                    if !still {
                        reload();
                    }
                }
            }
        });
    }
    on_cleanup(move || stop.store(true, Ordering::Relaxed));

    // ---- Models ----
    let removing = RwSignal::new(None::<String>);
    let remove = move |id: String| {
        removing.set(None);
        spawn_local(async move {
            if let Err(e) = tts::remove_model(&id).await {
                toast(e.error);
            }
            reload();
        });
    };

    // ---- Add a model ----
    let url = RwSignal::new(String::new());
    let listing = RwSignal::new(None::<RepoListing>);
    let weight = RwSignal::new(String::new());
    let listing_busy = RwSignal::new(false);
    let add_error = RwSignal::new(None::<String>);
    let list_files = move || {
        let text = url.get_untracked().trim().to_owned();
        if text.is_empty() || listing_busy.get_untracked() {
            return;
        }
        listing_busy.set(true);
        add_error.set(None);
        spawn_local(async move {
            match tts::inspect(&text).await {
                Ok(found) => {
                    weight.try_set(preselect(&found.weights).unwrap_or_default());
                    listing.try_set(Some(found));
                }
                Err(e) => {
                    listing.try_set(None);
                    add_error.try_set(Some(e.error));
                }
            }
            listing_busy.try_set(false);
        });
    };
    let download = move || {
        let Some(found) = listing.get_untracked() else {
            return;
        };
        let req = DownloadRequest {
            repo: found.repo,
            revision: found.revision,
            weights: weight.get_untracked(),
        };
        add_error.set(None);
        spawn_local(async move {
            match tts::start_download(&req).await {
                Ok(_) => {
                    if let Ok(s) = tts::get_status().await {
                        status.try_set(Some(s));
                    }
                }
                Err(e) => {
                    add_error.try_set(Some(e.error));
                }
            }
        });
    };
    let cancel = move || {
        spawn_local(async move {
            if let Err(e) = tts::cancel_download().await {
                toast(e.error);
            }
            if let Ok(s) = tts::get_status().await {
                status.try_set(Some(s));
            }
            reload();
        });
    };

    // ---- Upload ----
    let uploading = RwSignal::new(None::<(usize, usize)>);
    // `webkitdirectory` has no typed attribute: set it once the field exists.
    let folder_input = NodeRef::<leptos::html::Input>::new();
    Effect::new(move |_| {
        if let Some(el) = folder_input.get() {
            let _ = el.set_attribute("webkitdirectory", "");
        }
    });
    let upload_error = RwSignal::new(None::<String>);
    let on_pick = move |ev: web_sys::Event| {
        let Some(input) = ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
        else {
            return;
        };
        let mut files = Vec::new();
        if let Some(list) = input.files() {
            for i in 0..list.length() {
                if let Some(file) = list.get(i)
                    && uploadable(&file.name(), &relative_path(&file))
                {
                    files.push(file);
                }
            }
        }
        // So that choosing the same folder again fires another change.
        input.set_value("");
        upload_error.set(None);
        if files.is_empty() {
            upload_error.set(Some(t(T::UploadNothing).to_owned()));
            return;
        }
        if uploading.get_untracked().is_some() {
            return;
        }
        spawn_local(async move {
            let total = files.len();
            uploading.try_set(Some((0, total)));
            let result = async {
                let session = tts::upload_open().await?;
                for (i, file) in files.iter().enumerate() {
                    if let Err(e) = tts::upload_file(&session.id, &file.name(), file).await {
                        // Best effort: the hub also drops an idle upload when a new one opens.
                        let _ = tts::upload_abort(&session.id).await;
                        return Err(e);
                    }
                    uploading.try_set(Some((i + 1, total)));
                }
                match tts::upload_finish(&session.id).await {
                    Ok(_) => Ok(()),
                    Err(e) => {
                        let _ = tts::upload_abort(&session.id).await;
                        Err(e)
                    }
                }
            }
            .await;
            uploading.try_set(None);
            if let Err(e) = result {
                upload_error.try_set(Some(e.error));
            }
            reload();
        });
    };

    // ---- Token ----
    let token = RwSignal::new(String::new());
    let save_token = move || {
        let value = token.get_untracked();
        if value.trim().is_empty() {
            return;
        }
        token.set(String::new());
        spawn_local(async move {
            match tts::set_token(Some(value.trim())).await {
                Ok(()) => toast(t(T::TokenSaved).to_owned()),
                Err(e) => toast(e.error),
            }
            reload();
        });
    };
    let clear_token = move || {
        token.set(String::new());
        spawn_local(async move {
            match tts::set_token(None).await {
                Ok(()) => toast(t(T::TokenCleared).to_owned()),
                Err(e) => toast(e.error),
            }
            reload();
        });
    };

    let token_set = move || status.with(|s| s.as_ref().is_some_and(|s| s.token_set));

    view! {
        <div class="assist-set">
            <a class="assist-set__back" href="/assistant">
                <span aria-hidden="true">"←"</span>
                " "
                {move || t(T::Back)}
            </a>
            <h1 class="assist-set__title">{move || t(T::Title)}</h1>

            <Show when=move || load_failed.get()>
                <p class="assist-set__error">{move || t(T::CouldNotLoad)}</p>
            </Show>
            <Show when=move || !loaded.get()>
                <p class="assist-set__muted">{move || t(T::Loading)}</p>
            </Show>

            <LanguagesCard status=status models=models reload=reload_cb />

            <crate::screens::speech_input::SpeechInputSection />

            // ---- Installed models ----
            <section class="assist-set__card">
                <h2>{move || t(T::ModelsHeading)}</h2>
                <Show when=move || loaded.get() && models.with(Vec::is_empty)>
                    <p class="assist-set__muted">{move || t(T::ModelsEmpty)}</p>
                </Show>
                <ul class="assist-set__list">
                    {move || {
                        models
                            .get()
                            .into_iter()
                            .map(|m| {
                                let id = m.id.clone();
                                let ask_id = m.id.clone();
                                let asking = {
                                    let id = m.id.clone();
                                    move || removing.with(|r| r.as_deref() == Some(id.as_str()))
                                };
                                let yes_id = m.id.clone();
                                let name = model_name(&m, lang());
                                let detail = model_detail(&m, lang());
                                let speaks = if m.languages.is_empty() {
                                    t(T::LanguagesUnknown).to_owned()
                                } else {
                                    language_names(&m.languages, lang())
                                };
                                let used_by = (!m.used_by.is_empty())
                                    .then(|| language_names(&m.used_by, lang()));
                                let question = used_by.clone();
                                view! {
                                    <li class="assist-set__item" data-model=id>
                                        <div class="assist-set__row">
                                            <div class="assist-set__grow">
                                                <span class="assist-set__name">{name}</span>
                                                <span class="assist-set__muted">" " {detail} " · " {speaks}</span>
                                                {used_by.map(|names| view! {
                                                    <p class="assist-set__muted">{move || t(T::UsedBy)} ": " {names}</p>
                                                })}
                                            </div>
                                            <button
                                                type="button"
                                                class="btn assist-set__btn"
                                                on:click=move |_| removing.set(Some(ask_id.clone()))
                                            >
                                                {move || t(T::Remove)}
                                            </button>
                                        </div>
                                        <Show when=asking>
                                            <ConfirmBar
                                                question=Signal::derive({
                                                    let question = question.clone();
                                                    move || match &question {
                                                    Some(names) => format!("{} {names}", t(T::RemoveUsedQuestion)),
                                                    None => t(T::RemoveQuestion).to_owned(),
                                                }})
                                                yes=Signal::derive(move || t(T::RemoveYes).to_owned())
                                                on_yes=Callback::new({
                                                    let yes_id = yes_id.clone();
                                                    move |()| remove(yes_id.clone())
                                                })
                                                on_cancel=Callback::new(move |()| removing.set(None))
                                            />
                                        </Show>
                                    </li>
                                }
                            })
                            .collect_view()
                    }}
                </ul>
            </section>

            // ---- Add a model ----
            <section class="assist-set__card">
                <h2>{move || t(T::AddHeading)}</h2>
                <div class="assist-set__row">
                    <input
                        class="field assist-set__grow"
                        type="text"
                        autocomplete="off"
                        aria-label=move || t(T::UrlLabel)
                        placeholder=move || t(T::UrlPlaceholder)
                        prop:value=move || url.get()
                        on:input=move |ev| url.set(event_target_value(&ev))
                        on:keydown=move |ev: web_sys::KeyboardEvent| {
                            if ev.key() == "Enter" && !ev.is_composing() {
                                ev.prevent_default();
                                list_files();
                            }
                        }
                    />
                    <button
                        type="button"
                        class="btn assist-set__btn"
                        disabled=move || listing_busy.get()
                        on:click=move |_| list_files()
                    >
                        {move || if listing_busy.get() { t(T::Listing) } else { t(T::ListFiles) }}
                    </button>
                </div>
                {move || {
                    listing
                        .get()
                        .map(|found| {
                            let gated = found.gated;
                            let repo_url = format!("https://huggingface.co/{}", found.repo);
                            let repo = found.repo.clone();
                            let licence = found.licence.clone();
                            let weights = found.weights.clone();
                            view! {
                                <p>
                                    <span class="assist-set__name">{repo}</span>
                                    {licence.map(|l| view! { <span class="assist-set__muted">" · " {l}</span> })}
                                    " "
                                    <a href=repo_url target="_blank">
                                        {move || t(T::OpenRepo)}
                                    </a>
                                </p>
                                <Show when=move || gated && !token_set()>
                                    <p class="assist-set__warn">{move || t(T::NeedsToken)}</p>
                                </Show>
                                <fieldset class="assist-set__weights">
                                    <legend>{move || t(T::WeightsLabel)}</legend>
                                    {weights
                                        .into_iter()
                                        .map(|w| {
                                            let name = w.name.clone();
                                            let pick = w.name.clone();
                                            view! {
                                                <label class="assist-set__item">
                                                    <input
                                                        type="radio"
                                                        name="assist-weights"
                                                        prop:checked=move || weight.get() == name
                                                        on:change=move |_| weight.set(pick.clone())
                                                    />
                                                    <span class="assist-set__name">{w.name.clone()}</span>
                                                    <span class="assist-set__muted">
                                                        " " {move || size_label(w.size, lang())}
                                                    </span>
                                                </label>
                                            }
                                        })
                                        .collect_view()}
                                </fieldset>
                                <button
                                    type="button"
                                    class="btn assist-set__btn"
                                    disabled=move || {
                                        weight.with(String::is_empty)
                                            || status.with(|s| s.as_ref().is_some_and(needs_polling))
                                    }
                                    on:click=move |_| download()
                                >
                                    {move || t(T::Download)}
                                </button>
                            }
                        })
                }}
                {move || add_error.get().map(|e| view! { <p class="assist-set__error">{e}</p> })}
                {move || {
                    status
                        .get()
                        .and_then(|s| s.download)
                        .map(|d| {
                            let pct = progress_pct(d.done, d.total);
                            let running = d.state == JobState::Running;
                            let summary = match d.state {
                                JobState::Running => None,
                                JobState::Done => Some((T::DownloadDone, false)),
                                JobState::Failed => Some((T::DownloadFailed, true)),
                                JobState::Cancelled => Some((T::DownloadCancelled, false)),
                            };
                            view! {
                                <div class="assist-set__download">
                                    {running
                                        .then(|| {
                                            view! {
                                                <div class="assist-set__row">
                                                    <progress class="assist-set__bar" max="100" value=pct></progress>
                                                    <span class="assist-set__muted">{pct} " %"</span>
                                                    <button
                                                        type="button"
                                                        class="btn assist-set__btn"
                                                        on:click=move |_| cancel()
                                                    >
                                                        {move || t(T::Cancel)}
                                                    </button>
                                                </div>
                                                <p class="assist-set__muted">
                                                    {move || t(T::Downloading)} " " {d.file.clone()} " · "
                                                    {move || size_label(d.done, lang())} " / "
                                                    {move || size_label(d.total, lang())}
                                                </p>
                                            }
                                        })}
                                    {summary
                                        .map(|(text, bad)| {
                                            view! {
                                                <p class=if bad { "assist-set__error" } else { "assist-set__muted" }>
                                                    {move || t(text)}
                                                    {d.error.clone().map(|e| format!(": {e}"))}
                                                </p>
                                            }
                                        })}
                                </div>
                            }
                        })
                }}

                <h3>{move || t(T::UploadHeading)}</h3>
                <p class="assist-set__muted">{move || t(T::UploadHint)}</p>
                <label class="btn assist-set__btn assist-set__file">
                    <span>{move || t(T::UploadChoose)}</span>
                    <input
                        type="file"
                        multiple=true
                        node_ref=folder_input
                        class="assist-set__file-input"
                        disabled=move || uploading.get().is_some()
                        on:change=on_pick
                    />
                </label>
                {move || {
                    uploading
                        .get()
                        .map(|(done, total)| {
                            view! {
                                <p class="assist-set__muted">
                                    {move || t(T::Uploading)} " " {done} " / " {total}
                                </p>
                            }
                        })
                }}
                {move || upload_error.get().map(|e| view! { <p class="assist-set__error">{e}</p> })}

                <details class="assist-set__token">
                    <summary>
                        {move || t(T::TokenFold)}
                        " · "
                        {move || if token_set() { t(T::TokenSet) } else { t(T::TokenNotSet) }}
                    </summary>
                    <p class="assist-set__muted">{move || t(T::GatedNote)}</p>
                    <div class="assist-set__row">
                        <input
                            class="field assist-set__grow"
                            type="password"
                            autocomplete="off"
                            aria-label=move || t(T::TokenLabel)
                            prop:value=move || token.get()
                            on:input=move |ev| token.set(event_target_value(&ev))
                        />
                        <button
                            type="button"
                            class="btn assist-set__btn"
                            disabled=move || token.with(|v| v.trim().is_empty())
                            on:click=move |_| save_token()
                        >
                            {move || t(T::Save)}
                        </button>
                        <button
                            type="button"
                            class="btn assist-set__btn"
                            disabled=move || !token_set()
                            on:click=move |_| clear_token()
                        >
                            {move || t(T::Clear)}
                        </button>
                    </div>
                </details>
            </section>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nidoo_api::DownloadView;

    #[test]
    fn preselects_q8_then_q4k_then_first() {
        let w = |n: &str| WeightFile {
            name: n.into(),
            size: 1,
        };
        assert_eq!(
            preselect(&[w("model.safetensors"), w("model.q8.gguf")]).as_deref(),
            Some("model.q8.gguf")
        );
        assert_eq!(
            preselect(&[w("model.safetensors"), w("model.q4k.gguf")]).as_deref(),
            Some("model.q4k.gguf")
        );
        assert_eq!(
            preselect(&[w("model.safetensors")]).as_deref(),
            Some("model.safetensors")
        );
        assert_eq!(preselect(&[]), None);
    }

    #[test]
    fn progress_never_divides_by_zero_or_overflows() {
        assert_eq!(progress_pct(5, 0), 0);
        assert_eq!(progress_pct(50, 200), 25);
        assert_eq!(progress_pct(300, 200), 100);
    }

    #[test]
    fn only_model_files_straight_in_the_chosen_folder_go_up() {
        assert!(uploadable("config.json", "model/config.json"));
        assert!(uploadable("model.q8.gguf", ""));
        assert!(!uploadable("README.md", "model/README.md"));
        assert!(!uploadable("config.json", "model/onnx/config.json"));
    }

    #[test]
    fn the_page_polls_only_while_something_is_moving() {
        let language = |engine| nidoo_api::LanguageView {
            code: "fr".into(),
            model: Some("m".into()),
            voice: DEFAULT_VOICE.into(),
            engine,
        };
        let mut s = TtsStatus {
            languages: vec![language(EngineState::NoModel)],
            token_set: false,
            download: None,
            speech_built: true,
        };
        assert!(!needs_polling(&s));
        s.languages
            .push(language(EngineState::Loading { model: "m".into() }));
        assert!(needs_polling(&s));
        s.languages[1] = language(EngineState::Ready { model: "m".into() });
        assert!(!needs_polling(&s));
        let mut d = DownloadView {
            model: "m".into(),
            state: JobState::Running,
            done: 1,
            total: 2,
            file: "f".into(),
            error: None,
        };
        s.download = Some(d.clone());
        assert!(needs_polling(&s));
        d.state = JobState::Done;
        s.download = Some(d);
        assert!(!needs_polling(&s));
    }

    #[test]
    fn sizes_read_in_each_language() {
        assert_eq!(size_label(74_553_568, Lang::Fr), "74,6 Mo");
        assert_eq!(size_label(74_553_568, Lang::En), "74.6 MB");
    }
}
