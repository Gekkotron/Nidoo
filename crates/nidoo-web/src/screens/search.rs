//! `/search`: the shown devices that match the words of `?q=`, drawn as the dashboard's own live
//! cards grouped by room, with the matching rooms as tiles. The words live in the address (written
//! with replace, so typing leaves no history), which makes Back from a card's sheet return here
//! with the same results.

use gloo_timers::future::TimeoutFuture;
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::NavigateOptions;
use leptos_router::components::A;
use leptos_router::hooks::{use_navigate, use_query_map};

use crate::api::encode_id;
use crate::components::card_place::{CardPlace, provide_card_place};
use crate::format::area_name;
use crate::house::House;
use crate::i18n::{
    Key, SearchText, search_capped, search_count, search_no_match, search_text, use_lang, use_text,
};
use crate::icons::{MAGNIFIER, X_MARK};
use crate::nav::{use_back, use_back_anchor};
use crate::rooms::{FAVORITES, use_group_refresh};
use crate::screens::home::card_for;
use crate::screens::room::RoomSection;
use crate::search::{SearchResults, search_results};

/// Quiet time after the last keystroke before the address, and so the results, follow the field.
const DEBOUNCE_MS: u32 = 150;

/// The address of the search for `text`.
fn search_path(text: &str) -> String {
    if text.is_empty() {
        "/search".to_owned()
    } else {
        format!("/search?q={}", encode_id(text))
    }
}

/// One room of the results: its title and the cards that matched there.
#[component]
fn SearchRoom<M>(area: String, results: Memo<Option<SearchResults>>, mark: M) -> impl IntoView
where
    M: Fn(&str) + Clone + 'static,
{
    let house = expect_context::<House>();
    let lang = use_lang();
    // Under its own heading a card does not repeat the room.
    provide_card_place(CardPlace::Room);
    let anchor = format!("room-{}", encode_id(&area));
    // Runs before the back stack records the sheet: that waits for an effect.
    let on_click = {
        let anchor = anchor.clone();
        move |_| mark(&anchor)
    };
    let ids = Memo::new({
        let area = area.clone();
        move |_| {
            results.with(|r| {
                r.iter()
                    .flat_map(|r| r.sections.iter())
                    .find(|s| s.area == area)
                    .map(|s| s.ids.clone())
                    .unwrap_or_default()
            })
        }
    });
    let title = {
        let area = area.clone();
        move || {
            house.areas.with(|a| {
                let name = a
                    .iter()
                    .find(|a| a.id == area)
                    .map_or(area.as_str(), |a| &a.name);
                area_name(lang(), &area, name)
            })
        }
    };
    view! {
        <section class="room" id=anchor on:click=on_click>
            <div class="room__heading">
                <h2 class="room__title">{title}</h2>
            </div>
            <div class="grid grid--home">
                <For
                    each=move || ids.get()
                    key=|id| id.clone()
                    children=move |id| {
                        // One memo per card: it only changes when this entity does.
                        let entity = Memo::new(move |_| {
                            house.entities.with(|m| m.get(&id).cloned())
                        });
                        view! { {move || entity.get().map(card_for)} }
                    }
                />
            </div>
        </section>
    }
}

/// What the words found: counts, the cards by room, then the room tiles.
#[component]
fn SearchFound(results: Memo<Option<SearchResults>>, query: Memo<String>) -> impl IntoView {
    let lang = use_lang();
    let mark = use_back_anchor();
    let areas = Memo::new(move |_| {
        results.with(|r| {
            r.iter()
                .flat_map(|r| r.sections.iter().map(|s| s.area.clone()))
                .collect::<Vec<_>>()
        })
    });
    let rooms = Memo::new(move |_| {
        results.with(|r| r.iter().flat_map(|r| r.rooms.clone()).collect::<Vec<_>>())
    });
    let total = move || results.with(|r| r.as_ref().map_or(0, |r| r.total));
    let shown = move || results.with(|r| r.as_ref().map_or(0, |r| r.shown));
    let capped = move || total() > shown();
    let nothing = move || total() == 0 && rooms.with(Vec::is_empty);
    view! {
        <Show
            when=move || !nothing()
            fallback=move || {
                view! {
                    <p class="search__line" role="status">
                        {move || search_no_match(lang(), &query.get())}
                    </p>
                }
            }
        >
            <p class="search__line" role="status">
                {move || search_count(lang(), total(), rooms.with(Vec::len))}
            </p>
            <Show when=capped>
                <p class="search__line search__line--cap">
                    {move || search_capped(lang(), shown(), total())}
                </p>
            </Show>
            <For
                each=move || areas.get()
                key=|a| a.clone()
                children={
                    let mark = mark.clone();
                    move |area| view! { <SearchRoom area results mark=mark.clone() /> }
                }
            />
            <Show when=move || rooms.with(|r| !r.is_empty())>
                <section class="search__rooms">
                    <h2 class="search__heading">{move || search_text(lang(), SearchText::Rooms)}</h2>
                    <div class="search__tiles">
                        <For
                            each=move || rooms.get()
                            key=|r| r.id.clone()
                            children=|room| {
                                view! {
                                    <A href=format!("/rooms/{}", encode_id(&room.id)) attr:class="search__room">
                                        <span class="search__room-name">{room.name}</span>
                                        <span class="search__room-count">{room.count}</span>
                                    </A>
                                }
                            }
                        />
                    </div>
                </section>
            </Show>
        </Show>
    }
}

