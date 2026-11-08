#![cfg(target_arch = "wasm32")]

use std::cell::Cell;
use std::rc::Rc;

use gloo_timers::future::TimeoutFuture;
use leptos::prelude::*;
use leptos::task::tick;
use nidoo_api::{AreaView, EntityView, WsEvent};
use nidoo_core::model::{Capabilities, EntityCategory, EntityState, Kind, StateValue, UiOverride};
use nidoo_web::components::now::Now;
use nidoo_web::entity_picker::needs::{is_thermometer, keep};
use nidoo_web::entity_picker::view::{EntityPicker, Mode, Pinned};
use nidoo_web::entity_picker::{EntityPickField, EntityPickListField, Suggest};
use nidoo_web::house::House;
use nidoo_web::i18n::Lang;
use nidoo_web::i18n::picker::PickerText;
use nidoo_web::screens::alarm_settings::AlarmSettingsPage;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_test::*;
wasm_bindgen_test_configure!(run_in_browser);

#[path = "support/fetch_stub.rs"]
mod fetch_stub;
use fetch_stub::FetchStub;

fn entity(id: &str, name: &str, device: &str, kind: Kind, area: &str) -> EntityView {
    EntityView {
        id: id.into(),
        device_id: None,
        name: name.into(),
        device_name: (!device.is_empty()).then(|| device.to_owned()),
        label: None,
        kind,
        component: kind.component().to_string(),
        protocol_kind: kind,
        device_class: None,
        unit: None,
        icon: None,
        category: EntityCategory::Primary,
        enabled_by_default: true,
        area: area.into(),
        part_of: None,
        capabilities: Capabilities::default(),
        ui: UiOverride::default(),
        state: EntityState::default(),
    }
}

fn probe(value: f64) -> EntityView {
    let mut e = entity("t", "Temp", "Probe", Kind::Sensor, "hall");
    e.device_class = Some("temperature".into());
    e.unit = Some("°C".into());
    e.state.value = StateValue::Number(value);
    e.state.last_update = Some(1_700_000_000);
    e
}

fn area(id: &str, name: &str) -> AreaView {
    AreaView {
        id: id.into(),
        name: name.into(),
        icon: None,
        order: None,
        entity_count: 1,
        primary_count: 1,
        group_names: Default::default(),
        group_favorites: Default::default(),
    }
}

struct Fx {
    host: web_sys::Element,
    open: RwSignal<bool>,
    picked: RwSignal<Vec<Option<String>>>,
    house: House,
}

fn mount(mode: Mode, current: Vec<&str>, suggest: bool, pinned: bool) -> Fx {
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    let house = House::new();
    for e in [
        entity("a", "Mode", "Plug A", Kind::Switch, "kitchen"),
        entity("b", "Mode", "Plug B", Kind::Switch, "kitchen"),
        probe(20.0),
    ] {
        house.apply(WsEvent::EntityUpserted(e));
    }
    house.apply(WsEvent::AreasChanged {
        areas: vec![area("kitchen", "Kitchen"), area("hall", "Hall")],
    });
    let open = RwSignal::new(true);
    let picked = RwSignal::new(Vec::new());
    let current: Vec<String> = current.into_iter().map(str::to_owned).collect();
    let mounted = leptos::mount::mount_to(host.clone().unchecked_into(), move || {
        provide_context(house);
        provide_context(RwSignal::new(Lang::En));
        provide_context(Now(RwSignal::new(1_700_000_060)));
        let suggest = suggest.then(|| Suggest {
            text: PickerText::Thermometers,
            keep: keep(is_thermometer),
        });
        let pinned = pinned.then(|| Pinned {
            text: Signal::derive(|| "Nothing".to_string()),
        });
        view! {
            <EntityPicker
                open=open
                title="Pick"
                mode=mode
                current=Signal::derive(move || current.clone())
                suggest=suggest
                pinned=pinned
                on_pick=Callback::new(move |id| picked.update(|p| p.push(id)))
            />
        }
    });
    mounted.forget();
    Fx {
        host,
        open,
        picked,
        house,
    }
}

impl Fx {
    fn dialog(&self) -> web_sys::HtmlDialogElement {
        self.host
            .query_selector("dialog.picker")
            .unwrap()
            .unwrap()
            .unchecked_into()
    }

    fn options(&self) -> Vec<web_sys::Element> {
        let list = self.host.query_selector_all("[role=option]").unwrap();
        (0..list.length())
            .map(|i| list.item(i).unwrap().unchecked_into())
            .collect()
    }

