//! Human-readable state values for cards.

use nidoo_api::{AreaView, EntityView, WEATHER_DEVICE_NAME, weather_entity_name};
use nidoo_core::model::{EntityState, GenericValue, Kind, StateValue, SwitchType, UNASSIGNED_AREA};
use nidoo_core::types::{GenericCaps, TypeDef};

use crate::i18n::Lang;
use crate::i18n::format::{number, with_unit};
use crate::i18n::states::{self, StateCtx, on_off, state_word};

pub(crate) const DASH: &str = "—";

/// The value line of `entity`: [`display_value`], except that a switch shown as a light reads
/// "On" / "Off" the way a light does.
pub fn entity_value(lang: Lang, entity: &EntityView) -> String {
    match &entity.state.value {
        StateValue::OnOff(on) if shown_as_light(entity) => on_off(lang, true, *on).into(),
        _ => display_value(lang, &entity.state, entity.unit.as_deref()),
    }
}

/// The value a card shows: "On", "21.5 °C", "—" while nothing usable is known.
pub fn display_value(lang: Lang, state: &EntityState, unit: Option<&str>) -> String {
    match &state.value {
        StateValue::OnOff(on) => on_off(lang, false, *on).into(),
        StateValue::Number(n) => with_unit(number(lang, *n), unit),
        StateValue::Text(s) if is_unusable(s) => DASH.into(),
        StateValue::Text(s) => with_unit(date_time(s).unwrap_or_else(|| s.clone()), unit),
        StateValue::Light(l) => match (l.on, l.brightness) {
            (true, Some(b)) => format!(
                "{} · {}",
                on_off(lang, true, true),
                with_unit(((u32::from(b) * 100 + 127) / 255).to_string(), Some("%"))
            ),
            (on, _) => on_off(lang, true, on).into(),
        },
        StateValue::Cover(c) => {
            let word = c
                .state
                .as_deref()
                .map(|s| state_word(lang, StateCtx::Cover, s));
            let percent = |p: u8| with_unit(p.to_string(), Some("%"));
            match (word, c.position) {
                (Some(w), Some(p)) => format!("{w} · {}", percent(p)),
                (Some(w), None) => w,
                (None, Some(p)) => percent(p),
                (None, None) => DASH.into(),
            }
        }
        StateValue::Climate(c) => {
            // The hub sends no unit for climates; Home Assistant's default is Celsius.
            let unit = unit.filter(|u| !u.is_empty()).unwrap_or("°C");
            match (c.current, c.target) {
                (Some(cur), Some(tgt)) => {
                    format!(
                        "{} → {}",
                        number(lang, cur),
                        with_unit(number(lang, tgt), Some(unit))
                    )
                }
                (Some(v), None) | (None, Some(v)) => with_unit(number(lang, v), Some(unit)),
                (None, None) => c
                    .mode
                    .as_deref()
                    .map_or_else(|| DASH.into(), |m| state_word(lang, StateCtx::Hvac, m)),
            }
        }
        StateValue::Alarm(s) => state_word(lang, StateCtx::Alarm, s),
        StateValue::Vacuum(v) => v
            .state
            .as_deref()
            .map_or_else(|| DASH.into(), |s| state_word(lang, StateCtx::Vacuum, s)),
        StateValue::WaterHeater(w) => match (w.current, w.target) {
            (Some(v), _) | (None, Some(v)) => with_unit(number(lang, v), unit),
            (None, None) => w
                .mode
                .as_deref()
                .map_or_else(|| DASH.into(), |m| state_word(lang, StateCtx::Hvac, m)),
        },
        StateValue::Generic(_) => generic_value(lang, None, None, state),
        StateValue::MediaPlayer(m) => m
            .state
            .as_deref()
            .map_or_else(|| DASH.into(), |s| state_word(lang, StateCtx::Media, s)),
        StateValue::Unknown => DASH.into(),
    }
}

/// True when the entity is in an "on" state, which cards highlight.
pub fn is_on(state: &EntityState) -> bool {
    match &state.value {
        StateValue::OnOff(on) => *on,
        StateValue::Light(l) => l.on,
        StateValue::Cover(c) => {
            c.state.as_deref() == Some("open") || c.position.is_some_and(|p| p > 0)
        }
        _ => false,
    }
}

fn is_unusable(s: &str) -> bool {
    s.eq_ignore_ascii_case("unknown") || s.eq_ignore_ascii_case("unavailable")
}

