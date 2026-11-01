//! Theme switch: Night Glass, Ivoire, or follow the system.

use leptos::prelude::*;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsCast;

use crate::i18n::Lang;
#[cfg(target_arch = "wasm32")]
use crate::i18n::settings::LIGHT_SCHEME_QUERY;
use crate::i18n::settings::{SettingsText, settings_text};

/// Storage key for the persisted choice.
#[cfg(target_arch = "wasm32")]
const STORAGE_KEY: &str = "oikos.theme";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Theme {
    /// Night Glass (dark).
    Night,
    /// Ivoire (light).
    Ivoire,
    /// Follow `prefers-color-scheme`.
    Auto,
}

impl Theme {
    pub fn as_str(self) -> &'static str {
        match self {
            Theme::Night => "night",
            Theme::Ivoire => "ivoire",
            Theme::Auto => "auto",
        }
    }

    /// The next choice for the header button: Auto, Night, Ivoire, then Auto again.
    pub fn next(self) -> Theme {
        match self {
            Theme::Auto => Theme::Night,
            Theme::Night => Theme::Ivoire,
            Theme::Ivoire => Theme::Auto,
        }
    }

    /// The theme's name; names are not translated.
    pub fn label(self) -> &'static str {
        settings_text(
            Lang::En,
            match self {
                Theme::Auto => SettingsText::ThemeAuto,
                Theme::Night => SettingsText::ThemeNight,
                Theme::Ivoire => SettingsText::ThemeIvoire,
            },
        )
    }
}

/// Reads a stored value; anything missing or unknown is `Auto`.
pub fn parse(value: Option<&str>) -> Theme {
    match value {
        Some("night") => Theme::Night,
        // "warm" was saved before Ivoire replaced Warm Minimal.
        Some("ivoire") | Some("warm") => Theme::Ivoire,
        _ => Theme::Auto,
    }
}

/// The `data-theme` attribute for a theme, `None` to follow the system.
#[cfg(any(target_arch = "wasm32", test))]
fn attribute(theme: Theme) -> Option<&'static str> {
    match theme {
        Theme::Night => Some("dark"),
        Theme::Ivoire => Some("light"),
        Theme::Auto => None,
    }
}

/// The `<meta name="theme-color">` of an applied theme: its page colour (`--bg` in tokens.css).
#[cfg(any(target_arch = "wasm32", test))]
fn theme_color(light: bool) -> &'static str {
    if light { "#f3f0e8" } else { "#0b1020" }
}

/// `localStorage`, or `None` when it is missing, blocked or throws.
#[cfg(target_arch = "wasm32")]
fn storage() -> Option<web_sys::Storage> {
    web_sys::window().and_then(|w| w.local_storage().ok().flatten())
}

/// `matchMedia("(prefers-color-scheme: light)")`.
#[cfg(target_arch = "wasm32")]
fn system_light_query() -> Option<web_sys::MediaQueryList> {
    web_sys::window()?
        .match_media(LIGHT_SCHEME_QUERY)
        .ok()
        .flatten()
}

/// Sets `data-theme` on `<html>` and the browser's theme colour; Auto resolves to the system's
/// `light` or `dark`.
#[cfg(target_arch = "wasm32")]
fn apply(theme: Theme) {
    let value = attribute(theme).unwrap_or_else(|| {
        if system_light_query().is_some_and(|q| q.matches()) {
            "light"
        } else {
            "dark"
        }
    });
    let Some(document) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    if let Some(root) = document.document_element() {
        let _ = root.set_attribute("data-theme", value);
    }
    if let Some(meta) = document
        .query_selector("meta[name=\"theme-color\"]")
        .ok()
        .flatten()
    {
        let _ = meta.set_attribute("content", theme_color(value == "light"));
    }
}

