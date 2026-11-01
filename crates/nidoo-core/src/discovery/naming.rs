//! Display names from the device name and the entity label, with the room read from a
//! hierarchical Zigbee2MQTT device name (`rooms/bedroom1/thermometer`).

/// Splits a device name into its short name and path area: with three or more non-empty `/`
/// segments the last is the device and the second-to-last its room; shorter names are unchanged.
fn device_path(name: &str) -> (String, Option<String>) {
    let segments: Vec<&str> = name.split('/').map(str::trim).collect();
    if segments.len() < 3 || segments.iter().any(|s| s.is_empty()) {
        return (name.to_owned(), None);
    }
    (
        segments[segments.len() - 1].to_owned(),
        Some(segments[segments.len() - 2].to_owned()),
    )
}

/// Returns `(display_name, device_name, label, path_area)`.
///
/// The display name is `"<short device name> <label>"` when both exist, the label alone when
/// there is no device, and the short device name when the entity has no name. `device_name` and
/// `label` are set only when the display name is device + label.
pub fn split(
    device_name: Option<&str>,
    entity_name: Option<&str>,
) -> (String, Option<String>, Option<String>, Option<String>) {
    let device = device_name.map(|raw| {
        let (short, path_area) = device_path(raw);
        (raw, short, path_area)
    });
    let path_area = device.as_ref().and_then(|(_, _, area)| area.clone());
    let short = device.as_ref().map(|(_, short, _)| short.clone());

    // An entity name that repeats the full device path gets the short device name instead.
    let entity = entity_name
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(|name| match &device {
            Some((raw, short, _)) if raw != short => match name.strip_prefix(raw) {
                Some(rest) => format!("{short}{rest}"),
                None => name.to_owned(),
            },
            _ => name.to_owned(),
        });

    let (display, device_name, label) = match (short, entity) {
        (short, None) => (short.clone().unwrap_or_default(), short, None),
        (None, Some(name)) => (name, None, None),
        (Some(short), Some(name)) if name == short => (name, Some(short), None),
        (Some(short), Some(name)) => match name.strip_prefix(short.as_str()) {
            Some(rest) if rest.starts_with(' ') => {
                let label = rest.trim();
                let label = (!label.is_empty()).then(|| label.to_owned());
                (name.clone(), Some(short), label)
            }
            Some(_) => (name, None, None),
            None => (format!("{short} {name}"), Some(short), Some(name)),
        },
    };
    (display, device_name, label, path_area)
}
