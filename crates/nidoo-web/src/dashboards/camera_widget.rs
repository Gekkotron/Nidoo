//! The dashboard Camera widget: 1 to 4 Frigate cameras in one tile, laid out by the tile's
//! span (see [`camera_layout`]). While `[frigate] live` is on each visible picture fetches a
//! `latest.jpg` frame about once per second through the hub, one request at a time; with it
//! off the picture is the camera's latest alert snapshot. A tap opens the full-screen viewer.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use leptos::prelude::*;
use leptos_router::hooks::use_navigate;
use nidoo_api::Span;

use crate::cameras::{
    BADGE_WITHIN, CameraLayout, CameraList, PageVisible, camera_layout, latest_alert_for,
};
use crate::components::camera_viewer::CameraViewer;
use crate::components::now::{Now, unix_now};
use crate::house::House;
use crate::i18n::format::ago;
use crate::i18n::{WidgetText, use_lang, widget_text};
use crate::icons::channel_icon;
use nidoo_api::AlertChannel;

/// A swipe shorter than this (in CSS pixels) is a tap.
const SWIPE_PX: i32 = 40;

/// The display name of camera `id`: the hub's, else the bridge's `pretty` rule.
pub(crate) fn camera_name(list: CameraList, id: &str) -> String {
    list.0
        .with(|v| {
            v.cameras
                .iter()
                .find(|c| c.id == id)
                .map(|c| c.name.clone())
        })
        .unwrap_or_else(|| nidoo_frigate::pretty(id))
}

/// The clock the badge reads, ticking with the app's [`Now`] when there is one.
pub(crate) fn use_now() -> Signal<u64> {
    let now = use_context::<Now>();
    Signal::derive(move || now.map_or_else(unix_now, |n| n.0.get()))
}

#[component]
pub fn CameraWidget(cameras: Vec<String>, span: Span) -> impl IntoView {
    let layout = camera_layout(span, cameras.len());
    let count = cameras.len();
    let cameras = Arc::new(cameras);
    let viewer = RwSignal::new(None::<usize>);
    let root = NodeRef::<leptos::html::Div>::new();
    let on_screen = on_screen(root);
    let page = use_context::<PageVisible>();
    let seen = Signal::derive(move || on_screen.get() && page.is_none_or(|p| p.0.get()));
    let open = move |i: usize| Callback::new(move |()| viewer.set(Some(i)));

    let frame = {
        let cameras = Arc::clone(&cameras);
        move |i: usize| {
            view! { <CameraFrame id=cameras[i].clone() active=seen on_open=open(i) /> }
        }
    };
    let body = match layout {
        CameraLayout::Single => view! {
            <div class="camera-widget__single">{(count > 0).then(|| frame(0))}</div>
        }
        .into_any(),
        CameraLayout::Carousel => carousel(Arc::clone(&cameras), seen, viewer).into_any(),
        CameraLayout::Stack => view! {
            <div class="camera-widget__stack">{frame(0)} {frame(1)}</div>
        }
        .into_any(),
        CameraLayout::Grid => {
            let cells = (0..4)
                .map(|i| {
                    if i < count {
                        view! { <div class="camera-widget__cell">{frame(i)}</div> }.into_any()
                    } else {
                        view! { <div class="camera-widget__cell camera-widget__cell--empty"></div> }
                            .into_any()
                    }
                })
                .collect_view();
            view! { <div class="camera-widget__grid">{cells}</div> }.into_any()
        }
    };
    let layout_class = match layout {
        CameraLayout::Single => "camera-widget camera-widget--single",
        CameraLayout::Carousel => "camera-widget camera-widget--carousel",
        CameraLayout::Stack => "camera-widget camera-widget--stack",
        CameraLayout::Grid => "camera-widget camera-widget--grid",
    };
    let close = Callback::new(move |()| viewer.set(None));
    view! {
        <div class=layout_class node_ref=root>
            {body}
            {move || {
                viewer
                    .get()
                    .map(|start| {
                        view! {
                            <CameraViewer cameras=(*cameras).clone() start=start on_close=close />
                        }
                    })
            }}
        </div>
    }
}

