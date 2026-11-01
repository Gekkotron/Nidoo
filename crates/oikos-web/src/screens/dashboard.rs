//! The dashboard screen: for id "house" render the existing rooms-overview `HomeScreen`
//! verbatim; for any other id render the generic widget grid.

use leptos::prelude::*;
use leptos_router::hooks::{use_params_map, use_query_map};
use oikos_api::HOUSE_ID;

use crate::dashboards::config_sheet::{DashboardSettingsSheet, WidgetConfigSheet};
use crate::dashboards::edit::{EditState, EditToolbar, add_widget_handler};
use crate::dashboards::grid::Grid;
use crate::dashboards::library::WidgetLibrary;
use crate::home_header::view::{HomeHeader, use_header_ready};
use crate::house::House;
use crate::i18n::{DashText, dash_text, use_lang};
use crate::screens::home::HomeScreen;

#[component]
pub fn DashboardScreen() -> impl IntoView {
    let house = expect_context::<House>();
    let lang = use_lang();
    let params = use_params_map();
    // A Memo so a nav change updates the id without remounting the component.
    let query = use_query_map();
    let id = Memo::new(move |_| params.read().get("id").unwrap_or_default());
    // Whether the list holds this dashboard. The screen tracks this instead of the whole
    // list so Save writing its result back does not rebuild the editor mid-edit.
    let known = Memo::new(move |_| {
        let current = id.get();
        house
            .dashboards
            .with(|v| v.iter().any(|d| d.dashboard.id == current))
    });
    // A Memo too: the screen reads it, so only the flag's first change may rebuild the editor.
    let loaded = Memo::new(move |_| house.dashboards_loaded.get());
    let header_ready = use_header_ready();
    let loading = move || {
        view! {
            <p class="dashboard-loading" aria-busy="true">
                {move || dash_text(lang(), DashText::LoadingDashboard)}
            </p>
        }
    };

    // One closure that reads `id` first, not a `<Show>` on "is it the house" whose fallback reads
    // `id`: that shape is a diamond (effect → `when` memo → `id`, and effect → `id`). When the
    // `when` memo pulls a changed `id`, `id` skips marking the effect that asked, and `when`
    // itself stays false, so moving between two authored dashboards left the first on screen.
    move || {
        let current_id = id.get();
        if current_id == HOUSE_ID {
            return view! { <HomeScreen /> }.into_any();
        }
        // Reading `known` keeps the screen reactive to the fetch landing after first paint. It
        // must be read, not tracked: a memo runs (and subscribes to the list) only when read.
        let found = known.get().then(|| house.dashboard(&current_id)).flatten();
        match found {
            Some(view) => {
                // The sidebar's "New dashboard" lands here with ?edit=1.
                let initial_edit = query.read_untracked().get("edit").as_deref() == Some("1");
                let edit = EditState::new(view.dashboard.clone(), initial_edit);
                // A refetch (on a reconnect) can bring a copy saved elsewhere: take it
                // while nothing is being edited here. Edit mode and the unsaved flag are
                // tracked too, so a copy that came mid-edit lands once the edit ends clean.
                let dirty = edit.dirty();
                Effect::new(move |_| {
                    let editing = edit.active.get() || dirty.get();
                    let Some(fresh) = house
                        .dashboards
                        .with(|v| v.iter().find(|d| d.dashboard.id == current_id).cloned())
                    else {
                        return;
                    };
                    if !editing {
                        edit.adopt(&fresh.dashboard);
                    }
                });
                let widgets = Signal::derive(move || edit.draft.get().widgets);
                let show_header = move || edit.draft.with(|d| d.show_header);
                // With the header on, header and grid wait for the house together: painted
                // early, the header grows as chips and cards land and pushes the grid down.
                let ready = move || !show_header() || header_ready.get();
                view! {
                    <Show when=ready fallback=loading>
                        <Show when=show_header>
                            <HomeHeader />
                        </Show>
                        <EditToolbar state=edit />
                        // The library sits beside the grid while editing; the layout wraps
                        // both only then so the plain grid keeps its full width.
                        <div class:dashboard-edit-layout=move || edit.active.get()>
                            <Grid widgets=widgets edit=edit />
                            <Show when=move || edit.active.get()>
                                <aside class="dashboard-library-panel" aria-label=move || dash_text(lang(), DashText::WidgetLibrary)>
                                    <WidgetLibrary on_add=add_widget_handler(edit) />
                                </aside>
                            </Show>
                        </div>
                        // Rebuilt per selected index so the sheet never shows a stale widget.
                        {move || {
                            edit.selected_widget.get().map(|index| view! {
                                <WidgetConfigSheet
                                    state=edit
                                    index=index
                                    on_close=Callback::new(move |_| edit.selected_widget.set(None))
                                />
                            })
                        }}
                        <Show when=move || edit.settings_open.get()>
                            <DashboardSettingsSheet
                                state=edit
                                on_close=Callback::new(move |_| edit.settings_open.set(false))
                            />
                        </Show>
                    </Show>
                }
                    .into_any()
            }
            None if !loaded.get() => loading().into_any(),
            None => view! {
                <div class="dashboard-unknown">
                    <p>{move || dash_text(lang(), DashText::UnknownDashboard)}</p>
                    <a href="/">{move || dash_text(lang(), DashText::BackToHome)}</a>
                </div>
            }
            .into_any(),
        }
    }
}