    fn option(&self, id: &str) -> web_sys::HtmlElement {
        self.host
            .query_selector(&format!("[id$='-opt-{id}']"))
            .unwrap()
            .unwrap_or_else(|| panic!("no row {id}"))
            .unchecked_into()
    }

    fn pinned(&self) -> web_sys::HtmlElement {
        self.host
            .query_selector("[id$=-pin]")
            .unwrap()
            .expect("pinned row")
            .unchecked_into()
    }

    fn search(&self) -> web_sys::HtmlInputElement {
        self.host
            .query_selector("input[type=search]")
            .unwrap()
            .unwrap()
            .unchecked_into()
    }

    async fn type_query(&self, q: &str) {
        let input = self.search();
        input.set_value(q);
        let ev = web_sys::Event::new_with_event_init_dict("input", &{
            let i = web_sys::EventInit::new();
            i.set_bubbles(true);
            i
        })
        .unwrap();
        input.dispatch_event(&ev).unwrap();
        tick().await;
    }

    async fn key(&self, key: &str) {
        let init = web_sys::KeyboardEventInit::new();
        init.set_key(key);
        init.set_bubbles(true);
        init.set_cancelable(true);
        let ev =
            web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
        self.search().dispatch_event(&ev).unwrap();
        tick().await;
    }

    async fn finish(self) {
        self.open.set(false);
        tick().await;
        TimeoutFuture::new(30).await;
    }
}

fn pathname() -> String {
    web_sys::window().unwrap().location().pathname().unwrap()
}

/// Counts every `popstate` from now on.
fn count_popstates() -> Rc<Cell<u32>> {
    let n = Rc::new(Cell::new(0));
    let seen = n.clone();
    let cb = Closure::<dyn FnMut()>::new(move || seen.set(seen.get() + 1));
    web_sys::window()
        .unwrap()
        .add_event_listener_with_callback("popstate", cb.as_ref().unchecked_ref())
        .unwrap();
    cb.forget();
    n
}

#[wasm_bindgen_test]
async fn opens_modal_with_search_focused() {
    let fx = mount(Mode::Single, vec![], false, false);
    tick().await;
    tick().await;
    assert!(fx.dialog().open());
    let active = web_sys::window()
        .unwrap()
        .document()
        .unwrap()
        .active_element();
    assert_eq!(
        active.unwrap(),
        fx.search().unchecked_into::<web_sys::Element>()
    );
    fx.finish().await;
}

#[wasm_bindgen_test]
async fn typing_filters_rows() {
    let fx = mount(Mode::Single, vec![], false, true);
    tick().await;
    assert_eq!(fx.options().len(), 4, "pinned + three rows");
    fx.type_query("plug b").await;
    let rows = fx.options();
    assert_eq!(rows.len(), 2, "pinned + one match");
    assert!(rows[1].id().ends_with("-opt-b"));
    fx.finish().await;
}

#[wasm_bindgen_test]
async fn single_pick_reports_and_closes() {
    let fx = mount(Mode::Single, vec![], false, false);
    tick().await;
    fx.option("b").click();
    tick().await;
    assert_eq!(fx.picked.get_untracked(), vec![Some("b".to_string())]);
    assert!(!fx.open.get_untracked());
    assert!(!fx.dialog().open());
    fx.finish().await;
}

#[wasm_bindgen_test]
async fn pinned_row_reports_none() {
    let fx = mount(Mode::Single, vec![], false, true);
    tick().await;
    assert!(fx.host.text_content().unwrap().contains("Nothing"));
    fx.pinned().click();
    tick().await;
    assert_eq!(fx.picked.get_untracked(), vec![None]);
    fx.finish().await;
}

#[wasm_bindgen_test]
async fn multi_toggles_and_stays_open() {
    let fx = mount(Mode::Multi, vec!["a", "b"], false, false);
    tick().await;
    assert!(fx.host.text_content().unwrap().contains("Done (2)"));
    assert_eq!(
        fx.option("a").get_attribute("aria-selected").as_deref(),
        Some("true")
    );
    assert_eq!(
        fx.option("t").get_attribute("aria-selected").as_deref(),
        Some("false")
    );
    fx.option("a").click();
    fx.option("t").click();
    tick().await;
    assert_eq!(
        fx.picked.get_untracked(),
        vec![Some("a".to_string()), Some("t".to_string())]
    );
    assert!(fx.open.get_untracked());
    assert!(fx.dialog().open());
    fx.finish().await;
}

