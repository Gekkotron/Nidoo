#![cfg(target_arch = "wasm32")]

use gloo_timers::future::TimeoutFuture;
use leptos::prelude::*;
use leptos::task::tick;
use nidoo_api::SetupStep;
use nidoo_web::i18n::Lang;
use nidoo_web::setup::restore::{RestorePollMs, RestoreStep};
use nidoo_web::setup::wizard::WizardNav;
use nidoo_web::setup::{Applying, SetupState, reload_to};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
wasm_bindgen_test_configure!(run_in_browser);

#[path = "support/fetch_stub.rs"]
mod fetch_stub;
use fetch_stub::FetchStub;

const STATUS: &str = r#"{"mode":"restore","step":"restore","finished":false,"bundled":false,"location":null,"mqtt":null,"database":null,"restarting":false,"restore_pending":true}"#;
const OLD: &str = "nidoo-backup-20260101-020000.tar.zst.age";
const NEW: &str = "nidoo-backup-20260102-020000.tar.zst.age";
const LIST: &str = r#"[{"file":"nidoo-backup-20260101-020000.tar.zst.age","size":2000000000,"created_at":1767232800},{"file":"nidoo-backup-20260102-020000.tar.zst.age","size":2100000000,"created_at":1767319200}]"#;
const SUMMARY: &str = r#"{"created_at":"2026-01-02T02:00:00Z","nidoo_version":"0.9.0","host":"nidoo","flows":12,"dashboards":3,"history_bytes":1500000000}"#;
const RUNNING: &str = r#"{"state":"running","part":"history","bytes":50,"total":100,"error":null}"#;
const DONE: &str = r#"{"state":"done","part":"history","bytes":100,"total":100,"error":null}"#;
const SOURCE_BODY: &str =
    r#"{"host":"nas.local","port":21,"user":"nidoo","password":"ftp-secret","folder":"/backups"}"#;

struct Mounted {
    host: web_sys::Element,
    applying: Applying,
    _handle: Box<dyn std::any::Any>,
}

fn mount() -> Mounted {
    mount_polling(20)
}

fn mount_polling(poll_ms: u32) -> Mounted {
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    let applying = Applying::new(reload_to);
    let step = RwSignal::new(SetupStep::Restore);
    let status = serde_json::from_str(STATUS).unwrap();
    let handle = leptos::mount::mount_to(host.clone().unchecked_into(), move || {
        provide_context(RwSignal::new(Lang::En));
        provide_context(WizardNav { step });
        provide_context(applying);
        provide_context(SetupState(RwSignal::new(Some(status))));
        provide_context(RestorePollMs(poll_ms));
        view! { <RestoreStep /> }
    });
    Mounted {
        host,
        applying,
        _handle: Box::new(handle),
    }
}

fn query(host: &web_sys::Element, selector: &str) -> Option<web_sys::Element> {
    host.query_selector(selector).unwrap()
}

fn input(host: &web_sys::Element, selector: &str) -> web_sys::HtmlInputElement {
    query(host, selector)
        .unwrap_or_else(|| panic!("no {selector}"))
        .unchecked_into()
}

