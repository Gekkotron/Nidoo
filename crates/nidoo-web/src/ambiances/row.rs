//! The Home row "One mood, one tap": the pinned ambiances as tiles, or an invitation to make
//! some. Hidden until the list has loaded once.

use leptos::prelude::*;
use leptos_router::components::A;
use nidoo_api::Ambiance;

use super::actions::{add_examples, apply_and_toast};
use super::pinned_for_home;
use crate::api::encode_id;
use crate::components::toast::Toasts;
use crate::house::House;
use crate::i18n::{Key, Lang, use_text};
use crate::icons::ambiance_icon;

#[component]
pub fn HomeAmbiances() -> impl IntoView {
    let t = use_text();
    let house = expect_context::<House>();
    let toasts = expect_context::<Toasts>();
    let lang = use_context::<RwSignal<Lang>>().unwrap_or_else(|| RwSignal::new(Lang::En));
    // The id being applied: its tile is busy and pressed until the reply (or the error) is in.
    let busy = RwSignal::new(None::<String>);
    // True from the click on "Add the four examples" until the writes are over.
    let adding = RwSignal::new(false);

    let pinned = Memo::new(move |_| house.ambiances.with(|list| pinned_for_home(list)));
    let none_at_all = Memo::new(move |_| house.ambiances.with(Vec::is_empty));

    let tile = move |a: Ambiance| {
        let icon = ambiance_icon(a.icon.as_deref());
        let href = format!("/ambiances/{}", encode_id(&a.id));
        let name = a.name.clone();
        let empty = a.actions.is_empty();
        let sub = {
            let subtitle = a.subtitle.clone();
            move || {
                if empty {
                    Some(t(Key::AmbianceAddLamp).to_owned())
                } else {
                    subtitle.clone()
                }
            }
        };
        let body = view! {
            <span class="mood-tile__icon" inner_html=icon></span>
            <span class="mood-tile__name">{name.clone()}</span>
            <span class="mood-tile__sub">{sub}</span>
        };
        if empty {
            // Nothing to run yet: the tile leads to the editor instead.
            return view! { <A href=href attr:class="mood-tile">{body}</A> }.into_any();
        }
        let id = a.id.clone();
        let is_busy = {
            let id = id.clone();
            move || busy.with(|b| b.as_deref() == Some(id.as_str()))
        };
        view! {
            <button
                type="button"
                class="mood-tile"
                class:mood-tile--pressed=is_busy.clone()
                aria-busy=move || is_busy().to_string()
                disabled=move || busy.with(Option::is_some)
                on:click=move |_| {
                    busy.set(Some(id.clone()));
                    let (id, name) = (id.clone(), name.clone());
                    leptos::task::spawn_local(async move {
                        apply_and_toast(house, id, name, toasts, lang.get_untracked()).await;
                        busy.try_set(None);
                    });
                }
            >
                {body}
            </button>
        }
        .into_any()
    };

    let invitation = move || {
        if none_at_all.get() {
            view! {
                <li class="mood-invite">
                    <A href="/ambiances/new" attr:class="mood-tile mood-tile--invite">
                        <span class="mood-tile__name">{move || t(Key::AmbianceInvite)}</span>
                    </A>
                    <button
                        type="button"
                        class="btn mood-invite__examples"
                        disabled=move || adding.get()
                        on:click=move |_| {
                            adding.set(true);
                            leptos::task::spawn_local(async move {
                                add_examples(house, toasts, lang.get_untracked()).await;
                                adding.try_set(false);
                            });
                        }
                    >
                        {move || t(Key::AmbianceAddExamples)}
                    </button>
                </li>
            }
            .into_any()
        } else {
            view! {
                <li class="mood-invite">
                    <A href="/ambiances" attr:class="mood-tile mood-tile--invite">
                        <span class="mood-tile__name">{move || t(Key::AmbianceChoose)}</span>
                    </A>
                </li>
            }
            .into_any()
        }
    };

    view! {
        <Show when=move || house.ambiances_loaded.get()>
            <section class="ambiances-row">
                <header class="ambiances-row__header">
                    <h2 class="ambiances-row__title">{move || t(Key::AmbiancesRowTitle)}</h2>
                    <A href="/ambiances/about" attr:class="ambiances__link">
                        {move || t(Key::AmbiancesHowItWorks)}
                    </A>
                </header>
                <ul class="ambiances-row__tiles">
                    {move || {
                        if pinned.with(Vec::is_empty) {
                            invitation().into_any()
                        } else {
                            pinned
                                .get()
                                .into_iter()
                                .map(|a| view! { <li>{tile(a)}</li> })
                                .collect_view()
                                .into_any()
                        }
                    }}
                </ul>
            </section>
        </Show>
    }
}