#[wasm_bindgen_test]
async fn escape_closes() {
    let fx = mount(Mode::Single, vec![], false, false);
    tick().await;
    let ev = web_sys::Event::new_with_event_init_dict("cancel", &{
        let i = web_sys::EventInit::new();
        i.set_cancelable(true);
        i
    })
    .unwrap();
    fx.dialog().dispatch_event(&ev).unwrap();
    tick().await;
    assert!(!fx.open.get_untracked());
    assert!(!fx.dialog().open());
    fx.finish().await;
}

#[wasm_bindgen_test]
async fn arrows_and_enter_pick() {
    let fx = mount(Mode::Single, vec![], false, false);
    tick().await;
    fx.key("ArrowDown").await;
    fx.key("ArrowDown").await;
    let id = fx.search().get_attribute("aria-activedescendant").unwrap();
    assert!(id.ends_with("-opt-b"), "{id}");
    fx.key("Enter").await;
    assert_eq!(fx.picked.get_untracked(), vec![Some("b".to_string())]);
    fx.finish().await;
}

#[wasm_bindgen_test]
async fn echo_keeps_highlight() {
    let fx = mount(Mode::Single, vec![], false, false);
    tick().await;
    for _ in 0..3 {
        fx.key("ArrowDown").await;
    }
    let before = fx.search().get_attribute("aria-activedescendant").unwrap();
    assert!(before.ends_with("-opt-t"), "{before}");
    assert!(fx.option("t").text_content().unwrap().contains("20"));
    fx.house.apply(WsEvent::EntityUpserted(probe(21.5)));
    tick().await;
    let after = fx.search().get_attribute("aria-activedescendant").unwrap();
    assert_eq!(before, after);
    assert!(fx.option("t").text_content().unwrap().contains("21.5"));
    fx.finish().await;
}

#[wasm_bindgen_test]
async fn suggested_chip_then_all() {
    let fx = mount(Mode::Single, vec![], true, false);
    tick().await;
    assert_eq!(fx.options().len(), 1);
    let chips = fx.host.query_selector_all(".chip").unwrap();
    let all: web_sys::HtmlElement = (0..chips.length())
        .map(|i| {
            chips
                .item(i)
                .unwrap()
                .unchecked_into::<web_sys::HtmlElement>()
        })
        .find(|c| c.text_content().unwrap() == "All")
        .expect("All chip");
    all.click();
    tick().await;
    assert_eq!(fx.options().len(), 3);
    fx.finish().await;
}

#[wasm_bindgen_test]
async fn history_back_closes_picker() {
    let path = pathname();
    let fx = mount(Mode::Single, vec![], false, false);
    tick().await;
    let pops = count_popstates();
    let before = pops.get();
    web_sys::window()
        .unwrap()
        .history()
        .unwrap()
        .back()
        .unwrap();
    for _ in 0..40 {
        if pops.get() > before {
            break;
        }
        TimeoutFuture::new(25).await;
    }
    tick().await;
    assert!(!fx.open.get_untracked());
    assert!(!fx.dialog().open());
    assert_eq!(pathname(), path);
    fx.finish().await;
}

#[wasm_bindgen_test]
async fn escape_pops_the_pushed_entry() {
    let path = pathname();
    let fx = mount(Mode::Single, vec![], false, false);
    tick().await;
    let pops = count_popstates();
    let before = pops.get();
    let ev = web_sys::Event::new_with_event_init_dict("cancel", &{
        let i = web_sys::EventInit::new();
        i.set_cancelable(true);
        i
    })
    .unwrap();
    fx.dialog().dispatch_event(&ev).unwrap();
    for _ in 0..40 {
        if pops.get() > before {
            break;
        }
        TimeoutFuture::new(25).await;
    }
    TimeoutFuture::new(100).await;
    assert_eq!(pops.get() - before, 1);
    assert!(!fx.open.get_untracked());
    assert_eq!(pathname(), path);
    fx.finish().await;
}

#[wasm_bindgen_test]
async fn two_pickers_have_distinct_ids() {
    let one = mount(Mode::Single, vec![], false, false);
    let two = mount(Mode::Single, vec![], false, false);
    tick().await;
    let doc = web_sys::window().unwrap().document().unwrap();
    let mut titles = Vec::new();
    for fx in [&one, &two] {
        let label = fx.dialog().get_attribute("aria-labelledby").unwrap();
        let title = doc.get_element_by_id(&label).expect("title resolves");
        assert!(
            fx.host.contains(Some(&title)),
            "label points at its own title"
        );
        titles.push(label);
    }
    assert_ne!(titles[0], titles[1]);
    let list = |fx: &Fx| {
        fx.host
            .query_selector("[role=listbox]")
            .unwrap()
            .unwrap()
            .id()
    };
    assert_ne!(list(&one), list(&two));
    let controls = one.search().get_attribute("aria-controls").unwrap();
    assert_eq!(controls, list(&one));
    one.finish().await;
    two.finish().await;
}

