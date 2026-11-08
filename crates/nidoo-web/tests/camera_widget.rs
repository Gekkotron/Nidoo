#![cfg(target_arch = "wasm32")]

#[path = "support/fetch_stub.rs"]
mod fetch_stub;

use fetch_stub::FetchStub;
use leptos::prelude::*;
use leptos::task::tick;
use leptos_router::components::Router;
use nidoo_api::{AlertChannel, AlertSeverity, AlertView, CameraInfo, CamerasView, Span};
use nidoo_core::dashboard::{Dashboard, Widget, WidgetConfig, WidgetKind};
use nidoo_web::cameras::{CameraList, PageVisible};
use nidoo_web::components::now::Now;
use nidoo_web::dashboards::camera_widget::CameraWidget;
use nidoo_web::dashboards::config_sheet::WidgetConfigSheet;
use nidoo_web::dashboards::edit::EditState;
use nidoo_web::house::House;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

const NOW: u64 = 1_800_000_000;
const SNAPSHOT: &str = "/api/v1/media?url=snap";

fn alert() -> AlertView {
    AlertView {
        id: "frigate-1".into(),
        channel: AlertChannel::Camera,
        severity: AlertSeverity::Warning,
        title: "Front Door".into(),
        body: "Car detected".into(),
        image_url: Some(SNAPSHOT.into()),
        video_url: None,
        entity_id: None,
        camera: Some("front_door".into()),
        actions: vec![],
        source: Some("frigate".into()),
        silent: false,
        resolved: false,
        resolved_at: None,
        sent_at: NOW - 600,
    }
}

fn camera_list(live: bool) -> CamerasView {
    CamerasView {
        live,
        cameras: vec![
            CameraInfo {
                id: "front_door".into(),
                name: "Front Door".into(),
            },
            CameraInfo {
                id: "garden".into(),
                name: "Garden".into(),
            },
        ],
    }
}

fn mount(cameras: &[&str], span: Span, live: bool) -> web_sys::Element {
    mount_with(cameras, span, live, true)
}

fn mount_with(cameras: &[&str], span: Span, live: bool, page_shown: bool) -> web_sys::Element {
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    let cameras: Vec<String> = cameras.iter().map(|c| (*c).to_owned()).collect();
    std::mem::forget(leptos::mount::mount_to(
        host.clone().unchecked_into(),
        move || {
            let house = House::new();
            house.alerts.set(vec![alert()]);
            provide_context(house);
            provide_context(Now(RwSignal::new(NOW)));
            provide_context(CameraList(RwSignal::new(camera_list(live))));
            provide_context(PageVisible(RwSignal::new(page_shown)));
            let cameras = cameras.clone();
            view! { <Router><CameraWidget cameras=cameras span=span /></Router> }
        },
    ));
    host
}

fn all(host: &web_sys::Element, selector: &str) -> u32 {
    host.query_selector_all(selector).unwrap().length()
}

fn one(host: &web_sys::Element, selector: &str) -> web_sys::HtmlElement {
    host.query_selector(selector)
        .unwrap()
        .expect(selector)
        .unchecked_into()
}

fn text(host: &web_sys::Element, selector: &str) -> String {
    one(host, selector).text_content().unwrap_or_default()
}

async fn sleep(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms)
            .unwrap();
    });
    wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
}

#[wasm_bindgen_test]
async fn wide_tile_shows_one_camera_with_arrows() {
    let host = mount(&["front_door", "garden"], Span::X2x1, true);
    tick().await;
    assert_eq!(all(&host, "img"), 1);
    assert_eq!(all(&host, ".camera-widget__arrow"), 2);
    assert_eq!(text(&host, ".camera-frame__name"), "Front Door");
    one(&host, ".camera-widget__arrow--next").click();
    tick().await;
    assert_eq!(text(&host, ".camera-frame__name"), "Garden");
}

#[wasm_bindgen_test]
async fn big_tile_with_three_cameras_is_a_grid_with_an_empty_square() {
    let host = mount(&["front_door", "garden", "garden"], Span::X2x2, true);
    tick().await;
    assert_eq!(all(&host, ".camera-widget--grid .camera-widget__cell"), 4);
    assert_eq!(all(&host, ".camera-widget__cell--empty"), 1);
}

#[wasm_bindgen_test]
async fn badge_links_to_the_alert() {
    let host = mount(&["front_door"], Span::X2x1, true);
    tick().await;
    let badge = one(&host, ".camera-frame__badge");
    assert_eq!(
        badge.get_attribute("href").as_deref(),
        Some("/alerts/frigate-1")
    );
    assert!(badge.text_content().unwrap().starts_with("Car detected · "));
}

