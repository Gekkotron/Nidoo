#![cfg(target_arch = "wasm32")]

use leptos::prelude::*;
use leptos::task::tick;
use leptos_router::components::{Route, Router, Routes};
use leptos_router::path;
use nidoo_api::{FlowView, FlowWriteOutcome, RunStatus, RunView, StepOutcome, StepView};
use nidoo_core::flow::{Flow, FlowMode, Position};
use nidoo_web::components::now::install_now;
use nidoo_web::components::toast::Toasts;
use nidoo_web::house::House;
use nidoo_web::i18n::Lang;
use nidoo_web::screens::flow_canvas::{FlowCanvasScreen, NewFlowScreen};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
wasm_bindgen_test_configure!(run_in_browser);

const REFERENCE: &str = r#"{
  "id": "living_led_motion",
  "name": "Living LED on motion",
  "enabled": true,
  "mode": "restart",
  "nodes": [
    { "id": "motion",  "type": "trigger.state", "name": "Motion",
      "params": { "entity": "0x00158d00044c4d4c_occupancy_zigbee2mqtt" } },
    { "id": "moving", "type": "if",
      "params": { "op": "eq", "value": true } },
    { "id": "on",  "type": "command",
      "params": { "entity": "0x9035eafffec07012_light_zigbee2mqtt", "command": { "type": "turn_on" } } },
    { "id": "wait", "type": "delay", "params": { "for": "5m" } },
    { "id": "off", "type": "command",
      "params": { "entity": "0x9035eafffec07012_light_zigbee2mqtt", "command": { "type": "turn_off" } } }
  ],
  "edges": [
    { "from": "motion", "to": "moving" },
    { "from": "moving", "port": "true",  "to": "on" },
    { "from": "moving", "port": "false", "to": "wait" },
    { "from": "wait",   "to": "off" }
  ]
}"#;

fn host() -> web_sys::Element {
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    host
}

fn mount(host: &web_sys::Element, flows: Vec<FlowView>, path: &str) -> Box<dyn std::any::Any> {
    mount_house(host, flows, path).0
}

/// Mounts the canvas and also returns its house, for tests that push live updates.
fn mount_house(
    host: &web_sys::Element,
    flows: Vec<FlowView>,
    path: &str,
) -> (Box<dyn std::any::Any>, House) {
    mount_house_lang(host, flows, path, RwSignal::new(Lang::En))
}

/// [`mount_house`] with the language signal the test flips.
fn mount_house_lang(
    host: &web_sys::Element,
    flows: Vec<FlowView>,
    path: &str,
    lang: RwSignal<Lang>,
) -> (Box<dyn std::any::Any>, House) {
    web_sys::window()
        .unwrap()
        .history()
        .unwrap()
        .replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(path))
        .unwrap();
    let house = House::new();
    house.flows.set(flows);
    house.connection.set(nidoo_api::ConnectionStatus::Online);
    let mounted = Box::new(leptos::mount::mount_to(
        host.clone().unchecked_into(),
        move || {
            provide_context(house);
            provide_context(Toasts::new());
            provide_context(install_now());
            provide_context(lang);
            nidoo_web::nav::install_nav();
            view! {
                <Router>
                    <Routes fallback=|| "fallback">
                        <Route path=path!("/flows/new") view=NewFlowScreen />
                        <Route path=path!("/flows/:id") view=FlowCanvasScreen />
                    </Routes>
                </Router>
            }
        },
    ));
    (mounted, house)
}

fn step(node: &str, detail: &str) -> StepView {
    StepView {
        node: node.into(),
        at: 1,
        outcome: StepOutcome::Ok,
        detail: detail.into(),
        ports: None,
    }
}

fn count(host: &web_sys::Element, selector: &str) -> u32 {
    host.query_selector_all(selector).unwrap().length()
}

/// The run that went motion → moving (false) → wait → off.
fn done_run() -> RunView {
    RunView {
        id: "r1".into(),
        flow_id: "living_led_motion".into(),
        status: RunStatus::Done,
        started_at: 1_700_000_000,
        ended_at: Some(1_700_000_300),
        trigger: None,
        manual: false,
        steps: vec![
            step("motion", "occupancy = true"),
            step("moving", "false"),
            step("wait", "waited 5m"),
            step("off", "accepted"),
        ],
    }
}

fn reference_view(active_run: Option<&str>, runs: Vec<RunView>) -> FlowView {
    FlowView {
        definition: Flow::decode(REFERENCE.as_bytes()).unwrap(),
        invalid: None,
        active_run: active_run.map(Into::into),
        runs,
    }
}

fn text(host: &web_sys::Element, selector: &str) -> String {
    host.query_selector(selector)
        .unwrap()
        .and_then(|e| e.text_content())
        .unwrap_or_default()
}

fn click(host: &web_sys::Element, selector: &str) {
    host.query_selector(selector)
        .unwrap()
        .unwrap_or_else(|| panic!("no {selector}: {}", host.inner_html()))
        .unchecked_into::<web_sys::HtmlElement>()
        .click();
}

#[wasm_bindgen_test]
async fn canvas_lights_the_active_run() {
    let host = host();
    let _mounted = mount(
        &host,
        vec![reference_view(Some("r1"), vec![done_run()])],
        "/flows/living_led_motion",
    );
    tick().await;
    let html = host.inner_html();
    assert_eq!(count(&host, ".flow-node"), 5, "{html}");
    assert_eq!(count(&host, ".flow-wire"), 4, "{html}");
    assert_eq!(count(&host, ".flow-wire--lit"), 3, "{html}");
    assert_eq!(count(&host, ".flow-wire--live"), 3, "{html}");
    assert_eq!(count(&host, "[data-node=wait].flow-node--lit"), 1, "{html}");
    assert_eq!(count(&host, "[data-node=on].flow-node--lit"), 0, "{html}");
    assert_eq!(count(&host, ".flow-node--dim"), 0, "{html}");
    assert!(text(&host, ".run-list").contains("Done"), "{html}");
    drop(_mounted);

    // A finished run is not lit by default.
    let host = self::host();
    let _mounted = mount(
        &host,
        vec![reference_view(None, vec![done_run()])],
        "/flows/living_led_motion",
    );
    tick().await;
    let html = host.inner_html();
    assert_eq!(count(&host, ".flow-wire--lit"), 0, "{html}");
    assert_eq!(count(&host, ".flow-node--lit"), 0, "{html}");
}

#[wasm_bindgen_test]
async fn replay_dims_the_rest() {
    let host = host();
    let _mounted = mount(
        &host,
        vec![reference_view(None, vec![done_run()])],
        "/flows/living_led_motion",
    );
    tick().await;
    assert_eq!(text(&host, "[data-node=wait] .flow-node__step"), "");
    assert_eq!(count(&host, ".flow-drawer--open"), 0);
    click(&host, ".flow-toolbar__runs");
    tick().await;
    assert_eq!(count(&host, ".flow-drawer--open"), 1);
    click(&host, ".run-list li:first-child");
    tick().await;
    let html = host.inner_html();
    // Four of the five cards were visited; `on` is dimmed.
    assert_eq!(count(&host, ".flow-node--dim"), 1, "{html}");
    assert_eq!(count(&host, "[data-node=on].flow-node--dim"), 1, "{html}");
    assert_eq!(count(&host, ".flow-wire--lit"), 3, "{html}");
    assert_eq!(count(&host, ".flow-wire--live"), 0, "{html}");
    assert_eq!(
        text(&host, "[data-node=wait] .flow-node__step"),
        "+0\u{a0}s",
        "{html}"
    );
    click(&host, ".run-list li:first-child");
    tick().await;
    let html = host.inner_html();
    assert_eq!(count(&host, ".flow-node--dim"), 0, "{html}");
    assert_eq!(count(&host, ".flow-wire--lit"), 0, "{html}");
    assert_eq!(
        text(&host, "[data-node=wait] .flow-node__step"),
        "",
        "{html}"
    );
    // Esc stops a replay too.
    click(&host, ".run-list li:first-child");
    tick().await;
    assert_eq!(count(&host, ".flow-node--dim"), 1);
    let init = web_sys::KeyboardEventInit::new();
    init.set_key("Escape");
    let escape =
        web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
    web_sys::window().unwrap().dispatch_event(&escape).unwrap();
    tick().await;
    assert_eq!(count(&host, ".flow-node--dim"), 0, "{}", host.inner_html());
}

#[wasm_bindgen_test]
async fn pills_read_as_sentences() {
    let host = host();
    let _mounted = mount(
        &host,
        vec![reference_view(None, vec![])],
        "/flows/living_led_motion",
    );
    tick().await;
    let html = host.inner_html();
    assert_eq!(
        text(&host, "[data-node=wait] .flow-node__title"),
        "Wait 5\u{a0}min",
        "{html}"
    );
    assert_eq!(count(&host, ".flow-node--wait"), 1, "{html}");
    assert_eq!(count(&host, ".flow-node--check"), 1, "{html}");
    assert_eq!(count(&host, ".flow-node--trigger"), 1, "{html}");
    assert_eq!(count(&host, ".flow-node--action"), 2, "{html}");
}

#[wasm_bindgen_test]
async fn yes_no_labels_on_wires() {
    let host = host();
    let _mounted = mount(
        &host,
        vec![reference_view(None, vec![])],
        "/flows/living_led_motion",
    );
    tick().await;
    let html = host.inner_html();
    assert_eq!(count(&host, ".flow-wire-label"), 2, "{html}");
    assert_eq!(count(&host, ".flow-wire--yes"), 1, "{html}");
    assert_eq!(count(&host, ".flow-wire--no"), 1, "{html}");
    let labels: Vec<String> = (0..2)
        .map(|i| {
            host.query_selector_all(".flow-wire-label")
                .unwrap()
                .item(i)
                .and_then(|n| n.text_content())
                .unwrap_or_default()
        })
        .collect();
    assert!(labels.contains(&"yes".to_owned()), "{html}");
    assert!(labels.contains(&"no".to_owned()), "{html}");
}

#[wasm_bindgen_test]
async fn controls_and_minimap_present() {
    let host = host();
    let _mounted = mount(
        &host,
        vec![reference_view(None, vec![])],
        "/flows/living_led_motion",
    );
    tick().await;
    let html = host.inner_html();
    assert_eq!(count(&host, ".flow-controls button"), 3, "{html}");
    assert_eq!(count(&host, ".flow-minimap"), 1, "{html}");
    assert_eq!(count(&host, ".flow-toolbar__edit"), 1, "{html}");
    // One minimap rect per card.
    assert_eq!(count(&host, ".flow-minimap__node"), 5, "{html}");
}

#[wasm_bindgen_test]
async fn invalid_flow_shows_the_reason_and_no_canvas() {
    let flow = FlowView {
        definition: Flow {
            id: "broken".into(),
            name: "broken".into(),
            ..Default::default()
        },
        invalid: Some("missing port".into()),
        active_run: None,
        runs: vec![],
    };
    let host = host();
    let _mounted = mount(&host, vec![flow], "/flows/broken");
    tick().await;
    let html = host.inner_html();
    assert_eq!(count(&host, ".flow-invalid"), 1, "{html}");
    assert!(html.contains("missing port"), "{html}");
    assert_eq!(count(&host, ".flow-canvas"), 0, "{html}");
}

#[wasm_bindgen_test]
async fn unknown_flow_links_back() {
    let host = host();
    let _mounted = mount(&host, vec![], "/flows/nope");
    tick().await;
    let html = host.inner_html();
    assert!(html.contains("Unknown flow"), "{html}");
    assert!(html.contains("href=\"/flows\""), "{html}");
}

#[wasm_bindgen_test]
async fn parked_delay_counts_down() {
    // The shape the engine really produces while it waits: the delay step is only recorded
    // after the wait, so the last step is the `if` that took the false branch.
    let now = (js_sys::Date::now() / 1000.0) as u64;
    let mut moving = step("moving", "false");
    moving.at = now;
    let run = RunView {
        id: "r1".into(),
        flow_id: "living_led_motion".into(),
        status: RunStatus::Running,
        started_at: now,
        ended_at: None,
        trigger: None,
        manual: false,
        steps: vec![step("motion", "occupancy = false"), moving],
    };
    let flow = FlowView {
        definition: Flow::decode(REFERENCE.as_bytes()).unwrap(),
        invalid: None,
        active_run: Some("r1".into()),
        runs: vec![run],
    };
    let host = host();
    let _mounted = mount(&host, vec![flow], "/flows/living_led_motion");
    tick().await;
    let html = host.inner_html();
    assert_eq!(
        count(&host, "[data-node=wait].flow-node--pending"),
        1,
        "{html}"
    );
    assert_eq!(count(&host, ".flow-node--pending"), 1, "{html}");
    assert!(html.contains(" left"), "{html}");
    assert!(html.contains("4:5") || html.contains("5:00"), "{html}");
    assert!(html.contains("disabled"), "{html}");
    // The wire into the parked delay is lit.
    assert_eq!(count(&host, ".flow-wire--lit"), 2, "{html}");
    // The branch the if took is the lit one.
    assert_eq!(count(&host, ".flow-wire--lit.flow-wire--no"), 1, "{html}");
    assert_eq!(count(&host, ".flow-wire--lit.flow-wire--yes"), 0, "{html}");
}