fn fixture_house() -> House {
    let house = House::new();
    for e in [
        entity("a", "Mode", "Plug A", Kind::Switch, "kitchen"),
        entity("b", "Mode", "Plug B", Kind::Switch, "kitchen"),
    ] {
        house.apply(WsEvent::EntityUpserted(e));
    }
    house.apply(WsEvent::AreasChanged {
        areas: vec![area("kitchen", "Kitchen")],
    });
    house
}

fn mount_view<V: IntoView + 'static>(
    house: House,
    f: impl FnOnce() -> V + 'static,
) -> web_sys::Element {
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    let mounted = leptos::mount::mount_to(host.clone().unchecked_into(), move || {
        provide_context(house);
        provide_context(RwSignal::new(Lang::En));
        provide_context(Now(RwSignal::new(1_700_000_060)));
        f()
    });
    mounted.forget();
    host
}

fn field_text(value: Option<&'static str>, pinned: bool) -> String {
    let host = mount_view(fixture_house(), move || {
        let pinned = pinned.then(|| Signal::derive(|| "Nothing".to_string()));
        view! {
            <EntityPickField
                label="Plug"
                value=Signal::derive(move || value.map(str::to_owned))
                on_change=Callback::new(|_| {})
                pinned=pinned
            />
        }
    });
    host.query_selector("button.picker-field")
        .unwrap()
        .expect("field button")
        .text_content()
        .unwrap()
}

#[wasm_bindgen_test]
async fn closed_field_shows_row_with_device_and_room() {
    let text = field_text(Some("a"), false);
    tick().await;
    for want in ["Mode", "Plug A", "Kitchen"] {
        assert!(text.contains(want), "{want} missing from {text:?}");
    }
}

#[wasm_bindgen_test]
async fn missing_value_shows_raw_id() {
    let text = field_text(Some("gone"), false);
    assert!(text.contains("gone"), "{text:?}");
    assert!(text.contains("missing"), "{text:?}");
}

#[wasm_bindgen_test]
async fn pinned_text_when_none() {
    assert!(field_text(None, true).contains("Nothing"));
    assert!(field_text(None, false).contains("Choose an entity"));
}

#[wasm_bindgen_test]
async fn list_field_ticks_add_and_remove() {
    let value = RwSignal::new(Vec::<String>::new());
    let seen = RwSignal::new(Vec::<Vec<String>>::new());
    let host = mount_view(fixture_house(), move || {
        view! {
            <EntityPickListField
                label="Plugs"
                value=value
                on_change=Callback::new(move |v: Vec<String>| {
                    seen.update(|s| s.push(v.clone()));
                    value.set(v);
                })
            />
        }
    });
    tick().await;
    let add: web_sys::HtmlElement = host
        .query_selector("button.chip--add")
        .unwrap()
        .expect("add button")
        .unchecked_into();
    add.click();
    tick().await;
    let row = |id: &str| -> web_sys::HtmlElement {
        host.query_selector(&format!("[id$='-opt-{id}']"))
            .unwrap()
            .unwrap_or_else(|| panic!("no row {id}"))
            .unchecked_into()
    };
    for id in ["a", "b", "a"] {
        row(id).click();
        tick().await;
    }
    let s = |ids: &[&str]| ids.iter().map(|i| i.to_string()).collect::<Vec<_>>();
    assert_eq!(
        seen.get_untracked(),
        vec![s(&["a"]), s(&["a", "b"]), s(&["b"])]
    );
    let chips = host
        .query_selector_all(".picker-chips .picker-chip")
        .unwrap();
    assert_eq!(chips.length(), 1, "one chip left");
}

fn alarm_body() -> String {
    serde_json::json!({
        "enabled": true,
        "entity_id": "alarm_panel",
        "state": "disarmed",
        "mode": null,
        "source": null,
        "since": 0,
        "deadline": null,
        "cause": [],
        "open_at_arm": [],
        "settings": serde_json::to_value(nidoo_alarm::doc::AlarmSettings::default()).unwrap(),
        "settings_invalid": null,
        "warnings": [],
    })
    .to_string()
}

