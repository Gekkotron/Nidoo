//! The left sidebar: brand, the main sections and dashboards, the rooms, the house screens, then
//! Settings and the connection status.

use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::hooks::{use_location, use_navigate};
use wasm_bindgen::JsCast;

use super::route::{Section, section_for};
use crate::api::encode_id;
use crate::app::landing_path;
use crate::components::status_pill::StatusPill;
use crate::dashboards::create_dashboard;
use crate::house::House;
use crate::i18n::{Key, Lang, unseen_alerts, use_text};
use crate::icons::{
    BELL, BOLT, FLOW_NODES, GEAR, HOUSE, LAYERS, PLUS, SHIELD, SPARKLE, THERMOMETER,
};
use crate::nav::BackStack;
use crate::rooms::{sidebar_rooms, use_room_chips};

#[component]
pub fn Sidebar(
    climate_on: Signal<bool>,
    alarm_armed: Signal<bool>,
    solar_active: Signal<bool>,
    alerts_badge: Signal<Option<String>>,
    drawer_open: RwSignal<bool>,
) -> impl IntoView {
    let t = use_text();
    let lang = use_context::<RwSignal<Lang>>();
    let house = expect_context::<House>();
    let back = expect_context::<BackStack>().0;
    let location = use_location();
    let navigate = use_navigate();

    let section = Memo::new(move |_| {
        let path = location.pathname.get();
        back.with(|stack| section_for(&path, stack))
    });
    let current = move |entry: Section| move || (section.get() == entry).then_some("page");

    // The rooms that have something on Home; Favorites and Unassigned are not rooms of their own.
    let chips = use_room_chips();
    let rooms = Memo::new(move |_| chips.with(|list| sidebar_rooms(list)));

    // With two or more dashboards (the house counts) they unfold under Overview; "New dashboard"
    // is there at any count.
    let unfolded = Memo::new(move |_| house.dashboards.with(|list| list.len() >= 2));
    // The id in a `/dashboards/:id` path.
    let open_dashboard = Memo::new(move |_| {
        location
            .pathname
            .get()
            .strip_prefix("/dashboards/")
            .map(|rest| rest.split('/').next().unwrap_or("").to_owned())
    });
    // The open dashboard, or where "/" lands (the house) when no known dashboard is open.
    let overview_href = move || {
        house
            .dashboards
            .with(|list| {
                open_dashboard
                    .get()
                    .filter(|id| list.iter().any(|d| &d.dashboard.id == id))
            })
            .map_or_else(landing_path, |id| format!("/dashboards/{id}"))
    };
    // When the open dashboard is listed underneath, it is the page and Overview only its section.
    let overview_current = move || {
        (section.get() == Section::Overview).then(|| {
            let listed = unfolded.get()
                && open_dashboard.with(|open| {
                    open.as_ref().is_some_and(|id| {
                        house
                            .dashboards
                            .with(|list| list.iter().any(|d| &d.dashboard.id == id))
                    })
                });
            if listed { "true" } else { "page" }
        })
    };
    // The house first, then the others in the hub's order.
    let ordered = move || {
        let mut list = house.dashboards.get();
        list.sort_by_key(|d| d.dashboard.id != nidoo_api::HOUSE_ID);
        list
    };
    // True while a create call is in flight, so a double click makes one dashboard, not two.
    let creating = RwSignal::new(false);
    let new_dashboard = move |_| {
        if creating.get_untracked() {
            return;
        }
        creating.set(true);
        let navigate = navigate.clone();
        // Read now: the async block has no owner to find the language in.
        let language = lang.map_or(Lang::En, |l| l.get_untracked());
        spawn_local(async move {
            let created = create_dashboard(house, language).await;
            creating.try_set(false);
            if let Some(id) = created {
                navigate(&format!("/dashboards/{id}?edit=1"), Default::default());
            }
        });
    };

    // A small surprise for the curious.
    let wiggling = RwSignal::new(false);
    let wiggle = move |_| {
        crate::chime::ring();
        wiggling.set(true);
        gloo_timers::callback::Timeout::new(1200, move || {
            wiggling.try_set(false);
        })
        .forget();
    };

    // Following any link of the drawer closes it; the route change would too, but a link to the
    // page already open changes no route.
    let close_on_link = move |event: web_sys::MouseEvent| {
        let on_link = event
            .target()
            .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
            .is_some_and(|el| el.closest("a").ok().flatten().is_some());
        if on_link {
            drawer_open.set(false);
        }
    };

    view! {
        <aside
            class="sidebar"
            class:sidebar--open=move || drawer_open.get()
            id="sidebar"
            role=move || drawer_open.get().then_some("dialog")
            aria-modal=move || drawer_open.get().then_some("true")
            aria-label=move || drawer_open.get().then(|| t(Key::Navigation))
            on:click=close_on_link
        >
            <div class="sidebar__brand">
                <img
                    class="sidebar__logo"
                    class:sidebar__logo--wiggle=move || wiggling.get()
                    src="/assets/icon.svg"
                    alt=""
                    width="28"
                    height="28"
                    draggable="false"
                    on:dblclick=wiggle
                />
                <span class="sidebar__wordmark">"Nidoo."</span>
            </div>
            <nav class="sidebar__nav" aria-label=move || t(Key::Navigation)>
                <ul class="sidebar__list">
                    <li>
                        <a class="sidebar__link" href=overview_href aria-current=overview_current>
                            <span class="nav__glyph" inner_html=HOUSE></span>
                            <span class="nav__label">{move || t(Key::Overview)}</span>
                        </a>
                        <Show when=move || unfolded.get()>
                            <ul class="sidebar__dashboards">
                                <For
                                    each=ordered
                                    key=|dv| (dv.dashboard.id.clone(), dv.dashboard.name.clone())
                                    children=move |dv| {
                                        let id = dv.dashboard.id;
                                        let href = format!("/dashboards/{id}");
                                        let name = dv.dashboard.name;
                                        let builtin = dv.synthesized && id == nidoo_api::HOUSE_ID;
                                        // The hub names the built-in house in English; until the
                                        // user renames it, it reads in the shell's language.
                                        let label = move || {
                                            if builtin { t(Key::House).to_owned() } else { name.clone() }
                                        };
                                        let is_open = move || {
                                            open_dashboard
                                                .with(|open| open.as_deref() == Some(id.as_str()))
                                                .then_some("page")
                                        };
                                        view! {
                                            <li>
                                                <a class="sidebar__sublink" href=href aria-current=is_open>
                                                    {label}
                                                </a>
                                            </li>
                                        }
                                    }
                                />
                            </ul>
                        </Show>
                        // Always offered: a house with only the built-in dashboard makes its first one here.
                        <button
                            type="button"
                            class="sidebar__sublink sidebar__new"
                            disabled=move || creating.get()
                            aria-busy=move || creating.get().then_some("true")
                            on:click=new_dashboard
                        >
                            <span class="sidebar__new-glyph" inner_html=PLUS></span>
                            {move || t(Key::NewDashboard)}
                        </button>
                    </li>
                    <li>
                        <a class="sidebar__link" href="/ambiances" aria-current=current(Section::Ambiances)>
                            <span class="nav__glyph" inner_html=SPARKLE></span>
                            <span class="nav__label">{move || t(Key::Ambiances)}</span>
                        </a>
                    </li>
                    <li>
                        <a class="sidebar__link" href="/flows" aria-current=current(Section::Automations)>
                            <span class="nav__glyph" inner_html=FLOW_NODES></span>
                            <span class="nav__label">{move || t(Key::Automations)}</span>
                        </a>
                    </li>
                    <li>
                        <a class="sidebar__link" href="/groups" aria-current=current(Section::Groups)>
                            <span class="nav__glyph" inner_html=LAYERS></span>
                            <span class="nav__label">{move || t(Key::Groups)}</span>
                        </a>
                    </li>
                </ul>
                <h2 class="sidebar__heading">{move || t(Key::MyRooms)}</h2>
                <ul class="sidebar__rooms">
                    <For
                        each=move || rooms.get()
                        key=|r| (r.id.clone(), r.name.clone(), r.primary_count)
                        children=move |room| {
                            let entry = Section::Room(room.id.clone());
                            view! {
                                <li>
                                    <a
                                        class="sidebar__link sidebar__room"
                                        href=format!("/rooms/{}", encode_id(&room.id))
                                        aria-current=current(entry)
                                    >
                                        <span class="sidebar__room-name">{room.name}</span>
                                        <span class="sidebar__count">{room.primary_count}</span>
                                    </a>
                                </li>
                            }
                        }
                    />
                </ul>
                <h2 class="sidebar__heading">{move || t(Key::HomeSection)}</h2>
                <ul class="sidebar__list">
                    <li>
                        <a class="sidebar__link" href="/energy" aria-current=current(Section::Energy)>
                            <span
                                class="nav__glyph"
                                class:nav__glyph--solar=move || solar_active.get()
                                inner_html=BOLT
                            ></span>
                            <span class="nav__label">{move || t(Key::Energy)}</span>
                        </a>
                    </li>
                    <li>
                        <a class="sidebar__link" href="/security" aria-current=current(Section::Security)>
                            <span
                                class="nav__glyph"
                                class:nav__glyph--armed=move || alarm_armed.get()
                                inner_html=SHIELD
                            ></span>
                            <span class="nav__label">{move || t(Key::Security)}</span>
                        </a>
                    </li>
                    <Show when=move || climate_on.get()>
                        <li>
                            <a class="sidebar__link" href="/climate" aria-current=current(Section::Climate)>
                                <span class="nav__glyph" inner_html=THERMOMETER></span>
                                <span class="nav__label">{move || t(Key::Climate)}</span>
                                <span class="nav__badge nav__badge--beta">{move || t(Key::Beta)}</span>
                            </a>
                        </li>
                    </Show>
                    <li>
                        <a class="sidebar__link" href="/alerts" aria-current=current(Section::Alerts)>
                            <span class="nav__glyph" inner_html=BELL></span>
                            <span class="nav__label">{move || t(Key::Alerts)}</span>
                            {move || alerts_badge.get().map(|text| {
                                let label = unseen_alerts(lang.map_or(Lang::En, |l| l.get()), &text);
                                // The count is read out from the hidden text; the pill is for the eye.
                                view! {
                                    <span class="nav__badge" aria-hidden="true">
                                        {text}
                                    </span>
                                    <span class="visually-hidden">{label}</span>
                                }
                            })}
                        </a>
                    </li>
                </ul>
                <ul class="sidebar__list sidebar__footer">
                    <li>
                        <a class="sidebar__link" href="/settings" aria-current=current(Section::Settings)>
                            <span class="nav__glyph" inner_html=GEAR></span>
                            <span class="nav__label">{move || t(Key::Settings)}</span>
                        </a>
                    </li>
                </ul>
            </nav>
            <div class="sidebar__status">
                <StatusPill />
            </div>
        </aside>
    }
}
