#![cfg(target_arch = "wasm32")]

use leptos::prelude::*;
use leptos::task::tick;
use leptos_router::components::Router;
use nidoo_api::{EntityView, WsEvent};
use nidoo_core::model::{Capabilities, EntityCategory, EntityState, Kind, StateValue, UiOverride};
use nidoo_web::energy_view::EnergyView;
use nidoo_web::house::House;
use nidoo_web::i18n::Lang;
use nidoo_web::screens::energy::{EnergyScreen, KwhToday};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
use web_sys::{Element, Event, HtmlElement, KeyboardEvent, KeyboardEventInit};
#[path = "support/fetch_stub.rs"]
mod fetch_stub;
use fetch_stub::FetchStub;

fn sensor(id: &str, name: &str, unit: &str, value: f64, ui: UiOverride) -> EntityView {
    EntityView {
        id: id.into(),
        device_id: None,
        name: name.into(),
        device_name: None,
        label: None,
        kind: Kind::Sensor,
        protocol_kind: Kind::Sensor,
        component: Kind::Sensor.component().to_string(),
        device_class: None,
        unit: Some(unit.into()),
        icon: None,
        category: EntityCategory::Primary,
        enabled_by_default: true,
        area: "salon".into(),
        part_of: None,
        capabilities: Capabilities::default(),
        ui,
        state: EntityState {
            value: StateValue::Number(value),
            ..Default::default()
        },
    }
}

fn role(role: &str) -> UiOverride {
    UiOverride {
        energy_role: Some(role.into()),
        ..Default::default()
    }
}

fn mount(entities: Vec<EntityView>) -> Element {
    mount_lang(entities, RwSignal::new(Lang::En))
}

fn mount_lang(entities: Vec<EntityView>, lang: RwSignal<Lang>) -> Element {
    mount_with(entities, lang, EnergyView::Classic)
}

fn mount_view(entities: Vec<EntityView>, view: EnergyView) -> Element {
    mount_with(entities, RwSignal::new(Lang::En), view)
}

fn mount_with(entities: Vec<EntityView>, lang: RwSignal<Lang>, view: EnergyView) -> Element {
    let house = House::new();
    for e in entities {
        house.apply(WsEvent::EntityUpserted(e));
    }
    mount_house(house, lang, view)
}

fn mount_house(house: House, lang: RwSignal<Lang>, view: EnergyView) -> Element {
    house.home_settings.update(|s| s.energy_view = view);
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    std::mem::forget(leptos::mount::mount_to(
        host.clone().unchecked_into(),
        move || {
            provide_context(house);
            provide_context(lang);
            // App installs this through install_history_refresh, which also fetches.
            provide_context(KwhToday(RwSignal::new(Default::default())));
            view! {
                <Router>
                    <EnergyScreen />
                </Router>
            }
        },
    ));
    host
}

fn text(host: &Element, selector: &str) -> String {
    host.query_selector(selector)
        .unwrap()
        .unwrap_or_else(|| panic!("no {selector}"))
        .text_content()
        .unwrap_or_default()
}

fn count(host: &Element, selector: &str) -> u32 {
    host.query_selector_all(selector).unwrap().length()
}

fn node_with(host: &Element, needle: &str) -> Element {
    let nodes = host.query_selector_all(".energy-node").unwrap();
    (0..nodes.length())
        .filter_map(|i| nodes.item(i))
        .map(|n| n.unchecked_into::<Element>())
        .find(|n| n.text_content().unwrap_or_default().contains(needle))
        .unwrap_or_else(|| panic!("no node with {needle}"))
}

#[wasm_bindgen_test]
async fn live_house_shows_nodes_and_cards() {
    let mut grid = sensor("grid", "Grid power", "W", 253.3, role("grid"));
    grid.ui.energy_meter = Some("grid_energy".into());
    let host = mount(vec![
        grid,
        sensor(
            "grid_energy",
            "Grid energy",
            "kWh",
            6584.04,
            UiOverride::default(),
        ),
        sensor("solar", "Solar power", "W", 3.0, role("solar")),
        sensor("heater", "Water heater", "W", 120.0, role("consumer")),
        sensor("ac", "AC", "W", 60.0, role("consumer")),
        sensor("tank", "Tank", "L", 42.0, role("water")),
    ]);
    tick().await;

    assert_eq!(text(&host, ".energy__subtitle"), "Live");
    // solar, grid, the derived house, two consumers and water
    assert_eq!(count(&host, ".energy-node"), 6);
    assert_eq!(count(&host, ".energy-node--water"), 1);
    assert_eq!(count(&host, ".energy-edge"), 4);
    let aria = host
        .query_selector(".energy-diagram__svg")
        .unwrap()
        .unwrap()
        .get_attribute("aria-label")
        .unwrap();
    assert!(
        aria.contains("Importing 253\u{a0}watts from the grid."),
        "{aria}"
    );

    let cards = host.query_selector_all(".energy-card").unwrap();
    let grid_card = (0..cards.length())
        .filter_map(|i| cards.item(i))
        .map(|c| c.unchecked_into::<Element>())
        .find(|c| text(c, ".card__name") == "Grid")
        .expect("grid card");
    let body = grid_card.text_content().unwrap();
    let body = body.replace('\u{a0}', " ");
    assert!(body.contains("253 W"), "{body}");
    assert!(body.contains("6\u{2009}584 kWh"), "{body}");
}

