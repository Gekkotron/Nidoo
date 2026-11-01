//! The full-screen view of one camera of a Camera widget: Frigate's MJPEG stream through the
//! hub while `[frigate] live` is on, the latest alert snapshot otherwise. Arrows step through
//! the widget's cameras; the close button, Escape and Back (a history pop, or the dialog's
//! `cancel` on Android) close it. Closing empties the picture's `src` first, which ends the
//! stream's connection; so does a hidden tab, until it is shown again.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use leptos::ev;
use leptos::prelude::*;

use crate::cameras::{CameraList, PageVisible, frame_height, latest_alert_for};
use crate::components::back_close::BackClose;
use crate::dashboards::camera_widget::{camera_name, use_now};
use crate::house::House;
use crate::i18n::{DashText, WidgetText, dash_text, use_lang, widget_text};

/// The stream height for the whole window, capped like the tiles.
fn viewer_height() -> u32 {
    #[cfg(target_arch = "wasm32")]
    if let Some(window) = web_sys::window() {
        let css = window
            .inner_height()
            .ok()
            .and_then(|h| h.as_f64())
            .unwrap_or(0.0);
        return frame_height(css, window.device_pixel_ratio());
    }
    frame_height(360.0, 1.0)
}

#[component]
pub fn CameraViewer(cameras: Vec<String>, start: usize, on_close: Callback<()>) -> impl IntoView {
    let lang = use_lang();
    let list = use_context::<CameraList>().unwrap_or_default();
    let house = expect_context::<House>();
    let now = use_now();
    let count = cameras.len().max(1);
    let cameras = Arc::new(cameras);
    let index = RwSignal::new(start.min(count - 1));
    let current = {
        let cameras = Arc::clone(&cameras);
        Memo::new(move |_| cameras.get(index.get()).cloned().unwrap_or_default())
    };
    let live = Memo::new(move |_| list.0.with(|v| v.live));
    let snapshot = Memo::new(move |_| {
        let id = current.get();
        house.alerts.with(|alerts| {
            latest_alert_for(alerts, &id, now.get(), u64::MAX).and_then(|a| a.image_url.clone())
        })
    });
    let page = use_context::<PageVisible>();
    let src = move || {
        if live.get() {
            if !page.is_none_or(|p| p.0.get()) {
                return None;
            }
            Some(format!(
                "/api/v1/cameras/{}/stream?h={}",
                crate::api::encode_id(&current.get()),
                viewer_height()
            ))
        } else {
            snapshot.get()
        }
    };

    let dialog = NodeRef::<leptos::html::Dialog>::new();
    let picture = NodeRef::<leptos::html::Img>::new();
    let stop_stream = move || {
        if let Some(Some(img)) = picture.try_get_untracked() {
            img.set_src("");
        }
    };
    // Escape, Back, `cancel` and `close` can all arrive for one closing; only the first counts.
    let closed = Arc::new(AtomicBool::new(false));
    let close = Callback::new(move |()| {
        if closed.swap(true, Ordering::SeqCst) {
            return;
        }
        stop_stream();
        on_close.try_run(());
    });
    let hook = BackClose::new();
    Effect::new(move |_| {
        let Some(d) = dialog.get() else { return };
        if !d.open() {
            let _ = d.show_modal();
        }
        if !hook.is_attached() {
            hook.attach(close);
        }
    });
    let escape = window_event_listener(ev::keydown, move |ev| {
        if ev.key() == "Escape" {
            close.try_run(());
        }
    });
    on_cleanup(move || {
        escape.remove();
        stop_stream();
        hook.detach();
    });

    let step = move |forward: bool| {
        index.update(|i| {
            *i = if forward {
                (*i + 1) % count
            } else {
                (*i + count - 1) % count
            }
        });
    };
    let arrows = (count > 1).then(|| {
        view! {
            <button
                type="button"
                class="camera-viewer__arrow camera-viewer__arrow--prev"
                aria-label=move || widget_text(lang(), WidgetText::PrevCamera)
                on:click=move |_| step(false)
            >
                "‹"
            </button>
            <button
                type="button"
                class="camera-viewer__arrow camera-viewer__arrow--next"
                aria-label=move || widget_text(lang(), WidgetText::NextCamera)
                on:click=move |_| step(true)
            >
                "›"
            </button>
        }
    });
    let note = move || {
        (!live.get()).then(|| {
            let t = if snapshot.with(Option::is_some) {
                WidgetText::LiveOff
            } else {
                WidgetText::NoEvents
            };
            view! { <span class="camera-viewer__note">{widget_text(lang(), t)}</span> }
        })
    };
    view! {
        // The dialog must not close itself behind the app's back: `cancel` (Escape, Android's
        // Back) closes it through `close`, and a close the browser forces anyway ends it too.
        <dialog
            class="camera-viewer"
            node_ref=dialog
            on:cancel=move |ev: web_sys::Event| {
                ev.prevent_default();
                close.try_run(());
            }
            on:close=move |_| {
                close.try_run(());
            }
        >
            <div class="camera-viewer__bar">
                <span class="camera-viewer__name">{move || camera_name(list, &current.get())}</span>
                <button
                    type="button"
                    class="camera-viewer__close"
                    aria-label=move || dash_text(lang(), DashText::Close)
                    on:click=move |_| {
                        close.try_run(());
                    }
                >
                    "✕"
                </button>
            </div>
            <img class="camera-viewer__img" alt="" node_ref=picture src=src />
            {note}
            {arrows}
        </dialog>
    }
}
