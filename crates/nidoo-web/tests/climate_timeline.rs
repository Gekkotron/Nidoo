#![cfg(target_arch = "wasm32")]

use leptos::prelude::*;
use leptos::task::tick;
use nidoo_climate::doc::{Block, Day, Evidence, HhMm, ModeRules, Setpoint};
use nidoo_web::climate::timeline::WeekTimeline;
use nidoo_web::i18n::Lang;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
use web_sys::{Element, HtmlElement, PointerEvent, PointerEventInit};
wasm_bindgen_test_configure!(run_in_browser);

fn block(days: &[Day], from: &str, to: &str, t: f64) -> Block {
    Block {
        days: days.to_vec(),
        from: HhMm::parse(from).unwrap(),
        to: HhMm::parse(to).unwrap(),
        setpoint: Setpoint::Temp(t),
        evidence: None,
    }
}

/// Tests load no stylesheet: the columns need a height for the vertical pointer mapping.
fn install_layout() {
    let document = web_sys::window().unwrap().document().unwrap();
    if document.get_element_by_id("timeline-test-layout").is_some() {
        return;
    }
    let style = document.create_element("style").unwrap();
    style.set_id("timeline-test-layout");
    style.set_text_content(Some(
        ".climate-timeline__col { position: relative; height: 480px; } \
         .climate-timeline__bar { position: absolute; left: 0; right: 0; }",
    ));
    document.head().unwrap().append_child(&style).unwrap();
}

async fn mount(rules: RwSignal<ModeRules>) -> Element {
    mount_with(rules, true, None).await
}

async fn mount_with(
    rules: RwSignal<ModeRules>,
    editable: bool,
    now: Option<(Day, u16)>,
) -> Element {
    mount_in(rules, editable, now, RwSignal::new(Lang::En)).await
}

/// [`mount_with`] under the language signal the test flips.
async fn mount_in(
    rules: RwSignal<ModeRules>,
    editable: bool,
    now: Option<(Day, u16)>,
    lang: RwSignal<Lang>,
) -> Element {
    install_layout();
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    host.set_attribute("style", "width: 700px").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    let mounted = leptos::mount::mount_to(host.clone().unchecked_into(), move || {
        provide_context(lang);
        match now {
            Some(now) => view! {
                <WeekTimeline
                    rules=rules.into()
                    editable=editable
                    on_change=Callback::new(move |next| rules.set(next))
                    now=Signal::from(now)
                />
            }
            .into_any(),
            None => view! {
                <WeekTimeline
                    rules=rules.into()
                    editable=editable
                    on_change=Callback::new(move |next| rules.set(next))
                />
            }
            .into_any(),
        }
    });
    // Keep the view alive for the rest of the test.
    std::mem::forget(mounted);
    tick().await;
    host
}

/// The column of `day` (columns go Mon to Sun).
fn col(host: &Element, day: Day) -> Element {
    let index = Day::ALL.iter().position(|d| *d == day).unwrap() as u32;
    host.query_selector_all(".climate-timeline__col")
        .unwrap()
        .item(index)
        .unwrap()
        .unchecked_into()
}

/// Clicks `target` at `fraction` of its height (a browser click is a `PointerEvent`).
fn click_at(target: &Element, fraction: f64) {
    let rect = target.get_bounding_client_rect();
    let init = PointerEventInit::new();
    init.set_bubbles(true);
    init.set_client_x((rect.left() + rect.width() / 2.0) as i32);
    init.set_client_y((rect.top() + rect.height() * fraction) as i32);
    let event = PointerEvent::new_with_event_init_dict("click", &init).unwrap();
    target.dispatch_event(&event).unwrap();
}

/// Dispatches a pointer event of `kind` on `target` at `y` px below the top of `column`.
fn pointer(kind: &str, target: &Element, column: &Element, y: f64) {
    let rect = column.get_bounding_client_rect();
    let init = PointerEventInit::new();
    init.set_bubbles(true);
    init.set_pointer_id(1);
    init.set_client_x((rect.left() + rect.width() / 2.0) as i32);
    init.set_client_y((rect.top() + y) as i32);
    let event = PointerEvent::new_with_event_init_dict(kind, &init).unwrap();
    target.dispatch_event(&event).unwrap();
}

/// The `edge` ("from" or "to") handle of the `n`th bar in the Monday column.
fn monday_handle(host: &Element, n: u32, edge: &str) -> Element {
    col(host, Day::Mon)
        .query_selector_all(".climate-timeline__bar")
        .unwrap()
        .item(n)
        .expect("Monday bar")
        .unchecked_into::<Element>()
        .query_selector(&format!(".climate-timeline__handle--{edge}"))
        .unwrap()
        .expect("handle")
}