#[wasm_bindgen_test]
async fn four_consumers_fold_into_more() {
    let mut entities = vec![sensor("solar", "Solar power", "W", 500.0, role("solar"))];
    for (i, name) in ["Oven", "Dryer", "Fridge", "Washer"].iter().enumerate() {
        entities.push(sensor(
            &format!("c{i}"),
            name,
            "W",
            400.0 - 100.0 * i as f64,
            role("consumer"),
        ));
    }
    let host = mount(entities);
    tick().await;

    assert_eq!(count(&host, ".energy-more li"), 0);
    let more = node_with(&host, "+2");
    more.dispatch_event(&Event::new("click").unwrap()).unwrap();
    tick().await;
    assert_eq!(count(&host, ".energy-more li"), 4);
    let list = text(&host, ".energy-more");
    for name in ["Oven", "Dryer", "Fridge", "Washer"] {
        assert!(list.contains(name), "{list}");
    }

    let toggle: HtmlElement = host
        .query_selector(".energy__more-toggle")
        .unwrap()
        .expect("toggle")
        .unchecked_into();
    toggle.click();
    tick().await;
    assert_eq!(count(&host, ".energy-more li"), 0);
}

#[wasm_bindgen_test]
async fn no_roles_shows_empty_state() {
    let host = mount(vec![sensor(
        "plain",
        "Plain",
        "W",
        5.0,
        UiOverride::default(),
    )]);
    tick().await;
    assert!(text(&host, ".energy__empty").contains("energy role"));
    assert_eq!(count(&host, ".energy-diagram"), 0);
}

const GRID_DAY: &str = r#"{
    "date": "2026-10-24", "today": "2026-10-24", "first_date": "2026-01-15",
    "day_slots": 48,
    "slots": [{"start": 0, "local": "00:00", "off_peak": 0.4, "peak": 0.2, "export": 0.1}],
    "bands": [], "totals": {"off_peak": 7.2, "peak": 9.9, "export": 3.1},
    "has_export": true, "tariffs": true
}"#;

fn all_text(host: &Element) -> String {
    host.text_content().unwrap_or_default()
}

#[wasm_bindgen_test]
async fn energy_follows_the_language_toggle() {
    let stub = FetchStub::install();
    stub.reply("GET", "/api/v1/history/grid_day", 200, GRID_DAY);
    let mut grid = sensor("grid", "Grid power", "W", 1300.0, role("grid"));
    grid.ui.energy_meter = Some("grid_energy".into());
    let mut entities = vec![
        grid,
        sensor(
            "grid_energy",
            "Grid energy",
            "kWh",
            1250.0,
            UiOverride::default(),
        ),
        sensor("solar", "Solar power", "W", 500.0, role("solar")),
    ];
    for (i, name) in ["Oven", "Dryer", "Fridge", "Washer"].iter().enumerate() {
        entities.push(sensor(
            &format!("c{i}"),
            name,
            "W",
            400.0 - 100.0 * i as f64,
            role("consumer"),
        ));
    }
    let lang = RwSignal::new(Lang::Fr);
    let host = mount_lang(entities, lang);
    gloo_timers::future::TimeoutFuture::new(300).await;
    tick().await;

    let toggle: HtmlElement = host
        .query_selector(".energy__more-toggle")
        .unwrap()
        .expect("toggle")
        .unchecked_into();
    toggle.click();
    tick().await;

    let fr = all_text(&host);
    for word in [
        "Énergie",
        "En direct",
        "Heures creuses",
        "Heures pleines",
        "Injection solaire",
        "Consommation",
        "Afficher les 4 consommateurs",
        "Réseau",
        "Maison",
        "1,3\u{a0}kW",
        "1\u{202f}250\u{a0}kWh",
        "Acheté 17,1\u{a0}kWh",
        "Vendu 3,1\u{a0}kWh",
        "Aujourd'hui",
    ] {
        assert!(fr.contains(word), "missing {word:?} in {fr}");
    }
    for word in ["Off-peak", "Consumption", "Bought", "Today", "Grid"] {
        assert!(!fr.contains(word), "{word:?} left in {fr}");
    }
    assert!(fr.contains("Oven") && !fr.contains("Four"), "{fr}");
    assert!(text(&host, ".energy-day__detail").is_empty());
    let prev = host
        .query_selector(".energy-day__nav")
        .unwrap()
        .unwrap()
        .get_attribute("aria-label")
        .unwrap();
    assert_eq!(prev, "Jour précédent");

    lang.set(Lang::En);
    tick().await;
    let en = all_text(&host);
    for word in [
        "Energy",
        "Live",
        "Off-peak",
        "Peak",
        "Solar export",
        "All consumers",
        "Show all 4 consumers",
        "Grid",
        "House",
        "1.3\u{a0}kW",
        "1\u{2009}250\u{a0}kWh",
        "Bought 17.1\u{a0}kWh",
        "Sold 3.1\u{a0}kWh",
        "Today",
    ] {
        assert!(en.contains(word), "missing {word:?} in {en}");
    }
    for word in [
        "Heures creuses",
        "Consommation",
        "Acheté",
        "Aujourd'hui",
        "Réseau",
    ] {
        assert!(!en.contains(word), "{word:?} left in {en}");
    }
    assert!(en.contains("Oven"), "{en}");
    let prev = host
        .query_selector(".energy-day__nav")
        .unwrap()
        .unwrap()
        .get_attribute("aria-label")
        .unwrap();
    assert_eq!(prev, "Previous day");
    drop(stub);
}

#[wasm_bindgen_test]
async fn empty_state_follows_the_language() {
    let lang = RwSignal::new(Lang::Fr);
    let host = mount_lang(
        vec![sensor("plain", "Plain", "W", 5.0, UiOverride::default())],
        lang,
    );
    tick().await;
    assert!(text(&host, ".energy__empty").contains("rôle énergétique"));
    lang.set(Lang::En);
    tick().await;
    assert!(text(&host, ".energy__empty").contains("energy role"));
}

