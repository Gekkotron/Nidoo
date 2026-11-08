//! Draws text as a QR code, as an SVG string.

/// The SVG of a QR code for `text`: black modules on white with a quiet zone. `None` when the
/// text does not fit in a code.
pub fn qr_svg(text: &str) -> Option<String> {
    use qrcode::render::svg;
    use qrcode::{EcLevel, QrCode};
    let code = QrCode::with_error_correction_level(text, EcLevel::M).ok()?;
    Some(
        code.render::<svg::Color>()
            .dark_color(svg::Color("#000000"))
            .light_color(svg::Color("#ffffff"))
            .quiet_zone(true)
            .min_dimensions(200, 200)
            .build(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qr_svg_draws_a_code() {
        let s = qr_svg(r#"{"v":1}"#).unwrap();
        assert!(s.starts_with("<svg") || s.starts_with("<?xml"));
        assert!(s.contains("#000000") || s.contains("black"));
    }
}
