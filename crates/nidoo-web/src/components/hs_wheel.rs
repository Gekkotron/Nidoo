//! A hue and saturation picker: a colour disc where the angle is the hue (0 at the top, clockwise)
//! and the distance from the centre is the saturation (0 at the centre, 100 % at the rim).
//!
//! Drag, tap or use the arrow keys. While a finger or mouse is down the marker follows it; the
//! colour is sent once, when it lifts.

use leptos::prelude::*;
use nidoo_core::model::Hs;

use crate::i18n::devices::{DeviceText, colour_readout, use_device_text};
use crate::i18n::use_lang;
use wasm_bindgen::JsCast;

/// The colour under the point `(dx, dy)` of a disc of radius `radius`, measured from its centre
/// with `y` growing downwards. Points outside the rim are pulled onto it.
pub fn hs_at(dx: f64, dy: f64, radius: f64) -> Hs {
    let hue = dx.atan2(-dy).to_degrees().rem_euclid(360.0);
    let saturation = if radius > 0.0 {
        (dx.hypot(dy) / radius).min(1.0) * 100.0
    } else {
        0.0
    };
    Hs {
        h: hue,
        s: saturation,
    }
}

/// Where `hs` sits on the disc, as `(left, top)` percentages of its box.
pub fn marker_at(hs: Hs) -> (f64, f64) {
    let radius = hs.s.clamp(0.0, 100.0) / 100.0 * 50.0;
    let angle = hs.h.to_radians();
    (50.0 + radius * angle.sin(), 50.0 - radius * angle.cos())
}

/// `hs` moved by `dh` degrees of hue and `ds` points of saturation, hue wrapping and saturation
/// clamped.
fn nudged(hs: Hs, dh: f64, ds: f64) -> Hs {
    Hs {
        h: (hs.h + dh).rem_euclid(360.0),
        s: (hs.s + ds).clamp(0.0, 100.0),
    }
}

/// The picker. `value` is the light's current colour; `on_pick` receives the chosen one.
#[component]
pub fn HsWheel(#[prop(into)] value: Signal<Option<Hs>>, on_pick: Callback<Hs>) -> impl IntoView {
    // The colour under the pointer while it is down.
    let preview = RwSignal::new(None::<Hs>);
    let shown = move || preview.get().or_else(|| value.get());

    let point = move |ev: &web_sys::PointerEvent| -> Option<Hs> {
        let element = ev.current_target()?.dyn_into::<web_sys::Element>().ok()?;
        let rect = element.get_bounding_client_rect();
        let radius = rect.width() / 2.0;
        Some(hs_at(
            f64::from(ev.client_x()) - (rect.left() + radius),
            f64::from(ev.client_y()) - (rect.top() + rect.height() / 2.0),
            radius,
        ))
    };
    let down = move |ev: web_sys::PointerEvent| {
        if let Some(element) = ev
            .current_target()
            .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
        {
            let _ = element.set_pointer_capture(ev.pointer_id());
        }
        preview.set(point(&ev));
    };
    let moved = move |ev: web_sys::PointerEvent| {
        if preview.with_untracked(Option::is_some) {
            preview.set(point(&ev));
        }
    };
    let up = move |_| {
        if let Some(hs) = preview.get_untracked() {
            preview.set(None);
            on_pick.run(hs);
        }
    };
    let cancel = move |_| preview.set(None);
    let key = move |ev: web_sys::KeyboardEvent| {
        let step = if ev.shift_key() { 15.0 } else { 5.0 };
        let (dh, ds) = match ev.key().to_ascii_lowercase().as_str() {
            "arrowright" => (step, 0.0),
            "arrowleft" => (-step, 0.0),
            "arrowup" => (0.0, step),
            "arrowdown" => (0.0, -step),
            _ => return,
        };
        ev.prevent_default();
        let from = shown().unwrap_or(Hs { h: 0.0, s: 0.0 });
        on_pick.run(nudged(from, dh, ds));
    };

    let marker_style = move || {
        shown().map_or_else(
            || "display: none".to_owned(),
            |hs| {
                let (left, top) = marker_at(hs);
                format!(
                    "left: {left:.2}%; top: {top:.2}%; background: hsl({} {}% 50%)",
                    hs.h.round(),
                    hs.s.round()
                )
            },
        )
    };
    let lang = use_lang();
    let t = use_device_text();
    let readout = move || {
        shown().map_or_else(
            || t(DeviceText::NoColour).to_owned(),
            |hs| colour_readout(lang(), hs.h, hs.s),
        )
    };

    view! {
        <div class="hs">
            <div
                class="hs__wheel"
                role="slider"
                tabindex="0"
                aria-label=move || t(DeviceText::Colour)
                aria-valuemin="0"
                aria-valuemax="360"
                aria-valuenow=move || shown().map_or(0.0, |hs| hs.h.round()).to_string()
                aria-valuetext=readout
                on:pointerdown=down
                on:pointermove=moved
                on:pointerup=up
                on:pointercancel=cancel
                on:keydown=key
            >
                <span class="hs__marker" style=marker_style></span>
            </div>
            <p class="hs__readout">{readout}</p>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::{Hs, hs_at, marker_at, nudged};

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn angle_is_the_hue_clockwise_from_the_top() {
        assert!(close(hs_at(0.0, -50.0, 100.0).h, 0.0));
        assert!(close(hs_at(50.0, 0.0, 100.0).h, 90.0));
        assert!(close(hs_at(0.0, 50.0, 100.0).h, 180.0));
        assert!(close(hs_at(-50.0, 0.0, 100.0).h, 270.0));
    }

    #[test]
    fn distance_is_the_saturation_and_stops_at_the_rim() {
        assert!(close(hs_at(0.0, 0.0, 100.0).s, 0.0));
        assert!(close(hs_at(30.0, 40.0, 100.0).s, 50.0));
        assert!(close(hs_at(500.0, 0.0, 100.0).s, 100.0));
        assert!(close(hs_at(1.0, 1.0, 0.0).s, 0.0));
    }

    #[test]
    fn marker_inverts_the_pick() {
        let (left, top) = marker_at(Hs { h: 90.0, s: 100.0 });
        assert!(close(left, 100.0) && close(top, 50.0));
        let (left, top) = marker_at(Hs { h: 0.0, s: 0.0 });
        assert!(close(left, 50.0) && close(top, 50.0));
        let pick = hs_at(left - 50.0, top - 50.0, 50.0);
        assert!(close(pick.s, 0.0));
    }

    #[test]
    fn keys_wrap_the_hue_and_clamp_the_saturation() {
        let hs = nudged(Hs { h: 358.0, s: 98.0 }, 5.0, 5.0);
        assert!(close(hs.h, 3.0) && close(hs.s, 100.0));
        let hs = nudged(Hs { h: 2.0, s: 2.0 }, -5.0, -5.0);
        assert!(close(hs.h, 357.0) && close(hs.s, 0.0));
    }
}