fn flux_fixture(oven: f64) -> Vec<EntityView> {
    vec![
        sensor("solar", "Solar power", "W", 2000.0, role("solar")),
        sensor("grid", "Grid power", "W", 300.0, role("grid")),
        sensor("oven", "Oven", "W", oven, role("consumer")),
        sensor("pump", "Heat pump", "W", 600.0, role("consumer")),
    ]
}

fn bar(host: &Element, id: &str) -> Element {
    host.query_selector(&format!(".energy-flux__bar[data-id='{id}']"))
        .unwrap()
        .unwrap_or_else(|| panic!("no bar {id}"))
}

#[wasm_bindgen_test]
async fn classic_is_the_default() {
    let host = mount(flux_fixture(1200.0));
    tick().await;
    assert_eq!(count(&host, ".energy-diagram__svg"), 1);
    assert_eq!(count(&host, ".energy-flux"), 0);
    // Every circle carries its icon, in a nested svg rather than a foreignObject.
    assert_eq!(count(&host, "foreignObject"), 0);
    assert_eq!(
        count(&host, ".energy-node__glyph > svg"),
        count(&host, ".energy-node")
    );
    let icon = host
        .query_selector(".energy-node__glyph > svg")
        .unwrap()
        .unwrap()
        .get_bounding_client_rect();
    assert!(icon.width() > 0.0 && icon.height() > 0.0);
    let glyph = host.query_selector(".energy-node__glyph").unwrap().unwrap();
    assert_eq!(
        glyph.namespace_uri().as_deref(),
        Some("http://www.w3.org/2000/svg")
    );
}

#[wasm_bindgen_test]
async fn flux_replaces_the_diagram() {
    let host = mount_view(flux_fixture(1200.0), EnergyView::Flux);
    tick().await;
    assert_eq!(count(&host, ".energy-flux__svg"), 1);
    assert_eq!(count(&host, ".energy-diagram__svg"), 0);
    for id in ["solar", "grid", "house", "consumer:oven", "consumer:pump"] {
        bar(&host, id);
    }
    assert_eq!(
        count(&host, ".energy-flux__bar--c0, .energy-flux__bar--c1"),
        2
    );
    assert_eq!(count(&host, ".energy-flux__chip"), 4);
    assert!(text(&host, ".energy-flux__chips").contains("consumed now"));
    assert_eq!(count(&host, ".energy-day"), 1);
    assert!(count(&host, ".energy-card") > 0);
}

#[wasm_bindgen_test]
async fn flux_opens_a_sensor() {
    let host = mount_view(flux_fixture(1200.0), EnergyView::Flux);
    tick().await;
    let oven = bar(&host, "consumer:oven");
    oven.dispatch_event(&Event::new("click").unwrap()).unwrap();
    tick().await;
    let path = web_sys::window().unwrap().location().pathname().unwrap();
    assert_eq!(path, "/entity/oven");
}

#[wasm_bindgen_test]
async fn flux_updates_in_place() {
    let house = House::new();
    for e in flux_fixture(1200.0) {
        house.apply(WsEvent::EntityUpserted(e));
    }
    let host = mount_house(house, RwSignal::new(Lang::En), EnergyView::Flux);
    tick().await;
    let before = bar(&host, "consumer:oven");
    let height = before.get_attribute("style").unwrap();
    // A second reading from the same entity through the same house.
    house.apply(WsEvent::EntityUpserted(sensor(
        "oven",
        "Oven",
        "W",
        400.0,
        role("consumer"),
    )));
    tick().await;
    let after = bar(&host, "consumer:oven");
    assert!(before.is_same_node(Some(&after)));
    assert_ne!(after.get_attribute("style").unwrap(), height);
}

#[wasm_bindgen_test]
async fn flux_colours_follow_a_reorder() {
    let house = House::new();
    for e in flux_fixture(1200.0) {
        house.apply(WsEvent::EntityUpserted(e));
    }
    let host = mount_house(house, RwSignal::new(Lang::En), EnergyView::Flux);
    tick().await;
    let class = |id: &str| bar(&host, id).get_attribute("class").unwrap_or_default();
    assert!(
        class("consumer:oven").contains("--c0"),
        "{}",
        class("consumer:oven")
    );
    // A bigger consumer arrives (a new set reorders at once): the oven moves down one slot,
    // and its colour with its band.
    house.apply(WsEvent::EntityUpserted(sensor(
        "kettle",
        "Kettle",
        "W",
        2000.0,
        role("consumer"),
    )));
    tick().await;
    assert!(
        class("consumer:kettle").contains("--c0"),
        "{}",
        class("consumer:kettle")
    );
    assert!(
        class("consumer:oven").contains("--c1"),
        "{}",
        class("consumer:oven")
    );
    assert!(
        class("consumer:pump").contains("--c2"),
        "{}",
        class("consumer:pump")
    );
}

#[wasm_bindgen_test]
async fn flux_skips_unknown_chips() {
    let host = mount_view(
        vec![
            sensor("solar", "Solar power", "W", 2000.0, role("solar")),
            sensor("oven", "Oven", "W", 1200.0, role("consumer")),
        ],
        EnergyView::Flux,
    );
    tick().await;
    let chips = text(&host, ".energy-flux__chips");
    assert!(chips.contains("solar production"), "{chips}");
    assert!(!chips.contains("from the grid"), "{chips}");
}

