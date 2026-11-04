#![cfg(target_arch = "wasm32")]
//! The speech settings screen: its three cards render with no hub, the language tabs follow the
//! status order, ▶ speaks in the tab's language, and each state of a live hub shows.

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
/// The localStorage key of the selected tab, cleared before each mount.
const TAB_KEY: &str = "nidoo.assist.tts.tab";

struct Mounted {
    _app: Box<dyn std::any::Any>,
    host: web_sys::Element,
}

fn mount() -> Mounted {
    let window = web_sys::window().unwrap();
    if let Ok(Some(storage)) = window.local_storage() {
        let _ = storage.remove_item(TAB_KEY);
    }
    let document = window.document().unwrap();
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

fn texts(host: &web_sys::Element, selector: &str) -> Vec<String> {
    let nodes = host.query_selector_all(selector).unwrap();
    (0..nodes.length())
        .map(|i| {
            nodes
                .item(i)
                .unwrap()
                .text_content()
                .unwrap_or_default()
                .trim()
                .to_owned()
        })
        .collect()
}

fn click(host: &web_sys::Element, selector: &str, index: u32) {
    host.query_selector_all(selector)
        .unwrap()
        .item(index)
        .unwrap_or_else(|| panic!("no {selector} #{index}"))
        .unchecked_into::<web_sys::HtmlElement>()
        .click();
}

const MODELS: &str = r#"[
    {"id":"a--b--q8","repo":"a/b","revision":"0123456789abcdef","weights":"model.q8.gguf",
     "size":74553568,"licence":"cc-by-nc-4.0","languages":["fr"],"source":"download",
     "installed_at":1800000000,"used_by":["fr"]},
    {"id":"c--d--q8","repo":"c/d","revision":"fedcba9876543210","weights":"model.q8.gguf",
     "size":40000000,"licence":"cc-by-nc-4.0","languages":["en"],"source":"download",
     "installed_at":1800000001,"used_by":["en"]}]"#;

const VOICES: &str = r#"{"voices":[
      {"name":"zoe","language":"fr","gender":"female","age":"adult","description":"warm",
       "source":"catalog","on_disk":false},
      {"name":"default","language":null,"gender":null,"age":null,"description":null,
       "source":"model","on_disk":true}],
    "chosen":"default","fell_back":true,"catalog_offline":true}"#;

/// A hub speaking French (house) and English, each on its own ready model.
fn two_languages(stub: &FetchStub) {
    stub.reply(
        "GET",
        &format!("{BASE}/status"),
        200,
        r#"{"languages":[
              {"code":"fr","model":"a--b--q8","voice":"default",
               "engine":{"state":"ready","model":"a--b--q8"}},
              {"code":"en","model":"c--d--q8","voice":"default",
               "engine":{"state":"ready","model":"c--d--q8"}}],
            "token_set":false,"download":null,"speech_built":true}"#,
    );
    stub.reply("GET", &format!("{BASE}/models"), 200, MODELS);
    // gloo ends a URL that already has a query with `&`.
    stub.reply("GET", &format!("{BASE}/voices?language=fr&"), 200, VOICES);
    stub.reply("GET", &format!("{BASE}/voices?language=en&"), 200, VOICES);
}

#[wasm_bindgen_test]
async fn the_three_cards_render_even_when_the_hub_does_not_answer() {
    let _stub = FetchStub::install();
    let m = mount();
    settle().await;
    assert_eq!(
        texts(&m.host, ".assist-set__card > h2"),
        ["Languages", "Installed models", "Add a model"]
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
async fn tabs_follow_the_status_order() {
    let stub = FetchStub::install();
    two_languages(&stub);
    let m = mount();
    settle().await;
    let tabs = texts(&m.host, ".assist-set__tab");
    assert_eq!(tabs[..2], ["French", "English"], "{tabs:?}");
    let panel = texts(&m.host, ".assist-set__panel").join(" ");
    assert!(panel.contains("House language"), "{panel}");
    assert!(!panel.contains("Make house language"), "{panel}");
    click(&m.host, ".assist-set__tab", 1);
    settle().await;
    let panel = texts(&m.host, ".assist-set__panel").join(" ");
    assert!(panel.contains("Make house language"), "{panel}");
    assert!(!panel.contains("House language:"), "{panel}");
}

#[wasm_bindgen_test]
async fn preview_sends_the_tab_language() {
    let stub = FetchStub::install();
    two_languages(&stub);
    let m = mount();
    settle().await;
    click(&m.host, ".assist-set__tab", 1);
    settle().await;
    click(&m.host, ".assist-set__listen", 0);
    settle().await;
    let speech: Vec<_> = stub
        .requests()
        .into_iter()
        .filter(|r| r.method == "POST" && r.url == "/api/v1/assist/speech")
        .collect();
    assert_eq!(speech.len(), 1, "{speech:?}");
    assert!(
        speech[0].body.contains(r#""language":"en""#),
        "{}",
        speech[0].body
    );
    assert!(
        speech[0].body.contains(r#""voice":"default""#),
        "{}",
        speech[0].body
    );
}

#[wasm_bindgen_test]
async fn a_language_without_a_model_says_so() {
    let stub = FetchStub::install();
    stub.reply(
        "GET",
        &format!("{BASE}/status"),
        200,
        r#"{"languages":[{"code":"fr","model":null,"voice":"default",
               "engine":{"state":"no_model"}}],
            "token_set":false,"download":null,"speech_built":true}"#,
    );
    stub.reply("GET", &format!("{BASE}/models"), 200, "[]");
    let m = mount();
    settle().await;
    let panel = texts(&m.host, ".assist-set__panel").join(" ");
    assert!(panel.contains("No model"), "{panel}");
}

#[wasm_bindgen_test]
async fn a_live_hub_shows_its_models_and_voices() {
    let stub = FetchStub::install();
    two_languages(&stub);
    let m = mount();
    settle().await;
    let text = m.host.text_content().unwrap_or_default();
    assert!(text.contains("Ready"), "{text}");
    assert!(text.contains("a/b"), "{text}");
    assert!(text.contains("74.6 MB"), "{text}");
    assert!(text.contains("0123456"), "{text}");
    assert!(text.contains("Used by"), "{text}");
    assert!(text.contains("Default is used"), "{text}");
    assert!(text.contains("unavailable offline"), "{text}");
    // Default comes first, then the catalog voice with its facts.
    let names = texts(&m.host, "input[name=assist-voice] + .assist-set__name");
    assert_eq!(names[..2], ["Default", "zoe"], "{names:?}");
    assert!(text.contains("female · adult · warm"), "{text}");
    // The token is folded into the Add card.
    let fold = texts(&m.host, ".assist-set__card details > summary").join(" ");
    assert!(
        fold.contains("only needed for gated repositories"),
        "{fold}"
    );
}