/// A temperature as "21 °C" (Celsius when `unit` is unknown), "—" when there is none.
pub fn degrees(lang: Lang, value: Option<f64>, unit: Option<&str>) -> String {
    let unit = unit.filter(|u| !u.is_empty()).unwrap_or("°C");
    value.map_or_else(|| DASH.into(), |v| with_unit(number(lang, v), Some(unit)))
}

/// The CSS classes of a card's value line: values over nine characters get the smaller,
/// wrapping `card__value--long` instead of being ellipsised.
pub fn value_class(text: &str) -> &'static str {
    if text.chars().count() > 9 {
        "card__value card__value--long"
    } else {
        "card__value"
    }
}

/// The card title: the entity name without its device name prefix ("powers/ac energy" on
/// device "powers/ac" becomes "Energy"); the full name when there is no such prefix.
pub fn short_name(name: &str, device_name: Option<&str>) -> String {
    const SEPARATORS: &[char] = &[' ', '_', '/', '-'];
    let Some(device) = device_name.filter(|d| !d.is_empty()) else {
        return name.to_owned();
    };
    let mut rest = name.chars();
    for expected in device.chars() {
        match rest.next() {
            Some(c) if c.to_lowercase().eq(expected.to_lowercase()) => {}
            _ => return name.to_owned(),
        }
    }
    let rest = rest.as_str();
    if !rest.starts_with(SEPARATORS) {
        return name.to_owned();
    }
    let rest = rest.trim_matches(SEPARATORS);
    let mut chars = rest.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => name.to_owned(),
    }
}

/// `id` with a zero-width space after every `_` and `/`, so a long raw id wraps at those
/// boundaries (with `overflow-wrap: break-word`) instead of in the middle of a word. For display
/// only: never put the result in an attribute, a URL or something the user copies.
pub fn wrap_id(id: &str) -> String {
    let mut out = String::with_capacity(id.len() + 8);
    for c in id.chars() {
        out.push(c);
        if matches!(c, '_' | '/') {
            out.push('\u{200B}');
        }
    }
    out
}

/// "binary_sensor" as "Binary sensor" / "Détecteur".
pub fn kind_word(lang: Lang, kind: Kind) -> String {
    states::kind_word(lang, kind).to_owned()
}

/// The word for what an entity is: the type a switch is shown as ("Door opener"), else its kind.
pub fn type_word(lang: Lang, entity: &EntityView) -> String {
    shown_type(entity).map_or_else(
        || kind_word(lang, entity.kind),
        |t| states::switch_type_word(lang, t).to_owned(),
    )
}

/// What an entity is in both languages, lower case ("cover", "volet"), for searches that must
/// find it whatever the UI language.
pub fn kind_search_words(entity: &EntityView) -> [String; 2] {
    states::search_words(entity.kind, shown_type(entity))
}

/// True for a switch the user shows as a light.
pub fn shown_as_light(entity: &EntityView) -> bool {
    shown_type(entity) == Some(SwitchType::Light)
}

/// The type a switch is shown as, unless it is a plain switch.
fn shown_type(entity: &EntityView) -> Option<SwitchType> {
    SwitchType::of(entity.protocol_kind, &entity.ui).filter(|t| *t != SwitchType::Switch)
}

/// What tells an entity apart from its same-named siblings: set when the entity is named exactly
/// like its device (a siren whose four entities are all "door_siren"), then its device class
/// ("Siren"), else its kind ("Binary sensor"). `None` for an entity with a name of its own, and
/// when the suffix would repeat the name (a "Water heater" device's water heater).
pub fn title_suffix(lang: Lang, entity: &EntityView) -> Option<String> {
    let device = entity.device_name.as_deref().filter(|d| !d.is_empty())?;
    if !entity.name.eq_ignore_ascii_case(device) {
        return None;
    }
    let suffix = match shown_type(entity) {
        Some(shown) => states::switch_type_word(lang, shown).to_owned(),
        None => entity
            .device_class
            .as_deref()
            .filter(|c| !c.is_empty())
            .map_or_else(
                || kind_word(lang, entity.kind),
                |c| states::device_class_word(lang, c).map_or_else(|| word(c), str::to_owned),
            ),
    };
    (!suffix.eq_ignore_ascii_case(&entity.name)).then_some(suffix)
}

/// The entity's name as shown: a hub weather entity still under the hub's English name reads in
/// `lang`; every other name, a renamed weather entity included, is the one the house gives.
pub fn shown_name(lang: Lang, entity: &EntityView) -> &str {
    entity
        .id
        .strip_prefix("weather_")
        .and_then(weather_entity_name)
        .filter(|[en, _]| *en == entity.name)
        .map_or(&entity.name, |names| names[lang_index(lang)])
}