#[wasm_bindgen_test]
async fn flux_label_is_the_keyboard_target() {
    let host = mount_view(flux_fixture(1200.0), EnergyView::Flux);
    tick().await;
    let svg = host.query_selector(".energy-flux__svg").unwrap().unwrap();
    assert_eq!(svg.get_attribute("role").as_deref(), Some("group"));
    assert_eq!(bar(&host, "consumer:oven").get_attribute("tabindex"), None);
    let labels = host
        .query_selector_all(".energy-flux__label[tabindex='0']")
        .unwrap();
    let oven: HtmlElement = (0..labels.length())
        .filter_map(|i| labels.item(i))
        .map(|n| n.unchecked_into::<HtmlElement>())
        .find(|l| {
            l.get_attribute("aria-label")
                .is_some_and(|a| a.contains("Oven"))
        })
        .expect("a focusable, named oven label");
    assert_eq!(oven.get_attribute("role").as_deref(), Some("button"));
    let init = KeyboardEventInit::new();
    init.set_key(" ");
    init.set_cancelable(true);
    let ev = KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
    oven.dispatch_event(&ev).unwrap();
    tick().await;
    assert!(ev.default_prevented());
    let path = web_sys::window().unwrap().location().pathname().unwrap();
    assert_eq!(path, "/entity/oven");
}

fn mount_flux_narrow(entities: Vec<EntityView>) -> Element {
    let map: std::collections::BTreeMap<String, EntityView> =
        entities.into_iter().map(|e| (e.id.clone(), e)).collect();
    let ui = Memo::new(move |_| {
        nidoo_core::energy::EnergyFlow::build(&nidoo_web::energy::energy_entities(&map), 0, None)
    });
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    host.set_attribute("style", "width: 390px").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    std::mem::forget(leptos::mount::mount_to(
        host.clone().unchecked_into(),
        move || {
            provide_context(RwSignal::new(Lang::En));
            view! {
                <nidoo_web::screens::energy_flux::FluxDiagram
                    ui=ui
                    narrow=true
                    open=Callback::new(|_: String| {})
                    expanded=RwSignal::new(false)
                />
            }
        },
    ));
    host
}

#[wasm_bindgen_test]
async fn flux_narrow_runs_top_to_bottom() {
    let mut entities = vec![
        sensor("solar", "Solar power", "W", 2000.0, role("solar")),
        sensor("grid", "Grid power", "W", 600.0, role("grid")),
    ];
    for (i, (name, w)) in [
        ("Oven", 900.0),
        ("Heat pump", 800.0),
        ("Air conditioning unit", 500.0),
        ("Kettle", 300.0),
        ("Dryer", 200.0),
        ("Fridge", 100.0),
    ]
    .into_iter()
    .enumerate()
    {
        entities.push(sensor(&format!("c{i}"), name, "W", w, role("consumer")));
    }
    let host = mount_flux_narrow(entities);
    tick().await;
    let svg = host.query_selector(".energy-flux__svg").unwrap().unwrap();
    assert_eq!(svg.get_attribute("viewBox").unwrap(), "0 0 360 620");
    let rect = |e: &Element| e.get_bounding_client_rect();
    let house = rect(&bar(&host, "house"));
    assert!(house.width() > house.height());
    assert_eq!(
        count(
            &host,
            ".energy-flux__bar--c0, .energy-flux__bar--c1, .energy-flux__bar--c2"
        ),
        3
    );
    assert_eq!(count(&host, ".energy-flux__bar--others"), 1);
    // Every bar and label stays inside the svg.
    let frame = rect(&svg);
    let inside = |r: &web_sys::DomRect, what: &str| {
        assert!(
            r.left() >= frame.left() - 1.0
                && r.right() <= frame.right() + 1.0
                && r.top() >= frame.top() - 1.0
                && r.bottom() <= frame.bottom() + 1.0,
            "{what} overflows: {} {} {} {}",
            r.left(),
            r.top(),
            r.right(),
            r.bottom()
        );
    };
    let bars = host.query_selector_all(".energy-flux__bar").unwrap();
    for i in 0..bars.length() {
        let el: Element = bars.item(i).unwrap().unchecked_into();
        inside(&rect(&el), "bar");
    }
    let labels = host.query_selector_all(".energy-flux__label").unwrap();
    assert!(labels.length() >= 6);
    let mut names = String::new();
    for i in 0..labels.length() {
        let el: Element = labels.item(i).unwrap().unchecked_into();
        inside(&rect(&el), "label");
        names.push_str(&el.text_content().unwrap_or_default());
        // Two lines: value then name.
        assert_eq!(el.query_selector_all("tspan").unwrap().length(), 2);
    }
    // Names are cut at Classic's length.
    assert!(names.contains("Heat pump"), "{names}");
    assert!(!names.contains("Air conditioning unit"));
}

fn edges(host: &Element) -> Vec<Element> {
    let all = host.query_selector_all(".energy-edge").unwrap();
    (0..all.length())
        .filter_map(|i| all.item(i))
        .map(|n| n.unchecked_into::<Element>())
        .collect()
}

fn edge(host: &Element, selector: &str) -> Element {
    host.query_selector(selector)
        .unwrap()
        .unwrap_or_else(|| panic!("no {selector}"))
}

#[wasm_bindgen_test]
async fn circles_wire_solar_only_to_the_house() {
    let host = mount(vec![
        sensor("solar", "Solar power", "W", 3000.0, role("solar")),
        sensor("grid", "Grid power", "W", -1500.0, role("grid")),
        sensor("oven", "Oven", "W", 900.0, role("consumer")),
        sensor("pump", "Heat pump", "W", 600.0, role("consumer")),
    ]);
    tick().await;
    let all = edges(&host);
    assert!(!all.is_empty());
    let from_solar: Vec<_> = all
        .iter()
        .filter(|e| e.get_attribute("data-from").as_deref() == Some("solar"))
        .collect();
    assert_eq!(from_solar.len(), 1);
    for e in &all {
        let ends = (
            e.get_attribute("data-from").unwrap_or_default(),
            e.get_attribute("data-to").unwrap_or_default(),
        );
        if ends.0 == "solar" {
            assert_eq!(ends.1, "house");
        }
        let pair = [ends.0.as_str(), ends.1.as_str()];
        assert!(
            !(pair.contains(&"solar") && pair.contains(&"grid")),
            "{pair:?}"
        );
    }
    let grid = edge(&host, ".energy-edge--grid");
    assert_eq!(grid.get_attribute("data-from").as_deref(), Some("house"));
    assert_eq!(grid.get_attribute("data-to").as_deref(), Some("grid"));
    let class = grid.get_attribute("class").unwrap_or_default();
    assert!(class.contains("energy-tone--export"), "{class}");
}

