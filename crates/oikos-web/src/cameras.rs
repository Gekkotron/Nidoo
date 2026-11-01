//! Frigate cameras for the dashboard Camera widget: the list the hub serves, the tile layout
//! and the pure rules for the badge, the frame height and the polling back-off.

use leptos::prelude::*;
use oikos_api::{AlertChannel, AlertView, CamerasView, Span};

/// The hub's camera list and `live` switch, shared by the widgets and the library.
#[derive(Clone, Copy, Default)]
pub struct CameraList(pub RwSignal<CamerasView>);

/// Fetches the camera list into `list`; a failed fetch keeps the list it had.
pub fn load_cameras(list: CameraList) {
    leptos::task::spawn_local(async move {
        if let Ok(view) = crate::api::cameras().await {
            list.0.set(view);
        }
    });
}

/// Whether the page is shown (`document.visibilityState`), for the whole app: tiles stop
/// fetching frames while it is hidden.
#[derive(Clone, Copy)]
pub struct PageVisible(pub RwSignal<bool>);

/// Creates [`PageVisible`] with the page's one `visibilitychange` listener. Call it once.
pub fn install_page_visible() -> PageVisible {
    let visible = RwSignal::new(true);
    #[cfg(target_arch = "wasm32")]
    if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
        use wasm_bindgen::JsCast;
        visible.set(!doc.hidden());
        let listener = doc.clone();
        let closure = wasm_bindgen::closure::Closure::wrap(Box::new(move || {
            visible.set(!listener.hidden());
        }) as Box<dyn Fn()>);
        let _ = doc
            .add_event_listener_with_callback("visibilitychange", closure.as_ref().unchecked_ref());
        closure.forget();
    }
    PageVisible(visible)
}

/// How a Camera widget lays out its cameras.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CameraLayout {
    /// One camera fills the tile.
    Single,
    /// One camera at a time, with arrows and dots.
    Carousel,
    /// Two cameras, top and bottom.
    Stack,
    /// A 2×2 grid; with three cameras the fourth square is empty.
    Grid,
}

/// The layout for `count` cameras on a tile of `span`.
pub fn camera_layout(span: Span, count: usize) -> CameraLayout {
    match (span, count) {
        (_, 0 | 1) => CameraLayout::Single,
        (Span::X2x2 | Span::X4x2 | Span::X6x3, 2) => CameraLayout::Stack,
        (Span::X2x2 | Span::X4x2 | Span::X6x3, _) => CameraLayout::Grid,
        _ => CameraLayout::Carousel,
    }
}

/// The badge window: alerts from the last 24 hours.
pub const BADGE_WITHIN: u64 = 86_400;

/// The newest alert for `camera` sent no more than `within` seconds before `now`. An alert
/// matches on its `camera` field; an older alert without one matches a camera alert whose
/// title is the camera's display name.
pub fn latest_alert_for<'a>(
    alerts: &'a [AlertView],
    camera: &str,
    now: u64,
    within: u64,
) -> Option<&'a AlertView> {
    let name = oikos_frigate::pretty(camera);
    alerts
        .iter()
        .filter(|alert| match &alert.camera {
            Some(id) => id == camera,
            None => alert.channel == AlertChannel::Camera && alert.title == name,
        })
        .filter(|alert| alert.sent_at.saturating_add(within) >= now)
        .max_by_key(|alert| alert.sent_at)
}

/// How a frame request ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameOutcome {
    Loaded,
    /// Any error but a 404: Frigate or the hub is out of reach.
    Failed,
    /// The hub does not know the camera (404).
    NotFound,
}

/// Milliseconds before the next frame request, or `None` to stop polling. After a failure it
/// backs off 1 s, 2 s, 5 s, then every 10 s.
pub fn next_poll_delay(outcome: FrameOutcome, failures_in_a_row: u32) -> Option<u32> {
    match outcome {
        FrameOutcome::Loaded => Some(1000),
        FrameOutcome::NotFound => None,
        FrameOutcome::Failed => Some(match failures_in_a_row {
            0 | 1 => 1000,
            2 => 2000,
            3 => 5000,
            _ => 10_000,
        }),
    }
}

/// The tallest frame a tile asks for, in device pixels.
pub const MAX_FRAME_HEIGHT: u32 = 720;

/// The frame height for a picture `css_px` high on a screen of `dpr` device pixels per CSS
/// pixel, between 1 and [`MAX_FRAME_HEIGHT`]. A bad `dpr` counts as 1.
pub fn frame_height(css_px: f64, dpr: f64) -> u32 {
    let dpr = if dpr.is_finite() && dpr > 0.0 {
        dpr
    } else {
        1.0
    };
    let px = (css_px * dpr).round();
    if px.is_finite() && px >= 1.0 {
        (px as u32).min(MAX_FRAME_HEIGHT)
    } else {
        1
    }
}

