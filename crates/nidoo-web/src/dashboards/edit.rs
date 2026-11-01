//! Dashboard edit mode: draft/baseline state plus the pencil / Discard / Save toolbar.

use leptos::prelude::*;
use leptos::task::spawn_local;
use nidoo_core::dashboard::{
    Cell, Dashboard, Span, Widget, allowed_spans, append_widget, move_widget, resize_widget,
};

use crate::api;
use crate::house::House;
use crate::i18n::{DashText, Lang, dash_text, use_lang};
use crate::icons::{CHECK, GEAR, PENCIL, X_MARK};

/// Reactive state for the edit mode: active flag, the editable draft, and the saved baseline
/// for Discard. House never gets one of these.
#[derive(Clone, Copy)]
pub struct EditState {
    pub active: RwSignal<bool>,
    pub draft: RwSignal<Dashboard>,
    /// Last-saved baseline; Save replaces it with the hub's reply.
    pub original: RwSignal<Dashboard>,
    /// Index of the widget whose config sheet is open.
    pub selected_widget: RwSignal<Option<usize>>,
    pub settings_open: RwSignal<bool>,
    /// The id of the widget just added from the library, which glows for a moment.
    pub fresh: RwSignal<Option<String>>,
}

impl EditState {
    pub fn new(initial: Dashboard, active: bool) -> Self {
        Self {
            active: RwSignal::new(active),
            draft: RwSignal::new(initial.clone()),
            original: RwSignal::new(initial),
            selected_widget: RwSignal::new(None),
            settings_open: RwSignal::new(false),
            fresh: RwSignal::new(None),
        }
    }

    pub fn dirty(&self) -> Memo<bool> {
        let draft = self.draft;
        let original = self.original;
        Memo::new(move |_| draft.get() != original.get())
    }

    /// Moves widget `index` so its top-left corner is on cell `to`; see [`move_widget`]. The
    /// list order is left alone, so an open config sheet keeps its widget.
    pub fn move_to(&self, index: usize, to: Cell) {
        self.draft
            .update(|d| move_widget(&mut d.widgets, index, to));
    }

    /// Sets widget `index` to `span`; what it now covers moves down (see [`resize_widget`]).
    pub fn resize(&self, index: usize, span: Span) {
        self.draft
            .update(|d| resize_widget(&mut d.widgets, index, span));
    }

    /// Advances widget `index` to the next span its kind allows, wrapping around, so the
    /// editor can never produce a span the hub would reject.
    pub fn cycle_span(&self, index: usize) {
        let next = self.draft.with_untracked(|d| {
            d.widgets.get(index).map(|w| {
                let allowed = allowed_spans(w.kind);
                let cur = allowed.iter().position(|s| *s == w.span).unwrap_or(0);
                allowed[(cur + 1) % allowed.len()]
            })
        });
        if let Some(span) = next {
            self.resize(index, span);
        }
    }

    /// Leaves edit mode, closing the widget and dashboard sheets.
    pub fn close(&self) {
        self.active.set(false);
        self.selected_widget.set(None);
        self.settings_open.set(false);
    }

    pub fn discard(&self) {
        self.selected_widget.set(None);
        self.draft.set(self.original.get_untracked());
    }

    /// Takes the hub's reply to a Save of `sent`. Edits made while the request was in flight
    /// stay in the draft, still unsaved, and the edit goes on; otherwise the reply becomes the
    /// draft and the save ends the edit (the widget library and any open sheet close with it).
    pub fn saved(&self, sent: &Dashboard, saved: Dashboard) {
        self.original.set(saved.clone());
        if self.draft.with_untracked(|d| d == sent) {
            self.draft.set(saved);
            self.close();
        }
    }

    /// Takes a copy refetched from the hub when nothing is being edited here; a copy that
    /// arrives mid-edit waits in the house list for the next call.
    pub fn adopt(&self, fresh: &Dashboard) {
        let original = self.original.get_untracked();
        let idle = !self.active.get_untracked() && self.draft.with_untracked(|d| *d == original);
        if idle && original != *fresh {
            self.original.set(fresh.clone());
            self.draft.set(fresh.clone());
        }
    }
}