#[wasm_bindgen_test]
async fn cards_have_handles_and_orphans_fade() {
    let mut definition = Flow::decode(REFERENCE.as_bytes()).unwrap();
    definition.edges.retain(|e| e.to != "off");
    let flow = FlowView {
        definition,
        invalid: None,
        active_run: None,
        runs: vec![],
    };
    let host = host();
    let _mounted = mount(&host, vec![flow], "/flows/living_led_motion");
    tick().await;
    let html = host.inner_html();
    // Every node but the trigger has an input; every node has one output, the if two.
    assert_eq!(count(&host, ".flow-node__in"), 4, "{html}");
    assert_eq!(count(&host, ".flow-node__out"), 6, "{html}");
    assert_eq!(count(&host, ".flow-node--orphan"), 1, "{html}");
    assert_eq!(
        count(&host, "[data-node=off].flow-node--orphan"),
        1,
        "{html}"
    );
}

/// Answers every `fetch` with `body` as a 200 JSON response, until the returned guard drops.
struct FakeFetch(wasm_bindgen::JsValue);

impl FakeFetch {
    /// Also records every request as `{url, method, body}` for [`FakeFetch::requests`].
    fn answer(body: &str) -> Self {
        let global = js_sys::global();
        let real = js_sys::Reflect::get(&global, &"fetch".into()).unwrap();
        let script = format!(
            "return (async () => {{
                const req = input instanceof Request ? input : null;
                const url = req ? req.url : String(input);
                const method = req ? req.method : ((init && init.method) || 'GET');
                const text = req ? await req.clone().text() : ((init && init.body) || '');
                (globalThis.__fetches = globalThis.__fetches || []).push({{ url, method, body: text }});
                return new Response({}, {{ status: 200, headers: {{ 'content-type': 'application/json' }} }});
            }})()",
            serde_json::to_string(body).unwrap()
        );
        js_sys::Reflect::set(&global, &"__fetches".into(), &js_sys::Array::new()).unwrap();
        let fake = js_sys::Function::new_with_args("input, init", &script);
        js_sys::Reflect::set(&global, &"fetch".into(), &fake).unwrap();
        Self(real)
    }

    /// The (method, path, body) of every request made since `answer`.
    fn requests(&self) -> Vec<(String, String, String)> {
        let log: js_sys::Array = js_sys::Reflect::get(&js_sys::global(), &"__fetches".into())
            .unwrap()
            .unchecked_into();
        let field = |entry: &wasm_bindgen::JsValue, key: &str| {
            js_sys::Reflect::get(entry, &key.into())
                .unwrap()
                .as_string()
                .unwrap_or_default()
        };
        log.iter()
            .map(|e| {
                let url = field(&e, "url");
                let path = url
                    .find("/api/")
                    .map_or(url.clone(), |i| url[i..].to_owned());
                (field(&e, "method"), path, field(&e, "body"))
            })
            .collect()
    }
}

/// The hub's answer to a flow PUT that stored `flow` with no warning.
fn write_outcome(flow: &FlowView) -> String {
    serde_json::to_string(&FlowWriteOutcome {
        flow: flow.clone(),
        warning: None,
    })
    .unwrap()
}

/// The PUTs `fetch` saw; a save also refetches `/api/v1/config/flows` for the fallback badge.
fn puts(fetch: &FakeFetch) -> Vec<(String, String, String)> {
    fetch
        .requests()
        .into_iter()
        .filter(|(method, _, _)| method == "PUT")
        .collect()
}

impl Drop for FakeFetch {
    fn drop(&mut self) {
        js_sys::Reflect::set(&js_sys::global(), &"fetch".into(), &self.0).unwrap();
    }
}

#[wasm_bindgen_test]
async fn save_leaves_edit_mode() {
    let saved = reference_view(None, vec![]);
    let host = host();
    let _mounted = mount(&host, vec![saved.clone()], "/flows/living_led_motion");
    tick().await;
    click(&host, ".flow-toolbar__edit");
    tick().await;
    assert_eq!(count(&host, ".flow-graph--editing"), 1);
    let _fetch = FakeFetch::answer(&write_outcome(&saved));
    click(&host, ".flow-toolbar button[aria-label=Save]");
    gloo_timers::future::TimeoutFuture::new(50).await;
    let html = host.inner_html();
    assert_eq!(count(&host, ".flow-graph--editing"), 0, "{html}");
    assert_eq!(count(&host, ".flow-toolbar__edit"), 1, "{html}");
}

// ---- Editing --------------------------------------------------------------------------------

fn element(host: &web_sys::Element, selector: &str) -> web_sys::Element {
    host.query_selector(selector)
        .unwrap()
        .unwrap_or_else(|| panic!("no {selector}: {}", host.inner_html()))
}

/// The centre of `el` on screen, scrolled into view first so a hit test can find it.
fn centre(el: &web_sys::Element) -> (i32, i32) {
    el.scroll_into_view();
    let rect = el.get_bounding_client_rect();
    (
        (rect.left() + rect.width() / 2.0) as i32,
        (rect.top() + rect.height() / 2.0) as i32,
    )
}

/// A mouse pointer event of `kind` at client point `at`, dispatched on `target`.
fn pointer_at(target: &web_sys::Element, kind: &str, at: (i32, i32)) {
    let init = web_sys::PointerEventInit::new();
    init.set_bubbles(true);
    init.set_pointer_id(1);
    init.set_pointer_type("mouse");
    init.set_client_x(at.0);
    init.set_client_y(at.1);
    target
        .dispatch_event(&web_sys::PointerEvent::new_with_event_init_dict(kind, &init).unwrap())
        .unwrap();
}

/// A bubbling `keydown` of `key` on `target`; `ctrl` stands for the platform modifier.
fn press(target: &web_sys::EventTarget, key: &str, ctrl: bool, shift: bool) {
    let init = web_sys::KeyboardEventInit::new();
    init.set_key(key);
    init.set_ctrl_key(ctrl);
    init.set_shift_key(shift);
    init.set_bubbles(true);
    init.set_cancelable(true);
    target
        .dispatch_event(
            &web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap(),
        )
        .unwrap();
}

fn body() -> web_sys::EventTarget {
    web_sys::window()
        .unwrap()
        .document()
        .unwrap()
        .body()
        .unwrap()
        .into()
}

/// A bubbling `click` on `selector` (it works on SVG paths too).
fn click_any(host: &web_sys::Element, selector: &str) {
    let init = web_sys::EventInit::new();
    init.set_bubbles(true);
    element(host, selector)
        .dispatch_event(&web_sys::Event::new_with_event_init_dict("click", &init).unwrap())
        .unwrap();
}

fn style(host: &web_sys::Element, selector: &str) -> String {
    element(host, selector)
        .get_attribute("style")
        .unwrap_or_default()
}

/// Mounts the reference flow (or `view`) and enters edit mode once the page has settled in view
/// mode, as it does in the browser.
async fn editing(view: FlowView) -> (web_sys::Element, Box<dyn std::any::Any>, House) {
    let host = host();
    let (mounted, house) = mount_house(&host, vec![view], "/flows/living_led_motion");
    gloo_timers::future::TimeoutFuture::new(50).await;
    click(&host, ".flow-toolbar__edit");
    tick().await;
    assert_eq!(count(&host, ".flow-graph--editing"), 1);
    (host, mounted, house)
}

/// Clicks the card (a pointerdown and a pointerup without moving).
async fn select_card(host: &web_sys::Element, id: &str) {
    let card = element(host, &format!("[data-node={id}]"));
    let at = centre(&card);
    pointer_at(&card, "pointerdown", at);
    pointer_at(&card, "pointerup", at);
    tick().await;
}

#[wasm_bindgen_test]
async fn click_card_opens_settings() {
    let (host, _mounted, _) = editing(reference_view(None, vec![])).await;
    select_card(&host, "moving").await;
    let html = host.inner_html();
    assert_eq!(
        count(&host, "[data-node=moving].flow-node--selected"),
        1,
        "{html}"
    );
    assert_eq!(count(&host, ".flow-drawer--open"), 1, "{html}");
    assert!(count(&host, ".flow-drawer .field") > 0, "{html}");
    // The drawer shows the settings instead of the runs, with a delete button.
    assert_eq!(count(&host, ".flow-drawer .run-list"), 0, "{html}");
    assert_eq!(
        count(&host, ".flow-drawer .node-editor__delete"),
        1,
        "{html}"
    );
    // A click on the empty canvas clears the selection.
    let canvas = element(&host, ".flow-canvas");
    pointer_at(&canvas, "pointerdown", (1, 1));
    pointer_at(&canvas, "pointerup", (1, 1));
    tick().await;
    let html = host.inner_html();
    assert_eq!(count(&host, ".flow-node--selected"), 0, "{html}");
    assert_eq!(count(&host, ".flow-drawer .node-editor"), 0, "{html}");
    assert_eq!(count(&host, ".flow-library"), 1, "{html}");
}

#[wasm_bindgen_test]
async fn view_mode_click_opens_settings() {
    let host = host();
    let _mounted = mount(
        &host,
        vec![reference_view(None, vec![done_run()])],
        "/flows/living_led_motion",
    );
    tick().await;
    // Keys do nothing outside edit mode.
    press(&body(), "Backspace", false, false);
    tick().await;
    assert_eq!(count(&host, "[data-node=moving]"), 1);
    select_card(&host, "moving").await;
    let html = host.inner_html();
    assert_eq!(count(&host, ".flow-graph--editing"), 1, "{html}");
    assert_eq!(
        count(&host, "[data-node=moving].flow-node--selected"),
        1,
        "{html}"
    );
    assert_eq!(count(&host, ".flow-drawer--open"), 1, "{html}");
    assert_eq!(
        element(&host, ".node-editor__id").text_content().as_deref(),
        Some("moving")
    );
}

#[wasm_bindgen_test]
async fn delete_key_removes_card_and_bridges_its_wires() {
    let (host, _mounted, _) = editing(reference_view(None, vec![])).await;
    assert_eq!(count(&host, ".flow-wire"), 4);
    select_card(&host, "wait").await;
    press(&body(), "Backspace", false, false);
    tick().await;
    let html = host.inner_html();
    assert_eq!(count(&host, "[data-node=wait]"), 0, "{html}");
    // `moving --no--> wait --> off` becomes `moving --no--> off`.
    assert_eq!(count(&host, ".flow-wire"), 3, "{html}");
    assert_eq!(count(&host, ".flow-wire--no"), 1, "{html}");
    assert_eq!(count(&host, ".flow-node--selected"), 0, "{html}");
}

#[wasm_bindgen_test]
async fn backspace_in_input_keeps_card() {
    let (host, _mounted, _) = editing(reference_view(None, vec![])).await;
    select_card(&host, "wait").await;
    let input = element(&host, ".flow-drawer input.field");
    input
        .clone()
        .unchecked_into::<web_sys::HtmlElement>()
        .focus()
        .unwrap();
    press(&input, "Backspace", false, false);
    press(&input, "Delete", false, false);
    tick().await;
    let html = host.inner_html();
    assert_eq!(count(&host, "[data-node=wait]"), 1, "{html}");
    assert_eq!(count(&host, ".flow-wire"), 4, "{html}");
}

#[wasm_bindgen_test]
async fn keys_in_an_open_picker_do_not_delete_the_card() {
    let (host, _mounted, _) = editing(reference_view(None, vec![])).await;
    select_card(&host, "on").await;
    click_any(&host, ".flow-drawer button.picker-field");
    tick().await;
    gloo_timers::future::TimeoutFuture::new(30).await;
    let chip = element(&host, ".picker__chips .chip");
    chip.clone()
        .unchecked_into::<web_sys::HtmlElement>()
        .focus()
        .unwrap();
    press(&chip, "Backspace", false, false);
    press(&chip, "z", true, false);
    tick().await;
    let html = host.inner_html();
    assert_eq!(count(&host, "[data-node=on]"), 1, "{html}");
    assert_eq!(count(&host, ".flow-wire"), 4, "{html}");
}

#[wasm_bindgen_test]
async fn wire_click_then_cut() {
    let (host, _mounted, _) = editing(reference_view(None, vec![])).await;
    assert_eq!(count(&host, ".flow-wire-hit"), 4);
    assert_eq!(count(&host, ".flow-wire-cut"), 0);
    click_any(&host, ".flow-wire-hit");
    tick().await;
    let html = host.inner_html();
    assert_eq!(count(&host, ".flow-wire--selected"), 1, "{html}");
    assert_eq!(count(&host, ".flow-wire-cut"), 1, "{html}");
    click_any(&host, ".flow-wire-cut");
    tick().await;
    let html = host.inner_html();
    assert_eq!(count(&host, ".flow-wire"), 3, "{html}");
    assert_eq!(count(&host, ".flow-wire-cut"), 0, "{html}");
}

/// Drags from `[data-node=from]`'s output pin and drops over `[data-node=to]`.
async fn drag_wire(host: &web_sys::Element, from: &str, to: &str) {
    let pin = element(host, &format!("[data-node={from}] .flow-node__out"));
    let start = centre(&pin);
    pointer_at(&pin, "pointerdown", start);
    let end = centre(&element(host, &format!("[data-node={to}]")));
    pointer_at(
        &pin,
        "pointermove",
        ((start.0 + end.0) / 2, (start.1 + end.1) / 2),
    );
    tick().await;
    pointer_at(&pin, "pointermove", end);
    pointer_at(&pin, "pointerup", end);
    tick().await;
}

