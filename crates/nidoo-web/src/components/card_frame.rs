//! The shell every entity card shares: icon, name, status dots and a tap target.

use leptos::prelude::*;
use nidoo_api::EntityView;
use nidoo_core::model::Availability;

use crate::components::card_place::{CardPlace, use_card_place};
use crate::format::room_name;
use crate::house::House;
use crate::icons::entity_icon;

/// The prefix of the id of a hand-made group's entity.
pub const GROUP_PREFIX: &str = "group_";

/// The path of the sheet that opens for entity `id`: a group's own sheet for `group_<id>`.
pub fn sheet_path(id: &str) -> String {
    match id.strip_prefix(GROUP_PREFIX) {
        Some(group) => format!("/group/{}", crate::api::encode_id(group)),
        None => format!("/entity/{}", crate::api::encode_id(id)),
    }
}

fn availability_class(availability: Availability) -> &'static str {
    match availability {
        Availability::Online => "dot dot--online",
        Availability::Offline => "dot dot--offline",
        Availability::Unknown => "dot dot--unknown",
    }
}

/// A callback that opens the detail sheet of entity `id`. Call it while building a component
/// (it needs the router context).
pub fn open_entity(id: &str) -> Callback<()> {
    let navigate = leptos_router::hooks::use_navigate();
    let path = sheet_path(id);
    Callback::new(move |()| navigate(&path, Default::default()))
}

/// A card for one entity: the icon tile, the room (when the place does not already say it) and
/// the name form the header that opens the sheet; `trailing` (a toggle, a badge, a menu) sits
/// beside it, outside the button; `state_line` reads under it; `children` is the body.
/// `pending` shows a pulsing dot while a command awaits its state echo.
#[component]
pub fn CardFrame(
    entity: EntityView,
    on_click: Callback<()>,
    #[prop(into, optional)] pending: Signal<bool>,
    /// Replaces the title computed from the entity; empty keeps the computed one.
    #[prop(into, optional)]
    title: String,
    /// Draws the card in the alarm colour (a ringing siren).
    #[prop(optional)]
    alert: bool,
    /// Draws the card in the warning colour (an unlocked or jammed lock).
    #[prop(optional)]
    warn: bool,
    /// The control at the top right, beside the header.
    #[prop(optional)]
    trailing: Option<AnyView>,
    /// The short state under the name ("On · 14 %"); a signal so it follows the language.
    #[prop(optional, into)]
    state_line: Option<Signal<String>>,
    /// A background tint, as the class `card--<tint>` (`sage`, `cool`).
    #[prop(optional)]
    tint: Option<&'static str>,
    /// Spans two columns (a full row on a phone).
    #[prop(optional)]
    wide: bool,
    /// An inline style for the card, such as a media player's `--media-tint`; empty for none.
    #[prop(optional, into)]
    style: String,
    /// Puts the room and name beside the tile instead of under it.
    #[prop(optional)]
    row_header: bool,
    /// Replaces the tile icon computed from the entity (a satellite's speaker on its Volume).
    #[prop(optional, into)]
    icon: Option<&'static str>,
    /// The body; a card whose state line says everything has none.
    #[prop(optional)]
    children: Option<Children>,
) -> impl IntoView {
    let icon = icon.unwrap_or_else(|| entity_icon(&entity));
    let availability = entity.state.availability;
    let is_on = crate::format::is_on(&entity.state);
    let lang = crate::i18n::use_lang();
    let short = {
        let entity = entity.clone();
        move || {
            if title.is_empty() {
                crate::format::wrap_id(&crate::format::entity_title(lang(), &entity))
            } else {
                crate::format::wrap_id(&title)
            }
        }
    };
    let full_name = {
        let entity = entity.clone();
        move || crate::format::shown_name(lang(), &entity).to_owned()
    };
    let offline = availability == Availability::Offline;
    // A group's entity spans like the derived group card and carries its `×N` badge.
    let group = entity.id.strip_prefix(GROUP_PREFIX).map(str::to_owned);
    let is_group = group.is_some();
    let house = use_context::<House>();
    let count = Signal::derive(move || {
        let group = group.as_deref()?;
        house?
            .groups
            .with(|g| g.get(group).map(|g| g.members.len()))
    });
    // The room line, only where no heading names the room already.
    let room = (use_card_place() == CardPlace::Overview)
        .then_some(())
        .and(house)
        .map(|house| {
            let area = entity.area.clone();
            Signal::derive(move || house.areas.with(|a| room_name(a, &area)))
        });
    let tinted = !style.is_empty();
    let class = match tint {
        Some(tint) => format!("card card--{tint}"),
        None => "card".to_owned(),
    };

    view! {
        <article
            class=class
            class:card--tinted=tinted
            style=(tinted).then_some(style)
            class:card--wide=wide || is_group
            class:card--row-header=row_header
            class:card--on=is_on
            class:card--offline=offline
            class:card--pending=move || pending.get()
            class:card--alert=alert
            class:card--warn=warn
        >
            <div class="card__top">
                <button type="button" class="card__header" on:click=move |_| on_click.run(())>
                    <span class="card__tile">
                        <span class="card__icon" inner_html=icon></span>
                        <span class="card__dots">
                            <span class="dot dot--pending" class:dot--active=move || pending.get()></span>
                            <span class=availability_class(availability)></span>
                        </span>
                    </span>
                    <span class="card__titles">
                        {room.map(|room| {
                            move || room.get().map(|r| view! { <span class="card__room">{r}</span> })
                        })}
                        <span class="card__name" title=full_name>{short}</span>
                    </span>
                    {move || {
                        count.get().map(|n| view! { <span class="group__count">{format!("×{n}")}</span> })
                    }}
                </button>
                {trailing.map(|t| view! { <div class="card__trailing">{t}</div> })}
            </div>
            {state_line.map(|s| view! { <p class="card__state">{move || s.get()}</p> })}
            {children.map(|c| view! { <div class="card__body">{c()}</div> })}
        </article>
    }
}