/// The device name as shown: the hub's weather device reads in `lang`.
pub fn shown_device_name(lang: Lang, entity: &EntityView) -> Option<&str> {
    let device = entity.device_name.as_deref()?;
    if entity.id.starts_with("weather_") && device == WEATHER_DEVICE_NAME[0] {
        return Some(WEATHER_DEVICE_NAME[lang_index(lang)]);
    }
    Some(device)
}

/// The index of `lang` in an `[en, fr]` pair.
fn lang_index(lang: Lang) -> usize {
    match lang {
        Lang::En => 0,
        Lang::Fr => 1,
    }
}

/// The card title: [`short_name`] plus the [`title_suffix`] when there is one.
pub fn entity_title(lang: Lang, entity: &EntityView) -> String {
    let short = short_name(shown_name(lang, entity), shown_device_name(lang, entity));
    match title_suffix(lang, entity) {
        Some(suffix) => format!("{short} · {suffix}"),
        None => short,
    }
}

/// The label of an entity in a list of links: its title and its kind, where the suffix of a
/// same-named entity stands in for the kind.
pub fn list_label(lang: Lang, entity: &EntityView) -> String {
    let short = short_name(shown_name(lang, entity), shown_device_name(lang, entity));
    let tail = title_suffix(lang, entity).unwrap_or_else(|| type_word(lang, entity));
    if tail.eq_ignore_ascii_case(&short) {
        return short;
    }
    format!("{short} · {tail}")
}

/// The name of room `area`, or `None` outside any room. An id the house no longer lists stands
/// for its own name.
pub fn room_name(areas: &[AreaView], area: &str) -> Option<String> {
    (area != UNASSIGNED_AREA).then(|| {
        areas
            .iter()
            .find(|a| a.id == area)
            .map_or_else(|| area.to_owned(), |a| a.name.clone())
    })
}

/// The name of a room as shown: the hub's catch-all area under its default name ("Unassigned")
/// reads in the language on screen; every other name, a renamed catch-all included, is the one
/// the user gave.
pub fn area_name(lang: Lang, id: &str, name: &str) -> String {
    if id == UNASSIGNED_AREA && name == UNASSIGNED_AREA {
        crate::i18n::text(lang, crate::i18n::Key::Unassigned).to_owned()
    } else {
        name.to_owned()
    }
}

/// An RFC 3339 date-time as "2026-10-19 09:41" (the time as sent, no zone conversion).
fn date_time(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let digits = [0, 1, 2, 3, 5, 6, 8, 9, 11, 12, 14, 15];
    let shaped = b.len() >= 19
        && digits.iter().all(|&i| b[i].is_ascii_digit())
        && b[4] == b'-'
        && b[7] == b'-'
        && matches!(b[10], b'T' | b't')
        && b[13] == b':'
        && b[16] == b':'
        && b[17].is_ascii_digit()
        && b[18].is_ascii_digit()
        && s.is_char_boundary(16);
    shaped.then(|| format!("{} {}", &s[..10], &s[11..16]))
}

/// The date part of an RFC 3339 date-time as "2026-10-19", as sent (no zone conversion).
pub fn date_only(s: &str) -> Option<String> {
    let t = chrono::DateTime::parse_from_rfc3339(s).ok()?;
    Some(t.format("%Y-%m-%d").to_string())
}

/// `heat_cool` becomes "Heat cool".
pub fn word(s: &str) -> String {
    let spaced = s.replace('_', " ");
    let mut chars = spaced.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => DASH.into(),
    }
}

/// A canonical choice value as shown on a card: `not_home` is "Away" / "Absent", lock and cover
/// words are translated, the rest sentence case.
pub fn choice_word(lang: Lang, s: &str) -> String {
    state_word(lang, StateCtx::Choice, s)
}

