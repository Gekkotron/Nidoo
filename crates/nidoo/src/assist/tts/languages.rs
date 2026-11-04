//! The languages the hub speaks: an ordered list of (code, model, voice), the first being the
//! house language. Pure rules; the facade applies them and saves the result.

use std::collections::BTreeSet;

use nidoo_api::DEFAULT_VOICE;
use serde::{Deserialize, Serialize};

use super::TtsError;
use super::store::TtsState;

/// One language the hub speaks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LanguageEntry {
    /// The language code, normalised (`fr`, `en`, `pt-br`).
    pub code: String,
    /// The installed model that speaks it, when one is set.
    pub model: Option<String>,
    /// The voice it speaks with.
    pub voice: String,
}

/// A language code as the hub keeps it: trimmed and lower-cased, two or three letters with
/// optional `-` parts (`fr`, `pt-br`).
pub fn normalise_code(raw: &str) -> Result<String, TtsError> {
    let code = raw.trim().to_ascii_lowercase();
    let mut parts = code.split('-');
    let base_ok = parts
        .next()
        .is_some_and(|p| (2..=3).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_lowercase()));
    let rest_ok = parts.all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_alphanumeric()));
    if base_ok && rest_ok {
        Ok(code)
    } else {
        Err(TtsError::Invalid(format!("not a language code: {raw:?}")))
    }
}

/// Whether a model with these languages can speak `code`; a model that lists none speaks any.
pub fn speaks(model_languages: &[String], code: &str) -> bool {
    model_languages.is_empty() || model_languages.iter().any(|l| l == code)
}

/// Adds `code` at the end, or sets its model. A new model resets the voice to the default
/// unless one is given.
pub fn upsert(list: &mut Vec<LanguageEntry>, code: &str, model: &str, voice: Option<&str>) {
    match list.iter_mut().find(|l| l.code == code) {
        Some(entry) => {
            let same_model = entry.model.as_deref() == Some(model);
            entry.model = Some(model.to_owned());
            entry.voice = match (voice, same_model) {
                (Some(v), _) => v.to_owned(),
                (None, true) => std::mem::take(&mut entry.voice),
                (None, false) => DEFAULT_VOICE.to_owned(),
            };
        }
        None => list.push(LanguageEntry {
            code: code.to_owned(),
            model: Some(model.to_owned()),
            voice: voice.unwrap_or(DEFAULT_VOICE).to_owned(),
        }),
    }
}

/// Removes `code`; the next one becomes the house language when it was first.
pub fn remove(list: &mut Vec<LanguageEntry>, code: &str) -> bool {
    let before = list.len();
    list.retain(|l| l.code != code);
    list.len() != before
}

/// Moves `code` to the front, making it the house language.
pub fn make_house(list: &mut Vec<LanguageEntry>, code: &str) -> bool {
    let Some(i) = list.iter().position(|l| l.code == code) else {
        return false;
    };
    let entry = list.remove(i);
    list.insert(0, entry);
    true
}

/// The models some language uses, each once.
pub fn models_in_use(list: &[LanguageEntry]) -> BTreeSet<String> {
    list.iter().filter_map(|l| l.model.clone()).collect()
}

/// The codes using `model`, in list order.
pub fn used_by(list: &[LanguageEntry], model: &str) -> Vec<String> {
    list.iter()
        .filter(|l| l.model.as_deref() == Some(model))
        .map(|l| l.code.clone())
        .collect()
}

/// Leaves every language using `model` without a model, on the default voice.
pub fn detach_model(list: &mut [LanguageEntry], model: &str) {
    for entry in list
        .iter_mut()
        .filter(|l| l.model.as_deref() == Some(model))
    {
        entry.model = None;
        entry.voice = DEFAULT_VOICE.to_owned();
    }
}

