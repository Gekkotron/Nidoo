//! The phone block of the Done step: the QR code the Android app scans to set itself up, and the
//! remote-access fields that decide whether the code carries a gateway.

use leptos::prelude::*;
use nidoo_api::MobileView;
use serde_json::json;

use crate::api::setup::{get_mobile, get_mobile_code, put_mobile};
use crate::i18n::{MobileText, use_lang, use_mobile_text};
use crate::qr::qr_svg;

/// Connect your phone: a blurred QR code to reveal on demand, and the optional gateway fields.
/// It reads `Lang` from context and fetches its own data on mount.
#[component]
pub fn PhoneSetup() -> impl IntoView {
    let t = use_mobile_text();
    let lang = use_lang();
    let view_state = RwSignal::new(None::<MobileView>);
    let code = RwSignal::new(None::<String>);
    let shown = RwSignal::new(false);
    let url = RwSignal::new(String::new());
    let key = RwSignal::new(String::new());
    let problem = RwSignal::new(None::<String>);
    let saving = RwSignal::new(false);

    let load_code = move || {
        leptos::task::spawn_local(async move {
            match get_mobile_code().await {
                Ok(c) => {
                    code.try_set(qr_svg(&c.payload));
                    problem.try_set(None);
                }
                Err(e) => {
                    code.try_set(None);
                    problem.try_set(Some(e.error));
                }
            }
        });
    };
    leptos::task::spawn_local(async move {
        if let Ok(v) = get_mobile().await {
            url.try_set(v.gateway_url.clone().unwrap_or_default());
            view_state.try_set(Some(v));
        }
    });
    load_code();

    let save = move |_| {
        if saving.get_untracked() {
            return;
        }
        saving.set(true);
        let address = url.get_untracked().trim().to_owned();
        let body = json!({
            "gateway_url": if address.is_empty() { None } else { Some(address) },
            "gateway_key": key.get_untracked(),
        });
        leptos::task::spawn_local(async move {
            match put_mobile(&body).await {
                Ok(v) => {
                    url.try_set(v.gateway_url.clone().unwrap_or_default());
                    view_state.try_set(Some(v));
                    key.try_set(String::new());
                    problem.try_set(None);
                    load_code();
                }
                Err(e) => {
                    problem.try_set(Some(e.message(lang())));
                }
            }
            saving.try_set(false);
        });
    };

    let ready = move || view_state.with(|v| v.as_ref().is_some_and(|v| v.gateway_ready));
    let key_set = move || view_state.with(|v| v.as_ref().is_some_and(|v| v.gateway_key.set));
    let remote_open =
        move || view_state.with(|v| v.as_ref().is_some_and(|v| v.gateway_url.is_some()));

    view! {
        <section class="phone">
            <h2 class="phone__title">{move || t(MobileText::Title)}</h2>
            <p class="phone__lead">{move || t(MobileText::Lead)}</p>
            <p class="phone__status" class:phone__status--remote=ready>
                {move || t(if ready() { MobileText::HomeAndAway } else { MobileText::HomeOnly })}
            </p>
            <div
                class="phone__code"
                class:phone__code--hidden=move || !shown.get()
                inner_html=move || code.get().unwrap_or_default()
            ></div>
            <p class="phone__warning">{move || t(MobileText::Warning)}</p>
            <button type="button" class="btn phone__toggle" on:click=move |_| shown.update(|s| *s = !*s)>
                {move || t(if shown.get() { MobileText::Hide } else { MobileText::Show })}
            </button>
            <details class="phone__remote" prop:open=remote_open>
                <summary>{move || t(MobileText::Remote)}</summary>
                <label class="field" for="phone-gateway-url">
                    <span class="field__caption">{move || t(MobileText::GatewayAddress)}</span>
                    <input
                        id="phone-gateway-url"
                        class="field__input"
                        type="text"
                        autocomplete="off"
                        prop:value=move || url.get()
                        on:input=move |ev| url.set(event_target_value(&ev))
                    />
                </label>
                <label class="field" for="phone-gateway-key">
                    <span class="field__caption">{move || t(MobileText::GatewayKey)}</span>
                    <input
                        id="phone-gateway-key"
                        class="field__input"
                        type="password"
                        autocomplete="off"
                        placeholder=move || {
                            t(if key_set() { MobileText::Saved } else { MobileText::KeyPlaceholder })
                        }
                        prop:value=move || key.get()
                        on:input=move |ev| key.set(event_target_value(&ev))
                    />
                </label>
                <Show when=move || !ready()>
                    <p class="settings__hint">{move || t(MobileText::Hint)}</p>
                </Show>
                <button
                    type="button"
                    class="btn phone__save"
                    prop:disabled=move || saving.get()
                    on:click=save
                >
                    {move || t(MobileText::Save)}
                </button>
            </details>
            <p class="phone__problem" role="alert">{move || problem.get()}</p>
        </section>
    }
}
