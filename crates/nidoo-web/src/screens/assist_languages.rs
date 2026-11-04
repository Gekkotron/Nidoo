//! The Languages card of `/assistant/settings`: one tab per language the hub speaks (the house
//! language first) with its model, a test sentence and its voices, and a **+ Add** tab.

use std::collections::HashMap;

use leptos::prelude::*;
use leptos::task::spawn_local;
use nidoo_api::{
    AvailableLanguage, DEFAULT_VOICE, EngineState, LanguageRequest, LanguageView, ModelView,
    SpeechRequest, TtsStatus, VoicesView,
};
use wasm_bindgen_futures::JsFuture;

use super::assist_settings::{model_name, voice_detail, voices_in_order};
use crate::api::tts;
use crate::components::confirm::ConfirmBar;
use crate::components::toast::Toasts;
use crate::i18n::{
    AssistSettingsText as T, language_name, test_sentence, use_assist_settings_text, use_lang,
};

/// The localStorage key of the selected tab (a language code).
const TAB_KEY: &str = "nidoo.assist.tts.tab";

fn storage() -> Option<web_sys::Storage> {
    web_sys::window().and_then(|w| w.local_storage().ok().flatten())
}

/// The tab saved in this browser; `None` when storage is blocked or holds nothing.
fn saved_tab() -> Option<String> {
    storage()
        .and_then(|s| s.get_item(TAB_KEY).ok().flatten())
        .filter(|code| !code.is_empty())
}

/// Remembers the tab in this browser, when storage allows it.
fn save_tab(code: &str) {
    if let Some(s) = storage() {
        let _ = s.set_item(TAB_KEY, code);
    }
}

/// Whether a model with these languages can speak `code`; one that lists none speaks any (the
/// hub's rule).
pub fn speaks(model_languages: &[String], code: &str) -> bool {
    model_languages.is_empty() || model_languages.iter().any(|l| l == code)
}

/// The tab to show: the chosen one while the hub still speaks it, else the house language.
pub fn shown_tab(chosen: Option<&str>, languages: &[LanguageView]) -> Option<String> {
    chosen
        .filter(|c| languages.iter().any(|l| l.code == *c))
        .map(str::to_owned)
        .or_else(|| languages.first().map(|l| l.code.clone()))
}

/// The status dot's class for a language's engine.
fn dot_class(engine: &EngineState) -> &'static str {
    match engine {
        EngineState::Ready { .. } => "assist-set__dot assist-set__dot--ready",
        EngineState::Loading { .. } => "assist-set__dot assist-set__dot--loading",
        EngineState::Error { .. } => "assist-set__dot assist-set__dot--error",
        EngineState::NoModel => "assist-set__dot",
    }
}