/// Turns the single-model state of earlier hubs into a one-language list: the active model's
/// first language (`fr` when it lists none), with the chosen voice.
pub fn migrate(state: &mut TtsState, first_language_of: impl Fn(&str) -> Option<String>) {
    let Some(id) = state.active.take() else {
        return;
    };
    if !state.languages.is_empty() {
        return;
    }
    let code = first_language_of(&id).unwrap_or_else(|| "fr".to_owned());
    state.languages.push(LanguageEntry {
        code,
        model: Some(id),
        voice: std::mem::replace(&mut state.voice, DEFAULT_VOICE.to_owned()),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(code: &str, model: &str, voice: &str) -> LanguageEntry {
        LanguageEntry {
            code: code.into(),
            model: Some(model.into()),
            voice: voice.into(),
        }
    }

    fn codes(list: &[LanguageEntry]) -> Vec<&str> {
        list.iter().map(|l| l.code.as_str()).collect()
    }

    #[test]
    fn codes_are_normalised_before_comparing() {
        assert_eq!(normalise_code(" FR ").unwrap(), "fr");
        assert_eq!(normalise_code("pt-BR").unwrap(), "pt-br");
        for bad in ["french", "", "f", "fr_FR", "fr-", "1a"] {
            assert!(normalise_code(bad).is_err(), "{bad:?} is refused");
        }
        let mut list = vec![entry("fr", "m", "default")];
        upsert(&mut list, &normalise_code("FR").unwrap(), "m", None);
        assert_eq!(list.len(), 1);
    }

    #[test]
    fn upsert_appends_then_updates() {
        let mut list = vec![entry("fr", "m", "Élise")];
        upsert(&mut list, "en", "e", None);
        assert_eq!(codes(&list), ["fr", "en"]);
        assert_eq!(list[1].voice, "default");
        upsert(&mut list, "fr", "m", None);
        assert_eq!(list[0].voice, "Élise", "same model keeps the voice");
        upsert(&mut list, "fr", "m", Some("Eve"));
        assert_eq!(list[0].voice, "Eve");
        upsert(&mut list, "fr", "other", None);
        assert_eq!(list[0].model.as_deref(), Some("other"));
        assert_eq!(list[0].voice, "default", "a new model resets the voice");
        upsert(&mut list, "fr", "m", Some("Ambre"));
        assert_eq!(list[0].voice, "Ambre");
    }

    #[test]
    fn make_house_moves_to_front() {
        let mut list = vec![entry("fr", "m", "default"), entry("en", "e", "default")];
        assert!(make_house(&mut list, "en"));
        assert_eq!(codes(&list), ["en", "fr"]);
        assert!(!make_house(&mut list, "de"));
    }

    #[test]
    fn remove_first_promotes_next() {
        let mut list = vec![entry("fr", "m", "default"), entry("en", "e", "default")];
        assert!(remove(&mut list, "fr"));
        assert_eq!(codes(&list), ["en"]);
        assert!(!remove(&mut list, "fr"));
    }

    #[test]
    fn models_in_use_dedups() {
        let list = vec![
            entry("fr", "m", "default"),
            entry("en", "m", "default"),
            LanguageEntry {
                code: "de".into(),
                model: None,
                voice: "default".into(),
            },
        ];
        assert_eq!(models_in_use(&list).into_iter().collect::<Vec<_>>(), ["m"]);
        assert_eq!(used_by(&list, "m"), ["fr", "en"]);
        assert!(used_by(&list, "x").is_empty());
    }

    #[test]
    fn detach_leaves_languages_without_a_model() {
        let mut list = vec![entry("fr", "m", "Élise"), entry("en", "e", "Bob")];
        detach_model(&mut list, "m");
        assert_eq!(list[0].model, None);
        assert_eq!(list[0].voice, "default");
        assert_eq!(list[1], entry("en", "e", "Bob"));
    }

    #[test]
    fn empty_model_languages_speak_anything() {
        assert!(speaks(&[], "de"));
        assert!(speaks(&["fr".into()], "fr"));
        assert!(!speaks(&["fr".into()], "de"));
    }

    #[test]
    fn migrate_old_state() {
        let mut s = TtsState {
            active: Some("m".into()),
            voice: "Élise".into(),
            ..TtsState::default()
        };
        migrate(&mut s, |_| Some("fr".into()));
        assert_eq!(s.languages, [entry("fr", "m", "Élise")]);
        assert_eq!(s.active, None);

        let mut none = TtsState::default();
        migrate(&mut none, |_| Some("en".into()));
        assert!(none.languages.is_empty());

        let mut unknown = TtsState {
            active: Some("m".into()),
            ..TtsState::default()
        };
        migrate(&mut unknown, |_| None);
        assert_eq!(unknown.languages, [entry("fr", "m", "default")]);

        // Already migrated: the list is left alone.
        let mut done = TtsState {
            languages: vec![entry("en", "e", "Bob")],
            active: Some("m".into()),
            ..TtsState::default()
        };
        migrate(&mut done, |_| Some("fr".into()));
        assert_eq!(done.languages, [entry("en", "e", "Bob")]);
    }
}
