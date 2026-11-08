#![cfg(target_arch = "wasm32")]

use gloo_timers::future::TimeoutFuture;
use leptos::prelude::*;
use leptos::task::tick;
use nidoo_api::SetupStep;
use nidoo_web::i18n::Lang;
use nidoo_web::setup::connection::{DatabaseStep, MqttStep};
use nidoo_web::setup::wizard::WizardNav;
use nidoo_web::setup::{Applying, SetupState, reload_to};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

#[path = "support/fetch_stub.rs"]
mod fetch_stub;
use fetch_stub::FetchStub;

const OK: &str = r#"{"ok":true,"message":"Connected","detail":null}"#;
const DB_OK: &str =
    r#"{"ok":true,"message":"Connected","detail":{"history_since":"2026-01-15T08:00:00+00:00"}}"#;
const STATUS: &str = r#"{"mode":"setup","step":"database","finished":false,"bundled":false,"location":null,"mqtt":null,"database":null,"restarting":false}"#;
const BUNDLED: &str = r#"{"mode":"setup","step":"mqtt","finished":false,"bundled":true,"location":null,"mqtt":{"host":"localhost","port":1883,"user":"nidoo","password":{"set":true},"client_id":null,"z2m_base":null},"database":null,"restarting":false}"#;

struct Mounted {
    host: web_sys::Element,
    step: RwSignal<SetupStep>,
    applying: Applying,
    _handle: Box<dyn std::any::Any>,
}

fn mount(db: bool, status: Option<&str>, applying: Applying) -> Mounted {
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    let step = RwSignal::new(if db {
        SetupStep::Database
    } else {
        SetupStep::Mqtt
    });
    let status = status.map(|s| serde_json::from_str(s).unwrap());
    let handle = leptos::mount::mount_to(host.clone().unchecked_into(), move || {
        provide_context(RwSignal::new(Lang::En));
        provide_context(WizardNav { step });
        provide_context(applying);
        provide_context(SetupState(RwSignal::new(status)));
        if db {
            view! { <DatabaseStep /> }.into_any()
        } else {
            view! { <MqttStep /> }.into_any()
        }
    });
    Mounted {
        host,
        step,
        applying,
        _handle: Box::new(handle),
    }
}

fn input(host: &web_sys::Element, selector: &str) -> web_sys::HtmlInputElement {
    host.query_selector(selector)
        .unwrap()
        .unwrap_or_else(|| panic!("no {selector}"))
        .unchecked_into()
}

fn button(host: &web_sys::Element, selector: &str) -> web_sys::HtmlButtonElement {
    host.query_selector(selector)
        .unwrap()
        .unwrap_or_else(|| panic!("no {selector}"))
        .unchecked_into()
}

async fn type_into(host: &web_sys::Element, selector: &str, value: &str) {
    let el = input(host, selector);
    el.set_value(value);
    let event = web_sys::Event::new_with_event_init_dict("input", &{
        let init = web_sys::EventInit::new();
        init.set_bubbles(true);
        init
    })
    .unwrap();
    el.dispatch_event(&event).unwrap();
    tick().await;
}

async fn settle() {
    for _ in 0..8 {
        TimeoutFuture::new(15).await;
        tick().await;
    }
}

#[wasm_bindgen_test]
async fn next_waits_for_a_passing_test_of_these_values() {
    let stub = FetchStub::install();
    stub.reply("POST", "/api/v1/setup/test/mqtt", 200, OK);
    let m = mount(false, None, Applying::new(reload_to));
    tick().await;
    assert!(button(&m.host, ".loc__save").disabled());
    button(&m.host, ".conn__test").click();
    settle().await;
    assert!(m.host.text_content().unwrap().contains("Connected"));
    assert!(!button(&m.host, ".loc__save").disabled());
    type_into(&m.host, "#mqtt-host", "192.0.2.10").await;
    assert!(button(&m.host, ".loc__save").disabled());
    assert!(!m.host.text_content().unwrap().contains("Connected"));
}

#[wasm_bindgen_test]
async fn a_bad_port_names_the_field_and_blocks_the_test() {
    let _stub = FetchStub::install();
    let m = mount(false, None, Applying::new(reload_to));
    tick().await;
    type_into(&m.host, "#mqtt-port", "abc").await;
    assert!(
        m.host
            .text_content()
            .unwrap()
            .contains("The port is a number")
    );
    assert!(button(&m.host, ".conn__test").disabled());
}

#[wasm_bindgen_test]
async fn a_refused_save_shows_the_brokers_reason() {
    let stub = FetchStub::install();
    stub.reply("POST", "/api/v1/setup/test/mqtt", 200, OK);
    stub.reply(
        "PUT",
        "/api/v1/setup/mqtt",
        422,
        r#"{"ok":false,"message":"bad user name or password","detail":null}"#,
    );
    let m = mount(false, None, Applying::new(reload_to));
    tick().await;
    button(&m.host, ".conn__test").click();
    settle().await;
    button(&m.host, ".loc__save").click();
    settle().await;
    assert!(
        m.host
            .text_content()
            .unwrap()
            .contains("bad user name or password")
    );
    assert_eq!(m.step.get_untracked(), SetupStep::Mqtt);
    // The broker no longer vouches for these values: no green line, no Next, until a new test.
    assert!(!m.host.text_content().unwrap().contains("Connected"));
    assert!(button(&m.host, ".loc__save").disabled());
    assert!(
        !button(&m.host, ".conn__test").disabled(),
        "the user can retest"
    );
    button(&m.host, ".conn__test").click();
    settle().await;
    assert!(!button(&m.host, ".loc__save").disabled());
}