#[wasm_bindgen_test]
async fn export_flips_the_grid_wire_in_place() {
    let house = House::new();
    for e in [
        sensor("solar", "Solar power", "W", 600.0, role("solar")),
        sensor("grid", "Grid power", "W", 400.0, role("grid")),
        sensor("oven", "Oven", "W", 900.0, role("consumer")),
    ] {
        house.apply(WsEvent::EntityUpserted(e));
    }
    let host = mount_house(house, RwSignal::new(Lang::En), EnergyView::Classic);
    tick().await;
    let before = edge(&host, ".energy-edge--grid");
    assert_eq!(before.get_attribute("data-from").as_deref(), Some("grid"));
    house.apply(WsEvent::EntityUpserted(sensor(
        "grid",
        "Grid power",
        "W",
        -800.0,
        role("grid"),
    )));
    tick().await;
    let after = edge(&host, ".energy-edge--grid");
    assert!(before.is_same_node(Some(&after)));
    assert_eq!(after.get_attribute("data-from").as_deref(), Some("house"));
    assert_eq!(after.get_attribute("data-to").as_deref(), Some("grid"));
}

#[wasm_bindgen_test]
async fn house_shows_the_solar_share() {
    let host = mount(vec![
        sensor("solar", "Solar power", "W", 600.0, role("solar")),
        sensor("grid", "Grid power", "W", 200.0, role("grid")),
    ]);
    tick().await;
    assert_eq!(
        text(&host, ".energy-node--house .energy-node__share"),
        "75% solar"
    );
}

#[wasm_bindgen_test]
async fn circle_opens_its_sensor_from_the_keyboard() {
    // Other tests leave the page on /entity/oven: the router starts from "/", so the path
    // proves the key.
    web_sys::window()
        .unwrap()
        .history()
        .unwrap()
        .replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some("/"))
        .unwrap();
    assert_eq!(
        web_sys::window().unwrap().location().pathname().unwrap(),
        "/"
    );
    let host = mount(vec![
        sensor("solar", "Solar power", "W", 2000.0, role("solar")),
        sensor("oven", "Oven", "W", 1200.0, role("consumer")),
    ]);
    tick().await;
    let oven = node_with(&host, "Oven");
    assert_eq!(oven.get_attribute("tabindex").as_deref(), Some("0"));
    assert_eq!(oven.get_attribute("role").as_deref(), Some("button"));
    let init = KeyboardEventInit::new();
    init.set_key("Enter");
    init.set_cancelable(true);
    let ev = KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
    oven.dispatch_event(&ev).unwrap();
    tick().await;
    assert!(ev.default_prevented());
    let path = web_sys::window().unwrap().location().pathname().unwrap();
    assert_eq!(path, "/entity/oven");
}

#[wasm_bindgen_test]
async fn dots_move_and_stop_when_the_diagram_unmounts() {
    let map: std::collections::BTreeMap<String, EntityView> = [
        sensor("solar", "Solar power", "W", 2000.0, role("solar")),
        sensor("oven", "Oven", "W", 1200.0, role("consumer")),
    ]
    .into_iter()
    .map(|e| (e.id.clone(), e))
    .collect();
    let ui = Memo::new(move |_| {
        nidoo_core::energy::EnergyFlow::build(&nidoo_web::energy::energy_entities(&map), 0, None)
    });
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    let handle = leptos::mount::mount_to(host.clone().unchecked_into(), move || {
        provide_context(RwSignal::new(Lang::En));
        view! {
            <nidoo_web::screens::energy_circles::CirclesDiagram
                ui=ui
                narrow=false
                open=Callback::new(|_: String| {})
                expanded=RwSignal::new(false)
            />
        }
    });
    let wait = |ms| gloo_timers::future::TimeoutFuture::new(ms);
    wait(150).await;
    let layer = host
        .query_selector(".energy-circles__dots")
        .unwrap()
        .expect("dots layer");
    let cx = || {
        layer
            .query_selector("circle")
            .unwrap()
            .and_then(|c| c.get_attribute("cx"))
    };
    let first = cx().expect("dots drawn");
    // A halo then its head: opacity as an attribute, a class on the halo only.
    let circles = layer.query_selector_all("circle").unwrap();
    let (halo, head): (Element, Element) = (
        circles.item(0).unwrap().unchecked_into(),
        circles.item(1).unwrap().unchecked_into(),
    );
    assert_eq!(
        halo.get_attribute("class").as_deref(),
        Some("energy-dots__halo")
    );
    assert_eq!(head.get_attribute("class"), None);
    for c in [&halo, &head] {
        assert!(c.get_attribute("opacity").is_some());
        assert!(c.get_attribute("r").is_some());
        assert_eq!(c.get_attribute("style"), None);
    }
    wait(150).await;
    assert_ne!(cx().unwrap(), first, "dots move");

    drop(handle);
    wait(50).await;
    let parked = cx();
    wait(200).await;
    assert_eq!(cx(), parked, "the loop stops on unmount");
}