/// One camera at a time, with arrows, dots and a swipe. Only the camera shown polls.
fn carousel(
    cameras: Arc<Vec<String>>,
    seen: Signal<bool>,
    viewer: RwSignal<Option<usize>>,
) -> impl IntoView {
    let lang = use_lang();
    let count = cameras.len();
    let index = RwSignal::new(0usize);
    let step = move |forward: bool| {
        index.update(|i| {
            *i = if forward {
                (*i + 1) % count
            } else {
                (*i + count - 1) % count
            }
        });
    };
    // A swipe must not also count as the tap that opens the viewer.
    let down_x = StoredValue::new(None::<i32>);
    let swiped = StoredValue::new(false);
    let shown = {
        let cameras = Arc::clone(&cameras);
        move || {
            let i = index.get();
            let open = Callback::new(move |()| {
                if swiped.get_value() {
                    swiped.set_value(false);
                } else {
                    viewer.set(Some(i));
                }
            });
            view! { <CameraFrame id=cameras[i].clone() active=seen on_open=open /> }
        }
    };
    let dots = (0..count)
        .map(|i| {
            view! {
                <span
                    class="camera-widget__dot"
                    class:camera-widget__dot--on=move || index.get() == i
                ></span>
            }
        })
        .collect_view();
    view! {
        <div
            class="camera-widget__carousel"
            on:pointerdown=move |ev| down_x.set_value(Some(ev.client_x()))
            on:pointerup=move |ev| {
                if let Some(x) = down_x.get_value() {
                    let dx = ev.client_x() - x;
                    if dx.abs() > SWIPE_PX {
                        swiped.set_value(true);
                        step(dx < 0);
                    }
                }
                down_x.set_value(None);
            }
        >
            {shown}
            <button
                type="button"
                class="camera-widget__arrow camera-widget__arrow--prev"
                aria-label=move || widget_text(lang(), WidgetText::PrevCamera)
                on:click=move |ev| {
                    ev.stop_propagation();
                    step(false);
                }
            >
                "‹"
            </button>
            <button
                type="button"
                class="camera-widget__arrow camera-widget__arrow--next"
                aria-label=move || widget_text(lang(), WidgetText::NextCamera)
                on:click=move |ev| {
                    ev.stop_propagation();
                    step(true);
                }
            >
                "›"
            </button>
            <div class="camera-widget__dots" aria-hidden="true">{dots}</div>
        </div>
    }
}

/// Where a live picture stands. Only the browser build polls, so only it leaves `Waiting`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
enum FrameStatus {
    /// No frame yet.
    Waiting,
    /// Frames are flowing.
    Live,
    /// The last request failed; the last frame stays.
    Failed,
    /// The hub does not know the camera; polling stopped.
    NotFound,
}