/// The languages card; writes put the hub's answer in `status` and then call `reload`.
#[component]
pub fn LanguagesCard(
    status: RwSignal<Option<TtsStatus>>,
    models: RwSignal<Vec<ModelView>>,
    reload: Callback<()>,
) -> impl IntoView {
    let lang = use_lang();
    let t = use_assist_settings_text();
    let toasts = use_context::<Toasts>();
    let toast = move |text: String| {
        if let Some(toasts) = toasts {
            toasts.push(text);
        }
    };

    // ---- Tabs ----
    let chosen = RwSignal::new(saved_tab());
    let adding = RwSignal::new(false);
    let selected = Memo::new(move |_| {
        let chosen = chosen.get();
        status.with(|s| {
            s.as_ref()
                .and_then(|s| shown_tab(chosen.as_deref(), &s.languages))
        })
    });
    let language = move |code: &str| -> Option<LanguageView> {
        status.with(|s| {
            s.as_ref()?
                .languages
                .iter()
                .find(|l| l.code == code)
                .cloned()
        })
    };
    let pick_tab = move |code: String| {
        save_tab(&code);
        chosen.set(Some(code));
        adding.set(false);
    };
    // A write's answer: the new status, or a toast; the lists are fetched again either way.
    let apply = move |result: Result<TtsStatus, String>| {
        match result {
            Ok(s) => {
                status.try_set(Some(s));
            }
            Err(e) => toast(e),
        }
        reload.try_run(());
    };

    // ---- Voices of the selected tab ----
    let voices = RwSignal::new(None::<(String, VoicesView)>);
    // What the list depends on: the tab, its model, its voice and whether the model is loaded.
    let voice_key = Memo::new(move |_| {
        let l = language(&selected.get()?)?;
        let ready = matches!(l.engine, EngineState::Ready { .. });
        Some((l.code, l.model, l.voice, ready))
    });
    let fetch_voices = move |code: String| {
        spawn_local(async move {
            if let Ok(view) = tts::get_voices(&code).await
                // A late answer for a tab left meanwhile is dropped.
                && selected.try_get_untracked().flatten().as_deref() == Some(code.as_str())
            {
                voices.try_set(Some((code, view)));
            }
        });
    };
    Effect::new(move |_| {
        if let Some((code, ..)) = voice_key.get() {
            fetch_voices(code);
        }
    });

    // ---- + Add ----
    let available = RwSignal::new(None::<Result<Vec<AvailableLanguage>, String>>);
    let which = RwSignal::new(None::<AvailableLanguage>);
    let open_add = move || {
        adding.set(true);
        which.set(None);
        available.set(None);
        spawn_local(async move {
            let found = tts::get_available_languages().await.map_err(|e| e.error);
            available.try_set(Some(found));
        });
    };
    let add = move |code: String, model: String| {
        spawn_local(async move {
            let req = LanguageRequest { model, voice: None };
            match tts::set_language(&code, &req).await {
                Ok(s) => {
                    status.try_set(Some(s));
                    save_tab(&code);
                    chosen.try_set(Some(code));
                    adding.try_set(false);
                }
                Err(e) => toast(e.error),
            }
            reload.try_run(());
        });
    };
    let model_label = move |id: &str| {
        models.with(|list| {
            list.iter()
                .find(|m| m.id == id)
                .map(|m| model_name(m, lang()))
                .unwrap_or_else(|| id.to_owned())
        })
    };
    let add_panel = move || {
        let pick = move |a: AvailableLanguage| {
            if let [model] = a.models.as_slice() {
                add(a.code.clone(), model.clone());
            } else {
                which.set(Some(a));
            }
        };
        view! {
            {move || match available.get() {
                None => view! { <p class="assist-set__muted">{move || t(T::Loading)}</p> }.into_any(),
                Some(Err(e)) => view! { <p class="assist-set__error">{e}</p> }.into_any(),
                Some(Ok(list)) if list.is_empty() => {
                    view! { <p class="assist-set__muted">{move || t(T::NoOtherLanguage)}</p> }
                        .into_any()
                }
                Some(Ok(list)) => {
                    view! {
                        <div class="assist-set__row">
                            {list
                                .into_iter()
                                .map(|a| {
                                    let name = language_name(&a.code, lang());
                                    view! {
                                        <button
                                            type="button"
                                            class="btn assist-set__btn"
                                            on:click=move |_| pick(a.clone())
                                        >
                                            {name}
                                        </button>
                                    }
                                })
                                .collect_view()}
                        </div>
                    }
                        .into_any()
                }
            }}
            {move || {
                which
                    .get()
                    .map(|a| {
                        view! {
                            <p>
                                <span class="assist-set__name">{language_name(&a.code, lang())}</span>
                                " · "
                                {move || t(T::WhichModel)}
                            </p>
                            <div class="assist-set__row">
                                {a
                                    .models
                                    .iter()
                                    .map(|model| {
                                        let code = a.code.clone();
                                        let id = model.clone();
                                        view! {
                                            <button
                                                type="button"
                                                class="btn assist-set__btn"
                                                on:click=move |_| add(code.clone(), id.clone())
                                            >
                                                {model_label(model)}
                                            </button>
                                        }
                                    })
                                    .collect_view()}
                            </div>
                        }
                    })
            }}
        }
    };

    // Typed sentences per language, for this visit only.
    let sentences = RwSignal::new(HashMap::<String, String>::new());

    // ---- One language's panel ----
    let language_panel = move |code: String| {
        let current = {
            let code = code.clone();
            move || language(&code)
        };
        let is_house = {
            let code = code.clone();
            move || {
                status.with(|s| {
                    s.as_ref()
                        .and_then(|s| s.languages.first())
                        .is_some_and(|l| l.code == code)
                })
            }
        };
        let current_model = {
            let current = current.clone();
            move || current().and_then(|l| l.model)
        };

        let make_house = {
            let code = code.clone();
            move || {
                let code = code.clone();
                spawn_local(async move {
                    apply(tts::set_house(&code).await.map_err(|e| e.error));
                });
            }
        };
        let choose_model = {
            let code = code.clone();
            move |model: String| {
                if model.is_empty() {
                    return;
                }
                let code = code.clone();
                spawn_local(async move {
                    let req = LanguageRequest { model, voice: None };
                    apply(tts::set_language(&code, &req).await.map_err(|e| e.error));
                });
            }
        };
        let choose_voice = {
            let code = code.clone();
            let current_model = current_model.clone();
            move |voice: String| {
                let Some(model) = current_model() else {
                    return;
                };
                let code = code.clone();
                spawn_local(async move {
                    let req = LanguageRequest {
                        model,
                        voice: Some(voice),
                    };
                    let result = tts::set_language(&code, &req).await.map_err(|e| e.error);
                    if result.is_err() {
                        // Show the choice the hub really holds, not the one just clicked.
                        fetch_voices(code);
                    }
                    apply(result);
                });
            }
        };
        let removing = RwSignal::new(false);
        let remove = {
            let code = code.clone();
            move || {
                removing.set(false);
                let code = code.clone();
                spawn_local(async move {
                    apply(tts::remove_language(&code).await.map_err(|e| e.error));
                });
            }
        };

        // ---- Try it ----
        let sentences_key = code.clone();
        let text = {
            let code = code.clone();
            move || {
                sentences
                    .with(|m| m.get(&code).cloned())
                    .unwrap_or_else(|| test_sentence(&code).unwrap_or_default().to_owned())
            }
        };
        let test_busy = RwSignal::new(false);
        let test_error = RwSignal::new(None::<String>);
        let first_sound = RwSignal::new(None::<u64>);
        let clicked_at = RwSignal::new(None::<f64>);
        let audio = NodeRef::<leptos::html::Audio>::new();
        // The voice a ▶ is speaking with, while it asks the hub and starts playing.
        let previewing = RwSignal::new(None::<String>);
        // Speaks `req` through the player below and times the first sound; the error to show.
        let speak_and_play = move |req: SpeechRequest| async move {
            let start = js_sys::Date::now();
            let ticket = tts::speak(&req).await.map_err(|e| e.error)?;
            let Some(el) = audio.get_untracked() else {
                return Ok(());
            };
            clicked_at.try_set(Some(start));
            el.set_src(&ticket.url);
            let played = match el.play() {
                Ok(promise) => JsFuture::from(promise).await.is_ok(),
                Err(_) => false,
            };
            if played {
                Ok(())
            } else {
                Err(t(T::PlayFailed).to_owned())
            }
        };
        let speaking = move || test_busy.get() || previewing.with(Option::is_some);
        let cannot_speak = {
            let text = text.clone();
            move || speaking() || text().trim().is_empty()
        };
        let play = {
            let code = code.clone();
            let text = text.clone();
            move || {
                let typed = text().trim().to_owned();
                if typed.is_empty() || speaking() {
                    return;
                }
                test_busy.set(true);
                test_error.set(None);
                first_sound.set(None);
                let req = SpeechRequest {
                    text: typed,
                    language: Some(code.clone()),
                    voice: None,
                };
                spawn_local(async move {
                    if let Err(e) = speak_and_play(req).await {
                        test_error.try_set(Some(e));
                    }
                    test_busy.try_set(false);
                });
            }
        };
        // ▶ on a voice: this tab's sentence in that voice, without choosing it.
        let preview = {
            let code = code.clone();
            let text = text.clone();
            move |voice: String| {
                let typed = text().trim().to_owned();
                if typed.is_empty() || speaking() {
                    return;
                }
                previewing.set(Some(voice.clone()));
                test_error.set(None);
                first_sound.set(None);
                let req = SpeechRequest {
                    text: typed,
                    language: Some(code.clone()),
                    voice: Some(voice),
                };
                spawn_local(async move {
                    if let Err(e) = speak_and_play(req).await {
                        toast(e);
                    }
                    previewing.try_set(None);
                });
            }
        };

        let shown_voices = {
            let code = code.clone();
            move || {
                voices.with(|v| {
                    v.as_ref()
                        .filter(|(c, _)| *c == code)
                        .map(|(_, v)| v.clone())
                })
            }
        };

        view! {
            {
                let make_house = make_house.clone();
                move || {
                    if is_house() {
                        view! { <p class="assist-set__muted">{move || t(T::HouseLanguage)}</p> }
                            .into_any()
                    } else {
                        let make_house = make_house.clone();
                        view! {
                            <button
                                type="button"
                                class="btn assist-set__btn assist-set__start"
                                on:click=move |_| make_house()
                            >
                                {move || t(T::MakeHouse)}
                            </button>
                        }
                            .into_any()
                    }
                }
            }

            // ---- Model ----
            <div class="assist-set__row">
                <label class="assist-set__row assist-set__grow">
                    <span class="assist-set__muted">{move || t(T::ModelLabel)}</span>
                    <select
                        class="field assist-set__grow"
                        on:change=move |ev| choose_model(event_target_value(&ev))
                    >
                        {
                            let code = code.clone();
                            let current_model = current_model.clone();
                            move || {
                                let chosen = current_model();
                                let none = chosen.is_none().then(|| {
                                    view! {
                                        <option value="" selected=true>
                                            {move || t(T::NoModel)}
                                        </option>
                                    }
                                });
                                let list = models
                                    .get()
                                    .into_iter()
                                    .filter(|m| speaks(&m.languages, &code))
                                    .map(|m| {
                                        let unknown = m.languages.is_empty();
                                        let selected = chosen.as_deref() == Some(m.id.as_str());
                                        let name = model_name(&m, lang());
                                        view! {
                                            <option value=m.id.clone() selected=selected>
                                                {name}
                                                {unknown
                                                    .then(|| {
                                                        view! { " (" {move || t(T::LanguagesUnknown)} ")" }
                                                    })}
                                            </option>
                                        }
                                    })
                                    .collect_view();
                                view! {
                                    {none}
                                    {list}
                                }
                            }
                        }
                    </select>
                </label>
                {
                    let current = current.clone();
                    move || {
                        current()
                            .map(|l| {
                                let (label, class, detail) = match l.engine {
                                    EngineState::NoModel => (T::StateNoModel, "", None),
                                    EngineState::Loading { .. } => (T::StateLoading, "", None),
                                    EngineState::Ready { .. } => {
                                        (T::StateReady, "assist-set__state--ready", None)
                                    }
                                    EngineState::Error { message, .. } => {
                                        (T::StateError, "assist-set__state--error", Some(message))
                                    }
                                };
                                view! {
                                    <span class=format!("assist-set__state {class}")>
                                        {move || t(label)}
                                    </span>
                                    {detail.map(|d| view! { <span class="assist-set__muted">{d}</span> })}
                                }
                            })
                    }
                }
            </div>

            // ---- Try it ----
            <h3>{move || t(T::TestHeading)}</h3>
            <div class="assist-set__row">
                <input
                    class="field assist-set__grow"
                    type="text"
                    aria-label=move || t(T::TestLabel)
                    placeholder=move || t(T::TypeSentence)
                    prop:value=text.clone()
                    on:input=move |ev| {
                        let typed = event_target_value(&ev);
                        sentences.update(|m| {
                            m.insert(sentences_key.clone(), typed);
                        });
                    }
                    on:keydown={
                        let play = play.clone();
                        move |ev: web_sys::KeyboardEvent| {
                            if ev.key() == "Enter" && !ev.is_composing() {
                                ev.prevent_default();
                                play();
                            }
                        }
                    }
                />
                <button
                    type="button"
                    class="btn assist-set__btn"
                    disabled=cannot_speak.clone()
                    on:click=move |_| play()
                >
                    {move || if test_busy.get() { t(T::Speaking) } else { t(T::Play) }}
                </button>
            </div>
            <audio
                node_ref=audio
                controls=true
                class="assist-set__audio"
                on:playing=move |_| {
                    if let Some(start) = clicked_at.get_untracked() {
                        clicked_at.set(None);
                        first_sound.set(Some((js_sys::Date::now() - start).max(0.0) as u64));
                    }
                }
                on:error=move |_| {
                    if clicked_at.get_untracked().is_some() {
                        clicked_at.set(None);
                        test_error.set(Some(t(T::PlayFailed).to_owned()));
                    }
                }
            ></audio>
            {move || {
                first_sound
                    .get()
                    .map(|ms| {
                        view! {
                            <p class="assist-set__muted">
                                {move || t(T::FirstSound)} ": " {ms} " " {move || t(T::UnitMs)}
                            </p>
                        }
                    })
            }}
            {move || test_error.get().map(|e| view! { <p class="assist-set__error">{e}</p> })}

            // ---- Voice ----
            <h3>{move || t(T::VoiceHeading)}</h3>
            {move || {
                let no_model = current_model().is_none();
                shown_voices()
                    .map(|view| {
                        let list = voices_in_order(&view);
                        let chosen = view.chosen.clone();
                        let choose_voice = choose_voice.clone();
                        let preview = preview.clone();
                        let cannot_speak = cannot_speak.clone();
                        view! {
                            {view.fell_back.then(|| view! {
                                <p class="assist-set__warn">{move || t(T::VoiceFellBack)}</p>
                            })}
                            {view.catalog_offline.then(|| view! {
                                <p class="assist-set__muted">{move || t(T::VoiceCatalogOffline)}</p>
                            })}
                            {list.is_empty().then(|| view! { <p class="assist-set__muted">{move || t(T::VoiceEmpty)}</p> })}
                            <ul class="assist-set__list">
                                {list
                                    .into_iter()
                                    .map(|v| {
                                        let name = v.name.clone();
                                        let detail = voice_detail(&v);
                                        let is_default = v.name == DEFAULT_VOICE;
                                        let label = v.name.clone();
                                        let heard = v.name.clone();
                                        let this_one = {
                                            let name = v.name.clone();
                                            move || previewing.with(|p| p.as_deref() == Some(name.as_str()))
                                        };
                                        let listen_label = {
                                            let label = label.clone();
                                            move || {
                                                let shown = if is_default { t(T::VoiceDefault) } else { label.as_str() };
                                                format!("{} {shown}", t(T::Listen))
                                            }
                                        };
                                        let choose_voice = choose_voice.clone();
                                        let preview = preview.clone();
                                        view! {
                                            <li class="assist-set__item assist-set__voice">
                                                <label>
                                                    <input
                                                        type="radio"
                                                        name="assist-voice"
                                                        prop:checked=chosen == v.name
                                                        disabled=no_model
                                                        on:change=move |_| choose_voice(name.clone())
                                                    />
                                                    <span class="assist-set__name">
                                                        {move || {
                                                            if is_default {
                                                                t(T::VoiceDefault).to_owned()
                                                            } else {
                                                                label.clone()
                                                            }
                                                        }}
                                                    </span>
                                                    {(!detail.is_empty()).then(|| view! { <span class="assist-set__muted">" " {detail}</span> })}
                                                </label>
                                                <button
                                                    type="button"
                                                    class="btn assist-set__btn assist-set__listen"
                                                    title=listen_label.clone()
                                                    aria-label=listen_label
                                                    disabled=cannot_speak.clone()
                                                    on:click=move |_| preview(heard.clone())
                                                >
                                                    {move || if this_one() { "…" } else { "▶" }}
                                                </button>
                                            </li>
                                        }
                                    })
                                    .collect_view()}
                            </ul>
                        }
                    })
            }}

            // ---- Remove ----
            <button
                type="button"
                class="assist-set__link"
                on:click=move |_| removing.set(true)
            >
                {move || t(T::RemoveLanguage)}
            </button>
            <Show when=move || removing.get()>
                <ConfirmBar
                    question=Signal::derive(move || t(T::RemoveLanguageQuestion).to_owned())
                    yes=Signal::derive(move || t(T::RemoveYes).to_owned())
                    on_yes=Callback::new({
                        let remove = remove.clone();
                        move |()| remove()
                    })
                    on_cancel=Callback::new(move |()| removing.set(false))
                />
            </Show>
        }
    };

    let languages =
        move || status.with(|s| s.as_ref().map(|s| s.languages.clone()).unwrap_or_default());

    view! {
        <section class="assist-set__card">
            <h2>{move || t(T::LanguagesHeading)}</h2>
            <Show when=move || status.with(|s| s.as_ref().is_some_and(|s| !s.speech_built))>
                <p class="assist-set__error">{move || t(T::NotBuilt)}</p>
            </Show>
            <div class="assist-set__tabs" role="tablist">
                {move || {
                    languages()
                        .into_iter()
                        .map(|l| {
                            let on = {
                                let code = l.code.clone();
                                move || !adding.get() && selected.get().as_deref() == Some(code.as_str())
                            };
                            let code = l.code.clone();
                            view! {
                                <button
                                    type="button"
                                    role="tab"
                                    class={
                                        let on = on.clone();
                                        move || if on() { "assist-set__tab assist-set__tab--on" } else { "assist-set__tab" }
                                    }
                                    aria-selected=move || if on() { "true" } else { "false" }
                                    on:click=move |_| pick_tab(code.clone())
                                >
                                    <span class=dot_class(&l.engine) aria-hidden="true"></span>
                                    {language_name(&l.code, lang())}
                                </button>
                            }
                        })
                        .collect_view()
                }}
                <button
                    type="button"
                    role="tab"
                    class=move || if adding.get() { "assist-set__tab assist-set__tab--on" } else { "assist-set__tab" }
                    aria-selected=move || if adding.get() { "true" } else { "false" }
                    on:click=move |_| open_add()
                >
                    {move || t(T::AddLanguage)}
                </button>
            </div>
            <div class="assist-set__panel" role="tabpanel">
                {move || {
                    if adding.get() {
                        add_panel().into_any()
                    } else if let Some(code) = selected.get() {
                        language_panel(code).into_any()
                    } else if status.with(Option::is_some) {
                        view! { <p class="assist-set__muted">{move || t(T::NoLanguages)}</p> }.into_any()
                    } else {
                        ().into_any()
                    }
                }}
            </div>
        </section>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(code: &str) -> LanguageView {
        LanguageView {
            code: code.into(),
            model: None,
            voice: DEFAULT_VOICE.into(),
            engine: EngineState::NoModel,
        }
    }

    #[test]
    fn a_model_listing_no_language_speaks_any() {
        assert!(speaks(&[], "de"));
        assert!(speaks(&["fr".into(), "en".into()], "en"));
        assert!(!speaks(&["fr".into()], "en"));
    }

    #[test]
    fn the_tab_falls_back_to_the_house_language() {
        let list = [entry("fr"), entry("en")];
        assert_eq!(shown_tab(Some("en"), &list).as_deref(), Some("en"));
        assert_eq!(shown_tab(Some("de"), &list).as_deref(), Some("fr"));
        assert_eq!(shown_tab(None, &list).as_deref(), Some("fr"));
        assert_eq!(shown_tab(Some("fr"), &[]), None);
    }
}
