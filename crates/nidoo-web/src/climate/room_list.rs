//! The Climate screen's room list: one button per zone with its reading, a dot in the colour of
//! its decided temperature, an eco marker while it is in eco, and a line saying where it stands. A column beside the room panel on
//! wide screens, a row of chips on narrow ones (the CSS decides).

use leptos::prelude::*;
use nidoo_climate::view::ZoneView;

use crate::climate::{TempBand, degrees_text, in_eco, room_band, room_subline};
use crate::components::now::Now;
use crate::i18n::climate::{ClimateText, climate_text};
use crate::i18n::{Lang, use_lang};

#[component]
pub fn RoomList(
    zones: Signal<Vec<ZoneView>>,
    /// The zone id the panel shows.
    selected: Signal<Option<String>>,
    on_select: Callback<String>,
) -> impl IntoView {
    let lang = use_lang();
    let clock = expect_context::<Now>().0;
    let ids =
        Memo::new(move |_| zones.with(|z| z.iter().map(|z| z.zone.clone()).collect::<Vec<_>>()));
    view! {
        <nav class="climate-rooms" aria-label=move || climate_text(lang(), ClimateText::Rooms)>
            <For each=move || ids.get() key=|id| id.clone() let:id>
                {
                    let row = {
                        let id = id.clone();
                        Memo::new(move |_| zones.with(|z| z.iter().find(|z| z.zone == id).cloned()))
                    };
                    let field = move |f: fn(Lang, &ZoneView) -> String| {
                        move || row.with(|z| z.as_ref().map(|z| f(lang(), z)).unwrap_or_default())
                    };
                    let band = move || row.with(|z| z.as_ref().and_then(room_band));
                    let eco = move || {
                        let now = clock.get();
                        row.with(|z| z.as_ref().is_some_and(|z| in_eco(z, now)))
                    };
                    let is_selected = {
                        let id = id.clone();
                        move || selected.with(|s| s.as_deref() == Some(id.as_str()))
                    };
                    let current = is_selected.clone();
                    view! {
                        <button
                            type="button"
                            class="climate-room"
                            class:climate-room--selected=is_selected
                            aria-current=move || current().then_some("true")
                            on:click=move |_| on_select.run(id.clone())
                        >
                            <span class="climate-room__name">
                                <i
                                    class="climate-room__dot"
                                    class:climate-temp--low=move || band() == Some(TempBand::Low)
                                    class:climate-temp--mid=move || band() == Some(TempBand::Mid)
                                    class:climate-temp--high=move || band() == Some(TempBand::High)
                                    class:climate-temp--none=move || band().is_none()
                                ></i>
                                {field(|_, z| z.name.clone())}
                                {move || {
                                    eco()
                                        .then(|| {
                                            view! {
                                                <span class="chip climate-room__eco">
                                                    {move || climate_text(lang(), ClimateText::Eco)}
                                                </span>
                                            }
                                        })
                                }}
                            </span>
                            <span class="climate-room__temp">
                                {field(|l, z| z.reading.map_or_else(|| "—".to_owned(), |t| degrees_text(l, t)))}
                            </span>
                            <span class="climate-room__sub">{field(room_subline)}</span>
                        </button>
                    }
                }
            </For>
        </nav>
    }
}
