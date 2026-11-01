#![cfg(target_arch = "wasm32")]

use leptos::prelude::*;
use leptos::task::tick;
use nidoo_core::model::Hs;
use nidoo_web::components::hs_wheel::HsWheel;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
use web_sys::{HtmlElement, KeyboardEvent, KeyboardEventInit};
wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen_test]
async fn marker_follows_the_colour_and_arrows_nudge_it() {
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();

    let value = RwSignal::new(Some(Hs { h: 90.0, s: 100.0 }));
    let picked = RwSignal::new(None::<Hs>);
    let _mounted = leptos::mount::mount_to(host.clone().unchecked_into(), move || {
        view! { <HsWheel value=value on_pick=Callback::new(move |hs| picked.set(Some(hs))) /> }
    });
    tick().await;

    let marker: HtmlElement = host
        .query_selector(".hs__marker")
        .unwrap()
        .expect("marker")
        .unchecked_into();
    let style = marker.get_attribute("style").unwrap();
    assert!(style.contains("left: 100.00%"), "{style}");
    assert!(style.contains("top: 50.00%"), "{style}");

    value.set(Some(Hs { h: 0.0, s: 0.0 }));
    tick().await;
    assert!(
        marker
            .get_attribute("style")
            .unwrap()
            .contains("left: 50.00%")
    );

    let wheel = host.query_selector(".hs__wheel").unwrap().expect("wheel");
    let init = KeyboardEventInit::new();
    init.set_key("ArrowRight");
    init.set_bubbles(true);
    let event = KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
    wheel.dispatch_event(&event).unwrap();
    assert_eq!(picked.get_untracked(), Some(Hs { h: 5.0, s: 0.0 }));
}