#[wasm_bindgen_test]
async fn connect_by_drag() {
    let (host, _mounted, _) = editing(reference_view(None, vec![])).await;
    let pin = element(&host, "[data-node=on] .flow-node__out");
    let start = centre(&pin);
    pointer_at(&pin, "pointerdown", start);
    pointer_at(&pin, "pointermove", (start.0 + 30, start.1 + 30));
    tick().await;
    let html = host.inner_html();
    // A live preview, and only valid targets highlight.
    assert_eq!(count(&host, ".flow-wire-preview"), 1, "{html}");
    assert_eq!(
        count(&host, "[data-node=off].flow-node--target"),
        1,
        "{html}"
    );
    assert_eq!(
        count(&host, "[data-node=motion].flow-node--target"),
        0,
        "{html}"
    );
    assert_eq!(
        count(&host, "[data-node=on].flow-node--target"),
        0,
        "{html}"
    );
    // Released over no card (the test page has no stylesheet, so the pin's own spot on screen
    // is not where the graph thinks it is).
    pointer_at(&pin, "pointerup", (1, 1));
    tick().await;

    drag_wire(&host, "on", "off").await;
    let html = host.inner_html();
    assert_eq!(count(&host, ".flow-wire"), 5, "{html}");
    assert_eq!(count(&host, ".flow-wire-preview"), 0, "{html}");
    assert_eq!(count(&host, ".flow-node--target"), 0, "{html}");
}

#[wasm_bindgen_test]
async fn connect_into_trigger_refused() {
    let (host, _mounted, _) = editing(reference_view(None, vec![])).await;
    drag_wire(&host, "on", "motion").await;
    let html = host.inner_html();
    assert_eq!(count(&host, ".flow-wire"), 4, "{html}");
    // Nothing to undo either.
    assert!(
        element(&host, ".flow-toolbar button[aria-label=Undo]").has_attribute("disabled"),
        "{html}"
    );
}

#[wasm_bindgen_test]
async fn undo_restores() {
    let (host, _mounted, _) = editing(reference_view(None, vec![])).await;
    let undo = ".flow-toolbar button[aria-label=Undo]";
    assert!(element(&host, undo).has_attribute("disabled"));
    select_card(&host, "wait").await;
    press(&body(), "Delete", false, false);
    tick().await;
    assert_eq!(count(&host, "[data-node=wait]"), 0);
    assert!(!element(&host, undo).has_attribute("disabled"));
    press(&body(), "z", true, false);
    tick().await;
    let html = host.inner_html();
    assert_eq!(count(&host, "[data-node=wait]"), 1, "{html}");
    assert_eq!(count(&host, ".flow-wire"), 4, "{html}");
    assert!(element(&host, undo).has_attribute("disabled"), "{html}");
    // Shift-Mod-Z redoes.
    press(&body(), "Z", true, true);
    tick().await;
    assert_eq!(count(&host, "[data-node=wait]"), 0);
}

#[wasm_bindgen_test]
async fn duplicate_selects_the_copy() {
    let (host, _mounted, _) = editing(reference_view(None, vec![])).await;
    select_card(&host, "wait").await;
    press(&body(), "d", true, false);
    tick().await;
    let html = host.inner_html();
    assert_eq!(count(&host, ".flow-node"), 6, "{html}");
    assert_eq!(
        count(&host, "[data-node=delay_1].flow-node--selected"),
        1,
        "{html}"
    );
    // Unwired.
    assert_eq!(count(&host, ".flow-wire"), 4, "{html}");
}

#[wasm_bindgen_test]
async fn drag_is_one_undo_step() {
    let (host, _mounted, _) = editing(reference_view(None, vec![])).await;
    let selector = "[data-node=on]";
    let before = style(&host, selector);
    let card = element(&host, selector);
    let (x, y) = centre(&card);
    pointer_at(&card, "pointerdown", (x, y));
    for dx in [10, 20, 40] {
        pointer_at(&card, "pointermove", (x + dx, y + dx));
    }
    pointer_at(&card, "pointerup", (x + 40, y + 40));
    tick().await;
    let moved = style(&host, selector);
    assert_ne!(moved, before);
    press(&body(), "z", true, false);
    tick().await;
    let html = host.inner_html();
    assert_eq!(style(&host, selector), before, "{html}");
    assert!(
        element(&host, ".flow-toolbar button[aria-label=Undo]").has_attribute("disabled"),
        "{html}"
    );
}

/// A bubbling `kind` drag event at client point `at` on `target`.
fn drag_at(target: &web_sys::Element, kind: &str, at: (i32, i32)) {
    let init = web_sys::DragEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_client_x(at.0);
    init.set_client_y(at.1);
    target
        .dispatch_event(&web_sys::DragEvent::new_with_event_init_dict(kind, &init).unwrap())
        .unwrap();
}