/// The pencil's accessible name; it says so when switching edit mode off left an unsaved draft.
fn pencil_label(lang: Lang, active: bool, dirty: bool) -> &'static str {
    dash_text(
        lang,
        if dirty && !active {
            DashText::ToggleEditUnsaved
        } else {
            DashText::ToggleEdit
        },
    )
}

/// How long a widget added from the library glows.
#[cfg(target_arch = "wasm32")]
const FRESH_MS: u32 = 1200;

/// Marks widget `id` as just added, and unmarks it after [`FRESH_MS`] unless another widget
/// took the mark meanwhile.
#[cfg(target_arch = "wasm32")]
fn flash(fresh: RwSignal<Option<String>>, id: String) {
    fresh.set(Some(id.clone()));
    gloo_timers::callback::Timeout::new(FRESH_MS, move || {
        if fresh.try_get_untracked().flatten().as_deref() == Some(id.as_str()) {
            fresh.try_set(None);
        }
    })
    .forget();
}

/// Adds a picked widget after the last one on the grid and marks it fresh; the hub validates
/// it on Save.
pub fn add_widget_handler(state: EditState) -> Callback<Widget> {
    Callback::new(move |widget: Widget| {
        let id = widget.id.clone();
        state
            .draft
            .update(|d| append_widget(&mut d.widgets, widget));
        #[cfg(target_arch = "wasm32")]
        flash(state.fresh, id);
        #[cfg(not(target_arch = "wasm32"))]
        state.fresh.set(Some(id));
    })
}