#[wasm_bindgen_test]
async fn alarm_adds_siren() {
    let stub = FetchStub::install();
    stub.reply("GET", "/api/v1/alarm", 200, &alarm_body());
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    let house = House::new();
    house.apply(WsEvent::EntityUpserted(entity(
        "s1",
        "Siren",
        "Hall siren",
        Kind::Switch,
        "hall",
    )));
    leptos::mount::mount_to(host.clone().unchecked_into(), move || {
        provide_context(house);
        provide_context(RwSignal::new(Lang::En));
        provide_context(Now(RwSignal::new(1_700_000_060)));
        view! { <AlarmSettingsPage /> }
    })
    .forget();
    for _ in 0..40 {
        tick().await;
        if host.query_selector(".alarm-section").unwrap().is_some() {
            break;
        }
        TimeoutFuture::new(25).await;
    }
    let add: web_sys::HtmlElement = (0..host
        .query_selector_all("button.chip--add")
        .unwrap()
        .length())
        .map(|i| {
            host.query_selector_all("button.chip--add")
                .unwrap()
                .item(i)
                .unwrap()
                .unchecked_into::<web_sys::HtmlElement>()
        })
        .find(|b| b.text_content().unwrap().contains("siren"))
        .expect("Add siren button");
    add.click();
    tick().await;
    tick().await;
    let row = || {
        host.query_selector("[id$='-opt-s1']")
            .unwrap()
            .expect("siren row")
            .unchecked_into::<web_sys::HtmlElement>()
    };
    let rows = || host.query_selector_all(".alarm-row").unwrap().length();
    assert_eq!(rows(), 0);
    row().click();
    tick().await;
    assert_eq!(rows(), 1, "ticked siren is in the draft");
    assert!(host.text_content().unwrap().contains("Hall siren"));
    let dialog: web_sys::HtmlDialogElement = row()
        .closest("dialog")
        .unwrap()
        .expect("the siren picker's dialog")
        .unchecked_into();
    assert!(dialog.open(), "picker stays open");
    row().click();
    tick().await;
    assert_eq!(rows(), 0, "ticked again removes it");
    drop(stub);
}

fn counter_entity(id: &str, name: &str) -> EntityView {
    let mut e = entity(id, name, "", Kind::Sensor, "");
    e.unit = Some("kWh".into());
    e
}

#[wasm_bindgen_test]
async fn counter_automatic_and_pick() {
    use nidoo_web::components::toast::Toasts;
    use nidoo_web::override_editor::OverrideEditor;

    let house = House::new();
    let mut power = entity("mains", "Mains power", "", Kind::Sensor, "");
    power.unit = Some("W".into());
    power.ui.energy_role = Some("grid".into());
    power.ui.energy_meter = Some("meter_b".into());
    for e in [
        power.clone(),
        counter_entity("meter_a", "Meter A"),
        counter_entity("meter_b", "Meter B"),
        entity("lamp", "Lamp", "", Kind::Light, ""),
    ] {
        house.apply(WsEvent::EntityUpserted(e));
    }
    let host = mount_view(house, move || {
        provide_context(Toasts::new());
        view! { <OverrideEditor entity=power /> }
    });
    tick().await;
    let field = || {
        host.query_selector("button.picker-field")
            .unwrap()
            .expect("counter field")
            .unchecked_into::<web_sys::HtmlElement>()
    };
    assert!(field().text_content().unwrap().contains("Meter B"));
    field().click();
    tick().await;
    let has = |id: &str| {
        host.query_selector(&format!("[id$='-opt-{id}']"))
            .unwrap()
            .is_some()
    };
    assert!(has("meter_a") && has("meter_b"), "counters are offered");
    assert!(!has("lamp") && !has("mains"), "only counters are offered");
    let pin: web_sys::HtmlElement = host
        .query_selector("[id$=-pin]")
        .unwrap()
        .expect("Automatic row")
        .unchecked_into();
    pin.click();
    tick().await;
    assert!(field().text_content().unwrap().contains("Automatic"));
    TimeoutFuture::new(100).await;
    field().click();
    tick().await;
    let a: web_sys::HtmlElement = host
        .query_selector("[id$='-opt-meter_a']")
        .unwrap()
        .expect("meter row")
        .unchecked_into();
    a.click();
    tick().await;
    assert!(field().text_content().unwrap().contains("Meter A"));
    TimeoutFuture::new(100).await;
}
