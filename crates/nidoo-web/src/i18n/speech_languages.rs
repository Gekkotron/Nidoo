//! Speech languages: a language code's name in the UI language, and the sentence each language
//! is tried with on the speech settings screen.

use super::Lang;

/// The name of the language `code` (`fr`, `en`, `pt-br`) in the UI language `ui`, first letter
/// upper-cased; the code in capitals when the browser cannot name it (and always off wasm).
pub fn language_name(code: &str, ui: Lang) -> String {
    match browser_name(code, ui) {
        Some(name) if !name.is_empty() => capitalise(&name),
        _ => code.to_uppercase(),
    }
}

/// `Intl.DisplayNames([ui], {type: "language"}).of(code)`; `None` when the browser has no
/// `Intl.DisplayNames`, or throws on a malformed tag.
#[cfg(target_arch = "wasm32")]
fn browser_name(code: &str, ui: Lang) -> Option<String> {
    use js_sys::{Array, Function, Object, Reflect};
    use wasm_bindgen::{JsCast, JsValue};

    let intl = Reflect::get(&js_sys::global(), &JsValue::from_str("Intl")).ok()?;
    let ctor: Function = Reflect::get(&intl, &JsValue::from_str("DisplayNames"))
        .ok()?
        .dyn_into()
        .ok()?;
    let options = Object::new();
    Reflect::set(&options, &"type".into(), &"language".into()).ok()?;
    let args = Array::of2(&Array::of1(&ui.as_str().into()), &options);
    let names = Reflect::construct(&ctor, &args).ok()?;
    let of: Function = Reflect::get(&names, &"of".into()).ok()?.dyn_into().ok()?;
    of.call1(&names, &JsValue::from_str(code)).ok()?.as_string()
}

#[cfg(not(target_arch = "wasm32"))]
fn browser_name(_code: &str, _ui: Lang) -> Option<String> {
    None
}

/// `name` with its first letter upper-cased (`français` → `Français`).
fn capitalise(name: &str) -> String {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// The sentence a language is tried with, matched on the part before `-`; `None` for a language
/// with no sentence (the field then starts empty).
pub fn test_sentence(code: &str) -> Option<&'static str> {
    let base = code.split('-').next().unwrap_or(code).to_ascii_lowercase();
    Some(match base.as_str() {
        "fr" => "Bonjour, je suis l'assistant de la maison.",
        "en" => "Hello, I am the house assistant.",
        "de" => "Hallo, ich bin der Assistent des Hauses.",
        "es" => "Hola, soy el asistente de la casa.",
        "it" => "Ciao, sono l'assistente di casa.",
        "nl" => "Hallo, ik ben de assistent van het huis.",
        "pt" => "Olá, sou o assistente da casa.",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_matches_regions() {
        assert_eq!(test_sentence("pt-br"), test_sentence("pt"));
        assert!(test_sentence("pt").is_some());
        assert_eq!(test_sentence("EN"), test_sentence("en"));
        assert_eq!(test_sentence("sv"), None);
    }

    #[test]
    fn the_french_sentence_is_pinned() {
        assert_eq!(
            test_sentence("fr"),
            Some("Bonjour, je suis l'assistant de la maison.")
        );
    }

    #[test]
    fn native_name_falls_back_to_the_code() {
        assert_eq!(language_name("de", Lang::En), "DE");
        assert_eq!(language_name("pt-br", Lang::Fr), "PT-BR");
    }
}