#[wasm_bindgen_test]
async fn switching_to_flux_stops_the_dots() {
    let house = House::new();
    for e in flux_fixture(1200.0) {
        house.apply(WsEvent::EntityUpserted(e));
    }
    let host = mount_house(house, RwSignal::new(Lang::En), EnergyView::Classic);
    let wait = |ms| gloo_timers::future::TimeoutFuture::new(ms);
    wait(150).await;
    let layer = host
        .query_selector(".energy-circles__dots")
        .unwrap()
        .expect("dots layer");
    let cx = || {
        layer
            .query_selector("circle")
            .unwrap()
            .and_then(|c| c.get_attribute("cx"))
    };
    assert!(cx().is_some(), "dots drawn");
    house
        .home_settings
        .update(|s| s.energy_view = EnergyView::Flux);
    wait(50).await;
    assert_eq!(count(&host, ".energy-circles"), 0);
    let parked = cx();
    wait(200).await;
    assert_eq!(cx(), parked, "the loop stops when Classic leaves");
}

/// Mounts the circles diagram alone, with the real stylesheet, in a `width` px host; the returned
/// `<style>` is for the caller to remove, so the other tests keep running unstyled.
fn mount_circles_styled(entities: Vec<EntityView>, narrow: bool, width: u32) -> (Element, Element) {
    let document = web_sys::window().unwrap().document().unwrap();
    let style = document.create_element("style").unwrap();
    style.set_text_content(Some(concat!(
        include_str!("../styles/tokens.css"),
        include_str!("../styles/main.css")
    )));
    document.head().unwrap().append_child(&style).unwrap();
    let map: std::collections::BTreeMap<String, EntityView> =
        entities.into_iter().map(|e| (e.id.clone(), e)).collect();
    let ui = Memo::new(move |_| {
        nidoo_core::energy::EnergyFlow::build(&nidoo_web::energy::energy_entities(&map), 0, None)
    });
    let host = document.create_element("div").unwrap();
    host.set_attribute("style", &format!("width: {width}px"))
        .unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    std::mem::forget(leptos::mount::mount_to(
        host.clone().unchecked_into(),
        move || {
            provide_context(RwSignal::new(Lang::En));
            view! {
                <nidoo_web::screens::energy_circles::CirclesDiagram
                    ui=ui
                    narrow=narrow
                    open=Callback::new(|_: String| {})
                    expanded=RwSignal::new(false)
                />
            }
        },
    ));
    (host, style)
}

fn full_house() -> Vec<EntityView> {
    let mut entities = vec![
        sensor("solar", "Solar power", "W", 2000.0, role("solar")),
        sensor("grid", "Grid power", "W", 600.0, role("grid")),
        sensor("battery", "Battery power", "W", 800.0, role("battery")),
    ];
    for (i, (name, w)) in [
        ("Oven", 900.0),
        ("Heat pump", 800.0),
        ("Kettle", 500.0),
        ("Dryer", 300.0),
        ("Fridge", 100.0),
    ]
    .into_iter()
    .enumerate()
    {
        entities.push(sensor(&format!("c{i}"), name, "W", w, role("consumer")));
    }
    entities
}

/// Every circle's value and name lies inside the diagram's card.
async fn circle_texts_stay_in_the_card(narrow: bool, width: u32) {
    let (host, style) = mount_circles_styled(full_house(), narrow, width);
    // Past the nodes' 0.6 s glide from the origin to their place.
    gloo_timers::future::TimeoutFuture::new(800).await;
    let frame = host
        .query_selector(".energy-diagram")
        .unwrap()
        .expect("diagram")
        .get_bounding_client_rect();
    let texts = host
        .query_selector_all(".energy-node__label, .energy-node__value")
        .unwrap();
    assert!(texts.length() >= 6, "{}", texts.length());
    for i in 0..texts.length() {
        let el: Element = texts.item(i).unwrap().unchecked_into();
        let r = el.get_bounding_client_rect();
        assert!(
            r.left() >= frame.left() - 1.0
                && r.right() <= frame.right() + 1.0
                && r.top() >= frame.top() - 1.0
                && r.bottom() <= frame.bottom() + 1.0,
            "{:?} overflows the card: {} {} {} {} in {} {} {} {}",
            el.text_content(),
            r.left(),
            r.top(),
            r.right(),
            r.bottom(),
            frame.left(),
            frame.top(),
            frame.right(),
            frame.bottom()
        );
    }
    style.remove();
    host.remove();
}

#[wasm_bindgen_test]
async fn wide_circle_texts_stay_in_the_card() {
    circle_texts_stay_in_the_card(false, 800).await;
}

#[wasm_bindgen_test]
async fn narrow_circle_texts_stay_in_the_card() {
    circle_texts_stay_in_the_card(true, 390).await;
}

/// Injects the real stylesheet; the caller removes it so the other tests keep running unstyled.
fn inject_styles() -> Element {
    let document = web_sys::window().unwrap().document().unwrap();
    let style = document.create_element("style").unwrap();
    style.set_text_content(Some(concat!(
        include_str!("../styles/tokens.css"),
        include_str!("../styles/main.css")
    )));
    document.head().unwrap().append_child(&style).unwrap();
    style
}

fn computed(el: &Element, prop: &str) -> String {
    let style = web_sys::window()
        .unwrap()
        .get_computed_style(el)
        .unwrap()
        .unwrap();
    style
        .get_property_value(prop)
        .unwrap()
        .trim()
        .to_lowercase()
}

