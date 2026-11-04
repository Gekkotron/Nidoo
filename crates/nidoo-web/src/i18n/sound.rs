//! FR/EN text: Settings › Sound, the speaker satellites the sound service publishes.

use super::*;

/// A fixed word or sentence of the sound page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SoundText {
    Allowed,
    Mute,
    Volume,
    WaitingForApproval,
    ServiceOffline,
    Empty,
    EmptyHint,
    Connected,
    Disconnected,
    Waiting,
    Allow,
    TestSpeaker,
    SaySomething,
    SayPlaceholder,
    Room,
    AllRooms,
    Send,
}

/// A fixed word or sentence of the sound page.
pub fn sound_text(lang: Lang, k: SoundText) -> &'static str {
    use SoundText::*;
    let (en, fr) = match k {
        Allowed => ("Allowed", "Autorisé"),
        Mute => ("Mute", "Muet"),
        Volume => ("Volume", "Volume"),
        WaitingForApproval => ("Waiting for approval", "En attente d’approbation"),
        ServiceOffline => (
            "The sound service is not running. Start it to control the satellites.",
            "Le service son ne tourne pas. Démarrez-le pour piloter les satellites.",
        ),
        Empty => (
            "No sound satellite found yet.",
            "Aucun satellite sonore trouvé pour l’instant.",
        ),
        EmptyHint => (
            "Start the sound service and open a player: it shows up here once it connects. The Sound page of the user docs explains how.",
            "Démarrez le service son et ouvrez un lecteur : il apparaît ici dès qu’il se connecte. La page Son de la documentation utilisateur explique comment.",
        ),
        Connected => ("Connected", "Connecté"),
        Disconnected => ("Offline", "Hors ligne"),
        Waiting => ("waiting", "en attente"),
        Allow => ("Allow", "Autoriser"),
        TestSpeaker => ("Say a test sentence", "Dire une phrase de test"),
        SaySomething => ("Say something", "Dire quelque chose"),
        SayPlaceholder => (
            "A sentence for the speakers",
            "Une phrase pour les haut-parleurs",
        ),
        Room => ("Room", "Pièce"),
        AllRooms => ("All rooms", "Toutes les pièces"),
        Send => ("Send", "Envoyer"),
    };
    match lang {
        Lang::En => en,
        Lang::Fr => fr,
    }
}

/// The sentence the test button says on a satellite: "Speaker test: Kitchen speaker".
pub fn speaker_test(lang: Lang, name: &str) -> String {
    match lang {
        Lang::En => format!("Speaker test: {name}"),
        Lang::Fr => format!("Test du haut-parleur\u{a0}: {name}"),
    }
}

/// "Said on 2 satellites" / "Dit sur 2 satellites", after a sentence was sent.
pub fn said_on(lang: Lang, n: usize) -> String {
    let s = if n == 1 { "" } else { "s" };
    match lang {
        Lang::En => format!("Said on {n} satellite{s}"),
        Lang::Fr => format!("Dit sur {n} satellite{s}"),
    }
}

/// The text table for the current language.
pub fn use_sound_text() -> impl Fn(SoundText) -> &'static str + Copy + 'static {
    let lang = use_lang();
    move |k| sound_text(lang(), k)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sound_copy_has_both_languages() {
        use SoundText::*;
        let all = [
            Allowed,
            Mute,
            Volume,
            WaitingForApproval,
            ServiceOffline,
            Empty,
            EmptyHint,
            Connected,
            Disconnected,
            Waiting,
            Allow,
            TestSpeaker,
            SaySomething,
            SayPlaceholder,
            Room,
            AllRooms,
            Send,
        ];
        crate::i18n::assert_table(&all, sound_text, &[Volume]);
    }

    #[test]
    fn said_on_counts_satellites() {
        assert_eq!(said_on(Lang::En, 1), "Said on 1 satellite");
        assert_eq!(said_on(Lang::En, 3), "Said on 3 satellites");
        assert_eq!(said_on(Lang::Fr, 2), "Dit sur 2 satellites");
    }

    #[test]
    fn speaker_test_names_the_satellite() {
        assert_eq!(speaker_test(Lang::En, "Desk"), "Speaker test: Desk");
        assert_eq!(
            speaker_test(Lang::Fr, "Bureau"),
            "Test du haut-parleur\u{a0}: Bureau"
        );
    }
}