fn chip(host: &Element, label: &str) -> HtmlElement {
    let chips = host
        .query_selector_all(".climate-timeline__days .chip")
        .unwrap();
    (0..chips.length())
        .filter_map(|i| chips.item(i))
        .map(|n| n.unchecked_into::<HtmlElement>())
        .find(|c| c.text_content().as_deref() == Some(label))
        .expect("day chip")
}

#[wasm_bindgen_test]
async fn tapping_an_empty_spot_adds_a_block_on_that_day() {
    let rules = RwSignal::new(ModeRules {
        base: Setpoint::Temp(15.5),
        blocks: vec![block(&[Day::Sat], "00:00", "07:30", 18.0)],
    });
    let host = mount(rules).await;

    click_at(&col(&host, Day::Sun), 0.75);
    tick().await;
    let added = rules
        .with_untracked(|r| r.blocks.get(1).cloned())
        .expect("new block");
    assert_eq!(added.days, vec![Day::Sun]);
    assert_eq!((added.from, added.to), (HhMm(18 * 60), HhMm(20 * 60)));
    assert_eq!(added.setpoint, Setpoint::Temp(17.5));
    assert!(
        host.query_selector(".climate-timeline__editor")
            .unwrap()
            .is_some(),
        "the new block's popover opens"
    );

    // A click on a bar opens it instead of adding a block.
    let bar = col(&host, Day::Sat)
        .query_selector(".climate-timeline__bar")
        .unwrap()
        .expect("Saturday bar");
    click_at(&bar, 0.5);
    tick().await;
    assert_eq!(rules.with_untracked(|r| r.blocks.len()), 2);
}

#[wasm_bindgen_test]
async fn day_chips_extend_a_block_to_another_day() {
    let rules = RwSignal::new(ModeRules {
        base: Setpoint::Temp(15.5),
        blocks: vec![
            block(
                &[Day::Mon, Day::Tue, Day::Wed, Day::Thu, Day::Fri],
                "00:00",
                "07:30",
                18.0,
            ),
            block(&[Day::Sat], "00:00", "07:30", 18.0),
        ],
    });
    let host = mount(rules).await;

    let bar: HtmlElement = col(&host, Day::Sat)
        .query_selector(".climate-timeline__bar")
        .unwrap()
        .expect("Saturday bar")
        .unchecked_into();
    bar.click();
    tick().await;

    // Saturday is the block's only day; Monday holds the weekday block at the same time.
    assert!(chip(&host, "Sat").has_attribute("disabled"));
    assert!(chip(&host, "Mon").has_attribute("disabled"));
    let sunday = chip(&host, "Sun");
    assert_eq!(
        sunday.get_attribute("aria-pressed").as_deref(),
        Some("false")
    );
    sunday.click();
    tick().await;

    assert_eq!(
        rules.with_untracked(|r| r.blocks[1].days.clone()),
        vec![Day::Sat, Day::Sun]
    );
    assert_eq!(
        chip(&host, "Sun").get_attribute("aria-pressed").as_deref(),
        Some("true")
    );
    assert!(
        col(&host, Day::Sun)
            .query_selector(".climate-timeline__bar")
            .unwrap()
            .is_some(),
        "Sunday shows the block"
    );
}

fn text(el: &Element, selector: &str) -> Option<String> {
    el.query_selector(selector)
        .unwrap()
        .and_then(|n| n.text_content())
}

#[wasm_bindgen_test]
async fn blocks_stand_in_their_day_column_by_time() {
    let rules = RwSignal::new(ModeRules {
        base: Setpoint::Temp(15.0),
        blocks: vec![
            block(&[Day::Mon], "07:15", "14:00", 18.5),
            block(&[Day::Mon], "14:00", "24:00", 16.0),
        ],
    });
    let host = mount_with(rules, false, None).await;

    let bars = col(&host, Day::Mon)
        .query_selector_all(".climate-timeline__bar")
        .unwrap();
    assert_eq!(bars.length(), 2);
    let first: Element = bars.item(0).unwrap().unchecked_into();
    let style = first.get_attribute("style").unwrap();
    assert!(style.contains("top:30.208%"), "{style}");
    assert!(style.contains("height:28.125%"), "{style}");
    assert!(first.class_list().contains("climate-temp--mid"));
    let second: Element = bars.item(1).unwrap().unchecked_into();
    assert!(second.class_list().contains("climate-temp--low"));
    assert_eq!(
        text(&second, ".climate-timeline__temp").as_deref(),
        Some("16°")
    );
    assert_eq!(
        text(&second, ".climate-timeline__times").as_deref(),
        Some("14:00 – 24:00")
    );
}

