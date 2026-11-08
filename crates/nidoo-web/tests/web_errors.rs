#![cfg(target_arch = "wasm32")]

#[path = "support/fetch_stub.rs"]
mod fetch_stub;

use fetch_stub::FetchStub;
use leptos::prelude::*;
use leptos::task::tick;
use nidoo_core::model::Command;
use nidoo_web::commands::{Pending, fire_command};
use nidoo_web::components::toast::{ToastQueue, Toasts};
use nidoo_web::house::House;
use nidoo_web::i18n::Lang;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

struct Mounted {
    host: web_sys::Element,
    house: House,
    toasts: Toasts,
    pending: Pending,
    lang: RwSignal<Lang>,
}

fn mount(lang: Lang) -> Mounted {
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    let (house, toasts, pending, lang) = (
        House::new(),
        Toasts::new(),
        Pending::new(),
        RwSignal::new(lang),
    );
    let h = host.clone();
    let _app = leptos::mount::mount_to(h.unchecked_into(), move || {
        provide_context(lang);
        provide_context(toasts);
        // The shell reads the language all the time; a screen standing in for it does too.
        let current = nidoo_web::i18n::use_lang();
        view! {
            <p>{move || current().as_str()}</p>
            <ToastQueue />
        }
    });
    std::mem::forget(_app);
    Mounted {
        host,
        house,
        toasts,
        pending,
        lang,
    }
}

async fn settle() {
    tick().await;
    gloo_timers::future::TimeoutFuture::new(50).await;
    tick().await;
}

fn fire(m: &Mounted) {
    fire_command(
        m.house,
        "lamp".into(),
        Command::TurnOff,
        m.toasts,
        m.pending,
        None,
    );
}

fn toast_text(m: &Mounted) -> String {
    m.host.text_content().unwrap_or_default()
}

/// A 503 on a command is the broker being down; the toast says so in the active language.
#[wasm_bindgen_test]
async fn a_refused_command_reads_broker_offline_in_french() {
    let stub = FetchStub::install();
    stub.reply("POST", "/api/v1/entities/lamp/command", 503, "{}");
    let m = mount(Lang::Fr);
    settle().await;
    fire(&m);
    settle().await;
    let text = toast_text(&m);
    assert!(
        text.contains("Broker hors ligne — commande refusée"),
        "{text}"
    );
    assert!(!text.contains("Broker offline"), "{text}");
    let button = m.host.query_selector(".toast").unwrap().expect("a toast");
    assert_eq!(button.get_attribute("title").as_deref(), Some("Fermer"));
    // The toggle reaches the close label of the toast already on screen, without a remount.
    m.lang.set(Lang::En);
    settle().await;
    assert_eq!(button.get_attribute("title").as_deref(), Some("Dismiss"));
    m.lang.set(Lang::Fr);
    settle().await;
    assert_eq!(button.get_attribute("title").as_deref(), Some("Fermer"));
}

#[wasm_bindgen_test]
async fn a_refused_command_reads_broker_offline_in_english() {
    let stub = FetchStub::install();
    stub.reply("POST", "/api/v1/entities/lamp/command", 503, "{}");
    let m = mount(Lang::En);
    settle().await;
    fire(&m);
    settle().await;
    let text = toast_text(&m);
    assert!(text.contains("Broker offline — command refused"), "{text}");
}

/// A reply that is not the hub's JSON becomes "HTTP n" in the active language; the hub's own
/// message is shown as received.
#[wasm_bindgen_test]
async fn http_errors_lead_in_french_and_hub_text_is_kept() {
    let stub = FetchStub::install();
    stub.reply("POST", "/api/v1/entities/lamp/command", 500, "oops");
    let m = mount(Lang::Fr);
    settle().await;
    fire(&m);
    settle().await;
    let text = toast_text(&m);
    assert!(text.contains("Le hub a répondu HTTP 500"), "{text}");

    let stub = FetchStub::install();
    stub.reply(
        "POST",
        "/api/v1/entities/lamp/command",
        422,
        r#"{"error":"light does not support color"}"#,
    );
    let m = mount(Lang::Fr);
    settle().await;
    fire(&m);
    settle().await;
    let text = toast_text(&m);
    assert!(text.contains("light does not support color"), "{text}");
    drop(stub);
}

fn lampe() -> nidoo_api::EntityView {
    use nidoo_core::model::{Capabilities, EntityCategory, EntityState, Kind, UiOverride};
    nidoo_api::EntityView {
        id: "lamp".into(),
        device_id: None,
        name: "Lampe".into(),
        device_name: None,
        label: None,
        kind: Kind::Light,
        component: "light".into(),
        protocol_kind: Kind::Light,
        device_class: None,
        unit: None,
        icon: None,
        category: EntityCategory::Primary,
        enabled_by_default: true,
        area: "salon".into(),
        part_of: None,
        capabilities: Capabilities::default(),
        ui: UiOverride::default(),
        state: EntityState::default(),
    }
}

/// `fire` here runs with no reactive owner, as the thermostat dial's pointer and timer callbacks do.
#[wasm_bindgen_test]
async fn a_named_failure_toast_reads_french_with_no_owner() {
    let stub = FetchStub::install();
    stub.reply("POST", "/api/v1/entities/lamp/command", 503, "{}");
    let m = mount(Lang::Fr);
    m.house.entities.update(|e| {
        e.insert("lamp".into(), lampe());
    });
    settle().await;
    fire(&m);
    settle().await;
    let text = toast_text(&m);
    assert!(
        text.contains("Lampe\u{a0}: Broker hors ligne — commande refusée"),
        "{text}"
    );

    drop(stub);
    let stub = FetchStub::install();
    stub.reply(
        "POST",
        "/api/v1/entities/lamp/command",
        200,
        r#"{"accepted":false}"#,
    );
    let m = mount(Lang::Fr);
    m.house.entities.update(|e| {
        e.insert("lamp".into(), lampe());
    });
    settle().await;
    fire(&m);
    settle().await;
    let text = toast_text(&m);
    assert!(text.contains("Lampe\u{a0}: Commande refusée"), "{text}");
    drop(stub);
}

/// With an owner but no language context the answer is English; with no owner at all it is the
/// language last rendered.
#[wasm_bindgen_test]
async fn current_lang_fallback_is_for_ownerless_callers_only() {
    let _m = mount(Lang::Fr);
    settle().await;
    assert_eq!(nidoo_web::i18n::current_lang(), Lang::Fr);
    let owned = Owner::new().with(nidoo_web::i18n::current_lang);
    assert_eq!(owned, Lang::En);
}