#[wasm_bindgen_test]
async fn ivoire_uses_the_candy_palette() {
    let style = inject_styles();
    let root = web_sys::window()
        .unwrap()
        .document()
        .unwrap()
        .document_element()
        .unwrap();
    let flux = mount_view(flux_fixture(1200.0), EnergyView::Flux);
    let classic = mount_view(flux_fixture(1200.0), EnergyView::Classic);
    tick().await;
    let solar = bar(&flux, "solar");
    let tone = classic
        .query_selector(".energy-tone--solar")
        .unwrap()
        .expect("a solar tone element");
    // Night Glass: unchanged.
    root.remove_attribute("data-theme").unwrap();
    tick().await;
    assert_eq!(computed(&solar, "fill"), "rgb(255, 196, 87)");
    assert_eq!(computed(&tone, "--tone"), "#ffc457");
    // Ivoire: Candy.
    root.set_attribute("data-theme", "light").unwrap();
    tick().await;
    assert_eq!(computed(&solar, "fill"), "rgb(255, 178, 31)");
    assert_eq!(computed(&tone, "--tone"), "#ffb21f");
    root.remove_attribute("data-theme").unwrap();
    style.remove();
}

fn orbite_arc(host: &Element, id: &str) -> Element {
    host.query_selector(&format!(".energy-orbite__arc[data-id='{id}']"))
        .unwrap()
        .unwrap_or_else(|| panic!("no arc {id}"))
}

fn orbite_row(host: &Element, id: &str) -> Element {
    host.query_selector(&format!(".energy-orbite__row[data-id='{id}']"))
        .unwrap()
        .unwrap_or_else(|| panic!("no row {id}"))
}

#[wasm_bindgen_test]
async fn orbite_replaces_the_diagram() {
    let host = mount_view(flux_fixture(1200.0), EnergyView::Orbite);
    tick().await;
    assert_eq!(count(&host, ".energy-orbite__svg"), 1);
    assert_eq!(count(&host, ".energy-flux"), 0);
    assert_eq!(count(&host, ".energy-diagram__svg"), 0);
    for id in ["solar", "grid", "consumer:oven", "consumer:pump"] {
        let arc = orbite_arc(&host, id);
        assert_eq!(arc.get_attribute("aria-hidden").as_deref(), Some("true"));
        assert!(
            arc.get_attribute("stroke")
                .unwrap()
                .starts_with("var(--energy-")
        );
    }
    assert_eq!(count(&host, ".energy-orbite__dots"), 1);
    assert_eq!(count(&host, ".energy-day"), 1);
    assert!(count(&host, ".energy-card") > 0);
}

#[wasm_bindgen_test]
async fn orbite_dots_move() {
    let host = mount_view(flux_fixture(1200.0), EnergyView::Orbite);
    let wait = |ms| gloo_timers::future::TimeoutFuture::new(ms);
    wait(100).await;
    let layer = host
        .query_selector(".energy-orbite__dots")
        .unwrap()
        .expect("dots layer");
    assert!(count(&layer, "circle") > 0, "dots drawn");
    let cx = || {
        layer
            .query_selector("circle")
            .unwrap()
            .and_then(|c| c.get_attribute("cx"))
    };
    let first = cx().unwrap();
    wait(200).await;
    assert_ne!(cx().unwrap(), first, "dots move");
}

#[wasm_bindgen_test]
async fn orbite_dots_stop_after_unmount() {
    let house = House::new();
    for e in flux_fixture(1200.0) {
        house.apply(WsEvent::EntityUpserted(e));
    }
    let host = mount_house(house, RwSignal::new(Lang::En), EnergyView::Orbite);
    let wait = |ms| gloo_timers::future::TimeoutFuture::new(ms);
    wait(100).await;
    let layer = host
        .query_selector(".energy-orbite__dots")
        .unwrap()
        .expect("dots layer");
    let cx = || {
        layer
            .query_selector("circle")
            .unwrap()
            .and_then(|c| c.get_attribute("cx"))
    };
    assert!(cx().is_some(), "dots drawn");
    house
        .home_settings
        .update(|s| s.energy_view = EnergyView::Flux);
    wait(50).await;
    assert_eq!(count(&host, ".energy-orbite__dots"), 0);
    let parked = cx();
    wait(200).await;
    assert_eq!(cx(), parked, "the loop stops on unmount");
}

#[wasm_bindgen_test]
async fn orbite_rows_show_the_flux_values() {
    let flux = mount_view(flux_fixture(1200.0), EnergyView::Flux);
    tick().await;
    let labels = flux.query_selector_all(".energy-flux__label").unwrap();
    let values: Vec<String> = (0..labels.length())
        .map(|i| {
            let el: Element = labels.item(i).unwrap().unchecked_into();
            text(&el, ".energy-flux__value")
        })
        .collect();
    let host = mount_view(flux_fixture(1200.0), EnergyView::Orbite);
    tick().await;
    let rows = host.query_selector_all(".energy-orbite__row").unwrap();
    assert!(rows.length() >= 4);
    // Every row's value is a value Flux labels a node with.
    for i in 0..rows.length() {
        let el: Element = rows.item(i).unwrap().unchecked_into();
        let shown = text(&el, ".energy-orbite__val");
        assert!(values.contains(&shown), "{shown} not in {values:?}");
    }
}

#[wasm_bindgen_test]
async fn orbite_centre_shows_house_and_solar_share() {
    let flux = mount_view(flux_fixture(1200.0), EnergyView::Flux);
    tick().await;
    let chip = text(&flux, ".energy-flux__chip b");
    let host = mount_view(flux_fixture(1200.0), EnergyView::Orbite);
    tick().await;
    assert_eq!(text(&host, ".energy-orbite__house"), chip);
    let mix = text(&host, ".energy-orbite__mix");
    assert!(mix.contains('%') && mix.contains("solar"), "{mix}");
}

