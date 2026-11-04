//! FR/EN text: the Assistant's speech settings screen (`/assistant/settings`).

use super::*;

/// A fixed word or sentence of the speech settings screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssistSettingsText {
    /// The ⚙ link's label in the Assistant header.
    OpenSettings,
    /// The link back to the Assistant.
    Back,
    /// The page title.
    Title,
    // Section headings.
    SpeechHeading,
    TestHeading,
    VoiceHeading,
    ModelsHeading,
    AddHeading,
    TokenHeading,
    // Speech status.
    StateNoModel,
    StateLoading,
    StateReady,
    StateError,
    ActiveModel,
    NoActiveModel,
    NotBuilt,
    NoFrench,
    ActiveNoFrench,
    // Test.
    TestSentence,
    TestLabel,
    Play,
    Speaking,
    FirstSound,
    PlayFailed,
    // Voice.
    VoiceDefault,
    VoiceFellBack,
    VoiceCatalogOffline,
    VoiceEmpty,
    /// The start of a voice's ▶ label: "Listen to" + the voice name.
    Listen,
    // Installed models.
    ModelsEmpty,
    Uploaded,
    UseModel,
    /// The visible label of a model's radio.
    ActiveLabel,
    Remove,
    RemoveQuestion,
    RemoveYes,
    Cancel,
    SpeaksFrench,
    LicenceLabel,
    // Add a model.
    UrlLabel,
    UrlPlaceholder,
    ListFiles,
    Listing,
    WeightsLabel,
    NeedsToken,
    Download,
    Downloading,
    DownloadFailed,
    DownloadCancelled,
    DownloadDone,
    OpenRepo,
    GatedNote,
    // Upload.
    UploadHeading,
    UploadHint,
    UploadChoose,
    Uploading,
    UploadNothing,
    UploadDone,
    // Token.
    TokenLabel,
    TokenSet,
    TokenNotSet,
    Save,
    Clear,
    TokenSaved,
    TokenCleared,
    // Loading the page.
    Loading,
    CouldNotLoad,
    // Languages.
    LanguagesHeading,
    AddLanguage,
    HouseLanguage,
    MakeHouse,
    ModelLabel,
    NoModel,
    RemoveLanguage,
    RemoveLanguageQuestion,
    NoLanguages,
    NoOtherLanguage,
    WhichModel,
    UsedBy,
    RemoveUsedQuestion,
    LanguagesUnknown,
    TokenFold,
    TypeSentence,
    // Units.
    UnitB,
    UnitKb,
    UnitMb,
    UnitGb,
    UnitMs,
}