/// The search page.
#[component]
pub fn SearchScreen() -> impl IntoView {
    let t = use_text();
    let lang = use_lang();
    let house = expect_context::<House>();
    let navigate = use_navigate();
    let back = use_back();
    use_group_refresh();

    let params = use_query_map();
    let query = Memo::new(move |_| params.read().get("q").unwrap_or_default());
    let text = RwSignal::new(query.get_untracked());
    // The last words this page wrote to the address, so a change it made itself is not read back
    // into the field (which would undo keystrokes typed since), while Back to another query is.
    let sent = StoredValue::new(query.get_untracked());
    Effect::new(move |_| {
        let q = query.get();
        if sent.try_get_value().is_some_and(|s| s != q) {
            sent.set_value(q.clone());
            text.set(q);
        }
    });
    // Counts the keystrokes: only the last one of a burst writes the address.
    let strokes = StoredValue::new(0u32);
    on_cleanup(move || {
        strokes.try_update_value(|n| *n += 1);
    });

    let go = {
        let navigate = navigate.clone();
        move |words: String| {
            if sent.try_get_value().is_none_or(|s| s == words) {
                return;
            }
            sent.set_value(words.clone());
            navigate(
                &search_path(&words),
                NavigateOptions {
                    replace: true,
                    ..Default::default()
                },
            );
        }
    };
    let on_input = {
        let go = go.clone();
        move |ev| {
            let words = event_target_value(&ev);
            text.set(words.clone());
            let Some(stroke) = strokes.try_update_value(|n| {
                *n += 1;
                *n
            }) else {
                return;
            };
            let go = go.clone();
            spawn_local(async move {
                TimeoutFuture::new(DEBOUNCE_MS).await;
                if strokes.try_get_value() == Some(stroke) {
                    go(words);
                }
            });
        }
    };
    let input = NodeRef::<leptos::html::Input>::new();
    Effect::new(move |_| {
        if let Some(field) = input.get() {
            let _ = field.focus();
        }
    });
    let clear = {
        let go = go.clone();
        move || {
            strokes.try_update_value(|n| *n += 1);
            text.set(String::new());
            go(String::new());
        }
    };
    let on_keydown = {
        let clear = clear.clone();
        move |ev: web_sys::KeyboardEvent| {
            if ev.key() != "Escape" {
                return;
            }
            ev.prevent_default();
            if text.with_untracked(String::is_empty) {
                navigate(&back.get_untracked(), NavigateOptions::default());
            } else {
                clear();
            }
        }
    };

    let results = Memo::new(move |_| {
        let q = query.get();
        house.areas.with(|areas| {
            house.devices.with(|devices| {
                house.groups.with(|groups| {
                    house
                        .entities
                        .with(|m| search_results(&q, m.values(), areas, devices, groups))
                })
            })
        })
    });
    let searching = Memo::new(move |_| results.with(Option::is_some));

    view! {
        <div class="search">
            <label class="devices__search search__field">
                <span class="devices__search-icon" inner_html=MAGNIFIER></span>
                <input
                    node_ref=input
                    class="devices__search-input search__input"
                    type="search"
                    autocomplete="off"
                    placeholder=move || search_text(lang(), SearchText::Placeholder)
                    aria-label=move || t(Key::Search)
                    prop:value=move || text.get()
                    on:input=on_input
                    on:keydown=on_keydown
                />
                <Show when=move || text.with(|s| !s.is_empty())>
                    <button
                        type="button"
                        class="search__clear"
                        aria-label=move || search_text(lang(), SearchText::Clear)
                        title=move || search_text(lang(), SearchText::Clear)
                        inner_html=X_MARK
                        on:click={
                            let clear = clear.clone();
                            move |_| {
                                clear();
                                if let Some(field) = input.get_untracked() {
                                    let _ = field.focus();
                                }
                            }
                        }
                    ></button>
                </Show>
            </label>
            <Show
                when=move || searching.get()
                fallback=move || {
                    view! {
                        <p class="search__line search__hint">
                            {move || search_text(lang(), SearchText::Hint)}
                        </p>
                        <RoomSection id=FAVORITES.to_owned() anchored=true />
                    }
                }
            >
                <SearchFound results query />
            </Show>
        </div>
    }
}