/// The cameras the House overview shows: every camera the hub lists, in its order, while
/// `[frigate] live` is on; none while it is off.
pub fn house_cameras(view: &CamerasView) -> Vec<String> {
    if !view.live {
        return Vec::new();
    }
    view.cameras.iter().map(|c| c.id.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use oikos_api::{AlertSeverity, CameraInfo};

    fn cameras(live: bool) -> CamerasView {
        CamerasView {
            live,
            cameras: ["front_door", "garden"]
                .map(|id| CameraInfo {
                    id: id.into(),
                    name: id.into(),
                })
                .into(),
        }
    }

    #[test]
    fn house_shows_every_camera_while_live() {
        assert_eq!(house_cameras(&cameras(true)), ["front_door", "garden"]);
    }

    #[test]
    fn house_shows_no_camera_while_live_is_off() {
        assert!(house_cameras(&cameras(false)).is_empty());
    }

    const NOW: u64 = 1_800_000_000;
    const DAY: u64 = 86_400;

    fn alert(id: &str, camera: Option<&str>, title: &str, sent_at: u64) -> AlertView {
        AlertView {
            id: id.into(),
            channel: AlertChannel::Camera,
            severity: AlertSeverity::Warning,
            title: title.into(),
            body: "Car".into(),
            image_url: None,
            video_url: None,
            entity_id: None,
            camera: camera.map(str::to_owned),
            actions: vec![],
            source: None,
            silent: false,
            resolved: false,
            resolved_at: None,
            sent_at,
        }
    }

    #[test]
    fn layout_follows_the_spec_table() {
        use CameraLayout::*;
        for span in [Span::X2x1, Span::X1x2] {
            assert_eq!(camera_layout(span, 1), Single, "{span:?}");
            for count in 2..=4 {
                assert_eq!(camera_layout(span, count), Carousel, "{span:?} {count}");
            }
        }
        assert_eq!(camera_layout(Span::X2x2, 1), Single);
        assert_eq!(camera_layout(Span::X2x2, 2), Stack);
        assert_eq!(camera_layout(Span::X2x2, 3), Grid);
        assert_eq!(camera_layout(Span::X2x2, 4), Grid);
        for big in [Span::X4x2, Span::X6x3] {
            assert_eq!(camera_layout(big, 1), Single);
            assert_eq!(camera_layout(big, 2), Stack);
            assert_eq!(camera_layout(big, 3), Grid);
        }
        assert_eq!(camera_layout(Span::X1x1, 1), Single);
        assert_eq!(camera_layout(Span::X1x1, 3), Carousel);
        assert_eq!(camera_layout(Span::X2x2, 0), Single);
    }

    #[test]
    fn latest_alert_matches_the_camera_field() {
        let alerts = [
            alert("g", Some("garden"), "Garden", NOW - 10),
            alert("f", Some("front_door"), "Front Door", NOW - 60),
        ];
        let found = latest_alert_for(&alerts, "front_door", NOW, DAY);
        assert_eq!(found.map(|a| a.id.as_str()), Some("f"));
    }

    #[test]
    fn latest_alert_falls_back_to_the_title() {
        let alerts = [alert("old", None, "Front Door", NOW - 60)];
        let found = latest_alert_for(&alerts, "front_door", NOW, DAY);
        assert_eq!(found.map(|a| a.id.as_str()), Some("old"));
        let mut other_channel = alert("door", None, "Front Door", NOW - 60);
        other_channel.channel = AlertChannel::Security;
        assert!(latest_alert_for(&[other_channel], "front_door", NOW, DAY).is_none());
    }

    #[test]
    fn a_camera_field_for_another_camera_never_matches_by_title() {
        let alerts = [alert("x", Some("garden"), "Front Door", NOW - 60)];
        assert!(latest_alert_for(&alerts, "front_door", NOW, DAY).is_none());
    }

    #[test]
    fn latest_alert_keeps_the_window_inclusive() {
        let at_edge = [alert("edge", Some("front_door"), "Front Door", NOW - DAY)];
        assert!(latest_alert_for(&at_edge, "front_door", NOW, DAY).is_some());
        let past = [alert(
            "past",
            Some("front_door"),
            "Front Door",
            NOW - DAY - 1,
        )];
        assert!(latest_alert_for(&past, "front_door", NOW, DAY).is_none());
        assert!(latest_alert_for(&past, "front_door", NOW, u64::MAX).is_some());
    }

    #[test]
    fn latest_alert_is_the_newest() {
        let alerts = [
            alert("older", Some("front_door"), "Front Door", NOW - 600),
            alert("newer", Some("front_door"), "Front Door", NOW - 60),
        ];
        let found = latest_alert_for(&alerts, "front_door", NOW, DAY);
        assert_eq!(found.map(|a| a.id.as_str()), Some("newer"));
    }

    #[test]
    fn polling_backs_off_then_stops_on_not_found() {
        assert_eq!(next_poll_delay(FrameOutcome::Loaded, 0), Some(1000));
        let failed: Vec<_> = (1..=4)
            .map(|n| next_poll_delay(FrameOutcome::Failed, n))
            .collect();
        assert_eq!(failed, [Some(1000), Some(2000), Some(5000), Some(10_000)]);
        assert_eq!(next_poll_delay(FrameOutcome::Failed, 7), Some(10_000));
        assert_eq!(next_poll_delay(FrameOutcome::NotFound, 1), None);
    }

    #[test]
    fn frame_height_scales_and_caps() {
        assert_eq!(frame_height(180.0, 2.0), 360);
        assert_eq!(frame_height(600.0, 3.0), 720);
        assert_eq!(frame_height(0.0, 2.0), 1);
        assert_eq!(frame_height(100.0, f64::NAN), 100);
    }
}