#[wasm_bindgen_test]
async fn now_line_sits_on_today() {
    let rules = RwSignal::new(ModeRules {
        base: Setpoint::Temp(15.0),
        blocks: vec![],
    });
    let host = mount_with(rules, false, Some((Day::Sat, 20 * 60 + 30))).await;

    let lines = host.query_selector_all(".climate-timeline__now").unwrap();
    assert_eq!(lines.length(), 1);
    let saturday = col(&host, Day::Sat);
    assert!(
        saturday
            .class_list()
            .contains("climate-timeline__col--today")
    );
    let line = saturday
        .query_selector(".climate-timeline__now")
        .unwrap()
        .expect("the line is in Saturday's column");
    let style = line.get_attribute("style").unwrap();
    assert!(style.contains("top:85.417%"), "{style}");
}

#[wasm_bindgen_test]
async fn tapping_a_block_when_viewing_shows_its_times() {
    let mut learned = block(&[Day::Mon], "07:15", "14:00", 18.5);
    learned.evidence = Some(Evidence { agree: 3, of: 5 });
    let rules = RwSignal::new(ModeRules {
        base: Setpoint::Temp(15.0),
        blocks: vec![learned],
    });
    let host = mount_with(rules, false, None).await;
    let peek = || text(&host, ".climate-timeline__peek");
    assert_eq!(peek(), None);

    let bar: HtmlElement = col(&host, Day::Mon)
        .query_selector(".climate-timeline__bar")
        .unwrap()
        .expect("Monday bar")
        .unchecked_into();
    bar.click();
    tick().await;
    assert_eq!(peek().as_deref(), Some("07:15 – 14:00 · 18.5° · 3/5"));

    bar.click();
    tick().await;
    assert_eq!(peek(), None);
    assert!(
        host.query_selector(".climate-timeline__editor")
            .unwrap()
            .is_none()
    );
}

fn input(host: &Element, selector: &str) -> web_sys::HtmlInputElement {
    host.query_selector(selector)
        .unwrap()
        .unwrap_or_else(|| panic!("{selector}"))
        .unchecked_into()
}

fn change(field: &web_sys::HtmlInputElement, value: &str) {
    field.set_value(value);
    field
        .dispatch_event(&web_sys::Event::new("change").unwrap())
        .unwrap();
}

#[wasm_bindgen_test]
async fn the_timeline_reads_and_parses_in_french_and_flips_back() {
    let lang = RwSignal::new(Lang::Fr);
    let rules = RwSignal::new(ModeRules {
        base: Setpoint::Temp(15.5),
        blocks: vec![block(&[Day::Mon], "07:00", "09:00", 18.5)],
    });
    let host = mount_in(rules, true, None, lang).await;
    let bar: HtmlElement = col(&host, Day::Mon)
        .query_selector(".climate-timeline__bar")
        .unwrap()
        .expect("Monday bar")
        .unchecked_into();
    bar.click();
    // The editor opens in one tick, its `prop:value` effects settle in the next.
    tick().await;
    tick().await;
    let text = || host.text_content().unwrap_or_default();

    // French words, French decimals, French day names.
    for french in [
        "Lun.",
        "Mar.",
        "Dim.",
        "Température",
        "Supprimer la plage",
        "hors des plages",
        "Touchez un emplacement vide",
        "Base",
        "18,5°",
    ] {
        assert!(text().contains(french), "{french} in {}", text());
    }
    for english in ["Mon", "Delete block", "outside the blocks", "Tap an"] {
        assert!(!text().contains(english), "{english} in {}", text());
    }
    chip(&host, "Lun.");
    let days = || {
        host.query_selector(".climate-timeline__days")
            .unwrap()
            .and_then(|n| n.get_attribute("aria-label"))
    };
    assert_eq!(days().as_deref(), Some("Jours"));
    assert_eq!(
        input(&host, ".climate-timeline__base-input").value(),
        "15,5"
    );

    // A number typed with a comma is read as a decimal point.
    let temperature = input(&host, ".climate-timeline__editor input[inputmode=decimal]");
    assert_eq!(temperature.value(), "18,5");
    change(&temperature, "19,5");
    tick().await;
    assert_eq!(
        rules.with_untracked(|r| r.blocks[0].setpoint),
        Setpoint::Temp(19.5)
    );
    // Every typed digit is kept, not rounded to one decimal.
    change(&temperature, "20,25");
    tick().await;
    assert_eq!(
        rules.with_untracked(|r| r.blocks[0].setpoint),
        Setpoint::Temp(20.25)
    );
    assert_eq!(temperature.value(), "20,25");
    change(&input(&host, ".climate-timeline__base-input"), "16,5");
    tick().await;
    assert_eq!(rules.with_untracked(|r| r.base), Setpoint::Temp(16.5));
    // A point still works, and a word is refused.
    change(&temperature, "20.5");
    tick().await;
    assert_eq!(
        rules.with_untracked(|r| r.blocks[0].setpoint),
        Setpoint::Temp(20.5)
    );
    change(&temperature, "warm");
    tick().await;
    assert_eq!(
        rules.with_untracked(|r| r.blocks[0].setpoint),
        Setpoint::Temp(20.5)
    );

    lang.set(Lang::En);
    tick().await;
    assert_eq!(days().as_deref(), Some("Days"));
    for english in ["Mon", "Delete block", "outside the blocks", "20.5°"] {
        assert!(text().contains(english), "{english} in {}", text());
    }
    for french in ["Lun.", "Supprimer la plage", "hors des plages"] {
        assert!(!text().contains(french), "{french} in {}", text());
    }
    assert_eq!(
        input(&host, ".climate-timeline__editor input[inputmode=decimal]").value(),
        "20.5"
    );
    chip(&host, "Mon");
}

