//! "My moods": every ambiance as a card, with Apply, pin, reorder, Edit and Delete.

use leptos::prelude::*;
use leptos_router::components::A;
use nidoo_api::{Ambiance, AmbianceView};

use super::actions::{add_examples, apply_and_toast, upsert};
use super::pinned_for_home;
use crate::api::ambiances::{delete_ambiance, put_ambiance};
use crate::api::encode_id;
use crate::components::confirm::ConfirmBar;
use crate::components::toast::Toasts;
use crate::house::House;
use crate::i18n::{self, Key, Lang, use_text};
use crate::icons::{CHEVRON_DOWN, CHEVRON_UP, PLUS, ambiance_icon};

/// The listed moods in card order: readable ones by `(order, id)`, then unreadable ones by id.
fn card_order(list: &[AmbianceView]) -> Vec<AmbianceView> {
    let mut sorted = list.to_vec();
    sorted.sort_by(|a, b| {
        let key = |v: &AmbianceView| {
            v.ambiance
                .as_ref()
                .map_or((true, 0, v.id.clone()), |a| (false, a.order, v.id.clone()))
        };
        key(a).cmp(&key(b))
    });
    sorted
}

/// The orders `earlier` and `later` take once they trade places: they swap, unless they are
/// equal, in which case the one that now comes last moves one step up.
fn swapped_orders(earlier: &Ambiance, later: &Ambiance) -> (u32, u32) {
    if earlier.order == later.order {
        (earlier.order + 1, later.order)
    } else {
        (later.order, earlier.order)
    }
}

