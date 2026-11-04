//! Assistant → Speech settings → Speech input: the recognition model (download, state), its
//! settings and the satellites' shared key with the line to paste into `secrets.yaml`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use leptos::prelude::*;
use leptos::task::spawn_local;
use nidoo_api::{EngineState, JobState, SttSettings, SttStatus};

use crate::api::stt;
use crate::i18n::{SpeechInputText as T, use_speech_input_text};
use crate::screens::assist_settings::progress_pct;
use crate::screens::settings::CopyButton;

const POLL_MS: u32 = 1000;

/// Whether the card should ask again soon: a download runs or the model loads.
pub fn stt_moving(status: &SttStatus) -> bool {
    status
        .download
        .as_ref()
        .is_some_and(|d| d.state == JobState::Running)
        || matches!(status.engine, EngineState::Loading { .. })
}

/// The line each satellite's `secrets.yaml` takes.
pub fn secrets_line(key: &str) -> String {
    format!("voice_key: \"{key}\"")
}

#[component]
pub fn SpeechInputSection() -> impl IntoView {
    let t = use_speech_input_text();
    let status = RwSignal::new(None::<SttStatus>);
    let error = RwSignal::new(None::<String>);
    let key = RwSignal::new(None::<String>);
    let confirm_replace = RwSignal::new(false);
    let form = RwSignal::new(SttSettings::default());

    let load = move || {
        spawn_local(async move {
            if let Ok(s) = stt::get_status().await {
                form.try_set(s.settings.clone());
                status.try_set(Some(s));
            }
        });
    };
    load();
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
                    .try_with_untracked(|s| s.as_ref().is_some_and(stt_moving))
                    .unwrap_or(false);
                if moving && let Ok(s) = stt::get_status().await {
                    status.try_set(Some(s));
                }
            }
        });
    }
    on_cleanup(move || stop.store(true, Ordering::Relaxed));

    let failed = move |e: nidoo_api::ApiError| {
        error.try_set(Some(e.error));
    };
    let download = move |_| {
        error.set(None);
        spawn_local(async move {
            match stt::start_download().await {
                Ok(_) => load(),
                Err(e) => failed(e),
            }
        });
    };
    let cancel = move |_| {
        spawn_local(async move {
            if let Err(e) = stt::cancel_download().await {
                failed(e);
            }
            load();
        });
    };
    let remove = move |_| {
        spawn_local(async move {
            if let Err(e) = stt::remove_model().await {
                failed(e);
            }
            load();
        });
    };
    let save = move |_| {
        error.set(None);
        let settings = form.get_untracked();
        spawn_local(async move {
            match stt::put_settings(&settings).await {
                Ok(s) => {
                    form.try_set(s.settings.clone());
                    status.try_set(Some(s));
                }
                Err(e) => failed(e),
            }
        });
    };
    let make_key = move || {
        confirm_replace.set(false);
        spawn_local(async move {
            match stt::new_key().await {
                Ok(k) => {
                    key.try_set(Some(k.key));
                    load();
                }
                Err(e) => failed(e),
            }
        });
    };
    let has_key = move || status.with(|s| s.as_ref().is_some_and(|s| s.has_key));
    let number = move |value: String, set: &dyn Fn(&mut SttSettings, f64)| {
        if let Ok(v) = value.trim().parse::<f64>() {
            form.update(|f| set(f, v));
        }
    };

    let model_line = move || {
        let Some(s) = status.get() else {
            return ().into_any();
        };
        if !s.built {
            return view! { <p class="assist-set__muted">{t(T::NotBuilt)}</p> }.into_any();
        }
        let running = s
            .download
            .as_ref()
            .filter(|d| d.state == JobState::Running)
            .cloned();
        if let Some(d) = running {
            let pct = progress_pct(d.done, d.total);
            return view! {
                <p class="speech-in__state">{format!("{} {pct} %", t(T::Downloading))}</p>
                <div class="assist-set__row">
                    <button type="button" class="btn assist-set__btn speech-in__cancel" on:click=cancel>
                        {t(T::Cancel)}
                    </button>
                </div>
            }
            .into_any();
        }
        let last_error = s
            .download
            .as_ref()
            .filter(|d| d.state == JobState::Failed)
            .and_then(|d| d.error.clone());
        if !s.installed {
            return view! {
                <p class="speech-in__state">{t(T::NotInstalled)}</p>
                {last_error.map(|e| view! { <p class="assist-set__error">{e}</p> })}
                <div class="assist-set__row">
                    <button type="button" class="btn assist-set__btn speech-in__download" on:click=download>
                        {t(T::Download)}
                    </button>
                </div>
            }
            .into_any();
        }
        let state = match &s.engine {
            EngineState::Loading { .. } | EngineState::NoModel => t(T::Loading).to_owned(),
            EngineState::Ready { .. } => t(T::Ready).to_owned(),
            EngineState::Error { message, .. } => message.clone(),
        };
        view! {
            <p class="speech-in__state">{state}</p>
            <div class="assist-set__row">
                <button type="button" class="btn assist-set__btn speech-in__remove" on:click=remove>
                    {t(T::Remove)}
                </button>
            </div>
        }
        .into_any()
    };

    let port_note = move || {
        status.with(|s| {
            s.as_ref()
                .is_some_and(|s| s.settings.port != s.port_in_use)
                .then(|| view! { <p class="assist-set__muted">{t(T::PortAfterRestart)}</p> })
        })
    };

    view! {
        <section class="assist-set__card speech-in">
            <h2>{move || t(T::Heading)}</h2>
            <p class="assist-set__muted">{move || t(T::Intro)}</p>
            {model_line}
            <Show when=move || error.get().is_some()>
                <p class="assist-set__error speech-in__error">
                    {move || format!("{}{}", t(T::Failed), error.get().unwrap_or_default())}
                </p>
            </Show>
            <div class="speech-in__fields">
                <label>
                    <span>{move || t(T::Threads)}</span>
                    <input
                        class="speech-in__threads"
                        type="number"
                        min="1"
                        max="16"
                        prop:value=move || form.with(|f| f.threads.to_string())
                        on:input=move |ev| number(event_target_value(&ev), &|f, v| f.threads = v as u8)
                    />
                </label>
                <label>
                    <span>{move || t(T::Gain)}</span>
                    <input
                        class="speech-in__gain"
                        type="number"
                        min="1"
                        max="16"
                        step="0.5"
                        prop:value=move || form.with(|f| f.gain.to_string())
                        on:input=move |ev| number(event_target_value(&ev), &|f, v| f.gain = v as f32)
                    />
                </label>
                <label>
                    <span>{move || t(T::Margin)}</span>
                    <input
                        class="speech-in__margin"
                        type="number"
                        min="0"
                        max="2000"
                        step="50"
                        prop:value=move || form.with(|f| f.listen_margin_ms.to_string())
                        on:input=move |ev| number(event_target_value(&ev), &|f, v| f.listen_margin_ms = v as u32)
                    />
                </label>
                <label>
                    <span>{move || t(T::Port)}</span>
                    <input
                        class="speech-in__port"
                        type="number"
                        min="1024"
                        max="65535"
                        prop:value=move || form.with(|f| f.port.to_string())
                        on:input=move |ev| number(event_target_value(&ev), &|f, v| f.port = v as u16)
                    />
                </label>
            </div>
            {port_note}
            <div class="assist-set__row">
                <button type="button" class="btn assist-set__btn speech-in__save" on:click=save>
                    {move || t(T::Save)}
                </button>
            </div>

            <h3>{move || t(T::KeyHeading)}</h3>
            <Show
                when=has_key
                fallback=move || view! {
                    <div class="assist-set__row">
                        <button type="button" class="btn assist-set__btn speech-in__create" on:click=move |_| make_key()>
                            {t(T::CreateKey)}
                        </button>
                    </div>
                }
            >
                <Show when=move || key.get().is_none()>
                    <p class="assist-set__muted">{move || t(T::KeyOnce)}</p>
                </Show>
                <div class="assist-set__row">
                    <button
                        type="button"
                        class="btn assist-set__btn speech-in__replace"
                        on:click=move |_| confirm_replace.set(true)
                    >
                        {move || t(T::ReplaceKey)}
                    </button>
                </div>
                <Show when=move || confirm_replace.get()>
                    <p class="assist-set__error speech-in__warning">{move || t(T::ReplaceWarning)}</p>
                    <div class="assist-set__row">
                        <button type="button" class="btn assist-set__btn speech-in__confirm" on:click=move |_| make_key()>
                            {move || t(T::Replace)}
                        </button>
                        <button
                            type="button"
                            class="btn assist-set__btn speech-in__keep"
                            on:click=move |_| confirm_replace.set(false)
                        >
                            {move || t(T::Keep)}
                        </button>
                    </div>
                </Show>
            </Show>
            {move || {
                key.get().map(|k| {
                    let line = secrets_line(&k);
                    view! {
                        <p class="assist-set__muted">{t(T::KeyHint)}</p>
                        <div class="snippet speech-in__key">
                            <code>{line.clone()}</code>
                            <CopyButton text=line />
                        </div>
                    }
                })
            }}
        </section>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nidoo_api::DownloadView;

    fn status() -> SttStatus {
        SttStatus {
            built: true,
            installed: false,
            engine: EngineState::NoModel,
            download: None,
            settings: SttSettings::default(),
            has_key: false,
            port_in_use: 4960,
        }
    }

    #[test]
    fn the_card_polls_only_while_something_moves() {
        assert!(!stt_moving(&status()));
        let mut s = status();
        s.engine = EngineState::Loading { model: "m".into() };
        assert!(stt_moving(&s));
        let mut s = status();
        s.download = Some(DownloadView {
            model: "m".into(),
            state: JobState::Running,
            done: 1,
            total: 2,
            file: String::new(),
            error: None,
        });
        assert!(stt_moving(&s));
    }

    #[test]
    fn the_secrets_line_quotes_the_key() {
        assert_eq!(secrets_line("ab12"), "voice_key: \"ab12\"");
    }
}