/// `(x, y, zoom)` of a `transform:translate(Xpx,Ypx) scale(Z)` style.
fn transform(style: &str) -> (f64, f64, f64) {
    let nums: Vec<f64> = style
        .split(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
        .filter_map(|t| t.parse().ok())
        .collect();
    (nums[0], nums[1], nums[2])
}

/// `(left, top)` of a `left:Xpx;top:Ypx` style.
fn corner(style: &str) -> (f64, f64) {
    let px = |key: &str| {
        let rest = &style[style.find(key).unwrap() + key.len()..];
        rest[..rest.find("px").unwrap()].parse().unwrap()
    };
    (px("left:"), px("top:"))
}

#[wasm_bindgen_test]
async fn drop_adds_under_the_pointer() {
    let (host, _mounted, _) = editing(reference_view(None, vec![])).await;
    click(&host, ".flow-controls__btn[aria-label=\"Zoom in\"]");
    tick().await;
    let row = (0..count(&host, ".flow-library__item"))
        .filter_map(|i| {
            host.query_selector_all(".flow-library__item")
                .unwrap()
                .item(i)
        })
        .filter_map(|n| n.dyn_into::<web_sys::Element>().ok())
        .find(|el| el.text_content().unwrap_or_default().contains("Wait"))
        .unwrap();
    let canvas = element(&host, ".flow-canvas");
    let rect = canvas.get_bounding_client_rect();
    let at = (rect.left() as i32 + 200, rect.top() as i32 + 200);
    drag_at(&row, "dragstart", (0, 0));
    drag_at(&canvas, "dragover", at);
    drag_at(&canvas, "drop", at);
    drag_at(&row, "dragend", (0, 0));
    tick().await;
    let html = host.inner_html();
    assert_eq!(count(&host, "[data-node=delay_1]"), 1, "{html}");
    // The card's stored corner, through the camera (`screen = world * zoom + pan`), puts its
    // centre on the drop point. The test page has no stylesheet, so rects can't tell.
    let (tx, ty, zoom) = transform(&style(&host, ".flow-world"));
    let (left, top) = corner(&style(&host, "[data-node=delay_1]"));
    let cx = left * zoom + tx + 232.0 * zoom / 2.0;
    let cy = top * zoom + ty + 52.0 * zoom / 2.0;
    // The drop point within the canvas (the canvas' corner is fractional, the event's not).
    let (dx, dy) = (f64::from(at.0) - rect.left(), f64::from(at.1) - rect.top());
    assert!(
        (cx - dx).abs() < 0.01 && (cy - dy).abs() < 0.01,
        "card centred at ({cx}, {cy}) in the canvas, dropped at ({dx}, {dy})"
    );
    assert_eq!(count(&host, ".flow-library"), 1, "{html}");
}

#[wasm_bindgen_test]
async fn ws_update_keeps_draft() {
    let (host, _mounted, house) = editing(reference_view(None, vec![done_run()])).await;
    select_card(&host, "on").await;
    press(&body(), "Backspace", false, false);
    tick().await;
    assert_eq!(count(&host, "[data-node=on]"), 0);
    click(&host, ".flow-toolbar__runs");
    tick().await;
    assert_eq!(count(&host, ".flow-drawer--open"), 1);
    assert_eq!(count(&host, ".run-list > li"), 1);
    let mut newer = done_run();
    newer.id = "r2".into();
    house
        .flows
        .set(vec![reference_view(None, vec![newer, done_run()])]);
    tick().await;
    let html = host.inner_html();
    assert_eq!(count(&host, "[data-node=on]"), 0, "{html}");
    assert_eq!(count(&host, ".flow-graph--editing"), 1, "{html}");
    assert_eq!(count(&host, ".run-list > li"), 2, "{html}");
}

#[wasm_bindgen_test]
async fn tidy_is_undoable() {
    // Hand-placed positions that the automatic layout would not pick.
    let mut definition = Flow::decode(REFERENCE.as_bytes()).unwrap();
    for (i, node) in definition.nodes.iter_mut().enumerate() {
        node.position = Some(Position {
            x: 0.0,
            y: 300.0 * i as f64,
        });
    }
    let view = FlowView {
        definition,
        invalid: None,
        active_run: None,
        runs: vec![],
    };
    let (host, _mounted, _) = editing(view).await;
    let selector = "[data-node=off]";
    let before = style(&host, selector);
    click(&host, ".flow-toolbar button[aria-label=Tidy]");
    tick().await;
    assert_ne!(style(&host, selector), before);
    click(&host, ".flow-toolbar button[aria-label=Undo]");
    tick().await;
    assert_eq!(style(&host, selector), before, "{}", host.inner_html());
}

/// Sets `input`'s value and fires a bubbling `kind` event on it (`input` or `change`).
fn type_into(input: &web_sys::HtmlInputElement, value: &str, kind: &str) {
    input.set_value(value);
    let init = web_sys::EventInit::new();
    init.set_bubbles(true);
    input
        .dispatch_event(&web_sys::Event::new_with_event_init_dict(kind, &init).unwrap())
        .unwrap();
}

#[wasm_bindgen_test]
async fn field_edit_is_one_undo_step() {
    let (host, _mounted, _) = editing(reference_view(None, vec![])).await;
    select_card(&host, "wait").await;
    // The delay's fields: its name, then its duration.
    let duration: web_sys::HtmlInputElement = host
        .query_selector_all(".flow-drawer input.field")
        .unwrap()
        .item(1)
        .unwrap()
        .unchecked_into();
    duration.focus().unwrap();
    type_into(&duration, "1", "input");
    type_into(&duration, "10", "input");
    type_into(&duration, "10m", "input");
    type_into(&duration, "10m", "change");
    tick().await;
    assert_eq!(
        text(&host, "[data-node=wait] .flow-node__title"),
        "Wait 10\u{a0}min"
    );
    click(&host, ".flow-toolbar button[aria-label=Undo]");
    tick().await;
    let html = host.inner_html();
    assert_eq!(
        text(&host, "[data-node=wait] .flow-node__title"),
        "Wait 5\u{a0}min",
        "{html}"
    );
    assert!(
        element(&host, ".flow-toolbar button[aria-label=Undo]").has_attribute("disabled"),
        "{html}"
    );
    gloo_timers::future::TimeoutFuture::new(50).await;
    // The form shows the restored value.
    let duration: web_sys::HtmlInputElement = host
        .query_selector_all(".flow-drawer input.field")
        .unwrap()
        .item(1)
        .unwrap()
        .unchecked_into();
    assert_eq!(duration.value(), "5m");
}

// ---- Picker ---------------------------------------------------------------------------------

/// Opens the picker and types `query` into its search box.
async fn search(host: &web_sys::Element, query: &str) {
    let input: web_sys::HtmlInputElement = element(host, ".flow-library__search").unchecked_into();
    type_into(&input, query, "input");
    tick().await;
}

/// Clicks the library row titled `title`.
async fn add_row(host: &web_sys::Element, title: &str) {
    let rows = host.query_selector_all(".flow-library__item").unwrap();
    let row = (0..rows.length())
        .filter_map(|i| rows.item(i))
        .filter_map(|n| n.dyn_into::<web_sys::HtmlElement>().ok())
        .find(|el| {
            el.query_selector(".flow-library__title")
                .unwrap()
                .and_then(|t| t.text_content())
                .as_deref()
                == Some(title)
        })
        .unwrap_or_else(|| panic!("no library row {title}: {}", host.inner_html()));
    row.click();
    tick().await;
}

fn table_lamp() -> nidoo_api::EntityView {
    use nidoo_core::model::{
        Capabilities, EntityCategory, EntityState, Kind, StateValue, UiOverride,
    };
    nidoo_api::EntityView {
        id: "light.table".into(),
        device_id: None,
        name: "Lumière table".into(),
        device_name: None,
        label: None,
        kind: Kind::Light,
        protocol_kind: Kind::Light,
        component: Kind::Light.component().to_string(),
        device_class: None,
        unit: None,
        icon: None,
        category: EntityCategory::Primary,
        enabled_by_default: true,
        area: "salon".into(),
        part_of: None,
        capabilities: Capabilities::default(),
        ui: UiOverride::default(),
        state: EntityState {
            value: StateValue::OnOff(false),
            ..Default::default()
        },
    }
}

#[wasm_bindgen_test]
async fn library_open_in_edit_mode() {
    let host = host();
    let _mounted = mount(
        &host,
        vec![reference_view(None, vec![])],
        "/flows/living_led_motion",
    );
    tick().await;
    assert_eq!(count(&host, ".flow-drawer--open"), 0);
    let (host, _mounted, _) = editing(reference_view(None, vec![])).await;
    let html = host.inner_html();
    assert_eq!(count(&host, ".flow-drawer--open"), 1, "{html}");
    assert_eq!(count(&host, ".flow-library"), 1, "{html}");
    assert_eq!(count(&host, ".flow-library__item"), 15, "{html}");
    assert_eq!(count(&host, ".flow-picker-wrap"), 0, "{html}");
}

#[wasm_bindgen_test]
async fn library_click_adds_and_stays_open() {
    let (host, _mounted, _) = editing(reference_view(None, vec![])).await;
    add_row(&host, "Wait").await;
    add_row(&host, "Wait").await;
    let html = host.inner_html();
    assert_eq!(count(&host, "[data-node=delay_1]"), 1, "{html}");
    assert_eq!(count(&host, "[data-node=delay_2]"), 1, "{html}");
    assert_eq!(count(&host, ".flow-node--selected"), 0, "{html}");
    assert_eq!(count(&host, ".flow-library"), 1, "{html}");
    assert_ne!(
        style(&host, "[data-node=delay_1]"),
        style(&host, "[data-node=delay_2]"),
        "the second card does not sit on the first"
    );
    // Each add is one undo step.
    press(&body(), "z", true, false);
    tick().await;
    assert_eq!(count(&host, "[data-node=delay_2]"), 0);
    assert_eq!(count(&host, "[data-node=delay_1]"), 1);
}

#[wasm_bindgen_test]
async fn library_enter_adds_highlighted() {
    let (host, _mounted, _) = editing(reference_view(None, vec![])).await;
    search(&host, "Wait").await;
    press(
        &element(&host, ".flow-library__search"),
        "Enter",
        false,
        false,
    );
    tick().await;
    let html = host.inner_html();
    assert_eq!(count(&host, "[data-node=delay_1]"), 1, "{html}");
    assert_eq!(count(&host, ".flow-library"), 1, "{html}");
}

#[wasm_bindgen_test]
async fn undo_in_library_search_after_enter_undoes_the_add() {
    let (host, _mounted, _) = editing(reference_view(None, vec![])).await;
    search(&host, "Wait").await;
    let input = element(&host, ".flow-library__search");
    press(&input, "Enter", false, false);
    tick().await;
    assert_eq!(count(&host, "[data-node=delay_1]"), 1);
    // The search box keeps the focus, so Mod-Z lands there: it takes the card back out.
    press(&input, "z", true, false);
    tick().await;
    assert_eq!(
        count(&host, "[data-node=delay_1]"),
        0,
        "{}",
        host.inner_html()
    );
    press(&input, "z", true, true);
    tick().await;
    assert_eq!(
        count(&host, "[data-node=delay_1]"),
        1,
        "{}",
        host.inner_html()
    );
}

#[wasm_bindgen_test]
async fn undo_in_library_search_after_typing_leaves_the_flow() {
    let (host, _mounted, _) = editing(reference_view(None, vec![])).await;
    search(&host, "Wait").await;
    let input = element(&host, ".flow-library__search");
    press(&input, "Enter", false, false);
    tick().await;
    // Typing again hands Mod-Z back to the text.
    search(&host, "Wai").await;
    press(&input, "z", true, false);
    tick().await;
    assert_eq!(
        count(&host, "[data-node=delay_1]"),
        1,
        "{}",
        host.inner_html()
    );
}

#[wasm_bindgen_test]
async fn library_search_survives_a_trip_to_settings() {
    let (host, _mounted, _) = editing(reference_view(None, vec![])).await;
    search(&host, "Wait").await;
    let rows = count(&host, ".flow-library__item");
    select_card(&host, "moving").await;
    click(&host, ".node-editor__header .btn");
    // The new search box picks its value up from an effect.
    gloo_timers::future::TimeoutFuture::new(50).await;
    let input: web_sys::HtmlInputElement = element(&host, ".flow-library__search").unchecked_into();
    assert_eq!(input.value(), "Wait", "{}", host.inner_html());
    assert_eq!(count(&host, ".flow-library__item"), rows);
}

#[wasm_bindgen_test]
async fn leaving_edit_mode_clears_the_library_search() {
    let (host, _mounted, _) = editing(reference_view(None, vec![])).await;
    search(&host, "Wait").await;
    click(&host, ".flow-toolbar button[aria-label=\"Cancel editing\"]");
    tick().await;
    click(&host, ".flow-toolbar__edit");
    gloo_timers::future::TimeoutFuture::new(50).await;
    let input: web_sys::HtmlInputElement = element(&host, ".flow-library__search").unchecked_into();
    assert_eq!(input.value(), "", "{}", host.inner_html());
}

#[wasm_bindgen_test]
async fn leaving_edit_mode_slides_the_library_out() {
    let (host, _mounted, _) = editing(reference_view(None, vec![])).await;
    assert_eq!(count(&host, ".flow-library"), 1);
    click(&host, ".flow-toolbar button[aria-label=\"Cancel editing\"]");
    tick().await;
    // The closing drawer keeps the library; the runs do not flash in for the slide.
    let html = host.inner_html();
    assert_eq!(count(&host, ".flow-drawer--open"), 0, "{html}");
    assert_eq!(count(&host, ".flow-drawer .flow-library"), 1, "{html}");
    assert_eq!(count(&host, ".flow-drawer__close"), 0, "{html}");
}

#[wasm_bindgen_test]
async fn selecting_a_card_swaps_to_settings_and_back() {
    let (host, _mounted, _) = editing(reference_view(None, vec![])).await;
    select_card(&host, "moving").await;
    let html = host.inner_html();
    assert_eq!(count(&host, ".flow-drawer .node-editor"), 1, "{html}");
    assert_eq!(count(&host, ".flow-library"), 0, "{html}");
    click(&host, ".node-editor__header .btn");
    tick().await;
    assert_eq!(count(&host, ".flow-library"), 1, "{}", host.inner_html());
}

#[wasm_bindgen_test]
async fn edit_mode_refits_beside_the_library() {
    let host = host();
    let _mounted = mount(
        &host,
        vec![reference_view(None, vec![])],
        "/flows/living_led_motion",
    );
    gloo_timers::future::TimeoutFuture::new(50).await;
    let before = style(&host, ".flow-world");
    click(&host, ".flow-toolbar__edit");
    // The panel opens, then the camera refits: two effects.
    gloo_timers::future::TimeoutFuture::new(20).await;
    let after = style(&host, ".flow-world");
    assert_ne!(
        before, after,
        "the untouched view refits for the open panel"
    );
    // Fit frames the graph in what the panel leaves free.
    click(&host, ".flow-controls__btn[aria-label=\"Fit view\"]");
    tick().await;
    assert_eq!(style(&host, ".flow-world"), after);
}

#[wasm_bindgen_test]
async fn runs_button_shows_runs_over_settings() {
    let (host, _mounted, _) = editing(reference_view(None, vec![done_run()])).await;
    click(&host, ".flow-toolbar__runs");
    tick().await;
    select_card(&host, "moving").await;
    assert_eq!(count(&host, ".flow-drawer .node-editor"), 1);
    assert_eq!(
        element(&host, ".flow-toolbar__runs").get_attribute("aria-expanded"),
        Some("false".into()),
        "the button follows what the panel shows"
    );
    // One click brings the runs back.
    click(&host, ".flow-toolbar__runs");
    tick().await;
    let html = host.inner_html();
    assert_eq!(count(&host, ".flow-drawer .run-list"), 1, "{html}");
    assert_eq!(count(&host, ".flow-drawer .node-editor"), 0, "{html}");
}

#[wasm_bindgen_test]
async fn backspace_in_library_search_keeps_selection() {
    let (host, _mounted, _) = editing(reference_view(None, vec![])).await;
    // A selected wire leaves the library showing; ⌫ in its search edits the text only.
    click_any(&host, ".flow-wire-hit");
    tick().await;
    assert_eq!(count(&host, ".flow-wire--selected"), 1);
    let input: web_sys::HtmlElement = element(&host, ".flow-library__search").unchecked_into();
    input.focus().unwrap();
    press(&input, "Backspace", false, false);
    tick().await;
    assert_eq!(count(&host, ".flow-wire"), 4, "{}", host.inner_html());
}

#[wasm_bindgen_test]
async fn runs_from_edit_mode_returns_to_library() {
    let (host, _mounted, _) = editing(reference_view(None, vec![])).await;
    click(&host, ".flow-toolbar__runs");
    tick().await;
    let html = host.inner_html();
    assert!(
        count(
            &host,
            ".flow-drawer .run-list, .flow-drawer .flow-drawer__empty"
        ) >= 1,
        "{html}"
    );
    assert_eq!(count(&host, ".flow-library"), 0, "{html}");
    click(&host, ".flow-drawer__close");
    tick().await;
    assert_eq!(count(&host, ".flow-library"), 1, "{}", host.inner_html());
}

#[wasm_bindgen_test]
async fn picker_groups_entity_results() {
    let (host, _mounted, house) = editing(reference_view(None, vec![])).await;
    house
        .entities
        .update(|e| drop(e.insert("light.table".into(), table_lamp())));
    search(&host, "table").await;
    let headers: Vec<String> = (0..count(&host, ".flow-library__header"))
        .map(|i| {
            host.query_selector_all(".flow-library__header")
                .unwrap()
                .item(i)
                .unwrap()
                .text_content()
                .unwrap()
        })
        .collect();
    assert_eq!(
        headers,
        ["WHEN · TRIGGER", "AND · CHECK", "THEN · ACTION"],
        "{}",
        host.inner_html()
    );
}

// ---- New flow and renaming ------------------------------------------------------------------

#[wasm_bindgen_test]
async fn new_flow_starts_in_edit_mode() {
    let host = host();
    let _mounted = mount(&host, vec![], "/flows/new");
    gloo_timers::future::TimeoutFuture::new(50).await;
    let name: web_sys::HtmlInputElement =
        element(&host, ".flow-toolbar input[aria-label=\"Flow name\"]").unchecked_into();
    assert_eq!(name.value(), "New flow");
    assert_eq!(count(&host, ".flow-node"), 0);
    assert_eq!(count(&host, ".flow-switch"), 0);
    assert_eq!(
        count(&host, ".flow-graph--editing"),
        1,
        "{}",
        host.inner_html()
    );
}

#[wasm_bindgen_test]
async fn rename_updates_draft_name() {
    let (host, _mounted, _) = editing(reference_view(None, vec![])).await;
    gloo_timers::future::TimeoutFuture::new(50).await;
    let name: web_sys::HtmlInputElement =
        element(&host, ".flow-toolbar input[aria-label=\"Flow name\"]").unchecked_into();
    assert_eq!(name.value(), "Living LED on motion");
    name.focus().unwrap();
    type_into(&name, "Hall light", "input");
    tick().await;
    assert_eq!(name.value(), "Hall light");
    // Saving sends the new name under the original id.
    let fetch = FakeFetch::answer(&write_outcome(&reference_view(None, vec![])));
    click(&host, ".flow-toolbar button[aria-label=Save]");
    gloo_timers::future::TimeoutFuture::new(50).await;
    let requests = puts(&fetch);
    assert_eq!(requests.len(), 1, "{requests:?}");
    assert_eq!(requests[0].1, "/api/v1/flows/living_led_motion");
    let sent: serde_json::Value = serde_json::from_str(&requests[0].2).unwrap();
    assert_eq!(sent["id"], "living_led_motion");
    assert_eq!(sent["name"], "Hall light");
    drop(fetch);
    // Back in view mode the header shows what the hub holds.
    assert_eq!(
        text(&host, ".flow-toolbar h1.flow-toolbar__name"),
        "Living LED on motion"
    );
}

/// Saves a new flow called "Nouveau flow" with `existing` already in the house and the hub
/// answering with `answer_id`; returns the PUT it made and the path the app ended on.
async fn save_new_flow(
    existing: Vec<FlowView>,
    answer_id: &str,
) -> (Vec<(String, String, String)>, String, String) {
    let host = host();
    let _mounted = mount(&host, existing, "/flows/new");
    gloo_timers::future::TimeoutFuture::new(50).await;
    let saved = FlowView {
        definition: Flow {
            id: answer_id.into(),
            name: "Nouveau flow".into(),
            ..Default::default()
        },
        invalid: None,
        active_run: None,
        runs: vec![],
    };
    let name: web_sys::HtmlInputElement =
        element(&host, ".flow-toolbar input[aria-label=\"Flow name\"]").unchecked_into();
    type_into(&name, "Nouveau flow", "input");
    let fetch = FakeFetch::answer(&write_outcome(&saved));
    click(&host, ".flow-toolbar button[aria-label=Save]");
    gloo_timers::future::TimeoutFuture::new(100).await;
    let path = web_sys::window().unwrap().location().pathname().unwrap();
    assert_eq!(count(&host, ".flow-graph--editing"), 0);
    (puts(&fetch), path, host.inner_html())
}

/// The id a new flow's first Save sent, after checking it is a v4 UUID and the PUT's URL.
fn sent_uuid(requests: &[(String, String, String)]) -> String {
    assert_eq!(requests.len(), 1, "{requests:?}");
    let (method, url, body) = &requests[0];
    assert_eq!(method, "PUT");
    let sent: serde_json::Value = serde_json::from_str(body).unwrap();
    let id = sent["id"].as_str().unwrap().to_owned();
    assert_eq!(id.len(), 36, "{id}");
    assert_eq!(id.as_bytes()[14], b'4', "{id}");
    assert!(nidoo_core::flow::is_valid_flow_id(&id), "{id}");
    assert!(!id.contains("nouveau"), "{id}");
    assert_eq!(url, &format!("/api/v1/flows/{id}"));
    assert_eq!(sent["enabled"], false);
    id
}

#[wasm_bindgen_test]
async fn saving_a_new_flow_gives_it_a_uuid_and_opens_it() {
    let answer = "0b6f3c1e-2d4a-4f8b-9c7d-5e1a2b3c4d5e";
    let (requests, path, html) = save_new_flow(vec![], answer).await;
    sent_uuid(&requests);
    assert_eq!(path, format!("/flows/{answer}"), "{html}");
}

#[wasm_bindgen_test]
async fn new_flows_with_the_same_name_get_different_ids() {
    let answer = "0b6f3c1e-2d4a-4f8b-9c7d-5e1a2b3c4d5e";
    let (first, _, _) = save_new_flow(vec![], answer).await;
    let (second, _, _) = save_new_flow(vec![], answer).await;
    assert_ne!(sent_uuid(&first), sent_uuid(&second));
}

#[wasm_bindgen_test]
async fn cancel_on_an_unsaved_flow_goes_to_the_list() {
    let host = host();
    let _mounted = mount(&host, vec![], "/flows/new");
    gloo_timers::future::TimeoutFuture::new(50).await;
    click(&host, ".flow-toolbar button[aria-label=\"Cancel editing\"]");
    gloo_timers::future::TimeoutFuture::new(50).await;
    let path = web_sys::window().unwrap().location().pathname().unwrap();
    assert_eq!(path, "/flows");
}

#[wasm_bindgen_test]
async fn signal_trigger_editor_lists_signals() {
    use nidoo_core::model::{Capabilities, EntityCategory, EntityState, Kind, UiOverride};
    const SIGNAL_FLOW: &str = r#"{
      "id": "remote_flow", "name": "Remote", "enabled": true, "mode": "single",
      "nodes": [
        { "id": "press", "type": "trigger.signal",
          "params": { "entity": "remote_1", "signal": "on" } }
      ],
      "edges": []
    }"#;
    let view = FlowView {
        definition: Flow::decode(SIGNAL_FLOW.as_bytes()).unwrap(),
        invalid: None,
        active_run: None,
        runs: vec![],
    };
    let host = host();
    let (_mounted, house) = mount_house(&host, vec![view], "/flows/remote_flow");
    house.entities.update(|m| {
        m.insert(
            "remote_1".into(),
            nidoo_api::EntityView {
                id: "remote_1".into(),
                device_id: None,
                name: "Remote".into(),
                device_name: None,
                label: None,
                kind: Kind::Generic,
                component: "device_automation".into(),
                protocol_kind: Kind::Generic,
                device_class: None,
                unit: None,
                icon: None,
                category: EntityCategory::Primary,
                enabled_by_default: true,
                area: "salon".into(),
                part_of: None,
                capabilities: Capabilities {
                    generic: Some(nidoo_core::types::GenericCaps {
                        signals: vec!["on".into(), "off".into(), "brightness_move_up".into()],
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                ui: UiOverride::default(),
                state: EntityState::default(),
            },
        );
    });
    gloo_timers::future::TimeoutFuture::new(50).await;
    click(&host, ".flow-toolbar__edit");
    tick().await;
    select_card(&host, "press").await;
    let chips = text(&host, ".signal-chips");
    for name in ["any", "on", "off", "brightness_move_up"] {
        assert!(chips.contains(name), "{chips}");
    }
    assert_eq!(count(&host, ".signal-chips .chip"), 4);
    assert_eq!(count(&host, ".signal-chips .chip--selected"), 1);
}

/// Opens the entity picker of the open card's settings.
async fn open_entity_picker(host: &web_sys::Element) {
    let button: web_sys::HtmlElement = host
        .query_selector(".flow-drawer button.picker-field")
        .unwrap()
        .unwrap_or_else(|| panic!("no picker field: {}", host.inner_html()))
        .unchecked_into();
    button.click();
    tick().await;
}

/// Closing the picker pops the history entry it pushed; let that land before the next test
/// mounts at its own path.
async fn settle_history() {
    gloo_timers::future::TimeoutFuture::new(100).await;
}

#[wasm_bindgen_test]
async fn command_entity_picked_from_picker() {
    let (host, _mounted, house) = editing(reference_view(None, vec![])).await;
    let lamp = table_lamp();
    house.entities.update(|m| {
        m.insert(lamp.id.clone(), lamp);
    });
    select_card(&host, "on").await;
    assert_eq!(
        count(&host, ".flow-drawer input[placeholder='entity.id']"),
        0
    );
    open_entity_picker(&host).await;
    let row: web_sys::HtmlElement = host
        .query_selector("[id$='-opt-light.table']")
        .unwrap()
        .unwrap_or_else(|| panic!("no light row: {}", host.inner_html()))
        .unchecked_into();
    row.click();
    tick().await;
    let field = text(&host, ".flow-drawer button.picker-field");
    assert!(field.contains("Lumière table"), "{field}");
    settle_history().await;
}

#[wasm_bindgen_test]
async fn signal_picker_lists_only_signals() {
    use nidoo_core::model::Kind;
    const SIGNAL_FLOW: &str = r#"{
      "id": "remote_flow", "name": "Remote", "enabled": true, "mode": "single",
      "nodes": [ { "id": "press", "type": "trigger.signal", "params": { "entity": "remote_1" } } ],
      "edges": []
    }"#;
    let view = FlowView {
        definition: Flow::decode(SIGNAL_FLOW.as_bytes()).unwrap(),
        invalid: None,
        active_run: None,
        runs: vec![],
    };
    let host = host();
    let (_mounted, house) = mount_house(&host, vec![view], "/flows/remote_flow");
    let mut remote = table_lamp();
    remote.id = "remote_1".into();
    remote.name = "Remote".into();
    remote.kind = Kind::Generic;
    remote.protocol_kind = Kind::Generic;
    remote.component = "device_automation".into();
    let mut sensor = table_lamp();
    sensor.id = "sensor.temp".into();
    sensor.name = "Temperature".into();
    sensor.kind = Kind::Sensor;
    sensor.protocol_kind = Kind::Sensor;
    sensor.component = "sensor".into();
    house.entities.update(|m| {
        m.insert(remote.id.clone(), remote);
        m.insert(sensor.id.clone(), sensor);
    });
    gloo_timers::future::TimeoutFuture::new(50).await;
    click(&host, ".flow-toolbar__edit");
    tick().await;
    select_card(&host, "press").await;
    open_entity_picker(&host).await;
    let html = host.inner_html();
    assert_eq!(count(&host, "[id$='-opt-remote_1']"), 1, "{html}");
    assert_eq!(count(&host, "[id$='-opt-sensor.temp']"), 0, "{html}");
    click(&host, "[id$='-opt-remote_1']");
    tick().await;
    settle_history().await;
}

/// Stands in for the shell chrome the canvas sits beside.
fn fake_chrome(sidebar_position: &str) -> web_sys::Element {
    let document = web_sys::window().unwrap().document().unwrap();
    while let Some(stale) = document.query_selector(".sidebar").unwrap() {
        stale.parent_element().unwrap().remove();
    }
    let chrome = document.create_element("div").unwrap();
    chrome.set_inner_html(&format!(
        r#"<aside class="sidebar" style="position:{sidebar_position};left:0;top:0;width:210px;height:300px"></aside>
           <header class="content-header" style="position:fixed;left:210px;top:0;height:46px;width:300px"></header>"#
    ));
    document.body().unwrap().append_child(&chrome).unwrap();
    chrome
}

fn flow_style(host: &web_sys::Element) -> String {
    host.query_selector(".flow-canvas-screen")
        .unwrap()
        .unwrap()
        .get_attribute("style")
        .unwrap_or_default()
}

#[wasm_bindgen_test]
async fn canvas_inset_clears_the_sidebar() {
    let chrome = fake_chrome("sticky");
    let host = host();
    let _mounted = mount(
        &host,
        vec![reference_view(None, vec![])],
        "/flows/living_led_motion",
    );
    tick().await;
    let style = flow_style(&host);
    assert!(style.contains("--flow-top:46px"), "{style}");
    let right = chrome
        .query_selector(".sidebar")
        .unwrap()
        .unwrap()
        .get_bounding_client_rect()
        .right();
    assert!(right >= 210.0);
    assert!(style.contains(&format!("--flow-left:{right}px")), "{style}");
    chrome.remove();
}

#[wasm_bindgen_test]
async fn canvas_ignores_an_off_canvas_sidebar() {
    let chrome = fake_chrome("fixed");
    let host = host();
    let _mounted = mount(
        &host,
        vec![reference_view(None, vec![])],
        "/flows/living_led_motion",
    );
    tick().await;
    let style = flow_style(&host);
    assert!(style.contains("--flow-left:0px"), "{style}");
    chrome.remove();
}

#[wasm_bindgen_test]
async fn canvas_resets_the_scroll_before_measuring() {
    let chrome = fake_chrome("sticky");
    // The header scrolls with the page, as the real one does.
    chrome
        .query_selector(".content-header")
        .unwrap()
        .unwrap()
        .set_attribute(
            "style",
            "position:absolute;left:210px;top:0;height:46px;width:300px",
        )
        .unwrap();
    let document = web_sys::window().unwrap().document().unwrap();
    let spacer = document.create_element("div").unwrap();
    spacer.set_attribute("style", "height:5000px").unwrap();
    document.body().unwrap().append_child(&spacer).unwrap();
    let window = web_sys::window().unwrap();
    window.scroll_to_with_x_and_y(0.0, 1200.0);
    assert!(window.scroll_y().unwrap() > 0.0);
    let host = host();
    let _mounted = mount(
        &host,
        vec![reference_view(None, vec![])],
        "/flows/living_led_motion",
    );
    tick().await;
    assert_eq!(window.scroll_y().unwrap(), 0.0);
    let style = flow_style(&host);
    assert!(style.contains("--flow-top:46px"), "{style}");
    spacer.remove();
    chrome.remove();
}

// ---- French --------------------------------------------------------------------------------

fn headers(host: &web_sys::Element) -> Vec<String> {
    (0..count(host, ".flow-library__header"))
        .map(|i| {
            host.query_selector_all(".flow-library__header")
                .unwrap()
                .item(i)
                .unwrap()
                .text_content()
                .unwrap()
        })
        .collect()
}

/// The canvas, its toolbar and the card library read French, then English after the toggle,
/// on the same mounted screen.
#[wasm_bindgen_test]
async fn the_canvas_follows_the_language_without_a_remount() {
    let lang = RwSignal::new(Lang::Fr);
    let host = host();
    let (_mounted, house) = mount_house_lang(
        &host,
        vec![reference_view(None, vec![done_run()])],
        "/flows/living_led_motion",
        lang,
    );
    gloo_timers::future::TimeoutFuture::new(50).await;
    house
        .entities
        .update(|e| drop(e.insert("light.table".into(), table_lamp())));
    click(&host, ".flow-toolbar__edit");
    tick().await;
    search(&host, "table").await;
    let text = || host.text_content().unwrap_or_default();

    assert_eq!(
        headers(&host),
        ["Déclencheurs", "Conditions", "Actions"],
        "{}",
        host.inner_html()
    );
    assert!(text().contains("Allumer"), "{}", text());
    for english in ["When", "WHEN", "Turn on"] {
        assert!(!text().contains(english), "{english}: {}", text());
    }
    let back = element(&host, ".flow-toolbar__back");
    assert_eq!(back.get_attribute("aria-label").as_deref(), Some("Retour"));
    let html = host.inner_html();
    assert!(html.contains("aria-label=\"Ajouter une carte\""), "{html}");
    assert!(html.contains("aria-label=\"Zoom avant\""), "{html}");
    assert!(html.contains("aria-label=\"Enregistrer\""), "{html}");

    lang.set(Lang::En);
    tick().await;
    assert_eq!(
        headers(&host),
        ["WHEN · TRIGGER", "AND · CHECK", "THEN · ACTION"],
        "{}",
        host.inner_html()
    );
    assert!(text().contains("Turn on"), "{}", text());
    assert!(!text().contains("Allumer"), "{}", text());
    let back = element(&host, ".flow-toolbar__back");
    assert_eq!(back.get_attribute("aria-label").as_deref(), Some("Back"));
    let html = host.inner_html();
    assert!(html.contains("aria-label=\"Add a card\""), "{html}");
    assert!(html.contains("aria-label=\"Zoom in\""), "{html}");
}

/// Run times read "il y a …" with the exact time as the tooltip, and flip with the toggle.
#[wasm_bindgen_test]
async fn the_runs_drawer_follows_the_language_without_a_remount() {
    let lang = RwSignal::new(Lang::Fr);
    let host = host();
    let (_mounted, _house) = mount_house_lang(
        &host,
        vec![reference_view(None, vec![done_run()])],
        "/flows/living_led_motion",
        lang,
    );
    gloo_timers::future::TimeoutFuture::new(50).await;
    click(&host, ".flow-toolbar__runs");
    tick().await;
    let text = || text(&host, ".flow-drawer");
    assert!(text().contains("Lancements récents"), "{}", text());
    assert!(text().contains("Terminé"), "{}", text());
    assert!(text().contains("il y a"), "{}", text());
    assert!(!text().contains("ago"), "{}", text());
    let title = element(&host, ".run__time").get_attribute("title").unwrap();
    assert!(title.contains("nov."), "{title}");

    lang.set(Lang::En);
    tick().await;
    assert!(text().contains("Recent runs"), "{}", text());
    assert!(text().contains("Done"), "{}", text());
    assert!(text().contains("ago"), "{}", text());
    assert!(!text().contains("il y a"), "{}", text());
    let title = element(&host, ".run__time").get_attribute("title").unwrap();
    assert!(title.contains("Nov"), "{title}");
}

/// A flow created on the French UI gets a French default name, which the toggle leaves alone.
#[wasm_bindgen_test]
async fn a_new_flow_is_named_in_the_language_active_at_creation() {
    let lang = RwSignal::new(Lang::Fr);
    let host = host();
    let _mounted = mount_house_lang(&host, vec![], "/flows/new", lang);
    gloo_timers::future::TimeoutFuture::new(50).await;
    let name = || {
        element(&host, ".flow-toolbar input.flow-toolbar__name--input")
            .unchecked_into::<web_sys::HtmlInputElement>()
            .value()
    };
    assert_eq!(name(), "Nouvelle automatisation");
    lang.set(Lang::En);
    tick().await;
    assert_eq!(name(), "Nouvelle automatisation");
}

#[wasm_bindgen_test]
async fn the_mode_select_saves_the_other_mode() {
    let view = reference_view(None, vec![]);
    let (want, label) = match view.definition.mode {
        FlowMode::Single => ("restart", "single"),
        FlowMode::Restart => ("single", "restart"),
    };
    let (host, _mounted, _) = editing(view.clone()).await;
    let select: web_sys::HtmlSelectElement =
        element(&host, ".flow-toolbar select[aria-label=Mode]").unchecked_into();
    assert_eq!(select.value(), label);
    select.set_value(want);
    let init = web_sys::EventInit::new();
    init.set_bubbles(true);
    select
        .dispatch_event(&web_sys::Event::new_with_event_init_dict("change", &init).unwrap())
        .unwrap();
    tick().await;
    let fetch = FakeFetch::answer(&write_outcome(&view));
    click(&host, ".flow-toolbar button[aria-label=Save]");
    gloo_timers::future::TimeoutFuture::new(50).await;
    let requests = puts(&fetch);
    assert_eq!(requests.len(), 1, "{requests:?}");
    let sent: serde_json::Value = serde_json::from_str(&requests[0].2).unwrap();
    assert_eq!(sent["mode"], want);
}

/// An `if` with no value yet shows an empty entry, not the list's first value it never stored.
#[wasm_bindgen_test]
async fn if_value_without_a_value_shows_no_choice() {
    use nidoo_core::model::Kind;
    const UNSET: &str = r#"{
      "id": "living_led_motion", "name": "Unset", "enabled": true, "mode": "single",
      "nodes": [
        { "id": "shutter", "type": "trigger.state", "params": { "entity": "cover.living" } },
        { "id": "check", "type": "if", "params": { "op": "eq" } }
      ],
      "edges": [ { "from": "shutter", "to": "check" } ]
    }"#;
    let view = FlowView {
        definition: Flow::decode(UNSET.as_bytes()).unwrap(),
        invalid: None,
        active_run: None,
        runs: vec![],
    };
    let (host, _mounted, house) = editing(view).await;
    let mut cover = table_lamp();
    cover.id = "cover.living".into();
    cover.kind = Kind::Cover;
    cover.protocol_kind = Kind::Cover;
    cover.component = "cover".into();
    house.entities.update(|m| {
        m.insert(cover.id.clone(), cover);
    });
    select_card(&host, "check").await;
    let select: web_sys::HtmlSelectElement = host
        .query_selector(".flow-drawer select:has(option[value='other'])")
        .unwrap()
        .unwrap_or_else(|| panic!("no value list: {}", host.inner_html()))
        .unchecked_into();
    assert_eq!(select.value(), "", "{}", host.inner_html());
    settle_history().await;
}

// ---- Manual card ----------------------------------------------------------------------------

/// The reference flow with a manual card `go` wired into `moving`.
fn manual_view() -> FlowView {
    let mut view = reference_view(None, vec![]);
    let go: nidoo_core::flow::Node =
        serde_json::from_str(r#"{"id":"go","type":"trigger.manual","params":{"value":true}}"#)
            .unwrap();
    view.definition.nodes.push(go);
    view.definition.edges.push(nidoo_core::flow::Edge {
        from: "go".into(),
        port: None,
        to: "moving".into(),
    });
    view
}

fn runs(fetch: &FakeFetch) -> Vec<(String, String, String)> {
    fetch
        .requests()
        .into_iter()
        .filter(|(method, path, _)| method == "POST" && path.ends_with("/run"))
        .collect()
}

#[wasm_bindgen_test]
async fn manual_card_shows_run_button() {
    let host = host();
    let _mounted = mount(&host, vec![manual_view()], "/flows/living_led_motion");
    tick().await;
    let html = host.inner_html();
    assert_eq!(count(&host, ".flow-node__run"), 1, "{html}");
    assert_eq!(count(&host, "[data-node=go] .flow-node__run"), 1, "{html}");
    assert!(
        !element(&host, ".flow-node__run")
            .unchecked_into::<web_sys::HtmlButtonElement>()
            .disabled()
    );
}

#[wasm_bindgen_test]
async fn manual_button_posts_the_card() {
    let host = host();
    let _mounted = mount(&host, vec![manual_view()], "/flows/living_led_motion");
    tick().await;
    let fetch = FakeFetch::answer(&serde_json::to_string(&done_run()).unwrap());
    let button = element(&host, ".flow-node__run");
    let at = centre(&button);
    pointer_at(&button, "pointerdown", at);
    pointer_at(&button, "pointerup", at);
    click(&host, ".flow-node__run");
    gloo_timers::future::TimeoutFuture::new(50).await;
    let sent = runs(&fetch);
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!(sent[0].1, "/api/v1/flows/living_led_motion/run");
    let body: serde_json::Value = serde_json::from_str(&sent[0].2).unwrap();
    assert_eq!(body, serde_json::json!({ "node": "go" }));
    let html = host.inner_html();
    assert_eq!(count(&host, ".flow-node--selected"), 0, "{html}");
    assert_eq!(count(&host, ".flow-graph--editing"), 0, "{html}");
}

#[wasm_bindgen_test]
async fn manual_button_is_inert_in_edit_mode() {
    let (host, _mounted, _) = editing(manual_view()).await;
    let button = element(&host, ".flow-node__run").unchecked_into::<web_sys::HtmlButtonElement>();
    assert!(button.disabled());
    let fetch = FakeFetch::answer(&serde_json::to_string(&done_run()).unwrap());
    button.click();
    gloo_timers::future::TimeoutFuture::new(50).await;
    assert!(runs(&fetch).is_empty());
}

/// The manual card's number box keeps a decimal or negative number being typed. Mid-typing ("-",
/// "2.") a number box reports "" (bad input); the editor must not write that back over the text.
#[wasm_bindgen_test]
async fn manual_number_box_keeps_what_is_typed() {
    let mut view = manual_view();
    let go: nidoo_core::flow::Node =
        serde_json::from_str(r#"{"id":"go","type":"trigger.manual","params":{"value":1}}"#)
            .unwrap();
    view.definition.nodes.retain(|n| n.id != "go");
    view.definition.nodes.push(go);
    let saved = view.clone();
    let (host, _mounted, _) = editing(view).await;
    select_card(&host, "go").await;
    let input: web_sys::HtmlInputElement =
        element(&host, ".flow-drawer input[type=number]").unchecked_into();
    input.focus().unwrap();
    for step in ["-", "-1", "-1.", "-1.5"] {
        // `set_value` sanitises a half-typed number to "", as the browser reports it.
        type_into(&input, step, "input");
        gloo_timers::future::TimeoutFuture::new(10).await;
    }
    assert_eq!(input.value(), "-1.5");
    // Chrome reports "" while "-1.5" is half-typed; record any write the editor makes back.
    let spy = js_sys::Function::new_with_args(
        "el",
        "el.__writes = []; Object.defineProperty(el, 'value', { configurable: true, \
         get() { return ''; }, set(v) { el.__writes.push(v); } });",
    );
    spy.call1(&wasm_bindgen::JsValue::NULL, &input).unwrap();
    let init = web_sys::EventInit::new();
    init.set_bubbles(true);
    input
        .dispatch_event(&web_sys::Event::new_with_event_init_dict("input", &init).unwrap())
        .unwrap();
    gloo_timers::future::TimeoutFuture::new(10).await;
    let writes = js_sys::Reflect::get(&input, &"__writes".into()).unwrap();
    assert_eq!(
        js_sys::Array::from(&writes).length(),
        0,
        "the editor rewrote the box: {:?}",
        js_sys::JSON::stringify(&writes)
    );
    js_sys::Reflect::delete_property(&input, &"value".into()).unwrap();
    type_into(&input, "-1.5", "input");
    let fetch = FakeFetch::answer(&write_outcome(&saved));
    click(&host, ".flow-toolbar button[aria-label=Save]");
    gloo_timers::future::TimeoutFuture::new(50).await;
    let sent = puts(&fetch);
    assert_eq!(sent.len(), 1, "{sent:?}");
    let body: serde_json::Value = serde_json::from_str(&sent[0].2).unwrap();
    let go = body["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"] == "go")
        .unwrap_or_else(|| panic!("{body}"));
    assert_eq!(go["params"]["value"], serde_json::json!(-1.5), "{body}");
}

// ---- Math card ------------------------------------------------------------------------------

/// A power sensor reading 21, for the math card's preview.
fn power_sensor() -> nidoo_api::EntityView {
    use nidoo_core::model::{EntityState, Kind, StateValue};
    nidoo_api::EntityView {
        id: "sensor.power".into(),
        name: "Power meter".into(),
        kind: Kind::Sensor,
        protocol_kind: Kind::Sensor,
        component: Kind::Sensor.component().to_string(),
        unit: Some("W".into()),
        state: EntityState {
            value: StateValue::Number(21.0),
            ..Default::default()
        },
        ..table_lamp()
    }
}

/// Adds a Math card from the library and opens its drawer.
async fn open_math_drawer(host: &web_sys::Element) {
    add_row(host, "Math").await;
    select_card(host, "math_1").await;
    // Field values are set by effects.
    gloo_timers::future::TimeoutFuture::new(50).await;
}

fn input_named(host: &web_sys::Element, selector: &str) -> web_sys::HtmlInputElement {
    element(host, selector).unchecked_into()
}

#[wasm_bindgen_test]
async fn math_card_from_library_opens_its_drawer() {
    let (host, _mounted, _) = editing(reference_view(None, vec![])).await;
    open_math_drawer(&host).await;
    let drawer = text(&host, ".flow-drawer");
    for word in ["Inputs", "Formula", "Result name", "Save to"] {
        assert!(drawer.contains(word), "{word}: {drawer}");
    }
    // The card is named with the first free result name.
    assert_eq!(input_named(&host, ".math-name").value(), "result");
    assert_eq!(count(&host, ".math-input"), 0);
}

#[wasm_bindgen_test]
async fn math_formula_error_shows_and_blocks_save() {
    let (host, _mounted, _) = editing(reference_view(None, vec![])).await;
    open_math_drawer(&host).await;
    let formula = input_named(&host, ".math-formula");
    type_into(&formula, "gird + 1", "input");
    tick().await;
    let html = host.inner_html();
    assert_eq!(
        text(&host, ".math-line.node-editor__error"),
        "unknown name `gird`",
        "{html}"
    );
    let save = element(&host, ".flow-toolbar button[aria-label=Save]");
    assert!(save.has_attribute("disabled"), "{html}");
    let fetch = FakeFetch::answer(&write_outcome(&reference_view(None, vec![])));
    click(&host, ".flow-toolbar button[aria-label=Save]");
    gloo_timers::future::TimeoutFuture::new(50).await;
    assert!(puts(&fetch).is_empty(), "{:?}", puts(&fetch));
    // A formula that reads fine lifts the block.
    type_into(&formula, "1 + 1", "input");
    tick().await;
    assert!(
        !element(&host, ".flow-toolbar button[aria-label=Save]").has_attribute("disabled"),
        "{}",
        host.inner_html()
    );
    assert_eq!(text(&host, ".math-line__value"), "= 2");
}

#[wasm_bindgen_test]
async fn math_runtime_failure_shows_but_keeps_save_enabled() {
    let (host, _mounted, _) = editing(reference_view(None, vec![])).await;
    open_math_drawer(&host).await;
    type_into(&input_named(&host, ".math-formula"), "1 / 0", "input");
    tick().await;
    let html = host.inner_html();
    assert!(text(&host, ".math-line--fails").contains("Fails"), "{html}");
    assert!(
        !element(&host, ".flow-toolbar button[aria-label=Save]").has_attribute("disabled"),
        "{html}"
    );
}

#[wasm_bindgen_test]
async fn math_preview_uses_house_values() {
    let (host, _mounted, house) = editing(reference_view(None, vec![])).await;
    let sensor = power_sensor();
    house.entities.update(|m| {
        m.insert(sensor.id.clone(), sensor);
    });
    open_math_drawer(&host).await;
    click(&host, ".math-inputs__add");
    tick().await;
    assert_eq!(count(&host, ".math-input"), 1);
    open_entity_picker(&host).await;
    let row: web_sys::HtmlElement = host
        .query_selector("[id$='-opt-sensor.power']")
        .unwrap()
        .unwrap_or_else(|| panic!("no sensor row: {}", host.inner_html()))
        .unchecked_into();
    row.click();
    tick().await;
    // The input is named from the entity, then renamed by hand.
    assert_eq!(
        input_named(&host, ".math-input__name").value(),
        "power_meter"
    );
    type_into(&input_named(&host, ".math-input__name"), "p", "input");
    type_into(&input_named(&host, ".math-formula"), "p * 2", "input");
    tick().await;
    let html = host.inner_html();
    assert_eq!(text(&host, ".math-line__value"), "= 42", "{html}");
    // An average window reads the current value in the preview and says so.
    let window: web_sys::HtmlSelectElement = element(&host, ".math-input__window").unchecked_into();
    window.set_value("5m");
    let init = web_sys::EventInit::new();
    init.set_bubbles(true);
    window
        .dispatch_event(&web_sys::Event::new_with_event_init_dict("change", &init).unwrap())
        .unwrap();
    tick().await;
    let line = text(&host, ".math-line");
    assert!(line.contains("= 42") && line.contains("5m"), "{line}");
    settle_history().await;
}

#[wasm_bindgen_test]
async fn math_inputs_follow_undo_and_redo() {
    let (host, _mounted, _) = editing(reference_view(None, vec![])).await;
    open_math_drawer(&host).await;
    click(&host, ".math-inputs__add");
    tick().await;
    assert_eq!(count(&host, ".math-input"), 1);
    click(&host, ".math-inputs__add");
    tick().await;
    assert_eq!(count(&host, ".math-input"), 2);
    click(&host, ".flow-toolbar button[aria-label=Undo]");
    gloo_timers::future::TimeoutFuture::new(50).await;
    assert_eq!(count(&host, ".math-input"), 1, "{}", host.inner_html());
    assert_eq!(input_named(&host, ".math-input__name").value(), "input");
    press(&body(), "z", true, true);
    gloo_timers::future::TimeoutFuture::new(50).await;
    assert_eq!(count(&host, ".math-input"), 2, "{}", host.inner_html());
    // Undoing a remove brings the row back.
    click(&host, ".math-input__remove");
    tick().await;
    assert_eq!(count(&host, ".math-input"), 1);
    click(&host, ".flow-toolbar button[aria-label=Undo]");
    gloo_timers::future::TimeoutFuture::new(50).await;
    assert_eq!(count(&host, ".math-input"), 2, "{}", host.inner_html());
    // A window change is undone in the select too.
    let window: web_sys::HtmlSelectElement = element(&host, ".math-input__window").unchecked_into();
    window.set_value("5m");
    let init = web_sys::EventInit::new();
    init.set_bubbles(true);
    window
        .dispatch_event(&web_sys::Event::new_with_event_init_dict("change", &init).unwrap())
        .unwrap();
    tick().await;
    click(&host, ".flow-toolbar button[aria-label=Undo]");
    gloo_timers::future::TimeoutFuture::new(50).await;
    let window: web_sys::HtmlSelectElement = element(&host, ".math-input__window").unchecked_into();
    assert_eq!(window.value(), "now");
}

#[wasm_bindgen_test]
async fn math_row_keeps_focus_while_typing() {
    let (host, _mounted, _) = editing(reference_view(None, vec![])).await;
    open_math_drawer(&host).await;
    click(&host, ".math-inputs__add");
    tick().await;
    let name = input_named(&host, ".math-input__name");
    name.focus().unwrap();
    type_into(&name, "inputx", "input");
    tick().await;
    assert_eq!(count(&host, ".math-input"), 1);
    let active = web_sys::window()
        .unwrap()
        .document()
        .unwrap()
        .active_element();
    assert_eq!(
        active.as_ref(),
        Some(name.unchecked_ref::<web_sys::Element>()),
        "focus moved: {}",
        host.inner_html()
    );
    assert!(name.is_connected());
}

#[wasm_bindgen_test]
async fn functions_menu_inserts_with_slots() {
    let (host, _mounted, _) = editing(reference_view(None, vec![])).await;
    open_math_drawer(&host).await;
    assert_eq!(count(&host, ".math-functions__item"), 0);
    click(&host, ".math-functions__toggle");
    tick().await;
    assert_eq!(count(&host, ".math-functions__item"), 9);
    let items = host.query_selector_all(".math-functions__item").unwrap();
    let clamp: web_sys::HtmlElement = (0..items.length())
        .filter_map(|i| items.item(i))
        .find(|n| {
            n.text_content()
                .unwrap_or_default()
                .contains("clamp(x, low, high)")
        })
        .unwrap_or_else(|| panic!("no clamp: {}", host.inner_html()))
        .unchecked_into();
    clamp.click();
    tick().await;
    gloo_timers::future::TimeoutFuture::new(50).await;
    let formula = input_named(&host, ".math-formula");
    assert_eq!(formula.value(), "clamp(▢, ▢, ▢)");
    // The first slot is selected, in UTF-16 units.
    assert_eq!(formula.selection_start().unwrap(), Some(6));
    assert_eq!(formula.selection_end().unwrap(), Some(7));
    assert_eq!(
        text(&host, ".math-line.node-editor__error"),
        "fill in the ▢ slots"
    );
    assert_eq!(count(&host, ".math-functions__item"), 0, "menu closes");
    settle_history().await;
}

#[wasm_bindgen_test]
async fn typing_suggests_and_tab_accepts() {
    let (host, _mounted, _) = editing(reference_view(None, vec![])).await;
    open_math_drawer(&host).await;
    let formula = input_named(&host, ".math-formula");
    formula.focus().unwrap();
    type_into(&formula, "cl", "input");
    tick().await;
    assert_eq!(text(&host, ".math-suggestions__item"), "clamp");
    // Typing normally is not intercepted: a letter key is left alone.
    let init = web_sys::KeyboardEventInit::new();
    init.set_key("Tab");
    init.set_cancelable(true);
    let tab = web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
    formula.dispatch_event(&tab).unwrap();
    tick().await;
    gloo_timers::future::TimeoutFuture::new(50).await;
    assert!(tab.default_prevented());
    assert_eq!(formula.value(), "clamp(▢, ▢, ▢)");
    assert_eq!(count(&host, ".math-suggestions__item"), 0);
    // With no suggestion, Tab keeps its normal job.
    let tab = web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
    formula.dispatch_event(&tab).unwrap();
    assert!(!tab.default_prevented());
    settle_history().await;
}

#[wasm_bindgen_test]
async fn escape_closes_suggestions() {
    let (host, _mounted, _) = editing(reference_view(None, vec![])).await;
    open_math_drawer(&host).await;
    let formula = input_named(&host, ".math-formula");
    type_into(&formula, "ma", "input");
    tick().await;
    assert_eq!(count(&host, ".math-suggestions__item"), 1);
    let init = web_sys::KeyboardEventInit::new();
    init.set_key("Escape");
    formula
        .dispatch_event(
            &web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap(),
        )
        .unwrap();
    tick().await;
    assert_eq!(count(&host, ".math-suggestions__item"), 0);
}

#[wasm_bindgen_test]
async fn input_chip_inserts_name() {
    let (host, _mounted, _) = editing(reference_view(None, vec![])).await;
    open_math_drawer(&host).await;
    click(&host, ".math-inputs__add");
    tick().await;
    type_into(&input_named(&host, ".math-input__name"), "grid", "input");
    let formula = input_named(&host, ".math-formula");
    type_into(&formula, "1 + ", "input");
    tick().await;
    let chips = text(&host, ".math-chips");
    assert!(
        chips.contains("grid") && chips.contains("trigger"),
        "{chips}"
    );
    click(&host, ".math-chips__name");
    tick().await;
    gloo_timers::future::TimeoutFuture::new(50).await;
    assert_eq!(formula.value(), "1 + grid");
    // The input rows were not rebuilt.
    assert_eq!(count(&host, ".math-input"), 1);
    settle_history().await;
}

// ---- Cards that read a math result ----------------------------------------------------------

/// A flow with a math card `half`, a number command, a comparison and a push.
fn result_view() -> FlowView {
    const WITH_MATH: &str = r#"{
      "id": "living_led_motion", "name": "Half", "enabled": true, "mode": "single",
      "nodes": [
        { "id": "power", "type": "trigger.state", "params": { "entity": "sensor.power" } },
        { "id": "half", "type": "math", "params": { "inputs": [], "formula": "trigger / 2", "name": "half" } },
        { "id": "set", "type": "command",
          "params": { "entity": "number.target", "command": { "type": "set_number", "value": 7 } } },
        { "id": "check", "type": "if", "params": { "op": "gt", "value": 300, "average": "15m" } },
        { "id": "plug", "type": "command",
          "params": { "entity": "switch.plug", "command": { "type": "turn_on" }, "value_from": "half" } },
        { "id": "note", "type": "notify", "params": { "title": "Hi" } }
      ],
      "edges": [
        { "from": "power", "to": "half" },
        { "from": "half", "to": "set" },
        { "from": "half", "to": "check" },
        { "from": "half", "to": "plug" },
        { "from": "check", "port": "true", "to": "note" }
      ]
    }"#;
    FlowView {
        definition: Flow::decode(WITH_MATH.as_bytes()).unwrap(),
        invalid: None,
        active_run: None,
        runs: vec![],
    }
}

// ---- Value of ----------------------------------------------------------------------------

/// A vacuum `vacuum.robo` at 62 % battery.
fn robo() -> nidoo_api::EntityView {
    use nidoo_core::model::{Kind, StateValue, VacuumState};
    let mut robo = table_lamp();
    robo.id = "vacuum.robo".into();
    robo.name = "Roborock".into();
    robo.kind = Kind::Vacuum;
    robo.protocol_kind = Kind::Vacuum;
    robo.component = "vacuum".into();
    robo.state.value = StateValue::Vacuum(VacuumState {
        state: Some("cleaning".into()),
        battery: Some(62),
        fan_speed: None,
    });
    robo
}

/// A flow `living_led_motion` of `nodes` wired in a line.
fn line_view(nodes: &str) -> FlowView {
    let nodes: Vec<serde_json::Value> = serde_json::from_str(nodes).unwrap();
    let edges: Vec<serde_json::Value> = nodes
        .windows(2)
        .map(|w| serde_json::json!({ "from": w[0]["id"], "to": w[1]["id"], "port": (w[0]["type"] == "if").then_some("true") }))
        .collect();
    let flow = serde_json::json!({
        "id": "living_led_motion", "name": "Line", "enabled": true, "mode": "single",
        "nodes": nodes, "edges": edges
    });
    FlowView {
        definition: Flow::decode(flow.to_string().as_bytes()).unwrap(),
        invalid: None,
        active_run: None,
        runs: vec![],
    }
}

/// An editing canvas over [`result_view`] with a number entity for the command.
async fn editing_results() -> (web_sys::Element, Box<dyn std::any::Any>, House) {
    use nidoo_core::model::{EntityState, Kind, StateValue};
    let (host, mounted, house) = editing(result_view()).await;
    let target = nidoo_api::EntityView {
        id: "number.target".into(),
        name: "Target".into(),
        kind: Kind::Number,
        protocol_kind: Kind::Number,
        component: Kind::Number.component().to_string(),
        state: EntityState {
            value: StateValue::Number(3.0),
            ..Default::default()
        },
        ..table_lamp()
    };
    let plug = nidoo_api::EntityView {
        id: "switch.plug".into(),
        name: "Plug".into(),
        kind: Kind::Switch,
        protocol_kind: Kind::Switch,
        component: Kind::Switch.component().to_string(),
        state: EntityState {
            value: StateValue::OnOff(false),
            ..Default::default()
        },
        ..table_lamp()
    };
    house.entities.update(|m| {
        m.insert(target.id.clone(), target);
        m.insert(plug.id.clone(), plug);
    });
    (host, mounted, house)
}

fn change(el: &web_sys::Element) {
    let init = web_sys::EventInit::new();
    init.set_bubbles(true);
    el.dispatch_event(&web_sys::Event::new_with_event_init_dict("change", &init).unwrap())
        .unwrap();
}

/// The node `id` of the flow the next Save sends.
async fn saved_node(host: &web_sys::Element, id: &str) -> serde_json::Value {
    let fetch = FakeFetch::answer(&write_outcome(&result_view()));
    click(host, ".flow-toolbar button[aria-label=Save]");
    gloo_timers::future::TimeoutFuture::new(50).await;
    let sent = puts(&fetch);
    let (_, _, body) = sent
        .last()
        .unwrap_or_else(|| panic!("no save: {}", host.inner_html()));
    let flow: serde_json::Value = serde_json::from_str(body).unwrap();
    flow["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"] == id)
        .unwrap_or_else(|| panic!("no node {id}: {body}"))
        .clone()
}

