#![cfg(target_arch = "wasm32")]

use leptos::prelude::*;
use leptos::task::tick;
use leptos_router::components::{Route, Router, Routes};
use leptos_router::path;
use nidoo_web::house::House;
use nidoo_web::i18n::Lang;
use nidoo_web::screens::backup_settings::BackupSettingsPage;
use nidoo_web::screens::settings::SettingsScreen;
use nidoo_web::theme::Theme;
use serde_json::{Value, json};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
wasm_bindgen_test_configure!(run_in_browser);

#[path = "support/fetch_stub.rs"]
mod fetch_stub;
use fetch_stub::FetchStub;

const FILE: &str = "nidoo-backup-20261112-100000.tar.zst.age";

/// The backup section as the hub serves it.
fn view_json(password_set: bool, host: &str) -> Value {
    json!({
        "password": {"set": password_set},
        "ftp": {
            "host": host,
            "port": 21,
            "user": "nidoo",
            "password": {"set": true},
            "folder": "/backups",
        },
        "keep": 7,
        "notify_failure": true,
    })
}

fn progress_json() -> Value {
    json!({
        "state": "running",
        "destination": "ftp",
        "part": "history",
        "bytes": 12_000_000u64,
        "started_at": 1_700_000_000i64,
        "file": FILE,
        "error": null,
    })
}

fn last_json() -> Value {
    json!({
        "started_at": 1_700_000_000i64,
        "ended_at": 1_700_000_090i64,
        "destination": "ftp",
        "file": FILE,
        "size": 3_000_000u64,
        "ok": true,
        "error": null,
        "last_ok_at": 1_700_000_090i64,
    })
}

struct Mounted {
    _app: Box<dyn std::any::Any>,
    host: web_sys::Element,
}

fn mount(path_: &str) -> Mounted {
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    web_sys::window()
        .unwrap()
        .history()
        .unwrap()
        .replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(path_))
        .unwrap();
    let house = House::new();
    let app = leptos::mount::mount_to(host.clone().unchecked_into(), move || {
        provide_context(house);
        provide_context(RwSignal::new(Lang::En));
        provide_context(RwSignal::new(Theme::Auto));
        nidoo_web::nav::install_nav();
        view! {
            <Router>
                <Routes fallback=|| "fallback">
                    <Route path=path!("/settings") view=SettingsScreen />
                    <Route path=path!("/settings/backup") view=BackupSettingsPage />
                </Routes>
            </Router>
        }
    });
    Mounted {
        _app: Box::new(app),
        host,
    }
}

fn all(host: &web_sys::Element, selector: &str) -> Vec<web_sys::HtmlElement> {
    let nodes = host.query_selector_all(selector).unwrap();
    (0..nodes.length())
        .map(|i| nodes.item(i).unwrap().unchecked_into())
        .collect()
}

fn one(host: &web_sys::Element, selector: &str) -> web_sys::HtmlElement {
    all(host, selector)
        .into_iter()
        .next()
        .unwrap_or_else(|| panic!("no {selector:?}"))
}

fn text(host: &web_sys::Element, selector: &str) -> String {
    one(host, selector).text_content().unwrap_or_default()
}

fn texts(host: &web_sys::Element, selector: &str) -> Vec<String> {
    all(host, selector)
        .iter()
        .map(|e| e.text_content().unwrap_or_default())
        .collect()
}

async fn settle() {
    tick().await;
    gloo_timers::future::TimeoutFuture::new(50).await;
    tick().await;
}

/// Waits (up to `ms`) until `selector` matches at least `n` elements, then lets handlers attach.
async fn wait_for_ms(host: &web_sys::Element, selector: &str, n: usize, ms: u32) {
    for _ in 0..ms / 25 {
        if all(host, selector).len() >= n {
            break;
        }
        gloo_timers::future::TimeoutFuture::new(25).await;
    }
    let found = all(host, selector).len();
    assert!(found >= n, "waited for {n} x {selector:?}, found {found}");
    settle().await;
}

async fn wait_for(host: &web_sys::Element, selector: &str, n: usize) {
    wait_for_ms(host, selector, n, 2000).await;
}

