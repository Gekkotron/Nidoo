//! The user-defined entity groups: a top-level screen so Home-level groups are reachable
//! without a Devices tab in between.

use leptos::prelude::*;
use leptos_router::components::A;
use nidoo_api::GroupView;

use crate::components::toast::Toasts;
use crate::format::wrap_id;
use crate::i18n::groups::{GroupText, group_text, groups_count, members_count};
use crate::i18n::{Key, text, use_lang};
use crate::icons::{PLUS, kind_icon};

#[component]
pub fn GroupsScreen() -> impl IntoView {
    let toasts = expect_context::<Toasts>();
    let lang = use_lang();
    let groups = RwSignal::new(Vec::<GroupView>::new());
    leptos::task::spawn_local(async move {
        match crate::api::list_groups().await {
            Ok(list) => groups.set(list),
            Err(e) => toasts.push(e.error),
        }
    });

    view! {
        <div class="devices">
            <h1 class="devices__title">{move || text(lang(), Key::Groups)}</h1>
            <section class="sheet__panel groups-card">
                <div class="groups-card__head">
                    <h2 class="sheet__heading">{move || text(lang(), Key::Groups)}</h2>
                    <span class="groups-card__count">
                        {move || groups_count(lang(), groups.with(Vec::len))}
                    </span>
                </div>
                <Show
                    when=move || groups.with(|g| !g.is_empty())
                    fallback=move || view! {
                        <p class="groups-card__empty">
                            {move || group_text(lang(), GroupText::NoGroupsYet)}
                        </p>
                    }
                >
                    <ul class="group-list">
                        <For
                            each=move || groups.get()
                            key=|g| (g.id.clone(), g.name.clone(), g.members.len())
                            children=move |g| {
                                let href = format!("/group/{}", crate::api::encode_id(&g.id));
                                let glyph = kind_icon(g.kind, None);
                                let count = g.members.len();
                                view! {
                                    <li>
                                        <A href=href attr:class="group-list__row">
                                            <span class="group-list__glyph" inner_html=glyph></span>
                                            <span class="group-list__name">{wrap_id(&g.name)}</span>
                                            <span class="group-list__count">{move || members_count(lang(), count)}</span>
                                        </A>
                                    </li>
                                }
                            }
                        />
                    </ul>
                </Show>
                <div class="groups-card__actions">
                    <A href="/groups/new" attr:class="chip chip--add">
                        <span class="chip__glyph" inner_html=PLUS></span>
                        {move || text(lang(), Key::NewGroup)}
                    </A>
                </div>
            </section>
        </div>
    }
}
