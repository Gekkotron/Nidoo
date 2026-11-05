//! The Settings list's status for the Assistant rows: where speech output (the house language)
//! and speech input stand, in one or two words.

use nidoo_api::{EngineState, SttStatus, TtsStatus};

use crate::i18n::{AssistSettingsText as T, Lang, assist_settings_text};

/// An engine's state in a word: Ready, Loading, Error or No model.
fn engine_word(engine: &EngineState, lang: Lang) -> String {
    let key = match engine {
        EngineState::NoModel => T::StateNoModel,
        EngineState::Loading { .. } => T::StateLoading,
        EngineState::Ready { .. } => T::StateReady,
        EngineState::Error { .. } => T::StateError,
    };
    assist_settings_text(lang, key).to_owned()
}

/// Speech output: the house language's engine (the first language), else No model.
pub fn tts_row_status(status: &TtsStatus, lang: Lang) -> String {
    match status.languages.first() {
        Some(house) if status.speech_built => engine_word(&house.engine, lang),
        _ => engine_word(&EngineState::NoModel, lang),
    }
}

/// Speech input: the recognizer's engine once its model is on disk, else No model.
pub fn stt_row_status(status: &SttStatus, lang: Lang) -> String {
    if status.built && status.installed {
        engine_word(&status.engine, lang)
    } else {
        engine_word(&EngineState::NoModel, lang)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nidoo_api::{DEFAULT_VOICE, EngineState, LanguageView, SttSettings, SttStatus, TtsStatus};

    use crate::i18n::Lang;

    fn tts(engines: &[EngineState]) -> TtsStatus {
        TtsStatus {
            languages: engines
                .iter()
                .map(|engine| LanguageView {
                    code: "fr".into(),
                    model: None,
                    voice: DEFAULT_VOICE.into(),
                    engine: engine.clone(),
                })
                .collect(),
            token_set: false,
            download: None,
            speech_built: true,
        }
    }

    fn stt(installed: bool, engine: EngineState) -> SttStatus {
        SttStatus {
            built: true,
            installed,
            engine,
            download: None,
            settings: SttSettings::default(),
            has_key: false,
            port_in_use: 4960,
        }
    }

    fn ready() -> EngineState {
        EngineState::Ready { model: "m".into() }
    }

    fn error() -> EngineState {
        EngineState::Error {
            model: None,
            message: "boom".into(),
        }
    }

    #[test]
    fn tts_row_follows_the_house_language() {
        assert_eq!(tts_row_status(&tts(&[ready()]), Lang::Fr), "Prêt");
        assert_eq!(
            tts_row_status(
                &tts(&[EngineState::Loading { model: "m".into() }, ready()]),
                Lang::En
            ),
            "Loading"
        );
        assert_eq!(tts_row_status(&tts(&[error()]), Lang::Fr), "Erreur");
        assert_eq!(tts_row_status(&tts(&[]), Lang::Fr), "Aucun modèle");
        let mut not_built = tts(&[ready()]);
        not_built.speech_built = false;
        assert_eq!(tts_row_status(&not_built, Lang::En), "No model");
    }

    #[test]
    fn stt_row_needs_an_installed_model() {
        assert_eq!(stt_row_status(&stt(false, ready()), Lang::En), "No model");
        assert_eq!(stt_row_status(&stt(true, ready()), Lang::Fr), "Prêt");
        assert_eq!(stt_row_status(&stt(true, error()), Lang::Fr), "Erreur");
        let mut not_built = stt(true, ready());
        not_built.built = false;
        assert_eq!(stt_row_status(&not_built, Lang::Fr), "Aucun modèle");
    }
}