/// Loads the saved theme, applies it to `<html>` and keeps both in sync with the returned signal.
/// Every storage access is fallible and silent: a private window must never panic.
pub fn install_theme() -> RwSignal<Theme> {
    #[cfg(target_arch = "wasm32")]
    let initial = parse(
        storage()
            .and_then(|s| s.get_item(STORAGE_KEY).ok().flatten())
            .as_deref(),
    );
    #[cfg(not(target_arch = "wasm32"))]
    let initial = Theme::Auto;

    let theme = RwSignal::new(initial);

    #[cfg(target_arch = "wasm32")]
    {
        Effect::new(move |_| {
            let current = theme.get();
            apply(current);
            if let Some(storage) = storage() {
                let _ = storage.set_item(STORAGE_KEY, current.as_str());
            }
        });
        // Auto follows the system live. One listener for the page's lifetime; it does nothing
        // while the choice is not Auto.
        if let Some(query) = system_light_query() {
            let on_change = wasm_bindgen::closure::Closure::<dyn FnMut()>::new(move || {
                if theme.get_untracked() == Theme::Auto {
                    apply(Theme::Auto);
                }
            });
            let _ = query
                .add_event_listener_with_callback("change", on_change.as_ref().unchecked_ref());
            on_change.forget();
        }
    }

    theme
}

#[cfg(test)]
mod tests {
    use super::Theme;
    use super::parse;
    use std::collections::{BTreeSet, HashMap};

    #[test]
    fn parses_known_values() {
        assert_eq!(parse(Some("night")), Theme::Night);
        assert_eq!(parse(Some("ivoire")), Theme::Ivoire);
        assert_eq!(parse(Some("warm")), Theme::Ivoire); // saved before Ivoire replaced Warm Minimal
        assert_eq!(parse(Some("auto")), Theme::Auto);
        assert_eq!(parse(None), Theme::Auto);
        assert_eq!(parse(Some("gibberish")), Theme::Auto);
    }
    #[test]
    fn maps_to_the_data_theme_attribute() {
        assert_eq!(super::attribute(Theme::Night), Some("dark"));
        assert_eq!(super::attribute(Theme::Ivoire), Some("light"));
        assert_eq!(super::attribute(Theme::Auto), None);
    }
    #[test]
    fn serialises_known_values() {
        assert_eq!(Theme::Night.as_str(), "night");
        assert_eq!(Theme::Ivoire.as_str(), "ivoire");
        assert_eq!(Theme::Auto.as_str(), "auto");
    }
    #[test]
    fn the_header_button_cycles() {
        assert_eq!(Theme::Auto.next(), Theme::Night);
        assert_eq!(Theme::Night.next(), Theme::Ivoire);
        assert_eq!(Theme::Ivoire.next(), Theme::Auto);
    }
    #[test]
    fn labels_are_names() {
        assert_eq!(Theme::Auto.label(), "Auto");
        assert_eq!(Theme::Night.label(), "Night Glass");
        assert_eq!(Theme::Ivoire.label(), "Ivoire");
    }