/// Sets an input's value and fires `input` on it.
fn type_into(host: &web_sys::Element, selector: &str, value: &str) {
    let el = one(host, selector);
    js_sys::Reflect::set(&el, &"value".into(), &value.into()).unwrap();
    el.dispatch_event(&web_sys::Event::new("input").unwrap())
        .unwrap();
}

fn disabled(el: &web_sys::HtmlElement) -> bool {
    el.has_attribute("disabled")
}

#[wasm_bindgen_test]
async fn password_needs_two_matching_entries() {
    let stub = FetchStub::install();
    stub.reply(
        "GET",
        "/api/v1/settings/backup",
        200,
        &view_json(false, "nas.local").to_string(),
    );
    stub.reply("GET", "/api/v1/backup/current", 204, "");
    stub.reply("GET", "/api/v1/backup/last", 204, "");
    stub.reply(
        "PUT",
        "/api/v1/settings/backup",
        200,
        &view_json(true, "nas.local").to_string(),
    );
    let m = mount("/settings/backup");
    wait_for(&m.host, "#backup-password", 1).await;
    let save = || one(&m.host, ".backup__password-save");
    assert!(disabled(&save()), "empty");
    type_into(&m.host, "#backup-password", "short");
    type_into(&m.host, "#backup-password-repeat", "short");
    settle().await;
    assert!(disabled(&save()), "matching but under 8 characters");
    type_into(&m.host, "#backup-password", "long-enough-1");
    type_into(&m.host, "#backup-password-repeat", "different-12");
    settle().await;
    assert!(disabled(&save()), "long enough but different");
    type_into(&m.host, "#backup-password-repeat", "long-enough-1");
    settle().await;
    assert!(!disabled(&save()), "two matching entries of 8 or more");
    assert!(
        !text(&m.host, ".backup__password").contains("Set"),
        "no password stored yet"
    );
    save().click();
    let put = stub.wait_for("PUT").await.expect("a PUT");
    assert_eq!(put.url, "/api/v1/settings/backup");
    let body: Value = serde_json::from_str(&put.body).unwrap();
    assert_eq!(body["password"], "long-enough-1");
    assert_eq!(body["ftp"]["host"], "nas.local");
    assert_eq!(body["keep"], 7);
    assert_eq!(body["notify_failure"], true);
    wait_for(&m.host, ".backup__set", 1).await;
    assert_eq!(text(&m.host, ".backup__set"), "Set");
}

#[wasm_bindgen_test]
async fn test_connection_lists_steps() {
    let stub = FetchStub::install();
    stub.reply(
        "GET",
        "/api/v1/settings/backup",
        200,
        &view_json(true, "nas.local").to_string(),
    );
    stub.reply("GET", "/api/v1/backup/current", 204, "");
    stub.reply("GET", "/api/v1/backup/last", 204, "");
    stub.reply(
        "POST",
        "/api/v1/backup/test",
        200,
        &json!([
            {"step": "connect", "ok": true, "detail": null},
            {"step": "login", "ok": true, "detail": null},
            {"step": "folder", "ok": false, "detail": "folder not found"},
            {"step": "write", "ok": false, "detail": null},
        ])
        .to_string(),
    );
    let m = mount("/settings/backup");
    wait_for(&m.host, "#backup-ftp-host", 1).await;
    settle().await;
    one(&m.host, ".backup__test").click();
    wait_for(&m.host, ".backup__check", 4).await;
    let steps = texts(&m.host, ".backup__check");
    assert!(
        steps[0].contains('✓') && steps[0].contains("Connect to the server"),
        "{steps:?}"
    );
    assert!(
        steps[1].contains('✓') && steps[1].contains("Log in"),
        "{steps:?}"
    );
    assert!(
        steps[2].contains('✗') && steps[2].contains("folder not found"),
        "{steps:?}"
    );
    assert!(steps[3].contains('✗'), "{steps:?}");
    let sent = stub.wait_for("POST").await.expect("a POST");
    let body: Value = serde_json::from_str(&sent.body).unwrap();
    assert_eq!(body["host"], "nas.local");
    assert_eq!(body["port"], 21);
    assert_eq!(body["folder"], "/backups");
}