#[component]
pub fn AmbiancesScreen() -> impl IntoView {
    let t = use_text();
    let house = expect_context::<House>();
    let toasts = expect_context::<Toasts>();
    let lang = use_context::<RwSignal<Lang>>().unwrap_or_else(|| RwSignal::new(Lang::En));
    // True while a pin or reorder write is in flight; the controls wait for it.
    let writing = RwSignal::new(false);
    // The id being applied, and the id whose Delete is asking "are you sure?".
    let applying = RwSignal::new(None::<String>);
    let confirming = RwSignal::new(None::<String>);
    // True from the click on "Add the four examples" until the writes are over.
    let adding = RwSignal::new(false);

    let cards = Memo::new(move |_| house.ambiances.with(|list| card_order(list)));
    let too_many_pinned = Memo::new(move |_| {
        house.ambiances.with(|list| {
            list.iter()
                .filter_map(|v| v.ambiance.as_ref())
                .filter(|a| a.pinned)
                .count()
                > pinned_for_home(list).len()
        })
    });

    let save = move |changed: Vec<Ambiance>| {
        writing.set(true);
        leptos::task::spawn_local(async move {
            for ambiance in changed {
                match put_ambiance(&ambiance).await {
                    Ok(view) => upsert(house, view),
                    Err(e) => {
                        toasts.push(e.error);
                        break;
                    }
                }
            }
            writing.try_set(false);
        });
    };
    let pin = move |mut ambiance: Ambiance| {
        ambiance.pinned = !ambiance.pinned;
        save(vec![ambiance]);
    };
    // Moves the readable mood at `id` one place `up` or down, trading orders with its neighbour.
    let step = move |id: &str, up: bool| {
        let valid: Vec<Ambiance> =
            cards.with(|c| c.iter().filter_map(|v| v.ambiance.clone()).collect());
        let Some(i) = valid.iter().position(|a| a.id == id) else {
            return;
        };
        let j = if up { i.checked_sub(1) } else { Some(i + 1) };
        let Some(j) = j.filter(|j| *j < valid.len()) else {
            return;
        };
        let (mut earlier, mut later) = (valid[i.min(j)].clone(), valid[i.max(j)].clone());
        (earlier.order, later.order) = swapped_orders(&earlier, &later);
        save(vec![earlier, later]);
    };
    let delete = move |id: String| {
        confirming.set(None);
        leptos::task::spawn_local(async move {
            match delete_ambiance(&id).await {
                Ok(()) => house.ambiances.update(|list| list.retain(|v| v.id != id)),
                Err(e) => toasts.push(e.error),
            }
        });
    };
    let ask_delete = move |id: String, label: String| {
        let question = i18n::ambiance_delete_question(lang.get(), &label);
        let on_yes = {
            let id = id.clone();
            Callback::new(move |()| delete(id.clone()))
        };
        view! {
            <ConfirmBar
                question=question
                yes=i18n::text(lang.get_untracked(), Key::Delete)
                on_yes=on_yes
                on_cancel=Callback::new(move |()| confirming.set(None))
            />
        }
    };

    let card = move |v: AmbianceView| {
        let id = v.id.clone();
        let Some(a) = v.ambiance else {
            // Unreadable: the reason, and the one thing that can be done about it.
            let reason = v.invalid.unwrap_or_default();
            let delete_id = id.clone();
            let ask_id = id.clone();
            let label = id.clone();
            return view! {
                <li class="mood-card mood-card--invalid">
                    <div class="mood-card__head">
                        <div class="mood-card__titles">
                            <h2 class="mood-card__name">{id}</h2>
                            <p class="mood-card__sub">{reason}</p>
                        </div>
                    </div>
                    <div class="mood-card__actions">
                        {move || if confirming.with(|c| c.as_deref() == Some(delete_id.as_str())) {
                            ask_delete(ask_id.clone(), label.clone()).into_any()
                        } else {
                            let id = delete_id.clone();
                            view! {
                                <button
                                    type="button"
                                    class="btn btn--danger"
                                    on:click=move |_| confirming.set(Some(id.clone()))
                                >
                                    {move || t(Key::Delete)}
                                </button>
                            }
                            .into_any()
                        }}
                    </div>
                </li>
            }
            .into_any();
        };
        let icon = ambiance_icon(a.icon.as_deref());
        let edit_href = format!("/ambiances/{}", encode_id(&a.id));
        let report_href = format!("/ambiances/{}/report", encode_id(&a.id));
        let has_report = {
            let id = a.id.clone();
            move || house.ambiance_reports.with(|m| m.contains_key(&id))
        };
        let pinned = a.pinned;
        let (pin_a, apply_id, apply_name) = (a.clone(), a.id.clone(), a.name.clone());
        let (up_id, down_id) = (a.id.clone(), a.id.clone());
        let delete_id = a.id.clone();
        let (ask_id, ask_name) = (a.id.clone(), a.name.clone());
        let busy_id = a.id.clone();
        view! {
            <li class="mood-card">
                <div class="mood-card__head">
                    <span class="mood-card__icon" inner_html=icon></span>
                    <div class="mood-card__titles">
                        <h2 class="mood-card__name">{a.name.clone()}</h2>
                        {a.subtitle.clone().map(|s| view! { <p class="mood-card__sub">{s}</p> })}
                    </div>
                    <button
                        type="button"
                        role="switch"
                        class="switch"
                        class:switch--on=pinned
                        aria-checked=pinned.to_string()
                        aria-label=move || t(Key::AmbianceShowOnHome)
                        title=move || t(Key::AmbianceShowOnHome)
                        disabled=move || writing.get()
                        on:click=move |_| pin(pin_a.clone())
                    >
                        <span class="switch__knob"></span>
                    </button>
                </div>
                <div class="mood-card__actions">
                    {move || if confirming.with(|c| c.as_deref() == Some(delete_id.as_str())) {
                        ask_delete(ask_id.clone(), ask_name.clone()).into_any()
                    } else {
                        let (apply_id, apply_name) = (apply_id.clone(), apply_name.clone());
                        let busy_id = busy_id.clone();
                        let (up_id, down_id) = (up_id.clone(), down_id.clone());
                        let delete_id = delete_id.clone();
                        view! {
                            <button
                                type="button"
                                class="btn btn--primary mood-card__apply"
                                disabled=move || applying.with(|a| a.as_deref() == Some(busy_id.as_str()))
                                on:click=move |_| {
                                    applying.set(Some(apply_id.clone()));
                                    let (id, name) = (apply_id.clone(), apply_name.clone());
                                    leptos::task::spawn_local(async move {
                                        apply_and_toast(house, id, name, toasts, lang.get_untracked()).await;
                                        applying.try_set(None);
                                    });
                                }
                            >
                                {move || t(Key::AmbianceApply)}
                            </button>
                            <A href=edit_href.clone() attr:class="btn mood-card__edit">
                                {move || t(Key::Edit)}
                            </A>
                            <button
                                type="button"
                                class="btn btn--round mood-card__up"
                                aria-label=move || t(Key::MoveUp)
                                title=move || t(Key::MoveUp)
                                disabled=move || writing.get()
                                inner_html=CHEVRON_UP
                                on:click=move |_| step(&up_id, true)
                            ></button>
                            <button
                                type="button"
                                class="btn btn--round mood-card__down"
                                aria-label=move || t(Key::MoveDown)
                                title=move || t(Key::MoveDown)
                                disabled=move || writing.get()
                                inner_html=CHEVRON_DOWN
                                on:click=move |_| step(&down_id, false)
                            ></button>
                            <button
                                type="button"
                                class="btn btn--danger mood-card__delete"
                                on:click=move |_| confirming.set(Some(delete_id.clone()))
                            >
                                {move || t(Key::Delete)}
                            </button>
                        }
                        .into_any()
                    }}
                </div>
                <Show when=has_report>
                    <A href=report_href.clone() attr:class="mood-card__report">
                        {move || t(Key::AmbianceLastRun)}
                    </A>
                </Show>
            </li>
        }
        .into_any()
    };

    view! {
        <div class="sheet ambiances">
            <header class="sheet__header ambiances__header">
                <h1 class="sheet__title">{move || t(Key::Ambiances)}</h1>
                <A href="/ambiances/about" attr:class="ambiances__link">
                    {move || t(Key::AmbiancesHowItWorks)}
                </A>
                <A href="/ambiances/new" attr:class="btn btn--primary ambiances__new">
                    <span class="btn__glyph" inner_html=PLUS></span>
                    {move || t(Key::NewAmbiance)}
                </A>
            </header>
            <Show when=move || too_many_pinned.get()>
                <p class="ambiances__hint">{move || t(Key::AmbianceOnlyFour)}</p>
            </Show>
            <Show
                when=move || !cards.with(Vec::is_empty)
                fallback=move || {
                    house.ambiances_loaded.get().then(|| view! {
                        <section class="sheet__panel ambiances__empty">
                            <p>{move || t(Key::AmbianceEmpty)}</p>
                            <div class="sheet__actions">
                                <A href="/ambiances/new" attr:class="btn">
                                    {move || t(Key::NewAmbiance)}
                                </A>
                                <button
                                    type="button"
                                    class="btn btn--primary ambiances__examples"
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
                            </div>
                        </section>
                    })
                }
            >
                <ul class="mood-list">
                    {move || cards.get().into_iter().map(card).collect_view()}
                </ul>
            </Show>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mood(id: &str, order: u32) -> Ambiance {
        Ambiance {
            id: id.into(),
            name: id.into(),
            subtitle: None,
            icon: None,
            pinned: false,
            order,
            actions: Vec::new(),
            extra: Default::default(),
        }
    }

    fn view(a: Ambiance) -> AmbianceView {
        AmbianceView {
            id: a.id.clone(),
            ambiance: Some(a),
            invalid: None,
        }
    }

    #[test]
    fn cards_sort_by_order_then_id_with_unreadable_last() {
        let broken = AmbianceView {
            id: "aaa".into(),
            ambiance: None,
            invalid: Some("bad".into()),
        };
        let list = [
            view(mood("b", 2)),
            broken,
            view(mood("c", 1)),
            view(mood("a", 2)),
        ];
        let ids: Vec<_> = card_order(&list).into_iter().map(|v| v.id).collect();
        assert_eq!(ids, ["c", "a", "b", "aaa"]);
    }

    #[test]
    fn swapping_trades_orders_or_bumps_a_tie() {
        assert_eq!(swapped_orders(&mood("a", 1), &mood("b", 4)), (4, 1));
        assert_eq!(swapped_orders(&mood("a", 3), &mood("b", 3)), (4, 3));
    }
}