#[wasm_bindgen_test]
async fn command_value_from_result() {
    let (host, _mounted, _) = editing_results().await;
    select_card(&host, "set").await;
    assert_eq!(count(&host, ".flow-drawer input[type=number]"), 1);
    let select: web_sys::HtmlSelectElement =
        element(&host, ".flow-drawer select.value-from").unchecked_into();
    assert_eq!(select.value(), "");
    select.set_value("half");
    change(&select);
    tick().await;
    // The number box gives way while a result feeds the command.
    assert_eq!(
        count(&host, ".flow-drawer input[type=number]"),
        0,
        "{}",
        host.inner_html()
    );
    assert_eq!(
        text(&host, "[data-node=set] .flow-node__title"),
        "Set Target to half"
    );
    let node = saved_node(&host, "set").await;
    assert_eq!(node["params"]["value_from"], "half", "{node}");
    // The literal stays for when the link is cleared.
    assert_eq!(node["params"]["command"]["value"], 7.0, "{node}");
}

#[wasm_bindgen_test]
async fn value_from_is_offered_only_with_math_and_a_number() {
    let (host, _mounted, _) = editing_results().await;
    select_card(&host, "set").await;
    assert_eq!(count(&host, ".flow-drawer select.value-from"), 1);
    // A flow with no math card offers nothing to pick.
    let (plain, _m2, _) = editing(reference_view(None, vec![])).await;
    select_card(&plain, "on").await;
    assert_eq!(count(&plain, ".flow-drawer select.value-from"), 0);
}