#[wasm_bindgen_test]
async fn dragging_a_handle_down_moves_the_edge_and_snaps() {
    let rules = RwSignal::new(ModeRules {
        base: Setpoint::Temp(15.5),
        blocks: vec![block(&[Day::Mon], "08:00", "10:00", 18.0)],
    });
    let host = mount(rules).await;
    let monday = col(&host, Day::Mon);
    let handle = monday_handle(&host, 0, "to");

    // The column is 480px tall, so 1px is 3 min: 244px is minute 732, which snaps to 12:15.
    pointer("pointerdown", &handle, &monday, 240.0);
    pointer("pointermove", &handle, &monday, 244.0);
    pointer("pointerup", &handle, &monday, 244.0);
    tick().await;

    let moved = rules.with_untracked(|r| r.blocks[0].clone());
    assert_eq!(moved.to, HhMm(12 * 60 + 15));
    assert_eq!(moved.from, HhMm(8 * 60));
    assert!(
        host.query_selector(".climate-timeline__editor")
            .unwrap()
            .is_none(),
        "a drag does not open the editor"
    );
}

#[wasm_bindgen_test]
async fn dragging_a_handle_stops_at_the_neighbouring_block() {
    let rules = RwSignal::new(ModeRules {
        base: Setpoint::Temp(15.5),
        blocks: vec![
            block(&[Day::Mon], "06:00", "07:00", 17.0),
            block(&[Day::Mon], "08:00", "10:00", 18.0),
        ],
    });
    let host = mount(rules).await;
    let monday = col(&host, Day::Mon);
    let handle = monday_handle(&host, 1, "from");

    // 100px is minute 300 (05:00), past the first block's end: the start stops at 07:00.
    pointer("pointerdown", &handle, &monday, 160.0);
    pointer("pointermove", &handle, &monday, 100.0);
    pointer("pointerup", &handle, &monday, 100.0);
    tick().await;

    let blocks = rules.with_untracked(|r| r.blocks.clone());
    assert_eq!(blocks[1].from, HhMm(7 * 60));
    assert_eq!(blocks[1].to, HhMm(10 * 60));
    assert_eq!(blocks[0].to, HhMm(7 * 60));
}

#[wasm_bindgen_test]
async fn pressing_a_handle_without_dragging_opens_the_block() {
    let rules = RwSignal::new(ModeRules {
        base: Setpoint::Temp(15.5),
        blocks: vec![block(&[Day::Mon], "08:00", "10:00", 18.0)],
    });
    let host = mount(rules).await;
    let monday = col(&host, Day::Mon);
    let handle = monday_handle(&host, 0, "from");

    pointer("pointerdown", &handle, &monday, 160.0);
    pointer("pointerup", &handle, &monday, 160.0);
    tick().await;

    assert_eq!(
        rules.with_untracked(|r| r.blocks[0].from),
        HhMm(8 * 60),
        "the press leaves the block as it was"
    );
    assert!(
        host.query_selector(".climate-timeline__editor")
            .unwrap()
            .is_some(),
        "the block's editor opens"
    );
}