#[wasm_bindgen_test]
async fn live_off_shows_the_alert_snapshot() {
    let host = mount(&["front_door"], Span::X2x1, false);
    tick().await;
    assert_eq!(
        one(&host, "img").get_attribute("src").as_deref(),
        Some(SNAPSHOT)
    );
    assert_eq!(text(&host, ".camera-frame__note"), "Live view off");
    assert_eq!(all(&host, ".camera-frame__live"), 0);
}

#[wasm_bindgen_test]
async fn live_off_without_an_alert_says_no_events() {
    let host = mount(&["garden"], Span::X2x1, false);
    tick().await;
    assert_eq!(all(&host, "img"), 0);
    assert_eq!(text(&host, ".camera-frame__note"), "No events yet");
}

#[wasm_bindgen_test]
async fn missing_camera_shows_not_found() {
    // The stub answers 404 to every frame request. `porch` is this test's own camera: the
    // widgets earlier tests left mounted poll theirs.
    let stub = FetchStub::install();
    let host = mount(&["porch"], Span::X2x1, true);
    let mut waited = 0;
    while !host.inner_html().contains("Camera not found") && waited < 3000 {
        sleep(50).await;
        waited += 50;
    }
    assert_eq!(text(&host, ".camera-frame__note"), "Camera not found");
    sleep(1500).await;
    let probes = stub
        .requests()
        .into_iter()
        .filter(|r| r.url.starts_with("/api/v1/cameras/porch/latest.jpg"))
        .count();
    assert_eq!(probes, 1);
}

#[wasm_bindgen_test]
async fn viewer_streams_and_closes_on_escape() {
    let host = mount(&["front_door", "garden"], Span::X2x1, true);
    tick().await;
    one(&host, ".camera-frame").click();
    tick().await;
    let src = one(&host, ".camera-viewer img")
        .get_attribute("src")
        .unwrap_or_default();
    assert!(
        src.contains("/api/v1/cameras/front_door/stream?h="),
        "{src}"
    );
    let init = web_sys::KeyboardEventInit::new();
    init.set_key("Escape");
    let event =
        web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
    web_sys::window().unwrap().dispatch_event(&event).unwrap();
    tick().await;
    assert_eq!(all(&host, ".camera-viewer"), 0);
}

#[wasm_bindgen_test]
async fn viewer_closes_on_the_dialogs_cancel() {
    // Android's Back reaches a modal dialog as `cancel`, not as a history pop.
    let host = mount(&["front_door"], Span::X2x1, true);
    tick().await;
    one(&host, ".camera-frame").click();
    tick().await;
    let cancel = web_sys::Event::new_with_event_init_dict("cancel", &{
        let init = web_sys::EventInit::new();
        init.set_cancelable(true);
        init
    })
    .unwrap();
    one(&host, ".camera-viewer")
        .dispatch_event(&cancel)
        .unwrap();
    tick().await;
    assert_eq!(all(&host, ".camera-viewer"), 0);
}

#[wasm_bindgen_test]
async fn viewer_streams_nothing_while_the_tab_is_hidden() {
    let host = mount_with(&["front_door"], Span::X2x1, true, false);
    tick().await;
    one(&host, ".camera-frame").click();
    tick().await;
    let src = one(&host, ".camera-viewer img")
        .get_attribute("src")
        .unwrap_or_default();
    assert!(!src.contains("/stream"), "{src}");
}

#[wasm_bindgen_test]
async fn add_camera_select_resets_after_a_pick() {
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    let dashboard = Dashboard {
        id: "d".into(),
        name: "D".into(),
        order: 1,
        widgets: vec![Widget {
            id: "w".into(),
            kind: WidgetKind::Camera,
            span: Span::X2x1,
            config: WidgetConfig::Cameras {
                cameras: vec!["front_door".into()],
                title: None,
            },
            pos: None,
        }],
        show_header: false,
    };
    let mut list = camera_list(true);
    list.cameras.push(CameraInfo {
        id: "porch".into(),
        name: "Porch".into(),
    });
    std::mem::forget(leptos::mount::mount_to(
        host.clone().unchecked_into(),
        move || {
            provide_context(House::new());
            provide_context(CameraList(RwSignal::new(list.clone())));
            let state = EditState::new(dashboard.clone(), true);
            view! { <WidgetConfigSheet state=state index=0 on_close=Callback::new(|()| {}) /> }
        },
    ));
    tick().await;
    let select: web_sys::HtmlSelectElement = host
        .query_selector(".camera-list + select")
        .unwrap()
        .expect("the Add camera select")
        .unchecked_into();
    select.set_value("garden");
    select
        .dispatch_event(&web_sys::Event::new("change").unwrap())
        .unwrap();
    tick().await;
    assert_eq!(all(&host, ".camera-list__row"), 2);
    assert_eq!(select.value(), "");
}