#[wasm_bindgen_test]
async fn orbite_opens_a_sensor() {
    let host = mount_view(flux_fixture(1200.0), EnergyView::Orbite);
    tick().await;
    let path = || web_sys::window().unwrap().location().pathname().unwrap();
    orbite_row(&host, "consumer:oven")
        .dispatch_event(&Event::new("click").unwrap())
        .unwrap();
    tick().await;
    assert_eq!(path(), "/entity/oven");
    // The arc opens its own sensor too.
    orbite_arc(&host, "consumer:pump")
        .dispatch_event(&Event::new("click").unwrap())
        .unwrap();
    tick().await;
    assert_eq!(path(), "/entity/pump");
}

#[wasm_bindgen_test]
async fn orbite_others_toggle_the_list() {
    let mut entities = vec![
        sensor("solar", "Solar power", "W", 5000.0, role("solar")),
        sensor("grid", "Grid power", "W", 600.0, role("grid")),
    ];
    for i in 0..10 {
        entities.push(sensor(
            &format!("c{i}"),
            &format!("Load {i}"),
            "W",
            500.0 - 20.0 * i as f64,
            role("consumer"),
        ));
    }
    let host = mount_view(entities, EnergyView::Orbite);
    tick().await;
    assert_eq!(count(&host, ".energy-more"), 0);
    orbite_row(&host, "others")
        .dispatch_event(&Event::new("click").unwrap())
        .unwrap();
    tick().await;
    assert_eq!(count(&host, ".energy-more"), 1);
}

#[wasm_bindgen_test]
async fn orbite_flips_grid_to_export_in_place() {
    let house = House::new();
    for e in flux_fixture(1200.0) {
        house.apply(WsEvent::EntityUpserted(e));
    }
    let host = mount_house(house, RwSignal::new(Lang::En), EnergyView::Orbite);
    tick().await;
    let inner = ".energy-orbite__arc[data-id='grid']";
    assert_eq!(count(&host, inner), 1);
    // Core's convention: a negative grid reading is power sent to the grid.
    house.apply(WsEvent::EntityUpserted(sensor(
        "grid",
        "Grid power",
        "W",
        -500.0,
        role("grid"),
    )));
    tick().await;
    assert_eq!(count(&host, inner), 0);
    assert_eq!(count(&host, ".energy-orbite__arc[data-id='export']"), 1);
    assert_eq!(count(&host, ".energy-orbite__arc[data-id='solar']"), 1);
    assert_eq!(count(&host, ".energy-orbite__row[data-id='export']"), 1);
}

#[wasm_bindgen_test]
async fn orbite_narrow_stacks_lists_under_the_ring() {
    let style = inject_styles();
    let map: std::collections::BTreeMap<String, EntityView> = flux_fixture(1200.0)
        .into_iter()
        .map(|e| (e.id.clone(), e))
        .collect();
    let ui = Memo::new(move |_| {
        nidoo_core::energy::EnergyFlow::build(&nidoo_web::energy::energy_entities(&map), 0, None)
    });
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    host.set_attribute("style", "width: 390px").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    std::mem::forget(leptos::mount::mount_to(
        host.clone().unchecked_into(),
        move || {
            provide_context(RwSignal::new(Lang::En));
            view! {
                <nidoo_web::screens::energy_orbite::OrbiteDiagram
                    ui=ui
                    narrow=true
                    open=Callback::new(|_: String| {})
                    expanded=RwSignal::new(false)
                />
            }
        },
    ));
    tick().await;
    assert_eq!(count(&host, ".energy-orbite--narrow"), 1);
    let svg = host.query_selector(".energy-orbite__svg").unwrap().unwrap();
    let list = host
        .query_selector(".energy-orbite__list--in")
        .unwrap()
        .unwrap();
    assert!(list.get_bounding_client_rect().top() >= svg.get_bounding_client_rect().bottom() - 1.0);
    style.remove();
    host.remove();
}

#[wasm_bindgen_test]
async fn orbite_follows_the_language() {
    let lang = RwSignal::new(Lang::En);
    let host = mount_with(flux_fixture(1200.0), lang, EnergyView::Orbite);
    tick().await;
    let all = host.text_content().unwrap_or_default();
    assert!(
        all.contains("Coming in") && all.contains("Going out"),
        "{all}"
    );
    lang.set(Lang::Fr);
    tick().await;
    let all = host.text_content().unwrap_or_default();
    assert!(all.contains("Entrant") && all.contains("Sortant"), "{all}");
}

#[wasm_bindgen_test]
async fn an_unrelated_home_setting_keeps_the_diagram() {
    let house = House::new();
    for e in flux_fixture(1200.0) {
        house.apply(WsEvent::EntityUpserted(e));
    }
    let host = mount_house(house, RwSignal::new(Lang::En), EnergyView::Orbite);
    tick().await;
    let before = host.query_selector(".energy-orbite").unwrap().unwrap();
    house.home_settings.update(|s| s.eco_setpoint = Some(17.0));
    tick().await;
    let after = host.query_selector(".energy-orbite").unwrap().unwrap();
    assert!(before.is_same_node(Some(&after)));
    // A real change of view still swaps it.
    house
        .home_settings
        .update(|s| s.energy_view = EnergyView::Flux);
    tick().await;
    assert_eq!(count(&host, ".energy-orbite"), 0);
    assert_eq!(count(&host, ".energy-flux"), 1);
}

#[wasm_bindgen_test]
async fn orbite_arcs_are_dashes_of_a_full_circle() {
    let host = mount_view(flux_fixture(1200.0), EnergyView::Orbite);
    tick().await;
    let arc = orbite_arc(&host, "solar");
    assert_eq!(arc.tag_name().to_lowercase(), "circle");
    let style = arc.get_attribute("style").unwrap();
    assert!(style.contains("stroke-dasharray") && style.contains("stroke-dashoffset"));
}