#[wasm_bindgen_test]
async fn if_compares_a_result() {
    let (host, _mounted, _) = editing_results().await;
    select_card(&host, "check").await;
    open_entity_picker(&host).await;
    let row: web_sys::HtmlElement = host
        .query_selector("[id$='-opt-half']")
        .unwrap()
        .unwrap_or_else(|| panic!("no result row: {}", host.inner_html()))
        .unchecked_into();
    row.click();
    tick().await;
    let field = text(&host, ".flow-drawer button.picker-field");
    assert!(field.contains("half"), "{field}");
    assert_eq!(
        text(&host, "[data-node=check] .flow-node__title"),
        "half > 300?"
    );
    let node = saved_node(&host, "check").await;
    assert_eq!(node["params"]["result"], "half", "{node}");
    assert!(node["params"].get("entity").is_none(), "{node}");
    // Choosing a result drops the average the card had.
    assert!(node["params"].get("average").is_none(), "{node}");
    settle_history().await;
}

#[wasm_bindgen_test]
async fn notify_inserts_result() {
    let (host, _mounted, _) = editing_results().await;
    select_card(&host, "note").await;
    // The body's ＋ Value is the second button.
    let add: web_sys::HtmlElement = host
        .query_selector_all(".flow-drawer .node-editor__add-value")
        .unwrap()
        .item(1)
        .unwrap()
        .unchecked_into();
    add.click();
    tick().await;
    let row: web_sys::HtmlElement = host
        .query_selector("[id$='-opt-half']")
        .unwrap()
        .unwrap_or_else(|| panic!("no result row: {}", host.inner_html()))
        .unchecked_into();
    row.click();
    tick().await;
    let node = saved_node(&host, "note").await;
    assert_eq!(node["params"]["body"], "{half}", "{node}");
    settle_history().await;
}

