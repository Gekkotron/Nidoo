#![cfg(target_arch = "wasm32")]

use leptos::prelude::*;
use leptos::task::tick;
use nidoo_web::components::popup::{Popup, PopupSize};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
use web_sys::{Element, HtmlElement};

async fn settle() {
    tick().await;
    gloo_timers::future::TimeoutFuture::new(50).await;
    tick().await;
}

fn host() -> Element {
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    host
}

async fn mount(open: RwSignal<bool>, size: PopupSize) -> Element {
    let host = host();
    let mounted = leptos::mount::mount_to(host.clone().unchecked_into(), move || {
        view! {
            <Popup open=open title="Room settings".to_string() size=size>
                <p class="popup-test__child">"inside"</p>
            </Popup>
        }
    });
    std::mem::forget(mounted);
    settle().await;
    host
}

fn dialog(host: &Element) -> Option<HtmlElement> {
    host.query_selector("dialog")
        .unwrap()
        .map(|d| d.unchecked_into::<HtmlElement>())
}

fn dispatch(target: &Element, kind: &str) {
    let init = web_sys::EventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    let ev = web_sys::Event::new_with_event_init_dict(kind, &init).unwrap();
    target.dispatch_event(&ev).unwrap();
}

#[wasm_bindgen_test]
async fn the_popup_opens_and_the_cross_closes_it() {
    let open = RwSignal::new(false);
    let host = mount(open, PopupSize::Sheet).await;
    assert!(dialog(&host).is_none(), "nothing while closed");

    open.set(true);
    settle().await;
    let d = host
        .query_selector("dialog.popup[open]")
        .unwrap()
        .expect("an open popup");
    let text = d.text_content().unwrap();
    assert!(text.contains("Room settings"), "{text}");
    assert!(d.query_selector(".popup-test__child").unwrap().is_some());

    d.query_selector(".popup__close")
        .unwrap()
        .unwrap()
        .unchecked_into::<HtmlElement>()
        .click();
    settle().await;
    assert!(!open.get_untracked());
    assert!(dialog(&host).is_none());
}

#[wasm_bindgen_test]
async fn escape_and_backdrop_close_the_popup() {
    let open = RwSignal::new(true);
    let host = mount(open, PopupSize::Sheet).await;

    dispatch(&dialog(&host).expect("open").into(), "cancel");
    settle().await;
    assert!(!open.get_untracked(), "cancel closes");
    assert!(dialog(&host).is_none());

    open.set(true);
    settle().await;
    dialog(&host).expect("reopened").click();
    settle().await;
    assert!(!open.get_untracked(), "a click on the backdrop closes");

    open.set(true);
    settle().await;
    host.query_selector(".popup-test__child")
        .unwrap()
        .unwrap()
        .unchecked_into::<HtmlElement>()
        .click();
    settle().await;
    assert!(open.get_untracked(), "a click inside keeps it open");
    assert!(dialog(&host).is_some());
}

#[wasm_bindgen_test]
async fn small_popups_carry_their_class() {
    let open = RwSignal::new(true);
    let host = mount(open, PopupSize::Small).await;
    assert!(
        host.query_selector("dialog.popup.popup--small")
            .unwrap()
            .is_some()
    );
}

#[wasm_bindgen_test]
async fn closing_a_nested_overlay_keeps_the_popup_open() {
    let outer = RwSignal::new(true);
    let inner = RwSignal::new(false);
    let host = host();
    let mounted = leptos::mount::mount_to(host.clone().unchecked_into(), move || {
        view! {
            <Popup open=outer title="Outer".to_string()>
                <Popup open=inner title="Inner".to_string() size=PopupSize::Small>
                    <p>"picker"</p>
                </Popup>
            </Popup>
        }
    });
    std::mem::forget(mounted);
    settle().await;
    inner.set(true);
    settle().await;
    assert_eq!(host.query_selector_all("dialog").unwrap().length(), 2);

    // The inner one's ✕ drops its history entry with a Back of its own.
    host.query_selector(".popup--small .popup__close")
        .unwrap()
        .unwrap()
        .unchecked_into::<HtmlElement>()
        .click();
    settle().await;
    settle().await;
    assert!(!inner.get_untracked());
    assert!(outer.get_untracked(), "the outer popup stays open");
    assert_eq!(host.query_selector_all("dialog").unwrap().length(), 1);
}
