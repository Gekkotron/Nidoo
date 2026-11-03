//! Inline SVG icons, one per kind plus device-class overrides. 24x24 line icons that take the
//! current text colour.

use nidoo_api::AlertChannel;
use nidoo_core::flow::NodeKind;
use nidoo_core::model::{Kind, SwitchType};

macro_rules! svg {
    ($body:literal) => {
        concat!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" focusable="false">"#,
            $body,
            "</svg>"
        )
    };
}

const LIGHT: &str = svg!(
    r#"<path d="M9 18h6"/><path d="M10 21h4"/><path d="M12 3a6 6 0 0 0-3.5 10.9c.6.5 1 1.2 1 2.1h5c0-.9.4-1.6 1-2.1A6 6 0 0 0 12 3z"/>"#
);
const SWITCH: &str = svg!(
    r#"<rect x="2.5" y="7" width="19" height="10" rx="5"/><circle cx="16.5" cy="12" r="2.5"/>"#
);
const COVER: &str = svg!(
    r#"<path d="M4 4h16"/><path d="M5 4v6h14V4"/><path d="M5 10v2h14v-2"/><path d="M12 14v6"/><path d="M9.5 17.5 12 20l2.5-2.5"/>"#
);
/// A wall air conditioner blowing air, so a climate entity never reads as a thermometer.
const CLIMATE: &str = svg!(
    r#"<rect x="3" y="4" width="18" height="9" rx="2"/><path d="M6.5 10h11"/><path d="m8 16-1 3.5"/><path d="M12 16v4"/><path d="m16 16 1 3.5"/>"#
);
const SENSOR: &str = svg!(r#"<path d="M3 12h4l3-7 4 14 3-7h4"/>"#);
const BINARY_SENSOR: &str = svg!(
    r#"<circle cx="12" cy="12" r="3"/><path d="M6.3 6.3a8 8 0 0 0 0 11.4"/><path d="M17.7 6.3a8 8 0 0 1 0 11.4"/>"#
);
const ALARM: &str =
    svg!(r#"<path d="M12 3 5 6v5c0 4.5 3 8 7 10 4-2 7-5.5 7-10V6z"/><path d="m9 12 2 2 4-4"/>"#);
const NUMBER: &str =
    svg!(r#"<path d="M4 8h16"/><path d="M4 16h16"/><path d="M10 4 8 20"/><path d="m16 4-2 16"/>"#);
const SELECT: &str =
    svg!(r#"<rect x="3" y="5" width="18" height="14" rx="3"/><path d="m9 11 3 3 3-3"/>"#);
const BUTTON: &str = svg!(r#"<circle cx="12" cy="12" r="9"/><circle cx="12" cy="12" r="3.5"/>"#);
const VACUUM: &str = svg!(
    r#"<circle cx="12" cy="12" r="9"/><circle cx="12" cy="12" r="3"/><path d="M12 3v3"/><path d="M6 17.5l2-2"/>"#
);
const WATER_HEATER: &str = svg!(
    r#"<path d="M12 3s6 6.2 6 11a6 6 0 0 1-12 0c0-4.8 6-11 6-11z"/><path d="M9.5 14.5a2.5 2.5 0 0 0 2.5 2.5"/>"#
);
const DOOR: &str = svg!(
    r#"<path d="M6 21V4a1 1 0 0 1 1-1h10a1 1 0 0 1 1 1v17"/><path d="M3 21h18"/><circle cx="15" cy="12" r="0.8"/>"#
);
const WINDOW: &str = svg!(
    r#"<rect x="4" y="3" width="16" height="18" rx="2"/><path d="M12 3v18"/><path d="M4 12h16"/>"#
);
const MOTION: &str = svg!(
    r#"<circle cx="13" cy="4.5" r="1.8"/><path d="m9 21 2.5-6-2.5-2.5 1.5-4.5 3.5 2 3 .5"/><path d="m11.5 15 3.5 2 .5 4"/><path d="m4 11 3-2"/>"#
);
const OCCUPANCY: &str = svg!(r#"<circle cx="12" cy="8" r="3.5"/><path d="M5 20a7 7 0 0 1 14 0"/>"#);
const SMOKE: &str = svg!(
    r#"<path d="M4 15c0-2 1.5-3 3-3 .3-2.5 2.2-4 4.5-4S15.7 9.5 16 12c2 0 4 1 4 3s-1.5 3-3.5 3h-9C5.5 18 4 17 4 15z"/>"#
);
const POWER: &str = svg!(r#"<path d="M13 2 4 14h7l-1 8 9-12h-7z"/>"#);
const TEMPERATURE: &str = svg!(
    r#"<path d="M14 14.8V5a2 2 0 0 0-4 0v9.8a4 4 0 1 0 4 0z"/><path d="M12 9v7"/><path d="M17.5 6h3"/><path d="M17.5 10h3"/>"#
);
const HUMIDITY: &str = svg!(r#"<path d="M12 3s6 6.2 6 11a6 6 0 0 1-12 0c0-4.8 6-11 6-11z"/>"#);
const BATTERY: &str = svg!(
    r#"<rect x="2.5" y="7" width="17" height="10" rx="2.5"/><path d="M22 10.5v3"/><path d="M6 10.5v3"/><path d="M10 10.5v3"/>"#
);
const ILLUMINANCE: &str = svg!(
    r#"<circle cx="12" cy="12" r="4"/><path d="M12 2.5v2.5"/><path d="M12 19v2.5"/><path d="m4.9 4.9 1.8 1.8"/><path d="m17.3 17.3 1.8 1.8"/><path d="M2.5 12H5"/><path d="M19 12h2.5"/><path d="m4.9 19.1 1.8-1.8"/><path d="m17.3 6.7 1.8-1.8"/>"#
);

/// Cover control glyphs: open (up), stop and close (down).
pub const CHEVRON_UP: &str = svg!(r#"<path d="m6 15 6-6 6 6"/>"#);
pub const STOP: &str = svg!(r#"<rect x="7" y="7" width="10" height="10" rx="2"/>"#);
pub const CHEVRON_DOWN: &str = svg!(r#"<path d="m6 9 6 6 6-6"/>"#);

/// Vacuum action glyphs: pause, locate and spot clean (start is `PLAY`, stop `STOP`, dock `HOUSE`).
pub const PAUSE: &str = svg!(
    r#"<rect x="6.5" y="5" width="3.5" height="14" rx="1"/><rect x="14" y="5" width="3.5" height="14" rx="1"/>"#
);
const LOCATE: &str = svg!(
    r#"<circle cx="12" cy="12" r="7"/><circle cx="12" cy="12" r="2"/><path d="M12 2v3"/><path d="M12 19v3"/><path d="M2 12h3"/><path d="M19 12h3"/>"#
);
const SPOT: &str = svg!(
    r#"<circle cx="12" cy="12" r="2.5"/><path d="M12 5.5a6.5 6.5 0 1 1-6.5 6.5"/><path d="M12 2a10 10 0 1 1-10 10"/>"#
);
/// The glyph for a vacuum action, a generic button for one the hub does not name.
pub fn vacuum_action_icon(action: &str) -> &'static str {
    match action {
        "start" => PLAY,
        "pause" => PAUSE,
        "stop" => STOP,
        "return_to_base" => HOUSE,
        "locate" => LOCATE,
        "clean_spot" => SPOT,
        _ => BUTTON,
    }
}

/// The rename button.
pub const PENCIL: &str =
    svg!(r#"<path d="M16.5 3.5a2.1 2.1 0 0 1 3 3L8 18l-4 1 1-4z"/><path d="m14.5 5.5 3 3"/>"#);

/// Save confirm (check mark).
pub const CHECK: &str = svg!(r#"<path d="m5 12 5 5 9-11"/>"#);
/// Cancel / close (X).
pub const X_MARK: &str = svg!(r#"<path d="M6 6l12 12"/><path d="M18 6 6 18"/>"#);
/// Drag handle: two columns of three dots.
pub const GRIP: &str = svg!(
    r#"<circle cx="9" cy="6" r=".9"/><circle cx="15" cy="6" r=".9"/><circle cx="9" cy="12" r=".9"/><circle cx="15" cy="12" r=".9"/><circle cx="9" cy="18" r=".9"/><circle cx="15" cy="18" r=".9"/>"#
);

const NODE_BOLT: &str = svg!(r#"<path d="M13 2 4 14h7l-1 8 9-12h-7z"/>"#);
const NODE_FORK: &str = svg!(
    r#"<circle cx="6" cy="5" r="2"/><circle cx="18" cy="19" r="2"/><circle cx="18" cy="5" r="2"/><path d="M6 7v3a3 3 0 0 0 3 3h6a3 3 0 0 1 3 3v1"/><path d="M18 7v3"/>"#
);
const NODE_HOURGLASS: &str = svg!(
    r#"<path d="M6 3h12"/><path d="M6 21h12"/><path d="M7 3v3a5 5 0 0 0 2 4l3 2-3 2a5 5 0 0 0-2 4v3"/><path d="M17 3v3a5 5 0 0 1-2 4l-3 2 3 2a5 5 0 0 1 2 4v3"/>"#
);
const NODE_SILENT: &str = svg!(r#"<path d="M13 2 4 14h7l-1 8 9-12h-7z"/><path d="M3 3l18 18"/>"#);
const NODE_LIMIT: &str = svg!(r#"<circle cx="12" cy="12" r="9"/><path d="M5.6 5.6l12.8 12.8"/>"#);
const NODE_CLOCK: &str = svg!(r#"<circle cx="12" cy="12" r="9"/><path d="M12 7v5l3.5 2"/>"#);
const NODE_MOON: &str = svg!(r#"<path d="M20 14.5A8 8 0 0 1 9.5 4a8 8 0 1 0 10.5 10.5z"/>"#);
const NODE_SWITCH: &str =
    svg!(r#"<rect x="3" y="8" width="18" height="8" rx="4"/><circle cx="16" cy="12" r="2"/>"#);
const NODE_PLAY_CIRCLE: &str =
    svg!(r#"<circle cx="12" cy="12" r="9"/><path d="M10 8.5v7l6-3.5z"/>"#);
// A plus over an equals sign, the math card.
const NODE_MATH: &str = svg!(r#"<path d="M12 4v8M8 8h8M8 16.5h8M8 20h8"/>"#);
const NODE_PLAY: &str = svg!(r#"<path d="M7 4.5v15l12-7.5z"/>"#);

// Energy and Security glyphs.
pub const SUN: &str = svg!(
    r#"<circle cx="12" cy="12" r="4"/><path d="M12 2.5v2.5"/><path d="M12 19v2.5"/><path d="m4.9 4.9 1.8 1.8"/><path d="m17.3 17.3 1.8 1.8"/><path d="M2.5 12H5"/><path d="M19 12h2.5"/><path d="m4.9 19.1 1.8-1.8"/><path d="m17.3 6.7 1.8-1.8"/>"#
);
pub const PYLON: &str = svg!(
    r#"<path d="M9 21 11 3h2l2 18"/><path d="M5 8h14"/><path d="M6.5 13h11"/><path d="m9.5 21 5-8"/><path d="m14.5 21-5-8"/>"#
);
pub const HOUSE: &str =
    svg!(r#"<path d="M3 11.5 12 4l9 7.5"/><path d="M5.5 10v10h13V10"/><path d="M10 20v-6h4v6"/>"#);

/// Nav rail: alerts, flows, devices, settings — plus moon, sun and sun-moon for the theme button.
pub const BELL: &str = svg!(
    r#"<path d="M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9"/><path d="M10 21a2 2 0 0 0 4 0"/>"#
);
pub const FLOW_NODES: &str = svg!(
    r#"<circle cx="5" cy="6" r="2"/><circle cx="19" cy="6" r="2"/><circle cx="12" cy="18" r="2"/><path d="M6.5 7.4 10.5 16.6"/><path d="M17.5 7.4 13.5 16.6"/>"#
);
pub const LAYERS: &str = svg!(
    r#"<path d="M12 3 3 7.5l9 4.5 9-4.5z"/><path d="m3 12 9 4.5 9-4.5"/><path d="m3 16.5 9 4.5 9-4.5"/>"#
);
pub const DEVICES_GRID: &str = svg!(
    r#"<rect x="3" y="3" width="7" height="7" rx="1.5"/><rect x="14" y="3" width="7" height="7" rx="1.5"/><rect x="3" y="14" width="7" height="7" rx="1.5"/><rect x="14" y="14" width="7" height="7" rx="1.5"/>"#
);
pub const GEAR: &str = svg!(
    r#"<circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.7 1.7 0 0 0 .3 1.9l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.9-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1A1.7 1.7 0 0 0 9 19.4a1.7 1.7 0 0 0-1.9.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.9 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1 1.7 1.7 0 0 0-.3-1.9l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.9.3 1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.9-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.9 1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z"/>"#
);
pub const MOON: &str = svg!(r#"<path d="M20 14A9 9 0 1 1 10 4a7 7 0 0 0 10 10"/>"#);
/// Half-lit disc: the theme follows the system.
pub const SUN_MOON: &str = svg!(
    r#"<circle cx="12" cy="12" r="8.5"/><path d="M12 3.5a8.5 8.5 0 0 0 0 17z" fill="currentColor"/>"#
);
pub const ARROW_UP: &str = svg!(r#"<path d="M12 19V5"/><path d="m6 11 6-6 6 6"/>"#);
pub const PLUS: &str = svg!(r#"<path d="M12 5v14"/><path d="M5 12h14"/>"#);
pub const MAGNIFIER: &str = svg!(r#"<circle cx="11" cy="11" r="7"/><path d="m16.5 16.5 4 4"/>"#);
pub const TRASH: &str = svg!(
    r#"<path d="M4 7h16"/><path d="M9 7V4h6v3"/><path d="M6 7v13h12V7"/><path d="M10 11v6"/><path d="M14 11v6"/>"#
);
pub const PLUG: &str = svg!(
    r#"<path d="M9 3v5"/><path d="M15 3v5"/><path d="M6 8h12v3a6 6 0 0 1-12 0z"/><path d="M12 17v4"/>"#
);
pub const PLAY: &str = svg!(r#"<path d="M7 4.5v15l12-7.5z"/>"#);
const SIREN: &str = svg!(
    r#"<path d="M7 18v-6a5 5 0 0 1 10 0v6"/><path d="M5 18h14v3H5z"/><path d="M12 2v2M4.2 5.2l1.4 1.4M19.8 5.2l-1.4 1.4"/>"#
);
const CHIME: &str = svg!(
    r#"<path d="M9 18V5l11-2v13"/><circle cx="6" cy="18" r="3"/><circle cx="17" cy="16" r="3"/>"#
);
pub const WAVES: &str = svg!(
    r#"<path d="M2.5 9c2-2 4-2 6 0s4 2 6 0 4-2 6 0"/><path d="M2.5 15c2-2 4-2 6 0s4 2 6 0 4-2 6 0"/>"#
);
pub const LOCK_OPEN: &str = svg!(
    r#"<rect x="4" y="11" width="16" height="10" rx="2"/><path d="M8 11V7a4 4 0 0 1 7.6-1.7"/><circle cx="12" cy="16" r="1.2"/>"#
);
const LOCK: &str = svg!(
    r#"<rect x="4" y="11" width="16" height="10" rx="2"/><path d="M8 11V7a4 4 0 0 1 8 0v4"/><circle cx="12" cy="16" r="1.2"/>"#
);
const FAN: &str = svg!(
    r#"<circle cx="12" cy="12" r="1.8"/><path d="M12 10.2C12 6 10 3 8 4s-1 5 2.4 6.4M13.8 12c4.2 0 7.2-2 6.2-4s-5-1-6.4 2.4M12 13.8C12 18 14 21 16 20s1-5-2.4-6.4M10.2 12C6 12 3 14 4 16s5 1 6.4-2.4"/>"#
);
const VALVE: &str = svg!(
    r#"<path d="M3 12h4m10 0h4"/><path d="M7 8v8l5-4zM17 8v8l-5-4z"/><path d="M12 12V5m-3 0h6"/>"#
);
const IMAGE: &str = svg!(
    r#"<rect x="3" y="5" width="18" height="14" rx="2"/><circle cx="9" cy="10" r="1.5"/><path d="m4 18 5-5 4 4 3-3 4 4"/>"#
);
const SCENE: &str =
    svg!(r#"<path d="m12 3 2.1 5.4L20 9l-4.5 3.8L17 19l-5-3.2L7 19l1.5-6.2L4 9l5.9-.6z"/>"#);
const TAG: &str = svg!(r#"<path d="M3 12V4h8l10 10-8 8z"/><circle cx="7.5" cy="8.5" r="1.2"/>"#);
const UPDATE: &str = svg!(r#"<path d="M12 4v11m-4-4 4 4 4-4"/><path d="M5 20h14"/>"#);
pub const SHIELD: &str = CHANNEL_SECURITY;
pub const THERMOMETER: &str = CHANNEL_CLIMATE;
pub const BOLT: &str = CHANNEL_ENERGY;
pub const CAMERA: &str = CHANNEL_CAMERA;
pub const DROP: &str = HUMIDITY;
pub const WATER_HEATER_GLYPH: &str = WATER_HEATER;
// Weather conditions (clear reuses SUN and MOON).
pub const PARTLY_CLOUDY_DAY: &str = svg!(
    r#"<circle cx="8" cy="8" r="2.5"/><path d="M8 2.5v1.5"/><path d="M2.5 8H4"/><path d="m4.1 4.1 1 1"/><path d="m11.9 4.1-1 1"/><path d="M18 20.5h-8a3.5 3.5 0 0 1-.3-7A4.7 4.7 0 0 1 18.3 12.6 3.9 3.9 0 0 1 18 20.5z"/>"#
);
pub const PARTLY_CLOUDY_NIGHT: &str = svg!(
    r#"<path d="M10.5 3.5a4.5 4.5 0 1 0 5.5 6.2 3.5 3.5 0 0 1-5.5-6.2z"/><path d="M18 20.5h-8a3.5 3.5 0 0 1-.3-7A4.7 4.7 0 0 1 18.3 12.6 3.9 3.9 0 0 1 18 20.5z"/>"#
);
pub const CLOUDY: &str = svg!(
    r#"<path d="M17.5 19H8a4.5 4.5 0 0 1-.6-8.96A6 6 0 0 1 18.9 9.9 4.6 4.6 0 0 1 17.5 19z"/>"#
);
pub const FOG: &str = svg!(
    r#"<path d="M17.5 14.5H8a3.5 3.5 0 0 1-.4-6.98A5 5 0 0 1 17.3 7 3.8 3.8 0 0 1 17.5 14.5z"/><path d="M5 18h14"/><path d="M8 21h8"/>"#
);
pub const DRIZZLE: &str = svg!(
    r#"<path d="M17.5 14.5H8a3.5 3.5 0 0 1-.4-6.98A5 5 0 0 1 17.3 7 3.8 3.8 0 0 1 17.5 14.5z"/><path d="M9 18v.5"/><path d="M12 18v.5"/><path d="M15 18v.5"/><path d="M10.5 21v.5"/><path d="M13.5 21v.5"/>"#
);
pub const RAIN: &str = svg!(
    r#"<path d="M17.5 14.5H8a3.5 3.5 0 0 1-.4-6.98A5 5 0 0 1 17.3 7 3.8 3.8 0 0 1 17.5 14.5z"/><path d="M9 17.5l-1 3"/><path d="M13 17.5l-1 3"/><path d="M17 17.5l-1 3"/>"#
);
pub const FREEZING_RAIN: &str = svg!(
    r#"<path d="M17.5 14.5H8a3.5 3.5 0 0 1-.4-6.98A5 5 0 0 1 17.3 7 3.8 3.8 0 0 1 17.5 14.5z"/><path d="M9 17.5l-1 3"/><path d="M16 17.5l-1 3"/><path d="M12 17v4"/><path d="m10.5 18.5 3 1"/><path d="m13.5 18.5-3 1"/>"#
);
pub const SNOW: &str = svg!(
    r#"<path d="M17.5 14.5H8a3.5 3.5 0 0 1-.4-6.98A5 5 0 0 1 17.3 7 3.8 3.8 0 0 1 17.5 14.5z"/><path d="M8.5 18v3"/><path d="M7 19.5h3"/><path d="M15.5 18v3"/><path d="M14 19.5h3"/>"#
);
pub const SHOWERS: &str = svg!(
    r#"<path d="M17.5 14.5H8a3.5 3.5 0 0 1-.4-6.98A5 5 0 0 1 17.3 7 3.8 3.8 0 0 1 17.5 14.5z"/><path d="M8 17.5l-1.5 4"/><path d="M12.5 17.5 11 21.5"/><path d="M17 17.5l-1.5 4"/><path d="M10 17.5l-.5 1.5"/>"#
);
pub const THUNDERSTORM: &str = svg!(
    r#"<path d="M17.5 14.5H8a3.5 3.5 0 0 1-.4-6.98A5 5 0 0 1 17.3 7 3.8 3.8 0 0 1 17.5 14.5z"/><path d="m12.5 15-2.5 3.5h3.5L11 22"/>"#
);
pub const HEAT_PUMP: &str = svg!(
    r#"<rect x="3" y="6" width="18" height="12" rx="2"/><circle cx="12" cy="12" r="3.5"/><path d="M12 8.5v7"/><path d="M8.5 12h7"/>"#
);
pub const OVEN: &str = svg!(
    r#"<rect x="3" y="3" width="18" height="18" rx="2"/><path d="M3 8h18"/><rect x="6" y="11" width="12" height="7" rx="1"/><circle cx="7" cy="5.5" r="0.8"/><circle cx="10" cy="5.5" r="0.8"/>"#
);
pub const FRIDGE: &str = svg!(
    r#"<rect x="5" y="2.5" width="14" height="19" rx="2"/><path d="M5 10h14"/><path d="M9 6v2"/><path d="M9 13v3"/>"#
);
pub const WASHER: &str = svg!(
    r#"<rect x="3" y="3" width="18" height="18" rx="2"/><circle cx="12" cy="13" r="5"/><path d="M8.5 12.5c1.5-1.5 3.5 1.5 5 0s2 .5 2 .5"/><circle cx="7" cy="6" r="0.8"/>"#
);
pub const DRYER: &str = svg!(
    r#"<rect x="3" y="3" width="18" height="18" rx="2"/><circle cx="12" cy="13" r="5"/><path d="M10 11.5c0 2 4 1 4 3"/><circle cx="7" cy="6" r="0.8"/><circle cx="17" cy="6" r="0.8"/>"#
);
pub const DISHWASHER: &str = svg!(
    r#"<rect x="3" y="3" width="18" height="18" rx="2"/><path d="M3 8h18"/><path d="M7 12h10"/><path d="M7 15.5h10"/><circle cx="7" cy="5.5" r="0.8"/>"#
);

/// The glyph of a Security sensor tile, by device class: the kind icon's door, window, motion,
/// occupancy and smoke, plus a drop for leaks, a shield for SOS, waves for vibration and an
/// open lock for tamper.
pub fn sensor_icon(device_class: Option<&str>) -> &'static str {
    match device_class {
        Some("door" | "garage_door" | "opening") => DOOR,
        Some("window") => WINDOW,
        Some("motion") => MOTION,
        Some("occupancy" | "presence") => OCCUPANCY,
        Some("smoke" | "carbon_monoxide" | "gas") => SMOKE,
        Some("moisture") => DROP,
        Some("safety") => SHIELD,
        Some("vibration") => WAVES,
        Some("tamper") => LOCK_OPEN,
        _ => BINARY_SENSOR,
    }
}

/// The glyph of a consumer node by its `energyIcon` key; the plug when unset or unknown.
pub fn consumer_glyph(key: Option<&str>) -> &'static str {
    match key {
        Some("water_heater") => WATER_HEATER_GLYPH,
        Some("heat_pump") => HEAT_PUMP,
        Some("oven") => OVEN,
        Some("fridge") => FRIDGE,
        Some("washer") => WASHER,
        Some("dryer") => DRYER,
        Some("dishwasher") => DISHWASHER,
        _ => PLUG,
    }
}

/// The glyph of an Energy diagram node by its kind.
pub fn role_icon(kind: nidoo_core::energy::NodeKind) -> &'static str {
    use nidoo_core::energy::NodeKind as K;
    match kind {
        K::Solar => SUN,
        K::Grid => PYLON,
        K::Battery => BATTERY,
        K::House => HOUSE,
        K::Consumer | K::More => PLUG,
        K::Water => DROP,
    }
}

/// The glyph of a consumer node by its chosen icon; the plug when none.
pub fn consumer_icon(icon: Option<nidoo_core::energy::ConsumerIcon>) -> &'static str {
    consumer_glyph(icon.map(nidoo_core::energy::ConsumerIcon::key))
}

/// The glyph of a flow node on the canvas: a bolt for a trigger, a fork for an `if` or a
/// switch, a clock for a schedule or a time-of-day check, a sun for a solar event, a moon for an
/// `if` on darkness, an hourglass for a delay, a play triangle for a command, a toggle for a
/// flow switch and a bell for a notification.
pub fn node_icon(kind: &NodeKind) -> &'static str {
    match kind {
        NodeKind::TriggerState(_) => NODE_BOLT,
        NodeKind::TriggerSchedule(_) => NODE_CLOCK,
        NodeKind::TriggerSun(_) => SUN,
        NodeKind::TriggerOffPeak(_) => NODE_BOLT,
        NodeKind::TriggerSilent(_) => NODE_SILENT,
        NodeKind::TriggerSignal(_) => NODE_BOLT,
        NodeKind::TriggerManual(_) => NODE_PLAY_CIRCLE,
        NodeKind::If(p) if p.op == nidoo_core::flow::Op::IsDark => NODE_MOON,
        NodeKind::If(_) | NodeKind::Switch(_) => NODE_FORK,
        NodeKind::TimeBetween(_) => NODE_CLOCK,
        NodeKind::Delay(_) => NODE_HOURGLASS,
        NodeKind::Limit(_) => NODE_LIMIT,
        NodeKind::Command(_) => NODE_PLAY,
        NodeKind::FlowEnable(_) => NODE_SWITCH,
        NodeKind::Notify(_) => CHANNEL_ALARM,
        NodeKind::Math(_) => NODE_MATH,
    }
}

const NODE_SIGNAL: &str = svg!(
    r#"<circle cx="12" cy="12" r="2.5"/><path d="M7.5 7.5a6.4 6.4 0 0 0 0 9"/><path d="M16.5 7.5a6.4 6.4 0 0 1 0 9"/>"#
);

/// The icon of a flow in the Flows list, by its first trigger kind.
pub fn trigger_icon(kind: crate::flow_list::TriggerKind) -> &'static str {
    use crate::flow_list::TriggerKind;
    match kind {
        TriggerKind::State => NODE_BOLT,
        TriggerKind::Schedule => NODE_CLOCK,
        TriggerKind::Sun => SUN,
        TriggerKind::OffPeak => NODE_BOLT,
        TriggerKind::Silent => NODE_SILENT,
        TriggerKind::Signal => NODE_SIGNAL,
        TriggerKind::Manual => NODE_PLAY_CIRCLE,
    }
}

const CHANNEL_ALARM: &str =
    svg!(r#"<path d="M6 16V11a6 6 0 0 1 12 0v5l1.5 2h-15z"/><path d="M10 21h4"/>"#);
const CHANNEL_SECURITY: &str = svg!(r#"<path d="M12 3 5 6v5c0 4.5 3 8 7 10 4-2 7-5.5 7-10V6z"/>"#);
const CHANNEL_CAMERA: &str = svg!(
    r#"<path d="M4 8h3l1.5-2.5h7L17 8h3a1 1 0 0 1 1 1v9a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V9a1 1 0 0 1 1-1z"/><circle cx="12" cy="13" r="3.5"/>"#
);
const CHANNEL_ENERGY: &str = svg!(r#"<path d="M13 2 4 14h7l-1 8 9-12h-7z"/>"#);
const CHANNEL_CLIMATE: &str = svg!(
    r#"<path d="M14 14.8V5a2 2 0 0 0-4 0v9.8a4 4 0 1 0 4 0z"/><path d="M12 9v7"/><path d="M17.5 7h3"/>"#
);
const CHANNEL_SYSTEM: &str = svg!(
    r#"<circle cx="12" cy="12" r="3"/><path d="M12 2.5v3"/><path d="M12 18.5v3"/><path d="M2.5 12h3"/><path d="M18.5 12h3"/><path d="m5.3 5.3 2.1 2.1"/><path d="m16.6 16.6 2.1 2.1"/><path d="m5.3 18.7 2.1-2.1"/><path d="m16.6 7.4 2.1-2.1"/>"#
);
const CHANNEL_MISC: &str = svg!(
    r#"<circle cx="12" cy="12" r="9"/><path d="M12 11v5"/><circle cx="12" cy="7.8" r="0.8"/>"#
);

/// The glyph of an alert channel: bell, shield, camera, bolt, thermometer, gear, info circle.
pub fn channel_icon(channel: AlertChannel) -> &'static str {
    match channel {
        AlertChannel::Alarm => CHANNEL_ALARM,
        AlertChannel::Security => CHANNEL_SECURITY,
        AlertChannel::Camera => CHANNEL_CAMERA,
        AlertChannel::Energy => CHANNEL_ENERGY,
        AlertChannel::Climate => CHANNEL_CLIMATE,
        AlertChannel::System => CHANNEL_SYSTEM,
        AlertChannel::Misc => CHANNEL_MISC,
    }
}

/// The icon for an entity: the device class wins when we draw one, else the kind's.
/// The icon of an entity card or sheet: the switch type's glyph when a switch is shown as
/// something else, else [`kind_icon`].
pub fn entity_icon(entity: &nidoo_api::EntityView) -> &'static str {
    if entity.kind == Kind::Generic {
        return generic_icon(&entity.component);
    }
    match SwitchType::of(entity.protocol_kind, &entity.ui) {
        Some(SwitchType::Door) => DOOR,
        Some(SwitchType::Siren) => SIREN,
        Some(SwitchType::Chime) => CHIME,
        Some(SwitchType::Plug) => PLUG,
        _ => kind_icon(entity.kind, entity.device_class.as_deref()),
    }
}

/// Placeholder glyph for a table-described component until the generic cards land: the
/// table's icon name where we draw it, else the sensor glyph.
fn generic_icon(component: &str) -> &'static str {
    let name = nidoo_core::types::type_def(component).map(|d| d.icon.as_str());
    match name {
        Some("lock") => LOCK,
        Some("fan") => FAN,
        Some("valve") => VALVE,
        Some("image") => IMAGE,
        Some("scene") => SCENE,
        Some("tag") => TAG,
        Some("tracker") => OCCUPANCY,
        Some("notify") => CHIME,
        Some("update") => UPDATE,
        Some("text") => SELECT,
        Some("lawn_mower") => VACUUM,
        Some("siren") => SIREN,
        Some("humidifier") => HUMIDITY,
        Some("camera") => CHANNEL_CAMERA,
        _ => SENSOR,
    }
}

pub fn kind_icon(kind: Kind, device_class: Option<&str>) -> &'static str {
    if let Some(icon) = device_class.and_then(device_class_icon) {
        return icon;
    }
    match kind {
        Kind::Light => LIGHT,
        Kind::Switch => SWITCH,
        Kind::Cover => COVER,
        Kind::Climate => CLIMATE,
        Kind::Sensor => SENSOR,
        Kind::BinarySensor => BINARY_SENSOR,
        Kind::AlarmPanel => ALARM,
        Kind::Number => NUMBER,
        Kind::Select => SELECT,
        Kind::Button => BUTTON,
        Kind::Vacuum => VACUUM,
        Kind::WaterHeater => WATER_HEATER,
        Kind::Generic => SENSOR,
    }
}

fn device_class_icon(device_class: &str) -> Option<&'static str> {
    Some(match device_class {
        "door" => DOOR,
        "window" => WINDOW,
        "motion" => MOTION,
        "occupancy" => OCCUPANCY,
        "smoke" => SMOKE,
        "power" | "energy" => POWER,
        "temperature" => TEMPERATURE,
        "humidity" => HUMIDITY,
        "battery" => BATTERY,
        "illuminance" => ILLUMINANCE,
        _ => return None,
    })
}

// Settings list glyphs: one per sub-page that has no device glyph to borrow.
pub const PIN: &str = svg!(
    r#"<path d="M12 21s-7-6.2-7-11.5a7 7 0 0 1 14 0C19 14.8 12 21 12 21z"/><circle cx="12" cy="9.5" r="2.5"/>"#
);
pub const WIFI: &str = svg!(
    r#"<path d="M2.5 9a14 14 0 0 1 19 0"/><path d="M5.5 12.5a9.5 9.5 0 0 1 13 0"/><path d="M8.5 16a5 5 0 0 1 7 0"/><path d="M12 19.5h.01"/>"#
);
/// Three linked nodes: a Zigbee mesh.
pub const MESH: &str = svg!(
    r#"<circle cx="5.5" cy="6" r="2.2"/><circle cx="18.5" cy="6" r="2.2"/><circle cx="12" cy="18" r="2.2"/><path d="M7.7 6h8.6"/><path d="m6.6 8 4.3 8"/><path d="m17.4 8-4.3 8"/>"#
);
/// Messages both ways: the MQTT broker.
pub const BROKER: &str = svg!(
    r#"<path d="M4 8h14"/><path d="m14 4 4 4-4 4"/><path d="M20 16H6"/><path d="m10 12-4 4 4 4"/>"#
);
pub const DATABASE: &str = svg!(
    r#"<ellipse cx="12" cy="5.5" rx="7.5" ry="2.8"/><path d="M4.5 5.5v13c0 1.5 3.4 2.8 7.5 2.8s7.5-1.3 7.5-2.8v-13"/><path d="M4.5 12c0 1.5 3.4 2.8 7.5 2.8s7.5-1.3 7.5-2.8"/>"#
);
/// A heartbeat line: the hub's health.
pub const PULSE: &str = svg!(r#"<path d="M3 12h4l2.5-6 4 12 2.5-6H21"/>"#);
pub const INFO: &str =
    svg!(r#"<circle cx="12" cy="12" r="9"/><path d="M12 11v5.5"/><path d="M12 7.5h.01"/>"#);
pub const COPY: &str = svg!(
    r#"<rect x="8.5" y="8.5" width="12" height="12" rx="2.5"/><path d="M15.5 8.5V6A2.5 2.5 0 0 0 13 3.5H6A2.5 2.5 0 0 0 3.5 6v7A2.5 2.5 0 0 0 6 15.5h2.5"/>"#
);

/// A speech bubble: the assistant.
pub const SPEECH: &str = svg!(
    r#"<path d="M4 5.5A2.5 2.5 0 0 1 6.5 3h11A2.5 2.5 0 0 1 20 5.5v8a2.5 2.5 0 0 1-2.5 2.5H10l-4.5 4v-4h0A1.5 1.5 0 0 1 4 14.5z"/><path d="M8.5 9.5h.01"/><path d="M12 9.5h.01"/><path d="M15.5 9.5h.01"/>"#
);

// Ambiance glyphs. Moon, sun and house reuse the shared constants.
pub const SPARKLE: &str = svg!(
    r#"<path d="M12 3l1.8 5.2L19 10l-5.2 1.8L12 17l-1.8-5.2L5 10l5.2-1.8z"/><path d="M19 15v4"/><path d="M17 17h4"/>"#
);
const SOFA: &str = svg!(
    r#"<path d="M5 11V8a2 2 0 0 1 2-2h10a2 2 0 0 1 2 2v3"/><path d="M3 13a2 2 0 0 1 4 0v2h10v-2a2 2 0 0 1 4 0v4H3z"/><path d="M5 17v2"/><path d="M19 17v2"/>"#
);
const DOOR_OUT: &str = svg!(
    r#"<path d="M10 21H6a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1h4"/><path d="M14 8l4 4-4 4"/><path d="M18 12H9"/>"#
);
const BOOK: &str = svg!(
    r#"<path d="M4 5.5A1.5 1.5 0 0 1 5.5 4H19v13H5.5A1.5 1.5 0 0 0 4 18.5z"/><path d="M4 18.5A1.5 1.5 0 0 0 5.5 20H19v-3"/><path d="M9 8h6"/>"#
);
const FILM: &str = svg!(
    r#"<rect x="3" y="4" width="18" height="16" rx="2"/><path d="M7 4v16"/><path d="M17 4v16"/><path d="M3 9h4"/><path d="M3 15h4"/><path d="M17 9h4"/><path d="M17 15h4"/>"#
);
const PLATE: &str = svg!(r#"<circle cx="12" cy="12" r="8.5"/><circle cx="12" cy="12" r="4.5"/>"#);
const BATH: &str = svg!(
    r#"<path d="M3 12h18v2a5 5 0 0 1-5 5H8a5 5 0 0 1-5-5z"/><path d="M6 12V6a2 2 0 0 1 3.5-1.3"/><path d="M7 19l-1 2"/><path d="M17 19l1 2"/>"#
);
const LEAF: &str =
    svg!(r#"<path d="M5 19C5 10 10 5 20 4c0 10-4 15-13 15"/><path d="M5 19 14 10"/>"#);
const FLAME: &str = svg!(
    r#"<path d="M12 3c1 3.5 5.5 5.5 5.5 11a5.5 5.5 0 0 1-11 0c0-2.2 1-3.6 2-4.5.2 1.6 1 2.4 1.8 2.6C10 9 10.8 5.5 12 3z"/>"#
);

/// The glyph for an ambiance icon name; unknown or missing names fall back to the sparkle.
pub fn ambiance_icon(name: Option<&str>) -> &'static str {
    match name {
        Some("sofa") => SOFA,
        Some("moon") => MOON,
        Some("door_out") => DOOR_OUT,
        Some("book") => BOOK,
        Some("sun") => SUN,
        Some("film") => FILM,
        Some("plate") => PLATE,
        Some("bath") => BATH,
        Some("leaf") => LEAF,
        Some("flame") => FLAME,
        Some("house") => HOUSE,
        _ => SPARKLE,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BINARY_SENSOR, PLUG, SPARKLE, ambiance_icon, channel_icon, consumer_glyph, kind_icon,
        sensor_icon,
    };
    use nidoo_api::AlertChannel;
    use nidoo_core::model::Kind;

    fn switch(show_as: Option<&str>, kind: Kind) -> nidoo_api::EntityView {
        use nidoo_core::model::{Capabilities, EntityCategory, EntityState, UiOverride};
        nidoo_api::EntityView {
            id: "s".into(),
            device_id: None,
            name: "s".into(),
            device_name: None,
            label: None,
            kind,
            component: kind.component().to_string(),
            protocol_kind: kind,
            device_class: None,
            unit: None,
            icon: None,
            category: EntityCategory::Primary,
            enabled_by_default: true,
            area: "a".into(),
            part_of: None,
            capabilities: Capabilities::default(),
            ui: UiOverride {
                show_as: show_as.map(Into::into),
                ..UiOverride::default()
            },
            state: EntityState::default(),
        }
    }

    #[test]
    fn entity_icon_follows_the_switch_type() {
        use super::{CHIME, DOOR, LIGHT, SIREN, SWITCH, entity_icon};
        assert_eq!(entity_icon(&switch(Some("door"), Kind::Switch)), DOOR);
        assert_eq!(entity_icon(&switch(Some("siren"), Kind::Switch)), SIREN);
        assert_eq!(entity_icon(&switch(Some("chime"), Kind::Switch)), CHIME);
        assert_eq!(entity_icon(&switch(Some("plug"), Kind::Switch)), PLUG);
        assert_eq!(entity_icon(&switch(None, Kind::Switch)), SWITCH);
        let mut light_typed = switch(Some("light"), Kind::Switch);
        light_typed.kind = Kind::Light;
        assert_eq!(entity_icon(&light_typed), LIGHT);
        assert_eq!(
            entity_icon(&switch(Some("door"), Kind::Cover)),
            kind_icon(Kind::Cover, None)
        );
    }

    #[test]
    fn sensor_icons_cover_every_group() {
        let classes = [
            "door",
            "window",
            "garage_door",
            "opening",
            "motion",
            "occupancy",
            "presence",
            "smoke",
            "carbon_monoxide",
            "gas",
            "moisture",
            "safety",
            "vibration",
            "tamper",
        ];
        for class in classes {
            let icon = sensor_icon(Some(class));
            assert!(icon.starts_with("<svg"), "{class}");
            assert_ne!(icon, BINARY_SENSOR, "{class} has no glyph of its own");
        }
        assert_eq!(sensor_icon(Some("battery")), BINARY_SENSOR);
        assert_eq!(sensor_icon(None), BINARY_SENSOR);
    }

    #[test]
    fn role_icons_follow_the_app() {
        use nidoo_core::energy::{ConsumerIcon, NodeKind as K};
        assert_eq!(super::role_icon(K::Solar), super::SUN);
        assert_eq!(super::role_icon(K::Grid), super::PYLON);
        assert_eq!(super::role_icon(K::Battery), super::BATTERY);
        assert_eq!(super::role_icon(K::House), super::HOUSE);
        assert_eq!(super::role_icon(K::Consumer), PLUG);
        assert_eq!(super::role_icon(K::More), PLUG);
        assert_eq!(super::role_icon(K::Water), super::DROP);
        assert_eq!(super::consumer_icon(None), PLUG);
        assert_eq!(super::consumer_icon(Some(ConsumerIcon::Oven)), super::OVEN);
    }

    #[test]
    fn consumer_glyphs_default_to_the_plug() {
        assert_eq!(consumer_glyph(None), PLUG);
        assert_eq!(consumer_glyph(Some("fusion")), PLUG);
        let keys = [
            "water_heater",
            "heat_pump",
            "oven",
            "fridge",
            "washer",
            "dryer",
            "dishwasher",
        ];
        let glyphs: Vec<_> = keys.iter().map(|k| consumer_glyph(Some(k))).collect();
        for (i, g) in glyphs.iter().enumerate() {
            assert_ne!(*g, PLUG, "{}", keys[i]);
            assert!(!glyphs[..i].contains(g), "duplicate glyph for {}", keys[i]);
        }
    }

    const KINDS: [Kind; 12] = [
        Kind::Light,
        Kind::Switch,
        Kind::Cover,
        Kind::Climate,
        Kind::Sensor,
        Kind::BinarySensor,
        Kind::AlarmPanel,
        Kind::Number,
        Kind::Select,
        Kind::Button,
        Kind::Vacuum,
        Kind::WaterHeater,
    ];

    #[test]
    fn every_kind_has_a_distinct_svg() {
        let icons: Vec<_> = KINDS.iter().map(|k| kind_icon(*k, None)).collect();
        for (i, icon) in icons.iter().enumerate() {
            assert!(icon.starts_with("<svg") && icon.ends_with("</svg>"));
            assert!(
                !icons[..i].contains(icon),
                "duplicate icon for {:?}",
                KINDS[i]
            );
        }
    }

    #[test]
    fn every_channel_has_a_distinct_svg() {
        let icons: Vec<_> = AlertChannel::ALL.iter().map(|c| channel_icon(*c)).collect();
        for (i, icon) in icons.iter().enumerate() {
            assert!(icon.starts_with("<svg") && icon.ends_with("</svg>"));
            assert!(
                !icons[..i].contains(icon),
                "duplicate icon for {:?}",
                AlertChannel::ALL[i]
            );
        }
    }

    #[test]
    fn device_class_overrides_and_unknown_falls_back() {
        let door = kind_icon(Kind::BinarySensor, Some("door"));
        assert_ne!(door, kind_icon(Kind::BinarySensor, None));
        assert_eq!(
            kind_icon(Kind::BinarySensor, Some("no-such-class")),
            kind_icon(Kind::BinarySensor, None)
        );
    }

    #[test]
    fn every_ambiance_icon_has_a_glyph() {
        let glyphs: Vec<&str> = nidoo_api::AMBIANCE_ICONS
            .iter()
            .map(|n| ambiance_icon(Some(n)))
            .collect();
        for (i, g) in glyphs.iter().enumerate() {
            assert!(
                !g.is_empty(),
                "{} has no glyph",
                nidoo_api::AMBIANCE_ICONS[i]
            );
            for (j, other) in glyphs.iter().enumerate().skip(i + 1) {
                assert_ne!(
                    g,
                    other,
                    "{} and {} share a glyph",
                    nidoo_api::AMBIANCE_ICONS[i],
                    nidoo_api::AMBIANCE_ICONS[j]
                );
            }
        }
        assert_eq!(ambiance_icon(None), SPARKLE);
        assert_eq!(ambiance_icon(Some("no-such-icon")), SPARKLE);
    }
}