/// One camera picture: frames (or the snapshot), the name, the Live mark, the alert badge and
/// a note when something is off. Polls only while `active`.
#[component]
fn CameraFrame(
    id: String,
    #[prop(into)] active: Signal<bool>,
    on_open: Callback<()>,
) -> impl IntoView {
    let lang = use_lang();
    let list = use_context::<CameraList>().unwrap_or_default();
    let house = expect_context::<House>();
    let now = use_now();
    let navigate = use_navigate();
    let live = Memo::new(move |_| list.0.with(|v| v.live));
    let name = {
        let id = id.clone();
        Memo::new(move |_| camera_name(list, &id))
    };
    let frame = RwSignal::new(None::<String>);
    let status = RwSignal::new(FrameStatus::Waiting);
    let node = NodeRef::<leptos::html::Div>::new();
    start_polling(id.clone(), node, live, active, frame, status);

    let badge = {
        let id = id.clone();
        Memo::new(move |_| {
            house.alerts.with(|alerts| {
                latest_alert_for(alerts, &id, now.get(), BADGE_WITHIN)
                    .map(|a| (a.id.clone(), a.body.clone(), a.sent_at))
            })
        })
    };
    let snapshot = {
        let id = id.clone();
        Memo::new(move |_| {
            house.alerts.with(|alerts| {
                latest_alert_for(alerts, &id, now.get(), u64::MAX).and_then(|a| a.image_url.clone())
            })
        })
    };
    let picture = move || {
        if live.get() {
            view! { <img class="camera-frame__img" alt="" src=move || frame.get() /> }.into_any()
        } else {
            match snapshot.get() {
                Some(src) => view! { <img class="camera-frame__img" alt="" src=src /> }.into_any(),
                None => view! {
                    <span class="camera-frame__icon" inner_html=channel_icon(AlertChannel::Camera)></span>
                }
                .into_any(),
            }
        }
    };
    let note = move || {
        let t = if live.get() {
            match status.get() {
                FrameStatus::NotFound => WidgetText::CameraNotFound,
                FrameStatus::Failed => WidgetText::CameraUnavailable,
                FrameStatus::Waiting | FrameStatus::Live => return None,
            }
        } else if snapshot.with(Option::is_some) {
            WidgetText::LiveOff
        } else {
            WidgetText::NoEvents
        };
        Some(view! { <span class="camera-frame__note">{widget_text(lang(), t)}</span> })
    };
    let badge_view = move || {
        badge.get().map(|(alert_id, body, sent_at)| {
            let href = format!("/alerts/{}", crate::api::encode_id(&alert_id));
            let target = href.clone();
            let navigate = navigate.clone();
            let label = format!(
                "{body} · {}",
                ago(lang(), now.get().saturating_sub(sent_at))
            );
            view! {
                <a
                    class="camera-frame__badge"
                    href=href
                    on:click=move |ev| {
                        ev.stop_propagation();
                        ev.prevent_default();
                        navigate(&target, Default::default());
                    }
                >
                    {label}
                </a>
            }
        })
    };
    view! {
        <div class="camera-frame" node_ref=node on:click=move |_| on_open.run(())>
            {picture}
            <Show when=move || live.get() && status.get() == FrameStatus::Live>
                <span class="camera-frame__live">{move || widget_text(lang(), WidgetText::Live)}</span>
            </Show>
            {badge_view}
            {note}
            <span class="camera-frame__name">{move || name.get()}</span>
        </div>
    }
}

/// Fetches frames for camera `id` into `frame` while `live` and `active` hold: one request at
/// a time, the next one [`next_poll_delay`](crate::cameras::next_poll_delay) after the last
/// answer. Each run has a generation number, so a run outlived by a newer one (or by the
/// component) stops at its next step.
fn start_polling(
    id: String,
    node: NodeRef<leptos::html::Div>,
    live: Memo<bool>,
    active: Signal<bool>,
    frame: RwSignal<Option<String>>,
    status: RwSignal<FrameStatus>,
) {
    let generation = Arc::new(AtomicU32::new(0));
    // A Memo, so a signal re-set to the same value (the observer's first report) does not
    // start a second run.
    let wanted = Memo::new(move |_| live.get() && active.get());
    {
        let generation = Arc::clone(&generation);
        Effect::new(move |_| {
            let run = generation.fetch_add(1, Ordering::SeqCst) + 1;
            if !wanted.get() {
                return;
            }
            #[cfg(target_arch = "wasm32")]
            leptos::task::spawn_local(poll::run(
                id.clone(),
                node,
                frame,
                status,
                Arc::clone(&generation),
                run,
            ));
            #[cfg(not(target_arch = "wasm32"))]
            let _ = (&id, node, frame, status, run);
        });
    }
    on_cleanup(move || {
        generation.fetch_add(1, Ordering::SeqCst);
        #[cfg(target_arch = "wasm32")]
        if let Some(Some(url)) = frame.try_get_untracked() {
            let _ = web_sys::Url::revoke_object_url(&url);
        }
    });
}

#[cfg(target_arch = "wasm32")]
mod poll {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    use leptos::prelude::*;
    use wasm_bindgen_futures::JsFuture;

    use super::FrameStatus;
    use crate::cameras::{FrameOutcome, frame_height, next_poll_delay};

    /// The picture height assumed before the tile is laid out, in CSS pixels.
    const FALLBACK_CSS_HEIGHT: f64 = 180.0;