#[wasm_bindgen_test]
async fn changing_the_action_drops_the_result_link() {
    let (host, _mounted, _) = editing_results().await;
    select_card(&host, "plug").await;
    assert_eq!(count(&host, ".flow-drawer select.value-from"), 1);
    // The Action select is the one that offers "Turn off".
    let all = host
        .query_selector_all(".flow-drawer select option")
        .unwrap();
    let off: web_sys::Element = (0..all.length())
        .filter_map(|i| all.item(i))
        .map(|n| n.unchecked_into::<web_sys::Element>())
        .find(|o| o.text_content().unwrap_or_default().contains("Turn off"))
        .unwrap_or_else(|| panic!("no Turn off: {}", host.inner_html()));
    let action: web_sys::HtmlSelectElement = off.parent_element().unwrap().unchecked_into();
    action.set_value(&off.get_attribute("value").unwrap());
    change(&action);
    tick().await;
    let node = saved_node(&host, "plug").await;
    assert_eq!(node["params"]["command"]["type"], "turn_off", "{node}");
    assert!(node["params"].get("value_from").is_none(), "{node}");
}

/// Picks the math result `half` on card `id` of a flow whose If and Switch compare the text
/// entity `sensor.mode` with "idle", then returns the card as the next Save sends it.
async fn pick_half_on_a_text_comparison(id: &str) -> serde_json::Value {
    const TEXTY: &str = r#"{
      "id": "living_led_motion", "name": "Texty", "enabled": true, "mode": "single",
      "nodes": [
        { "id": "power", "type": "trigger.state", "params": { "entity": "sensor.power" } },
        { "id": "half", "type": "math", "params": { "inputs": [], "formula": "1", "name": "half" } },
        { "id": "check", "type": "if", "params": { "op": "eq", "value": "idle", "entity": "sensor.mode" } },
        { "id": "pick", "type": "switch", "params": { "entity": "sensor.mode", "rules": [ { "op": "eq", "value": "idle" } ] } }
      ],
      "edges": [ { "from": "power", "to": "half" }, { "from": "half", "to": "check" }, { "from": "half", "to": "pick" } ]
    }"#;
    let view = FlowView {
        definition: Flow::decode(TEXTY.as_bytes()).unwrap(),
        invalid: None,
        active_run: None,
        runs: vec![],
    };
    let (host, _mounted, _) = editing(view).await;
    select_card(&host, id).await;
    open_entity_picker(&host).await;
    let row: web_sys::HtmlElement = element(&host, "[id$='-opt-half']").unchecked_into();
    row.click();
    tick().await;
    let node = saved_node(&host, id).await;
    settle_history().await;
    node
}

