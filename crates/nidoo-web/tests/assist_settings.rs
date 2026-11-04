#![cfg(target_arch = "wasm32")]
//! The speech settings screen: its six sections render with no hub, and each state of a live hub
//! shows (engine ready, voices with a fallback note, a model that does not speak French).

use leptos::prelude::*;
use nidoo_web::components::toast::Toasts;
use nidoo_web::i18n::Lang;
use nidoo_web::screens::assist_settings::AssistSettingsScreen;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
wasm_bindgen_test_configure!(run_in_browser);

#[path = "support/fetch_stub.rs"]
mod fetch_stub;
use fetch_stub::FetchStub;

const BASE: &str = "/api/v1/assist/tts";

struct Mounted {
    _app: Box<dyn std::any::Any>,
    host: web_sys::Element,
}

fn mount() -> Mounted {
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    let app = leptos::mount::mount_to(host.clone().unchecked_into(), move || {
        provide_context(Toasts::new());
        provide_context(RwSignal::new(Lang::En));
        view! { <AssistSettingsScreen /> }
    });
    Mounted {
        _app: Box::new(app),
        host,
    }
}

async fn settle() {
    leptos::task::tick().await;
    gloo_timers::future::TimeoutFuture::new(150).await;
    leptos::task::tick().await;
}

fn headings(host: &web_sys::Element) -> Vec<String> {
    let nodes = host.query_selector_all(".assist-set__card > h2").unwrap();
    (0..nodes.length())
        .map(|i| nodes.item(i).unwrap().text_content().unwrap_or_default())
        .collect()
}

#[wasm_bindgen_test]
async fn the_six_sections_render_even_when_the_hub_does_not_answer() {
    let _stub = FetchStub::install();
    let m = mount();
    settle().await;
    assert_eq!(
        headings(&m.host),
        [
            "Speech",
            "Try it",
            "Voice",
            "Installed models",
            "Add a model",
            "Hugging Face token"
        ]
    );
    let text = m.host.text_content().unwrap_or_default();
    assert!(
        text.contains("could not be loaded"),
        "a failed load is said: {text}"
    );
    let back = m.host.query_selector(".assist-set__back").unwrap().unwrap();
    assert_eq!(back.get_attribute("href").as_deref(), Some("/assistant"));
}

#[wasm_bindgen_test]
async fn a_live_hub_shows_its_state_voices_and_models() {
    let stub = FetchStub::install();
    stub.reply(
        "GET",
        &format!("{BASE}/status"),
        200,
        r#"{"engine":{"state":"ready","model":"a--b--q8"},"active":"a--b--q8","voice":"zoe",
            "token_set":false,"download":null,"speech_built":true}"#,
    );
    stub.reply(
        "GET",
        &format!("{BASE}/models"),
        200,
        r#"[{"id":"a--b--q8","repo":"a/b","revision":"0123456789abcdef","weights":"model.q8.gguf",
             "size":74553568,"licence":"cc-by-nc-4.0","languages":["en"],"french":false,
             "source":"download","installed_at":1800000000,"active":true}]"#,
    );
    stub.reply(
        "GET",
        &format!("{BASE}/voices"),
        200,
        r#"{"voices":[
              {"name":"zoe","language":"fr","gender":"female","age":"adult","description":"warm",
               "source":"catalog","on_disk":false},
              {"name":"default","language":null,"gender":null,"age":null,"description":null,
               "source":"model","on_disk":true}],
            "chosen":"default","fell_back":true,"catalog_offline":true}"#,
    );
    let m = mount();
    settle().await;
    let text = m.host.text_content().unwrap_or_default();
    assert!(text.contains("Ready"), "{text}");
    assert!(text.contains("a/b"), "{text}");
    assert!(text.contains("74.6 MB"), "{text}");
    assert!(text.contains("0123456"), "{text}");
    assert!(text.contains("does not speak French"), "{text}");
    assert!(text.contains("Default is used"), "{text}");
    assert!(text.contains("unavailable offline"), "{text}");
    // Default comes first, then the catalog voice with its facts.
    let names = m
        .host
        .query_selector_all("input[name=assist-voice] + .assist-set__name");
    let names = names.unwrap();
    assert_eq!(
        names.item(0).unwrap().text_content().as_deref(),
        Some("Default")
    );
    assert_eq!(
        names.item(1).unwrap().text_content().as_deref(),
        Some("zoe")
    );
    assert!(text.contains("female · adult · warm"), "{text}");
}