    pub(super) async fn run(
        id: String,
        node: NodeRef<leptos::html::Div>,
        frame: RwSignal<Option<String>>,
        status: RwSignal<FrameStatus>,
        generation: Arc<AtomicU32>,
        run: u32,
    ) {
        let current = || generation.load(Ordering::SeqCst) == run;
        let mut failures = 0u32;
        let mut counter = 0u64;
        loop {
            if !current() {
                return;
            }
            let css = node
                .try_get_untracked()
                .flatten()
                .map_or(0.0, |el| f64::from(el.client_height()));
            let css = if css > 0.0 { css } else { FALLBACK_CSS_HEIGHT };
            let dpr = web_sys::window().map_or(1.0, |w| w.device_pixel_ratio());
            counter += 1;
            let url = format!(
                "/api/v1/cameras/{}/latest.jpg?h={}&t={counter}",
                crate::api::encode_id(&id),
                frame_height(css, dpr)
            );
            let outcome = fetch_frame(&url).await;
            if !current() {
                if let Ok(object_url) = &outcome {
                    let _ = web_sys::Url::revoke_object_url(object_url);
                }
                return;
            }
            let outcome = match outcome {
                Ok(object_url) => {
                    failures = 0;
                    let old = frame.try_get_untracked().flatten();
                    let _ = frame.try_set(Some(object_url));
                    if let Some(old) = old {
                        let _ = web_sys::Url::revoke_object_url(&old);
                    }
                    let _ = status.try_set(FrameStatus::Live);
                    FrameOutcome::Loaded
                }
                Err(FrameOutcome::NotFound) => {
                    let _ = status.try_set(FrameStatus::NotFound);
                    FrameOutcome::NotFound
                }
                Err(_) => {
                    failures += 1;
                    let _ = status.try_set(FrameStatus::Failed);
                    FrameOutcome::Failed
                }
            };
            let Some(delay) = next_poll_delay(outcome, failures) else {
                return;
            };
            gloo_timers::future::TimeoutFuture::new(delay).await;
        }
    }

    /// One frame as a decoded object URL, or why there is none: a 404 is `NotFound`, any
    /// other failure `Failed`.
    async fn fetch_frame(url: &str) -> Result<String, FrameOutcome> {
        let response = gloo_net::http::Request::get(url)
            .send()
            .await
            .map_err(|_| FrameOutcome::Failed)?;
        match response.status() {
            404 => return Err(FrameOutcome::NotFound),
            200..=299 => {}
            _ => return Err(FrameOutcome::Failed),
        }
        let bytes = response.binary().await.map_err(|_| FrameOutcome::Failed)?;
        let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(bytes.as_slice()));
        let blob =
            web_sys::Blob::new_with_u8_array_sequence(&parts).map_err(|_| FrameOutcome::Failed)?;
        let object_url =
            web_sys::Url::create_object_url_with_blob(&blob).map_err(|_| FrameOutcome::Failed)?;
        // Swap the frame in only once it decodes, so a torn or bad frame never shows.
        let image = web_sys::HtmlImageElement::new().map_err(|_| FrameOutcome::Failed)?;
        image.set_src(&object_url);
        if JsFuture::from(image.decode()).await.is_err() {
            let _ = web_sys::Url::revoke_object_url(&object_url);
            return Err(FrameOutcome::Failed);
        }
        Ok(object_url)
    }
}

/// Whether `node` is on screen, kept up to date by an IntersectionObserver that goes with the
/// component. `true` until the observer says otherwise, and where the browser cannot tell.
fn on_screen(node: NodeRef<leptos::html::Div>) -> RwSignal<bool> {
    let seen = RwSignal::new(true);
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::JsCast;
        use wasm_bindgen::closure::Closure;
        use web_sys::{IntersectionObserver, IntersectionObserverEntry};

        let held = StoredValue::new_local(
            None::<(IntersectionObserver, Closure<dyn FnMut(js_sys::Array)>)>,
        );
        Effect::new(move |_| {
            let Some(el) = node.get() else { return };
            let on_change =
                Closure::<dyn FnMut(js_sys::Array)>::new(move |entries: js_sys::Array| {
                    if let Some(entry) = entries
                        .iter()
                        .filter_map(|e| e.dyn_into::<IntersectionObserverEntry>().ok())
                        .next_back()
                    {
                        let _ = seen.try_set(entry.is_intersecting());
                    }
                });
            if let Ok(observer) = IntersectionObserver::new(on_change.as_ref().unchecked_ref()) {
                observer.observe(&el);
                held.set_value(Some((observer, on_change)));
            }
        });
        on_cleanup(move || {
            if let Some(Some((observer, _closure))) = held.try_update_value(Option::take) {
                observer.disconnect();
            }
        });
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = node;
    seen
}