    /// The body of the first block whose selector line contains `needle`.
    fn block<'a>(css: &'a str, needle: &str) -> &'a str {
        let at = css
            .match_indices('{')
            .map(|(i, _)| i)
            .find(|&i| {
                let line_start = css[..i].rfind('\n').map_or(0, |n| n + 1);
                css[line_start..i].contains(needle)
            })
            .unwrap_or_else(|| panic!("no block for {needle}"));
        let end = css[at..].find('}').expect("closing brace") + at;
        &css[at + 1..end]
    }

    /// `(name, value)` for every `--name: value;` declaration in a block.
    fn declarations(body: &str) -> Vec<(String, String)> {
        let mut text = body.to_owned();
        while let Some(start) = text.find("/*") {
            let end = text[start..]
                .find("*/")
                .map_or(text.len(), |e| start + e + 2);
            text.replace_range(start..end, "");
        }
        text.split(';')
            .filter_map(|d| {
                let (name, value) = d.split_once(':')?;
                name.trim()
                    .starts_with("--")
                    .then(|| (name.trim().to_owned(), value.trim().to_owned()))
            })
            .collect()
    }

    fn token_sets(css: &str) -> (BTreeSet<String>, BTreeSet<String>) {
        let names = |body| declarations(body).into_iter().map(|d| d.0).collect();
        (
            names(block(css, ":root")),
            names(block(css, "[data-theme=\"light\"]")),
        )
    }

    fn ivoire_values(css: &str) -> HashMap<String, String> {
        declarations(block(css, "[data-theme=\"light\"]"))
            .into_iter()
            .collect()
    }

    fn channel(c: u8) -> f64 {
        let c = f64::from(c) / 255.0;
        if c <= 0.03928 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    }

    fn luminance(hex: &str) -> f64 {
        let h = hex
            .strip_prefix('#')
            .unwrap_or_else(|| panic!("not #rrggbb: {hex}"));
        assert_eq!(h.len(), 6, "not #rrggbb: {hex}");
        let byte = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).expect("hex");
        0.2126 * channel(byte(0)) + 0.7152 * channel(byte(2)) + 0.0722 * channel(byte(4))
    }

    /// WCAG 2.1 contrast ratio between two `#rrggbb` colours.
    fn contrast(a: &str, b: &str) -> f64 {
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn both_themes_define_the_same_tokens() {
        let css = include_str!("../styles/tokens.css");
        let (night, ivoire) = token_sets(css);
        // No orphan: everything Ivoire sets exists in :root.
        for t in &ivoire {
            assert!(night.contains(t), "{t} is set by Ivoire but not in :root");
        }
        // Every colour, gradient and shadow of :root is overridden by Ivoire.
        for (name, value) in declarations(block(css, ":root")) {
            if value.contains('#') || value.contains("rgb") {
                assert!(ivoire.contains(&name), "{name} has no Ivoire value");
            }
        }
        for t in [
            "--muted",
            "--sidebar-bg",
            "--sidebar-w",
            "--nav-active-bg",
            "--cool-bg",
        ] {
            assert!(night.contains(t), "{t}");
        }
        assert!(!css.contains("Fraunces"));
    }

    /// The browser chrome takes each theme's page colour, in Rust and in the pre-boot script.
    #[test]
    fn theme_color_is_the_page_background() {
        let css = include_str!("../styles/tokens.css");
        let bg = |body| {
            declarations(body)
                .into_iter()
                .find(|(name, _)| name == "--bg")
                .map(|(_, value)| value)
                .expect("--bg")
        };
        assert_eq!(super::theme_color(false), bg(block(css, ":root")));
        assert_eq!(
            super::theme_color(true),
            bg(block(css, "[data-theme=\"light\"]"))
        );
        let html = include_str!("../index.html");
        assert!(html.contains(&format!(
            "light ? \"{}\" : \"{}\"",
            super::theme_color(true),
            super::theme_color(false)
        )));
    }

    #[test]
    fn ivoire_text_passes_aa() {
        let v = ivoire_values(include_str!("../styles/tokens.css"));
        for text in [
            "--ink",
            "--muted",
            "--accent-mint",
            "--accent-amber",
            "--accent-cyan",
            "--accent-rose",
            "--accent-violet",
            "--danger",
            "--offline",
            "--connected",
            "--select",
            "--warning",
        ] {
            for surface in ["--bg", "--card-bg"] {
                let ratio = contrast(&v[text], &v[surface]);
                assert!(ratio >= 4.5, "{text} on {surface}: {ratio:.2}");
            }
        }
        let ratio = contrast(&v["--warning"], &v["--warning-bg"]);
        assert!(ratio >= 4.5, "--warning on --warning-bg: {ratio:.2}");
        assert!(contrast(&v["--nav-active-ink"], &v["--nav-active-bg"]) >= 4.5);
        assert!(contrast(&v["--card-on-ink"], &v["--card-on-bg"]) >= 4.5);
    }
}