#[wasm_bindgen_test]
async fn bundled_credentials_prefill_and_keep_the_password() {
    let stub = FetchStub::install();
    stub.reply("POST", "/api/v1/setup/test/mqtt", 200, OK);
    let m = mount(false, Some(BUNDLED), Applying::new(reload_to));
    tick().await;
    assert_eq!(input(&m.host, "#mqtt-user").value(), "nidoo");
    let pw = input(&m.host, "#mqtt-password");
    assert_eq!(pw.type_(), "password");
    assert_eq!(pw.placeholder(), "Saved, enter a new one to replace it");
    button(&m.host, ".conn__test").click();
    settle().await;
    let sent = stub
        .requests()
        .into_iter()
        .find(|r| r.method == "POST")
        .expect("a test request");
    assert!(sent.body.contains(r#""password":"""#), "{}", sent.body);
    assert!(!sent.body.contains("null"), "{}", sent.body);
}

#[wasm_bindgen_test]
async fn a_saved_mqtt_step_moves_to_the_database() {
    let stub = FetchStub::install();
    stub.reply("POST", "/api/v1/setup/test/mqtt", 200, OK);
    stub.reply("PUT", "/api/v1/setup/mqtt", 200, STATUS);
    let m = mount(false, None, Applying::new(reload_to));
    tick().await;
    button(&m.host, ".conn__test").click();
    settle().await;
    button(&m.host, ".loc__save").click();
    settle().await;
    assert_eq!(m.step.get_untracked(), SetupStep::Database);
}

#[wasm_bindgen_test]
async fn the_database_test_shows_how_far_the_history_goes() {
    let stub = FetchStub::install();
    stub.reply("POST", "/api/v1/setup/test/database", 200, DB_OK);
    let m = mount(true, None, Applying::new(reload_to));
    tick().await;
    button(&m.host, ".conn__test").click();
    settle().await;
    let text = m.host.text_content().unwrap();
    assert!(text.contains("history since 2026-01-15"), "{text}");
    assert!(text.contains("No Skip here"));
}

#[wasm_bindgen_test]
async fn saving_the_database_in_setup_mode_starts_applying() {
    let stub = FetchStub::install();
    stub.reply("POST", "/api/v1/setup/test/database", 200, OK);
    let restarting = STATUS.replace(r#""restarting":false"#, r#""restarting":true"#);
    stub.reply("PUT", "/api/v1/setup/database", 200, &restarting);
    let m = mount(true, None, Applying::new(|_| {}));
    tick().await;
    button(&m.host, ".conn__test").click();
    settle().await;
    button(&m.host, ".loc__save").click();
    settle().await;
    assert_eq!(
        m.applying.target.get_untracked().as_deref(),
        Some("/setup?step=devices")
    );
    assert_eq!(m.step.get_untracked(), SetupStep::Database);
    assert!(button(&m.host, ".loc__save").disabled(), "no second save");
    let puts = stub
        .requests()
        .into_iter()
        .filter(|r| r.method == "PUT")
        .count();
    assert_eq!(puts, 1);
}

const SAVED_MQTT: &str = r#"{"mode":"setup","step":"database","finished":false,"bundled":false,"location":null,"mqtt":{"host":"192.0.2.10","port":1883,"user":"nidoo","password":{"set":true},"client_id":null,"z2m_base":null},"database":null,"restarting":false}"#;

#[wasm_bindgen_test]
async fn back_after_a_saved_mqtt_step_shows_what_was_saved() {
    let stub = FetchStub::install();
    stub.reply("POST", "/api/v1/setup/test/mqtt", 200, OK);
    stub.reply("PUT", "/api/v1/setup/mqtt", 200, SAVED_MQTT);
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    let step = RwSignal::new(SetupStep::Mqtt);
    let _handle = leptos::mount::mount_to(host.clone().unchecked_into(), move || {
        provide_context(RwSignal::new(Lang::En));
        provide_context(WizardNav { step });
        provide_context(Applying::new(|_| {}));
        provide_context(SetupState(RwSignal::new(None)));
        move || {
            if step.get() == SetupStep::Mqtt {
                view! { <MqttStep /> }.into_any()
            } else {
                view! { <p>"elsewhere"</p> }.into_any()
            }
        }
    });
    tick().await;
    button(&host, ".conn__test").click();
    settle().await;
    button(&host, ".loc__save").click();
    settle().await;
    assert_eq!(step.get_untracked(), SetupStep::Database);
    step.set(SetupStep::Mqtt);
    settle().await;
    assert_eq!(input(&host, "#mqtt-host").value(), "192.0.2.10");
    assert_eq!(
        input(&host, "#mqtt-password").placeholder(),
        "Saved, enter a new one to replace it"
    );
}

#[wasm_bindgen_test]
async fn a_database_save_in_setup_mode_without_restart_goes_back_to_the_broker() {
    let stub = FetchStub::install();
    stub.reply("POST", "/api/v1/setup/test/database", 200, OK);
    stub.reply("PUT", "/api/v1/setup/database", 200, STATUS);
    let m = mount(true, None, Applying::new(|_| {}));
    tick().await;
    button(&m.host, ".conn__test").click();
    settle().await;
    button(&m.host, ".loc__save").click();
    settle().await;
    assert_eq!(m.step.get_untracked(), SetupStep::Mqtt);
}