/// A fixed word or sentence of the speech settings screen.
pub fn assist_settings_text(lang: Lang, k: AssistSettingsText) -> &'static str {
    use AssistSettingsText::*;
    let (en, fr) = match k {
        OpenSettings => ("Speech settings", "Réglages de la voix"),
        Back => ("Assistant", "Assistant"),
        Title => ("Speech settings", "Réglages de la voix"),
        SpeechHeading => ("Speech", "Synthèse vocale"),
        TestHeading => ("Try it", "Essai"),
        VoiceHeading => ("Voice", "Voix"),
        ModelsHeading => ("Installed models", "Modèles installés"),
        AddHeading => ("Add a model", "Ajouter un modèle"),
        TokenHeading => ("Hugging Face token", "Jeton Hugging Face"),
        StateNoModel => ("No model", "Aucun modèle"),
        StateLoading => ("Loading", "Chargement"),
        StateReady => ("Ready", "Prêt"),
        StateError => ("Error", "Erreur"),
        ActiveModel => ("Active model", "Modèle actif"),
        NoActiveModel => ("none", "aucun"),
        NotBuilt => (
            "This hub was built without speech.",
            "Ce hub a été compilé sans la voix.",
        ),
        NoFrench => ("does not speak French", "ne parle pas français"),
        ActiveNoFrench => (
            "The active model does not speak French: speech will sound wrong.",
            "Le modèle actif ne parle pas français : la voix sonnera faux.",
        ),
        TestSentence => (
            "Bonjour, je suis l'assistant de la maison.",
            "Bonjour, je suis l'assistant de la maison.",
        ),
        TestLabel => ("Sentence to speak", "Phrase à prononcer"),
        Play => ("Play", "Lire"),
        Speaking => ("Speaking…", "Lecture…"),
        FirstSound => ("Time to first sound", "Délai avant le premier son"),
        PlayFailed => (
            "The audio could not be played.",
            "Le son n'a pas pu être lu.",
        ),
        VoiceDefault => ("Default", "Par défaut"),
        VoiceFellBack => (
            "The chosen voice is not available with this model; Default is used.",
            "La voix choisie n'est pas disponible avec ce modèle : la voix par défaut est utilisée.",
        ),
        VoiceCatalogOffline => (
            "Catalog voices unavailable offline.",
            "Les voix du catalogue sont indisponibles hors ligne.",
        ),
        VoiceEmpty => ("No voices to choose from.", "Aucune voix à choisir."),
        Listen => ("Listen to", "Écouter"),
        ModelsEmpty => (
            "No model installed yet. Add one below.",
            "Aucun modèle installé. Ajoutez-en un ci-dessous.",
        ),
        Uploaded => ("Uploaded", "Importé"),
        UseModel => ("Use this model", "Utiliser ce modèle"),
        ActiveLabel => ("Active", "Actif"),
        Remove => ("Remove", "Supprimer"),
        RemoveQuestion => ("Remove this model?", "Supprimer ce modèle ?"),
        RemoveYes => ("Yes, remove", "Oui, supprimer"),
        Cancel => ("Cancel", "Annuler"),
        SpeaksFrench => ("speaks French", "parle français"),
        LicenceLabel => ("Licence", "Licence"),
        UrlLabel => (
            "Model page or owner/repo",
            "Page du modèle ou propriétaire/dépôt",
        ),
        UrlPlaceholder => (
            "https://huggingface.co/owner/repo",
            "https://huggingface.co/propriétaire/dépôt",
        ),
        ListFiles => ("List files", "Lister les fichiers"),
        Listing => ("Reading the repository…", "Lecture du dépôt…"),
        WeightsLabel => ("Weight file", "Fichier de poids"),
        NeedsToken => (
            "This model needs a Hugging Face token.",
            "Ce modèle demande un jeton Hugging Face.",
        ),
        Download => ("Download", "Télécharger"),
        Downloading => ("Downloading", "Téléchargement"),
        DownloadFailed => ("Download failed", "Échec du téléchargement"),
        DownloadCancelled => ("Download cancelled", "Téléchargement annulé"),
        DownloadDone => ("Model installed", "Modèle installé"),
        OpenRepo => ("Open on Hugging Face", "Ouvrir sur Hugging Face"),
        GatedNote => (
            "The 40m models are gated and licensed CC BY-NC (non-commercial): accept the licence on their Hugging Face page first.",
            "Les modèles 40m sont soumis à accord préalable et sous licence CC BY-NC (non commerciale) : acceptez la licence sur leur page Hugging Face d'abord.",
        ),
        UploadHeading => (
            "Upload from this computer",
            "Importer depuis cet ordinateur",
        ),
        UploadHint => (
            "Choose a model folder: config, tokenizer and one weight file.",
            "Choisissez un dossier de modèle : config, tokenizer et un fichier de poids.",
        ),
        UploadChoose => ("Choose a folder", "Choisir un dossier"),
        Uploading => ("Uploading", "Envoi"),
        UploadNothing => (
            "No model file found in that folder.",
            "Aucun fichier de modèle dans ce dossier.",
        ),
        UploadDone => ("Model installed", "Modèle installé"),
        TokenLabel => ("Token", "Jeton"),
        TokenSet => ("A token is set", "Un jeton est enregistré"),
        TokenNotSet => ("No token set", "Aucun jeton enregistré"),
        Save => ("Save", "Enregistrer"),
        Clear => ("Clear", "Effacer"),
        TokenSaved => ("Token saved", "Jeton enregistré"),
        TokenCleared => ("Token cleared", "Jeton effacé"),
        Loading => ("Loading…", "Chargement…"),
        LanguagesHeading => ("Languages", "Langues"),
        AddLanguage => ("+ Add", "+ Ajouter"),
        HouseLanguage => (
            "House language: speaks when no language is asked",
            "Langue de la maison : parle quand aucune langue n'est demandée",
        ),
        MakeHouse => ("Make house language", "Choisir comme langue de la maison"),
        ModelLabel => ("Model", "Modèle"),
        NoModel => ("No model", "Aucun modèle"),
        RemoveLanguage => ("Remove language", "Retirer la langue"),
        RemoveLanguageQuestion => ("Remove this language?", "Retirer cette langue ?"),
        NoLanguages => (
            "No language yet. Add a model, then a language.",
            "Aucune langue. Ajoutez un modèle, puis une langue.",
        ),
        NoOtherLanguage => (
            "Installed models speak no other language.",
            "Les modèles installés ne parlent aucune autre langue.",
        ),
        WhichModel => ("Which model?", "Quel modèle ?"),
        UsedBy => ("Used by", "Utilisé par"),
        RemoveUsedQuestion => (
            "Remove it? These languages will have no model:",
            "Le supprimer ? Ces langues n'auront plus de modèle :",
        ),
        LanguagesUnknown => ("languages unknown", "langues inconnues"),
        TokenFold => (
            "Hugging Face token: only needed for gated repositories",
            "Jeton Hugging Face : seulement pour les dépôts protégés",
        ),
        TypeSentence => ("Type a sentence", "Tapez une phrase"),
        UnitB => ("B", "o"),
        UnitKb => ("KB", "Ko"),
        UnitMb => ("MB", "Mo"),
        UnitGb => ("GB", "Go"),
        UnitMs => ("ms", "ms"),
        CouldNotLoad => (
            "The speech settings could not be loaded.",
            "Les réglages de la voix n'ont pas pu être chargés.",
        ),
    };
    if lang == Lang::Fr { fr } else { en }
}

