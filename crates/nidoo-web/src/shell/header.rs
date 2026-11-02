//! The content header: the breadcrumb of the page, then the search, theme and language buttons.

use leptos::prelude::*;
use leptos_router::hooks::use_location;

use super::route::{Lookup, crumbs};
use crate::house::House;
use crate::i18n::{Key, Lang, SearchText, search_text, text, use_text};
use crate::icons::{HOUSE, MAGNIFIER, MOON, SUN, SUN_MOON};
use crate::theme::Theme;

/// The name the user gave the item a crumb points at, from what the house already holds. The
/// built-in house, while the user has not renamed it, reads in the shell's language.
fn name_of(house: House, lang: Lang, lookup: Lookup) -> Option<String> {
    match lookup {
        Lookup::Dashboard(id) => house.dashboards.with(|list| {
            list.iter().find(|d| d.dashboard.id == id).map(|d| {
                if d.synthesized && d.dashboard.id == nidoo_api::HOUSE_ID {
                    text(lang, Key::House).to_owned()
                } else {
                    d.dashboard.name.clone()
                }
            })
        }),
        Lookup::Room(id) => house.areas.with(|list| {
            list.iter()
                .find(|a| a.id == id)
                .map(|a| crate::format::area_name(lang, &a.id, &a.name))
        }),
        Lookup::Flow(id) => house.flows.with(|list| {
            list.iter()
                .find(|f| f.definition.id == id)
                .map(|f| f.definition.name.clone())
        }),
        Lookup::Group(id) => house
            .groups
            .with(|groups| groups.get(id).map(|g| g.name.clone())),
        Lookup::Ambiance(id) => house.ambiances.with(|list| {
            list.iter().find(|v| v.id == id).map(|v| {
                v.ambiance
                    .as_ref()
                    .map_or_else(|| v.id.clone(), |a| a.name.clone())
            })
        }),
        Lookup::Entity(id) => house.entities.with(|entities| {
            entities
                .get(id)
                .map(|e| crate::format::shown_name(lang, e).to_owned())
        }),
        Lookup::Alert(id) => house
            .alerts
            .with(|list| list.iter().find(|a| a.id == id).map(|a| a.title.clone())),
    }
}

#[component]
pub fn ContentHeader(drawer_open: RwSignal<bool>) -> impl IntoView {
    let t = use_text();
    let house = expect_context::<House>();
    let theme = expect_context::<RwSignal<Theme>>();
    let lang = use_context::<RwSignal<Lang>>().unwrap_or_else(|| RwSignal::new(Lang::En));
    let location = use_location();

    let trail = Memo::new(move |_| {
        let path = location.pathname.get();
        let lang = lang.get();
        crumbs(&path, lang, |lookup| name_of(house, lang, lookup))
    });
    let theme_icon = move || match theme.get() {
        Theme::Auto => SUN_MOON,
        Theme::Night => MOON,
        Theme::Ivoire => SUN,
    };

    view! {
        <header class="content-header">
            // Only shown below 900px, where the sidebar is a drawer.
            <button
                type="button"
                class="content-header__menu"
                aria-controls="sidebar"
                aria-expanded=move || drawer_open.get().to_string()
                aria-label=move || if drawer_open.get() { t(Key::CloseMenu) } else { t(Key::OpenMenu) }
                on:click=move |_| drawer_open.update(|open| *open = !*open)
            >
                <span class="content-header__menu-bars" aria-hidden="true">"☰"</span>
            </button>
            <nav class="breadcrumb" aria-label=move || t(Key::Breadcrumb)>
                <ol class="breadcrumb__list">
                    {move || {
                        trail
                            .get()
                            .into_iter()
                            .enumerate()
                            .map(|(i, crumb)| {
                                let home = (i == 0)
                                    .then(|| view! { <span class="breadcrumb__home" inner_html=HOUSE></span> });
                                let label = crumb.label;
                                let item = match crumb.href.filter(|href| !href.is_empty()) {
                                    Some(href) => view! { <a href=href>{label}</a> }.into_any(),
                                    None => view! { <span aria-current="page">{label}</span> }.into_any(),
                                };
                                view! { <li class="breadcrumb__item">{home}{item}</li> }
                            })
                            .collect_view()
                    }}
                </ol>
            </nav>
            <div class="content-header__tools">
                <a
                    href="/search"
                    class="content-header__search"
                    aria-label=move || search_text(lang.get(), SearchText::Open)
                    aria-current=move || {
                        location.pathname.with(|p| p == "/search").then_some("page")
                    }
                    inner_html=MAGNIFIER
                ></a>
                <button
                    type="button"
                    class="content-header__theme"
                    aria-label=move || format!("{} ({})", t(Key::ChangeTheme), theme.get().label())
                    title=move || theme.get().label()
                    inner_html=theme_icon
                    on:click=move |_| theme.update(|th| *th = th.next())
                ></button>
                // The name keeps the visible "FR EN" and adds, hidden, the language the button
                // switches to, written in that language.
                <button
                    type="button"
                    class="content-header__lang"
                    title=move || t(Key::ChangeLanguage)
                    on:click=move |_| lang.update(|l| *l = l.next())
                >
                    <span class:content-header__lang--on=move || lang.get() == Lang::Fr>"FR"</span>
                    <span aria-hidden="true">"·"</span>
                    <span class:content-header__lang--on=move || lang.get() == Lang::En>"EN"</span>
                    <span class="visually-hidden" lang=move || lang.get().next().as_str()>
                        {move || t(Key::ChangeLanguage)}
                    </span>
                </button>
            </div>
        </header>
    }
}