#[wasm_bindgen_test]
async fn if_result_makes_the_value_a_number() {
    let check = pick_half_on_a_text_comparison("check").await;
    assert_eq!(check["params"]["result"], "half", "{check}");
    assert!(check["params"]["value"].is_number(), "{check}");
}

#[wasm_bindgen_test]
async fn switch_result_makes_the_rule_values_numbers() {
    let pick = pick_half_on_a_text_comparison("pick").await;
    assert_eq!(pick["params"]["result"], "half", "{pick}");
    assert!(pick["params"]["rules"][0]["value"].is_number(), "{pick}");
}

/// Saves the draft and returns the nodes the PUT sent.
async fn saved_nodes(host: &web_sys::Element, view: &FlowView) -> Vec<serde_json::Value> {
    let fetch = FakeFetch::answer(&write_outcome(view));
    click(host, ".flow-toolbar button[aria-label=Save]");
    gloo_timers::future::TimeoutFuture::new(50).await;
    let requests = puts(&fetch);
    assert_eq!(requests.len(), 1, "{requests:?}");
    let sent: serde_json::Value = serde_json::from_str(&requests[0].2).unwrap();
    sent["nodes"].as_array().unwrap().clone()
}

#[wasm_bindgen_test]
async fn if_card_value_of_sets_field() {
    let view = line_view(
        r#"[
          { "id": "start", "type": "trigger.state", "params": { "entity": "vacuum.robo" } },
          { "id": "check", "type": "if",
            "params": { "entity": "vacuum.robo", "op": "eq", "value": "cleaning" } }
        ]"#,
    );
    let (host, _mounted, house) = editing(view.clone()).await;
    house.entities.update(|m| {
        let robo = robo();
        m.insert(robo.id.clone(), robo);
    });
    select_card(&host, "check").await;
    let select: web_sys::HtmlSelectElement = host
        .query_selector(".flow-drawer select[aria-label='Value of']")
        .unwrap()
        .unwrap_or_else(|| panic!("no Value of: {}", host.inner_html()))
        .unchecked_into();
    let battery = select
        .query_selector("option[value='field:battery']")
        .unwrap()
        .unwrap_or_else(|| panic!("no battery option: {}", select.inner_html()));
    assert!(battery.text_content().unwrap().contains("Battery"));
    select.set_value("field:battery");
    change(&select);
    tick().await;
    assert_eq!(
        count(&host, ".flow-drawer input[type=number]"),
        1,
        "{}",
        host.inner_html()
    );
    let nodes = saved_nodes(&host, &view).await;
    assert_eq!(nodes[1]["params"]["field"], "battery");
    settle_history().await;
}

#[wasm_bindgen_test]
async fn entity_change_clears_field_value_to_from() {
    let view = line_view(
        r#"[
          { "id": "start", "type": "trigger.state",
            "params": { "entity": "vacuum.robo", "field": "battery", "to": 50, "from": 60 } },
          { "id": "wait", "type": "delay", "params": { "for": "1m" } }
        ]"#,
    );
    let (host, _mounted, house) = editing(view.clone()).await;
    house.entities.update(|m| {
        for e in [robo(), table_lamp()] {
            m.insert(e.id.clone(), e);
        }
    });
    select_card(&host, "start").await;
    open_entity_picker(&host).await;
    let row: web_sys::HtmlElement = host
        .query_selector("[id$='-opt-light.table']")
        .unwrap()
        .unwrap_or_else(|| panic!("no light row: {}", host.inner_html()))
        .unchecked_into();
    row.click();
    tick().await;
    let nodes = saved_nodes(&host, &view).await;
    let params = &nodes[0]["params"];
    assert_eq!(params["entity"], "light.table");
    assert!(params.get("field").is_none(), "{params}");
    assert!(params.get("to").is_none(), "{params}");
    assert!(params.get("from").is_none(), "{params}");
    settle_history().await;
}

/// An average the card's value cannot have stays on screen, so it can be cleared.
#[wasm_bindgen_test]
async fn a_set_average_stays_visible_when_the_field_has_no_history() {
    let view = line_view(
        r#"[
          { "id": "start", "type": "trigger.state",
            "params": { "entity": "light.table", "field": "brightness" } },
          { "id": "check", "type": "if", "params": { "op": "gt", "value": 50, "average": "15m" } },
          { "id": "plain", "type": "if", "params": { "op": "gt", "value": 50 } }
        ]"#,
    );
    let (host, _mounted, house) = editing(view).await;
    house.entities.update(|m| {
        let lamp = table_lamp();
        m.insert(lamp.id.clone(), lamp);
    });
    let average_boxes =
        |host: &web_sys::Element| count(host, ".flow-drawer input[placeholder*='15m']");
    select_card(&host, "check").await;
    assert_eq!(average_boxes(&host), 1, "{}", host.inner_html());
    select_card(&host, "plain").await;
    assert_eq!(average_boxes(&host), 0, "{}", host.inner_html());
    settle_history().await;
}

// ---- Speak ---------------------------------------------------------------------------------

#[wasm_bindgen_test]
async fn speak_card_edits_its_text_and_room() {
    use nidoo_core::model::{Kind, StateValue};
    let view = line_view(
        r#"[
          { "id": "power", "type": "trigger.state", "params": { "entity": "sensor.power" } },
          { "id": "say", "type": "speak", "params": { "text": "Hello" } }
        ]"#,
    );
    let (host, _mounted, house) = editing(view).await;
    // One satellite, in the kitchen: the only room the card offers besides every room.
    let mut volume = nidoo_api::EntityView {
        id: "nidoo_sound_x_volume".into(),
        device_id: Some("nidoo_sound_x".into()),
        name: "Volume".into(),
        kind: Kind::Number,
        protocol_kind: Kind::Number,
        component: Kind::Number.component().to_string(),
        area: "kitchen".into(),
        ..table_lamp()
    };
    volume.state.value = StateValue::Number(40.0);
    house.entities.update(|m| {
        m.insert(volume.id.clone(), volume);
    });
    house.areas.update(|areas| {
        areas.push(nidoo_api::AreaView {
            id: "kitchen".into(),
            name: "Kitchen".into(),
            icon: None,
            order: None,
            entity_count: 1,
            primary_count: 1,
            group_names: Default::default(),
            group_favorites: Default::default(),
        });
    });
    tick().await;
    assert!(text(&host, "[data-node=say]").contains("Speak"));
    select_card(&host, "say").await;
    let select: web_sys::HtmlSelectElement =
        element(&host, ".flow-drawer select.speak-room").unchecked_into();
    let options = select.query_selector_all("option").unwrap();
    let labels: Vec<String> = (0..options.length())
        .filter_map(|i| options.item(i))
        .map(|o| o.text_content().unwrap_or_default())
        .collect();
    assert_eq!(labels, ["All rooms", "Kitchen"]);
    select.set_value("kitchen");
    change(&select);
    tick().await;
    let node = saved_node(&host, "say").await;
    assert_eq!(node["params"]["text"], "Hello", "{node}");
    assert_eq!(node["params"]["room"], "kitchen", "{node}");
    settle_history().await;
}
