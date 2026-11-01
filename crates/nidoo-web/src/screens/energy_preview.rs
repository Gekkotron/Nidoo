//! The small picture under the energy-view chips in Settings › Appearance: a fixed sketch of the
//! chosen view with sample shapes, so the choice can be seen before opening Energy.

use leptos::prelude::*;

use crate::energy_view::EnergyView;
use crate::i18n::settings::{SettingsText as T, settings_text};
use crate::i18n::use_lang;

fn view_name(view: EnergyView) -> &'static str {
    match view {
        EnergyView::Classic => "classic",
        EnergyView::Flux => "flux",
        EnergyView::Orbite => "orbite",
    }
}

/// A source or consumer circle with a faint halo.
fn node(x: u32, y: u32, r: u32, colour: &str) -> String {
    format!(
        r#"<circle cx="{x}" cy="{y}" r="{halo}" fill="var({colour})" opacity=".12"/><circle cx="{x}" cy="{y}" r="{r}" fill="var({colour})" fill-opacity=".22" stroke="var({colour})" stroke-width="1.6"/>"#,
        halo = r + 5
    )
}

/// A wire with a dashed overlay that the CSS runs along it.
fn wire(d: &str, colour: &str) -> String {
    format!(
        r#"<path d="{d}" fill="none" stroke="var({colour})" stroke-width="1.6" opacity=".55"/><path class="energy-preview__flow" d="{d}" fill="none" stroke-width="2.2" stroke-linecap="round"/>"#
    )
}

fn classic() -> String {
    [
        wire("M38 28 Q60 30 64 42", "--energy-solar"),
        wire("M38 70 Q60 68 64 56", "--energy-grid"),
        wire("M90 44 Q104 28 118 24", "--energy-c0"),
        wire("M92 48 L118 48", "--energy-c1"),
        wire("M90 54 Q104 68 118 72", "--energy-c2"),
        node(28, 26, 10, "--energy-solar"),
        node(28, 72, 10, "--energy-grid"),
        node(77, 48, 16, "--ink"),
        node(126, 24, 7, "--energy-c0"),
        node(126, 48, 7, "--energy-c1"),
        node(126, 72, 7, "--energy-c2"),
    ]
    .concat()
}

/// A fill: a theme variable's name, or a paint server such as `url(#preview-hatch)`.
fn paint(colour: &str) -> String {
    if colour.starts_with("--") {
        format!("var({colour})")
    } else {
        colour.to_string()
    }
}

/// A band from the bar at `x1` (spanning `a0..a1`) to the bar at `x2` (spanning `b0..b1`).
fn band(x1: u32, a: (u32, u32), x2: u32, b: (u32, u32), colour: &str) -> String {
    let m = (x1 + x2) / 2;
    let ((a0, a1), (b0, b1)) = (a, b);
    format!(
        r#"<path d="M{x1} {a0}C{m} {a0} {m} {b0} {x2} {b0}L{x2} {b1}C{m} {b1} {m} {a1} {x1} {a1}Z" fill="{fill}" opacity=".38"/>"#,
        fill = paint(colour)
    )
}

fn bar(x: u32, y: u32, h: u32, colour: &str) -> String {
    format!(
        r#"<rect x="{x}" y="{y}" width="6" height="{h}" rx="2" fill="{fill}"/>"#,
        fill = paint(colour)
    )
}

fn flux() -> String {
    [
        band(16, (12, 58), 70, (12, 58), "--energy-solar"),
        band(16, (64, 82), 70, (60, 78), "--energy-grid"),
        band(76, (12, 40), 132, (8, 36), "--energy-c0"),
        band(76, (40, 56), 132, (42, 58), "--energy-c1"),
        band(76, (56, 66), 132, (64, 74), "--energy-c2"),
        // What no consumer accounts for is hatched, as on the Energy screen.
        r#"<pattern id="preview-hatch" patternUnits="userSpaceOnUse" width="4" height="4" patternTransform="rotate(45)"><rect width="4" height="4" fill="var(--energy-hatch-ground)"/><rect width="1.6" height="4" fill="var(--energy-hatch)"/></pattern>"#.to_string(),
        band(76, (66, 78), 132, (80, 92), "url(#preview-hatch)"),
        bar(10, 12, 46, "--energy-solar"),
        bar(10, 64, 18, "--energy-grid"),
        // The house bar in what flows into it.
        bar(70, 12, 46, "--energy-solar"),
        bar(70, 58, 20, "--energy-grid"),
        bar(132, 8, 28, "--energy-c0"),
        bar(132, 42, 16, "--energy-c1"),
        bar(132, 64, 10, "--energy-c2"),
        bar(132, 80, 12, "url(#preview-hatch)"),
    ]
    .concat()
}

/// One ring cut into arcs by `stroke-dasharray`: `(share, colour)` pairs, clockwise from the top.
fn ring(r: f64, width: f64, parts: &[(f64, &str)]) -> String {
    let circumference = 2.0 * std::f64::consts::PI * r;
    let mut start = 0.0;
    let mut out = String::new();
    for (share, colour) in parts {
        let len = share * circumference - 1.4;
        out.push_str(&format!(
            r#"<circle cx="75" cy="48" r="{r}" fill="none" stroke="var({colour})" stroke-width="{width}" stroke-dasharray="{len:.2} {circumference:.2}" stroke-dashoffset="{offset:.2}" transform="rotate(-90 75 48)"/>"#,
            offset = -start
        ));
        start += share * circumference;
    }
    out
}

fn orbite() -> String {
    format!(
        r#"<circle class="energy-preview__track" cx="75" cy="48" r="38" stroke-width="9"/><circle class="energy-preview__track" cx="75" cy="48" r="27" stroke-width="6"/><g class="energy-preview__spin">{outer}{inner}</g><text class="energy-preview__centre" x="75" y="51" text-anchor="middle">2 260</text>"#,
        outer = ring(
            38.0,
            9.0,
            &[
                (0.10, "--energy-battery"),
                (0.48, "--energy-c0"),
                (0.21, "--energy-c1"),
                (0.08, "--energy-c2"),
                (0.13, "--energy-muted"),
            ]
        ),
        inner = ring(
            27.0,
            6.0,
            &[(0.85, "--energy-solar"), (0.15, "--energy-grid")]
        ),
    )
}

/// A sketch of `view` with sample values. Decorative: the chips carry the meaning.
#[component]
pub fn EnergyPreview(view: Signal<EnergyView>) -> impl IntoView {
    let lang = use_lang();
    let drawing = move || match view.get() {
        EnergyView::Classic => classic(),
        EnergyView::Flux => flux(),
        EnergyView::Orbite => orbite(),
    };
    view! {
        <div class="energy-preview" data-view=move || view_name(view.get())>
            <svg viewBox="0 -8 150 112" aria-hidden="true" inner_html=drawing></svg>
            <p class="energy-preview__caption">
                {move || settings_text(lang(), T::EnergyPreview)}
            </p>
        </div>
    }
}