/// The value a generic card shows: the type's primary field (a lone or `state` field without a
/// type), "••••" for a secret, "" for a picture, "—" while nothing is known.
pub fn generic_value(
    lang: Lang,
    def: Option<&TypeDef>,
    caps: Option<&GenericCaps>,
    state: &EntityState,
) -> String {
    if caps.is_some_and(|c| c.secret) {
        return "••••".into();
    }
    let StateValue::Generic(g) = &state.value else {
        return DASH.into();
    };
    let value = match def {
        Some(d) => {
            if let Some(field) = d.primary.as_deref().and_then(|p| d.fields.get(p))
                && field.shape == nidoo_core::types::Shape::Picture
            {
                return String::new();
            }
            nidoo_core::types::state::primary(d, g)
        }
        None => g.fields.get("state").or_else(|| {
            (g.fields.len() == 1)
                .then(|| g.fields.values().next())
                .flatten()
        }),
    };
    match value {
        Some(GenericValue::Bool(on)) => on_off(lang, false, *on).into(),
        Some(GenericValue::Number(n)) => number(lang, *n),
        Some(GenericValue::Text(s)) if is_unusable(s) || s.is_empty() => DASH.into(),
        Some(GenericValue::Text(s)) => choice_word(lang, s),
        None => DASH.into(),
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod ts_tests {
    use super::display_value;
    use crate::i18n::Lang;
    use nidoo_core::model::LightState;
    use nidoo_core::model::{EntityState, GenericValue, StateValue};

    fn generic_state(field: &str, v: GenericValue) -> EntityState {
        let mut g = nidoo_core::model::GenericState::default();
        g.fields.insert(field.into(), v);
        EntityState {
            value: StateValue::Generic(g),
            ..Default::default()
        }
    }

    #[test]
    fn generic_value_humanises_choices() {
        use super::generic_value;
        use nidoo_core::model::GenericValue::Text;
        let lock = nidoo_core::types::type_def("lock");
        let tracker = nidoo_core::types::type_def("device_tracker");
        assert_eq!(
            generic_value(
                Lang::En,
                lock,
                None,
                &generic_state("state", Text("locked".into()))
            ),
            "Locked"
        );
        assert_eq!(
            generic_value(
                Lang::En,
                tracker,
                None,
                &generic_state("state", Text("not_home".into()))
            ),
            "Away"
        );
        assert_eq!(
            generic_value(
                Lang::En,
                tracker,
                None,
                &generic_state("state", Text("home".into()))
            ),
            "Home"
        );
    }

    #[test]
    fn generic_value_hides_secrets() {
        use super::generic_value;
        let caps = nidoo_core::types::GenericCaps {
            secret: true,
            ..Default::default()
        };
        let s = generic_state("state", GenericValue::Text("hunter2".into()));
        assert_eq!(
            generic_value(
                Lang::En,
                nidoo_core::types::type_def("text"),
                Some(&caps),
                &s
            ),
            "••••"
        );
    }

    #[test]
    fn the_catch_all_area_reads_in_the_language_on_screen() {
        use super::area_name;
        assert_eq!(
            area_name(Lang::Fr, "Unassigned", "Unassigned"),
            "Sans pièce"
        );
        assert_eq!(
            area_name(Lang::En, "Unassigned", "Unassigned"),
            "Unassigned"
        );
        assert_eq!(area_name(Lang::Fr, "salon", "Living room"), "Living room");
        // A catch-all the user named keeps the name they typed.
        assert_eq!(area_name(Lang::Fr, "Unassigned", "Garage"), "Garage");
    }

    #[test]
    fn room_name_resolves_ids_and_skips_unassigned() {
        use super::room_name;
        use nidoo_api::AreaView;
        let areas = vec![AreaView {
            id: "salon".into(),
            name: "Living room".into(),
            icon: None,
            order: None,
            entity_count: 0,
            primary_count: 0,
            group_names: Default::default(),
            group_favorites: Default::default(),
        }];
        assert_eq!(room_name(&areas, "salon").as_deref(), Some("Living room"));
        assert_eq!(room_name(&areas, "gone").as_deref(), Some("gone"));
        assert_eq!(room_name(&areas, nidoo_core::model::UNASSIGNED_AREA), None);
    }
    #[test]
    fn french_card_values() {
        let state = |value| EntityState {
            value,
            ..Default::default()
        };
        let light = LightState {
            on: true,
            brightness: Some(204),
            ..Default::default()
        };
        let cover = nidoo_core::model::CoverState {
            state: Some("open".into()),
            position: Some(40),
        };
        assert_eq!(
            display_value(Lang::Fr, &state(StateValue::OnOff(true)), None),
            "Activé"
        );
        assert_eq!(
            display_value(Lang::Fr, &state(StateValue::Light(light)), None),
            "Allumé · 80\u{a0}%"
        );
        assert_eq!(
            display_value(Lang::Fr, &state(StateValue::Number(21.46)), Some("°C")),
            "21,5\u{a0}°C"
        );
        assert_eq!(
            display_value(Lang::Fr, &state(StateValue::Cover(cover)), None),
            "Ouvert · 40\u{a0}%"
        );
    }
}