/// The pencil / Discard / Save bar shown on authored dashboards. Pencil toggles edit mode and
/// lights amber when on; in edit mode a gear opens the dashboard settings sheet, and Discard /
/// Save book-end the row with Save as the primary action.
#[component]
pub fn EditToolbar(state: EditState) -> impl IntoView {
    let house = expect_context::<House>();
    let lang = use_lang();
    let dirty = state.dirty();
    // True while a Save request is in flight; blocks a second PUT and a Discard mid-save.
    let saving = RwSignal::new(false);
    view! {
        <div class="dashboard-edit-toolbar">
            <button
                type="button"
                class="btn btn--round dashboard-edit-toolbar__pencil"
                class:btn--active=move || state.active.get()
                class:dashboard-edit-toolbar__pencil--dirty=move || {
                    !state.active.get() && dirty.get()
                }
                aria-label=move || pencil_label(lang(), state.active.get(), dirty.get())
                aria-pressed=move || state.active.get().to_string()
                on:click=move |_| {
                    if state.active.get_untracked() {
                        state.close();
                    } else {
                        state.active.set(true);
                    }
                }
            >
                <span class="btn__glyph" inner_html=PENCIL></span>
            </button>
            <Show when=move || state.active.get()>
                <button
                    type="button"
                    class="btn btn--round dashboard-edit-toolbar__gear"
                    aria-label=move || dash_text(lang(), DashText::DashboardSettings)
                    on:click=move |_| state.settings_open.update(|o| *o = !*o)
                >
                    <span class="btn__glyph" inner_html=GEAR></span>
                </button>
                <button
                    type="button"
                    class="btn btn--with-icon dashboard-edit-toolbar__discard"
                    on:click=move |_| state.discard()
                    disabled=move || !dirty.get() || saving.get()
                >
                    <span class="btn__glyph" inner_html=X_MARK></span>
                    {move || dash_text(lang(), DashText::Discard)}
                </button>
                <button
                    type="button"
                    class="btn btn--with-icon btn--primary dashboard-edit-toolbar__save"
                    on:click=move |_| {
                        if saving.get_untracked() {
                            return;
                        }
                        saving.set(true);
                        let d = state.draft.get_untracked();
                        spawn_local(async move {
                            if let Ok(saved) = api::put_dashboard(&d).await {
                                state.saved(&d, saved.clone());
                                house.store_dashboard(saved);
                            }
                            saving.set(false);
                        });
                    }
                    disabled=move || !dirty.get() || saving.get()
                >
                    <span class="btn__glyph" inner_html=CHECK></span>
                    {move || dash_text(lang(), DashText::Save)}
                </button>
            </Show>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nidoo_core::dashboard::{Span, Widget, WidgetConfig, WidgetKind};

    fn widget(id: &str, kind: WidgetKind, span: Span) -> Widget {
        Widget {
            id: id.into(),
            kind,
            span,
            config: WidgetConfig::Entity {
                entity_id: "light.a".into(),
                title: None,
            },
            pos: None,
        }
    }

    fn state(widgets: Vec<Widget>) -> EditState {
        EditState::new(
            Dashboard {
                id: "d".into(),
                name: "D".into(),
                order: 0,
                widgets,
                show_header: false,
            },
            true,
        )
    }

    #[test]
    fn dirty_memo_true_after_a_move_that_keeps_the_list_order() {
        let s = state(vec![
            widget("a", WidgetKind::Light, Span::X1x1),
            widget("b", WidgetKind::Light, Span::X1x1),
        ]);
        let dirty = s.dirty();
        assert!(!dirty.get());
        s.move_to(1, Cell { row: 1, col: 0 });
        assert!(dirty.get());
        let d = s.draft.get();
        assert_eq!(d.widgets[1].id, "b");
        assert_eq!(d.widgets[1].pos, Some(Cell { row: 1, col: 0 }));
        assert_eq!(d.widgets[0].pos, Some(Cell { row: 0, col: 0 }));
    }

    #[test]
    fn close_leaves_edit_mode_and_closes_the_sheets() {
        let s = state(vec![widget("a", WidgetKind::Light, Span::X1x1)]);
        s.selected_widget.set(Some(0));
        s.settings_open.set(true);
        s.close();
        assert!(!s.active.get());
        assert_eq!(s.selected_widget.get(), None);
        assert!(!s.settings_open.get());
    }

    #[test]
    fn a_save_reply_becomes_the_draft_and_ends_the_edit() {
        let s = state(vec![widget("a", WidgetKind::Light, Span::X1x1)]);
        s.cycle_span(0);
        let sent = s.draft.get();
        let mut reply = sent.clone();
        reply.name = "From hub".into();
        s.saved(&sent, reply.clone());
        assert_eq!(s.original.get(), reply);
        assert_eq!(s.draft.get(), reply);
        assert!(!s.active.get());
    }

    #[test]
    fn edits_made_while_a_save_is_in_flight_survive_the_reply() {
        let s = state(vec![widget("a", WidgetKind::Light, Span::X1x1)]);
        s.cycle_span(0);
        let sent = s.draft.get();
        s.selected_widget.set(Some(0));
        // Edited after the PUT went out.
        s.cycle_span(0);
        let edited = s.draft.get();
        s.saved(&sent, sent.clone());
        assert_eq!(s.original.get(), sent);
        assert_eq!(s.draft.get(), edited);
        assert!(s.dirty().get());
        assert!(s.active.get());
        assert_eq!(s.selected_widget.get(), Some(0));
    }

    #[test]
    fn a_refetched_copy_is_adopted_only_when_idle() {
        let s = state(vec![widget("a", WidgetKind::Light, Span::X1x1)]);
        let mut fresh = s.original.get();
        fresh.name = "Elsewhere".into();

        // Editing: held back.
        s.adopt(&fresh);
        assert_eq!(s.draft.get().name, "D");

        // Not editing, but an unsaved draft is left: still held back.
        s.cycle_span(0);
        s.close();
        s.adopt(&fresh);
        assert_ne!(s.original.get(), fresh);

        // Draft discarded: taken.
        s.discard();
        s.adopt(&fresh);
        assert_eq!(s.original.get(), fresh);
        assert_eq!(s.draft.get(), fresh);
    }

    #[test]
    fn pencil_label_flags_unsaved_changes_only_when_inactive() {
        assert_eq!(
            pencil_label(Lang::En, false, true),
            "Toggle edit mode (unsaved changes)"
        );
        assert_eq!(pencil_label(Lang::En, true, true), "Toggle edit mode");
        assert_eq!(pencil_label(Lang::En, false, false), "Toggle edit mode");
    }

    #[test]
    fn resize_cycles_only_allowed_spans_for_kind() {
        let s = state(vec![widget("a", WidgetKind::Light, Span::X1x1)]);
        s.cycle_span(0);
        assert_eq!(s.draft.get().widgets[0].span, Span::X2x1);
        s.cycle_span(0);
        assert_eq!(s.draft.get().widgets[0].span, Span::X1x1);
    }
}