fn click(host: &web_sys::Element, selector: &str) {
    query(host, selector)
        .unwrap_or_else(|| panic!("no {selector}"))
        .unchecked_into::<web_sys::HtmlElement>()
        .click();
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

/// The value of the summary row whose caption is `caption`.
fn summary_row(host: &web_sys::Element, caption: &str) -> String {
    let rows = host.query_selector_all(".restore__row").unwrap();
    for i in 0..rows.length() {
        let row: web_sys::Element = rows.get(i).unwrap().unchecked_into();
        let dt = row.query_selector("dt").unwrap().unwrap().text_content();
        if dt.as_deref() == Some(caption) {
            return row
                .query_selector("dd")
                .unwrap()
                .unwrap()
                .text_content()
                .unwrap();
        }
    }
    panic!("no summary row {caption}");
}

fn text(host: &web_sys::Element) -> String {
    host.text_content().unwrap_or_default()
}

async fn fill_ftp(host: &web_sys::Element) {
    type_into(host, "#restore-host", "nas.local").await;
    type_into(host, "#restore-user", "nidoo").await;
    type_into(host, "#restore-ftp-password", "ftp-secret").await;
    type_into(host, "#restore-folder", "/backups").await;
}

fn choose_file(host: &web_sys::Element, bytes: usize) {
    let parts = js_sys::Array::of1(&wasm_bindgen::JsValue::from_str(&"a".repeat(bytes)));
    let file = web_sys::File::new_with_str_sequence(&parts, "backup.age").unwrap();
    let transfer = web_sys::DataTransfer::new().unwrap();
    transfer.items().add_with_file(&file).unwrap();
    let el = input(host, "#restore-file");
    el.set_files(transfer.files().as_ref());
    el.dispatch_event(&web_sys::Event::new("change").unwrap())
        .unwrap();
}

#[wasm_bindgen_test]
async fn ftp_flow_lists_inspects_and_restores() {
    let stub = FetchStub::install();
    stub.reply("POST", "/api/v1/setup/restore/ftp/list", 200, LIST);
    stub.reply("POST", "/api/v1/setup/restore/ftp/inspect", 200, SUMMARY);
    stub.reply("POST", "/api/v1/setup/restore/ftp", 202, RUNNING);
    stub.reply("GET", "/api/v1/setup/restore/progress", 200, DONE);
    let m = mount();
    tick().await;
    fill_ftp(&m.host).await;
    assert_eq!(input(&m.host, "#restore-port").value(), "21");
    click(&m.host, ".restore__find");
    settle().await;
    let list = stub.wait_for("POST").await.expect("a list request");
    assert_eq!(list.body, SOURCE_BODY);
    let radios = m.host.query_selector_all(".restore__item input").unwrap();
    assert_eq!(radios.length(), 2);
    // Newest first, and the newest is selected.
    let first: web_sys::HtmlInputElement = radios.get(0).unwrap().unchecked_into();
    assert_eq!(first.value(), NEW);
    assert!(first.checked());
    let second: web_sys::HtmlInputElement = radios.get(1).unwrap().unchecked_into();
    assert_eq!(second.value(), OLD);
    click(&m.host, ".restore__next");
    tick().await;
    type_into(&m.host, "#restore-password", "correct horse").await;
    click(&m.host, ".restore__next");
    settle().await;
    let inspect = stub
        .requests()
        .into_iter()
        .find(|r| r.url.ends_with("/ftp/inspect"))
        .expect("an inspect request");
    assert_eq!(
        inspect.body,
        format!(r#"{{"source":{SOURCE_BODY},"file":"{NEW}","password":"correct horse"}}"#)
    );
    let shown = text(&m.host);
    assert!(shown.contains("0.9.0"), "{shown}");
    assert_eq!(summary_row(&m.host, "Dashboards"), "3");
    assert_eq!(summary_row(&m.host, "Automations"), "12");
    assert!(shown.contains("1.5 GB"), "{shown}");
    click(&m.host, ".restore__go");
    settle().await;
    let start = stub
        .requests()
        .into_iter()
        .find(|r| r.url == "/api/v1/setup/restore/ftp")
        .expect("a restore request");
    assert_eq!(
        start.body,
        format!(r#"{{"source":{SOURCE_BODY},"file":"{NEW}","password":"correct horse"}}"#)
    );
    assert_eq!(m.applying.target.get_untracked().as_deref(), Some("/"));
}

#[wasm_bindgen_test]
async fn wrong_password_message_is_shown() {
    let stub = FetchStub::install();
    stub.reply("POST", "/api/v1/setup/restore/ftp/list", 200, LIST);
    stub.reply(
        "POST",
        "/api/v1/setup/restore/ftp/inspect",
        422,
        r#"{"error":"the backup password is wrong"}"#,
    );
    let m = mount();
    tick().await;
    fill_ftp(&m.host).await;
    click(&m.host, ".restore__find");
    settle().await;
    click(&m.host, ".restore__next");
    tick().await;
    type_into(&m.host, "#restore-password", "nope nope").await;
    click(&m.host, ".restore__next");
    settle().await;
    assert!(text(&m.host).contains("the backup password is wrong"));
    // Still on the password step, so it can be corrected.
    assert!(query(&m.host, "#restore-password").is_some());
}

#[wasm_bindgen_test]
async fn unreachable_ftp_offers_retry() {
    let stub = FetchStub::install();
    stub.reply(
        "POST",
        "/api/v1/setup/restore/ftp/list",
        502,
        r#"{"error":"Can't reach the FTP server. Is it switched on?"}"#,
    );
    let m = mount();
    tick().await;
    fill_ftp(&m.host).await;
    click(&m.host, ".restore__find");
    settle().await;
    assert!(
        text(&m.host).contains("Can't reach the FTP server. Is it switched on?"),
        "{}",
        text(&m.host)
    );
    stub.reply("POST", "/api/v1/setup/restore/ftp/list", 200, LIST);
    click(&m.host, ".restore__retry");
    settle().await;
    let lists = stub
        .requests()
        .into_iter()
        .filter(|r| r.url.ends_with("/ftp/list"))
        .count();
    assert_eq!(lists, 2);
    assert_eq!(
        m.host
            .query_selector_all(".restore__item")
            .unwrap()
            .length(),
        2
    );
}

#[wasm_bindgen_test]
async fn upload_inspect_sends_only_the_head() {
    let stub = FetchStub::install();
    stub.reply("POST", "/api/v1/setup/restore/upload/inspect", 200, SUMMARY);
    let m = mount();
    tick().await;
    choose_file(&m.host, 2 * 1024 * 1024);
    settle().await;
    type_into(&m.host, "#restore-password", "correct horse").await;
    click(&m.host, ".restore__next");
    settle().await;
    let sent = stub
        .requests()
        .into_iter()
        .find(|r| r.url.ends_with("/upload/inspect"))
        .expect("an inspect request");
    assert!(sent.body.len() <= 1024 * 1024, "{} bytes", sent.body.len());
    assert!(!sent.body.is_empty());
    assert!(
        sent.headers.contains("x-backup-password: correct%20horse"),
        "{}",
        sent.headers
    );
    assert!(text(&m.host).contains("0.9.0"));
}

#[wasm_bindgen_test]
async fn a_failed_upload_shows_the_hubs_own_error() {
    let stub = FetchStub::install();
    stub.reply("POST", "/api/v1/setup/restore/upload/inspect", 200, SUMMARY);
    // The hub may close the connection before the browser reads its answer; the page then asks
    // the progress route what went wrong.
    stub.reply("PUT", "/api/v1/setup/restore/upload", 500, "oops");
    // A poll so slow that only the fallback after the failed PUT can read the progress.
    let m = mount_polling(60_000);
    settle().await;
    // Only now: on opening, the step would show a failed restore it finds.
    stub.reply(
        "GET",
        "/api/v1/setup/restore/progress",
        200,
        r#"{"state":"failed","part":"history","bytes":10,"total":100,"error":"the history is damaged"}"#,
    );
    choose_file(&m.host, 1024);
    settle().await;
    type_into(&m.host, "#restore-password", "correct horse").await;
    click(&m.host, ".restore__next");
    settle().await;
    click(&m.host, ".restore__go");
    settle().await;
    let shown = text(&m.host);
    assert!(shown.contains("the history is damaged"), "{shown}");
    assert!(!shown.contains("HTTP 500"), "{shown}");
    // An upload asks for its file again.
    click(&m.host, ".restore__retry");
    tick().await;
    assert!(query(&m.host, "#restore-file").is_some());
}

#[wasm_bindgen_test]
async fn the_progress_bar_follows_the_restore() {
    let stub = FetchStub::install();
    stub.reply("POST", "/api/v1/setup/restore/ftp/list", 200, LIST);
    stub.reply("POST", "/api/v1/setup/restore/ftp/inspect", 200, SUMMARY);
    stub.reply("POST", "/api/v1/setup/restore/ftp", 202, RUNNING);
    let m = mount();
    settle().await;
    // Only now: on opening, the step would follow a running restore it finds.
    stub.reply("GET", "/api/v1/setup/restore/progress", 200, RUNNING);
    fill_ftp(&m.host).await;
    click(&m.host, ".restore__find");
    settle().await;
    click(&m.host, ".restore__next");
    tick().await;
    type_into(&m.host, "#restore-password", "correct horse").await;
    click(&m.host, ".restore__next");
    settle().await;
    click(&m.host, ".restore__go");
    settle().await;
    let bar = query(&m.host, ".restore__bar").expect("a progress bar");
    assert_eq!(bar.get_attribute("value").as_deref(), Some("50"));
    assert_eq!(bar.get_attribute("max").as_deref(), Some("100"));
    assert!(text(&m.host).contains("History"));
    assert!(m.applying.target.get_untracked().is_none());
}

#[wasm_bindgen_test]
async fn abandon_returns_to_devices() {
    let stub = FetchStub::install();
    stub.reply(
        "PUT",
        "/api/v1/setup/restore",
        200,
        r#"{"mode":"restore","step":"restore","finished":false,"bundled":false,"location":null,"mqtt":null,"database":null,"restarting":true,"restore_pending":false}"#,
    );
    let m = mount();
    tick().await;
    click(&m.host, ".restore__without");
    settle().await;
    let put = stub.wait_for("PUT").await.expect("a PUT");
    assert_eq!(put.body, r#"{"pending":false}"#);
    assert_eq!(
        m.applying.target.get_untracked().as_deref(),
        Some("/setup?step=devices")
    );
}

#[wasm_bindgen_test]
async fn a_second_find_drops_the_first_servers_answer() {
    let stub = FetchStub::install();
    let first = r#"[{"file":"nidoo-backup-20250101-020000.tar.zst.age","size":1000000,"created_at":1735696800}]"#;
    stub.reply_after("POST", "/api/v1/setup/restore/ftp/list", 200, first, 300);
    let m = mount();
    tick().await;
    fill_ftp(&m.host).await;
    click(&m.host, ".restore__find");
    tick().await;
    // Back out while the first list is still loading, point at another server, ask again.
    click(&m.host, ".restore__back");
    tick().await;
    type_into(&m.host, "#restore-host", "other.local").await;
    stub.reply("POST", "/api/v1/setup/restore/ftp/list", 200, LIST);
    click(&m.host, ".restore__find");
    TimeoutFuture::new(500).await;
    tick().await;
    let radios = m.host.query_selector_all(".restore__item input").unwrap();
    assert_eq!(radios.length(), 2, "the second server's list shows");
    let first_radio: web_sys::HtmlInputElement = radios.get(0).unwrap().unchecked_into();
    assert_eq!(first_radio.value(), NEW);
}

/// Picks the newest backup on the FTP server, types the password and starts the restore.
async fn start_ftp_restore(host: &web_sys::Element) {
    fill_ftp(host).await;
    click(host, ".restore__find");
    settle().await;
    click(host, ".restore__next");
    tick().await;
    type_into(host, "#restore-password", "correct horse").await;
    click(host, ".restore__next");
    settle().await;
    click(host, ".restore__go");
    settle().await;
}

#[wasm_bindgen_test]
async fn a_reloaded_step_follows_the_running_restore() {
    let stub = FetchStub::install();
    stub.reply("GET", "/api/v1/setup/restore/progress", 200, RUNNING);
    let m = mount();
    settle().await;
    let bar = query(&m.host, ".restore__bar").expect("the running restore");
    assert_eq!(bar.get_attribute("value").as_deref(), Some("50"));
    assert!(
        query(&m.host, "#restore-host").is_none(),
        "not the first screen"
    );
    assert!(query(&m.host, ".restore__without").is_none());
    // It keeps following it to the end.
    stub.reply("GET", "/api/v1/setup/restore/progress", 200, DONE);
    settle().await;
    assert_eq!(m.applying.target.get_untracked().as_deref(), Some("/"));
}

#[wasm_bindgen_test]
async fn a_reloaded_step_shows_a_failed_restore_and_retry_asks_for_the_source() {
    let stub = FetchStub::install();
    stub.reply(
        "GET",
        "/api/v1/setup/restore/progress",
        200,
        r#"{"state":"failed","part":"history","bytes":10,"total":100,"error":"the history is damaged"}"#,
    );
    let m = mount_polling(60_000);
    settle().await;
    assert!(
        text(&m.host).contains("the history is damaged"),
        "{}",
        text(&m.host)
    );
    // The page does not know where the backup came from: back to the first screen.
    click(&m.host, ".restore__retry");
    tick().await;
    assert!(query(&m.host, "#restore-host").is_some());
    assert!(
        !stub.requests().iter().any(|r| r.method == "POST"),
        "no restore started blind"
    );
}

#[wasm_bindgen_test]
async fn an_ftp_server_lost_during_the_restore_is_worded_by_the_page() {
    let stub = FetchStub::install();
    stub.reply("POST", "/api/v1/setup/restore/ftp/list", 200, LIST);
    stub.reply("POST", "/api/v1/setup/restore/ftp/inspect", 200, SUMMARY);
    stub.reply("POST", "/api/v1/setup/restore/ftp", 202, RUNNING);
    let m = mount();
    settle().await;
    stub.reply(
        "GET",
        "/api/v1/setup/restore/progress",
        200,
        r#"{"state":"failed","part":"history","bytes":10,"total":100,"error":"can't reach the FTP server (timed out)","code":"ftp_unreachable"}"#,
    );
    start_ftp_restore(&m.host).await;
    let shown = text(&m.host);
    assert!(
        shown.contains("Can't reach the FTP server. Is it switched on?"),
        "{shown}"
    );
    assert!(!shown.contains("timed out"), "{shown}");
}
