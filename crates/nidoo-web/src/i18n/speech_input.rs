//! FR/EN text: Assistant → Speech settings → Speech input (talking to a satellite).

use super::*;

/// A fixed word or sentence of the speech input card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpeechInputText {
    Heading,
    Intro,
    NotBuilt,
    NotInstalled,
    Download,
    Cancel,
    Downloading,
    Loading,
    Ready,
    Remove,
    Threads,
    Gain,
    Margin,
    Port,
    PortAfterRestart,
    Save,
    KeyHeading,
    KeyHint,
    CreateKey,
    KeyOnce,
    ReplaceKey,
    ReplaceWarning,
    Replace,
    Keep,
    Failed,
}

/// A fixed word or sentence of the speech input card.
pub fn speech_input_text(lang: Lang, k: SpeechInputText) -> &'static str {
    use SpeechInputText::*;
    let (en, fr) = match k {
        Heading => ("Speech input", "Écoute vocale"),
        Intro => (
            "Talk to a satellite: say “Hey Jarvis”, then your request. The house answers on that satellite. Audio is sent only after the wake word and is never stored.",
            "Parlez à un satellite : dites « Hey Jarvis », puis votre demande. La maison répond sur ce satellite. Le son n’est envoyé qu’après le mot d’éveil et n’est jamais enregistré.",
        ),
        NotBuilt => (
            "This hub was built without speech input.",
            "Ce hub a été construit sans l’écoute vocale.",
        ),
        NotInstalled => (
            "Recognition model not installed: about 490 MB to download, 670 MB on disk.",
            "Modèle de reconnaissance non installé : environ 490 Mo à télécharger, 670 Mo sur le disque.",
        ),
        Download => ("Download", "Télécharger"),
        Cancel => ("Cancel", "Annuler"),
        Downloading => ("Downloading", "Téléchargement"),
        Loading => ("Loading the model…", "Chargement du modèle…"),
        Ready => (
            "Ready: the satellites can be heard.",
            "Prêt : les satellites peuvent être entendus.",
        ),
        Remove => ("Remove the model", "Supprimer le modèle"),
        Threads => ("Recognition threads", "Threads de reconnaissance"),
        Gain => ("Microphone gain", "Gain du micro"),
        Margin => (
            "Wait before a follow-up (ms)",
            "Attente avant une réponse (ms)",
        ),
        Port => ("Hub UDP port", "Port UDP du hub"),
        PortAfterRestart => (
            "The new port applies when the hub restarts.",
            "Le nouveau port s’applique au redémarrage du hub.",
        ),
        Save => ("Save", "Enregistrer"),
        KeyHeading => ("Satellites’ key", "Clé des satellites"),
        KeyHint => (
            "Copy this line now, it is not shown again: paste it into each satellite’s secrets.yaml, then flash it.",
            "Copiez cette ligne maintenant, elle n’est plus affichée ensuite : collez-la dans le secrets.yaml de chaque satellite, puis flashez-le.",
        ),
        CreateKey => ("Create key", "Créer la clé"),
        KeyOnce => (
            "A key is set. The hub shows a key only once, when it is made: to set up another satellite without its line, replace it and flash every satellite again.",
            "Une clé est définie. Le hub n’affiche une clé qu’une fois, à sa création : pour un autre satellite sans sa ligne, remplacez-la et flashez à nouveau tous les satellites.",
        ),
        ReplaceKey => ("Replace key", "Remplacer la clé"),
        ReplaceWarning => (
            "Satellites with the old key stop being heard until they are flashed with the new one.",
            "Les satellites qui ont l’ancienne clé ne sont plus entendus tant qu’ils ne sont pas flashés avec la nouvelle.",
        ),
        Replace => ("Replace", "Remplacer"),
        Keep => ("Keep the old key", "Garder l’ancienne clé"),
        Failed => ("That did not work: ", "Cela n’a pas marché : "),
    };
    match lang {
        Lang::En => en,
        Lang::Fr => fr,
    }
}

/// The text table for the current language.
pub fn use_speech_input_text() -> impl Fn(SpeechInputText) -> &'static str + Copy + 'static {
    let lang = use_lang();
    move |k| speech_input_text(lang(), k)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speech_input_copy_has_both_languages() {
        use SpeechInputText::*;
        let all = [
            Heading,
            Intro,
            NotBuilt,
            NotInstalled,
            Download,
            Cancel,
            Downloading,
            Loading,
            Ready,
            Remove,
            Threads,
            Gain,
            Margin,
            Port,
            PortAfterRestart,
            Save,
            KeyHeading,
            KeyHint,
            CreateKey,
            KeyOnce,
            ReplaceKey,
            ReplaceWarning,
            Replace,
            Keep,
            Failed,
        ];
        crate::i18n::assert_table(&all, speech_input_text, &[]);
    }
}