#[wasm_bindgen_test]
async fn back_up_now_shows_progress_then_last_backup() {
    let stub = FetchStub::install();
    stub.reply(
        "GET",
        "/api/v1/settings/backup",
        200,
        &view_json(true, "nas.local").to_string(),
    );
    stub.reply("GET", "/api/v1/backup/current", 204, "");
    stub.reply("GET", "/api/v1/backup/last", 204, "");
    stub.reply(
        "POST",
        "/api/v1/backup/runs",
        202,
        &progress_json().to_string(),
    );
    let m = mount("/settings/backup");
    wait_for(&m.host, ".backup__run", 1).await;
    settle().await;
    assert!(!disabled(&one(&m.host, ".backup__run")));
    assert!(all(&m.host, ".backup__progress").is_empty());
    stub.reply(
        "GET",
        "/api/v1/backup/current",
        200,
        &progress_json().to_string(),
    );
    one(&m.host, ".backup__run").click();
    wait_for(&m.host, ".backup__progress", 1).await;
    let shown = text(&m.host, ".backup__progress");
    assert!(shown.contains("History"), "{shown}");
    assert!(shown.contains("12 MB"), "{shown}");
    assert_eq!(all(&m.host, ".backup__cancel").len(), 1);
    // The job ends: nothing running, and a record.
    stub.reply("GET", "/api/v1/backup/current", 204, "");
    stub.reply("GET", "/api/v1/backup/last", 200, &last_json().to_string());
    wait_for_ms(&m.host, ".backup__last-file", 1, 5000).await;
    assert!(all(&m.host, ".backup__progress").is_empty());
    assert_eq!(text(&m.host, ".backup__last-file"), FILE);
    let last = text(&m.host, ".backup__last");
    assert!(last.contains("OK"), "{last}");
    assert!(last.contains("3 MB"), "{last}");
}

#[wasm_bindgen_test]
async fn download_link_points_at_the_route() {
    let stub = FetchStub::install();
    stub.reply(
        "GET",
        "/api/v1/settings/backup",
        200,
        &view_json(true, "").to_string(),
    );
    stub.reply("GET", "/api/v1/backup/current", 204, "");
    stub.reply("GET", "/api/v1/backup/last", 204, "");
    let m = mount("/settings/backup");
    wait_for(&m.host, "a.backup__download[href]", 1).await;
    let link = one(&m.host, "a.backup__download");
    assert_eq!(
        link.get_attribute("href").as_deref(),
        Some("/api/v1/backup/download")
    );
    assert!(link.has_attribute("download"));
    // No FTP host: "Back up now" is off, the download is not.
    assert!(disabled(&one(&m.host, ".backup__run")));
    drop(stub);

    let stub = FetchStub::install();
    stub.reply(
        "GET",
        "/api/v1/settings/backup",
        200,
        &view_json(false, "nas.local").to_string(),
    );
    stub.reply("GET", "/api/v1/backup/current", 204, "");
    stub.reply("GET", "/api/v1/backup/last", 204, "");
    let m = mount("/settings/backup");
    wait_for(&m.host, "#backup-ftp-host", 1).await;
    settle().await;
    let link = one(&m.host, "a.backup__download");
    assert_eq!(link.get_attribute("aria-disabled").as_deref(), Some("true"));
    assert!(!link.has_attribute("href"));
    assert!(disabled(&one(&m.host, ".backup__run")));
}

#[wasm_bindgen_test]
async fn settings_list_shows_backup_after_system() {
    let _stub = FetchStub::install();
    let m = mount("/settings");
    wait_for(&m.host, "a[href=\"/settings/backup\"]", 1).await;
    let links: Vec<String> = all(&m.host, "a.settings__item")
        .iter()
        .filter_map(|a| a.get_attribute("href"))
        .collect();
    let system = links
        .iter()
        .position(|h| h == "/settings/system")
        .expect("system");
    assert_eq!(links[system + 1], "/settings/backup", "{links:?}");
    let entry = texts(&m.host, "a[href=\"/settings/backup\"]").remove(0);
    assert!(entry.contains("Backup"), "{entry}");
}