/// The text table for the current language.
pub fn use_assist_settings_text() -> impl Fn(AssistSettingsText) -> &'static str + Copy + 'static {
    let lang = use_lang();
    move |k| assist_settings_text(lang(), k)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Adding a variant without listing it in `ALL` fails to compile here.
    const fn listed(k: AssistSettingsText) -> bool {
        use AssistSettingsText::*;
        match k {
            OpenSettings
            | Back
            | Title
            | SpeechHeading
            | TestHeading
            | VoiceHeading
            | ModelsHeading
            | AddHeading
            | TokenHeading
            | StateNoModel
            | StateLoading
            | StateReady
            | StateError
            | ActiveModel
            | NoActiveModel
            | NotBuilt
            | NoFrench
            | ActiveNoFrench
            | TestSentence
            | TestLabel
            | Play
            | Speaking
            | FirstSound
            | PlayFailed
            | VoiceDefault
            | VoiceFellBack
            | VoiceCatalogOffline
            | VoiceEmpty
            | Listen
            | ModelsEmpty
            | Uploaded
            | UseModel
            | ActiveLabel
            | Remove
            | RemoveQuestion
            | RemoveYes
            | Cancel
            | SpeaksFrench
            | LicenceLabel
            | UrlLabel
            | UrlPlaceholder
            | ListFiles
            | Listing
            | WeightsLabel
            | NeedsToken
            | Download
            | Downloading
            | DownloadFailed
            | DownloadCancelled
            | DownloadDone
            | OpenRepo
            | GatedNote
            | UploadHeading
            | UploadHint
            | UploadChoose
            | Uploading
            | UploadNothing
            | UploadDone
            | TokenLabel
            | TokenSet
            | TokenNotSet
            | Save
            | Clear
            | TokenSaved
            | TokenCleared
            | Loading
            | CouldNotLoad
            | UnitB
            | UnitKb
            | UnitMb
            | UnitGb
            | UnitMs
            | LanguagesHeading
            | AddLanguage
            | HouseLanguage
            | MakeHouse
            | ModelLabel
            | NoModel
            | RemoveLanguage
            | RemoveLanguageQuestion
            | NoLanguages
            | NoOtherLanguage
            | WhichModel
            | UsedBy
            | RemoveUsedQuestion
            | LanguagesUnknown
            | TokenFold
            | TypeSentence => true,
        }
    }

    const ALL: &[AssistSettingsText] = {
        use AssistSettingsText::*;
        &[
            OpenSettings,
            Back,
            Title,
            SpeechHeading,
            TestHeading,
            VoiceHeading,
            ModelsHeading,
            AddHeading,
            TokenHeading,
            StateNoModel,
            StateLoading,
            StateReady,
            StateError,
            ActiveModel,
            NoActiveModel,
            NotBuilt,
            NoFrench,
            ActiveNoFrench,
            TestSentence,
            TestLabel,
            Play,
            Speaking,
            FirstSound,
            PlayFailed,
            VoiceDefault,
            VoiceFellBack,
            VoiceCatalogOffline,
            VoiceEmpty,
            Listen,
            ModelsEmpty,
            Uploaded,
            UseModel,
            ActiveLabel,
            Remove,
            RemoveQuestion,
            RemoveYes,
            Cancel,
            SpeaksFrench,
            LicenceLabel,
            UrlLabel,
            UrlPlaceholder,
            ListFiles,
            Listing,
            WeightsLabel,
            NeedsToken,
            Download,
            Downloading,
            DownloadFailed,
            DownloadCancelled,
            DownloadDone,
            OpenRepo,
            GatedNote,
            UploadHeading,
            UploadHint,
            UploadChoose,
            Uploading,
            UploadNothing,
            UploadDone,
            TokenLabel,
            TokenSet,
            TokenNotSet,
            Save,
            Clear,
            TokenSaved,
            TokenCleared,
            Loading,
            CouldNotLoad,
            LanguagesHeading,
            AddLanguage,
            HouseLanguage,
            MakeHouse,
            ModelLabel,
            NoModel,
            RemoveLanguage,
            RemoveLanguageQuestion,
            NoLanguages,
            NoOtherLanguage,
            WhichModel,
            UsedBy,
            RemoveUsedQuestion,
            LanguagesUnknown,
            TokenFold,
            TypeSentence,
        ]
    };

    #[test]
    fn every_text_has_both_languages() {
        use AssistSettingsText::*;
        assert!(ALL.iter().all(|&k| listed(k)));
        // The test sentence is French in both languages; the rest differ or is a shared word.
        assert_table(
            ALL,
            assist_settings_text,
            &[TestSentence, Back, LicenceLabel, UnitMs],
        );
    }

    #[test]
    fn the_test_sentence_is_the_pinned_one() {
        assert_eq!(
            assist_settings_text(Lang::En, AssistSettingsText::TestSentence),
            "Bonjour, je suis l'assistant de la maison."
        );
    }
}
