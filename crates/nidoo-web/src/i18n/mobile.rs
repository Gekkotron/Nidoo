//! FR/EN text: the phone setup block on the Done step.

use super::*;

/// A fixed word or sentence of the phone setup block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MobileText {
    Title,
    Lead,
    Show,
    Hide,
    Warning,
    HomeOnly,
    HomeAndAway,
    Remote,
    GatewayAddress,
    GatewayKey,
    KeyPlaceholder,
    Saved,
    Save,
    Hint,
}

/// A fixed word or sentence of the phone setup block.
pub fn mobile_text(lang: Lang, k: MobileText) -> &'static str {
    use MobileText::*;
    let (en, fr) = match k {
        Title => ("Connect your phone", "Connectez votre téléphone"),
        Lead => (
            "Install the Nidoo app, open it, tap Scan setup code and point the camera here.",
            "Installez l'application Nidoo, ouvrez-la, touchez Scanner le code et visez ce code.",
        ),
        Show => ("Show code", "Afficher le code"),
        Hide => ("Hide", "Masquer"),
        Warning => (
            "This code holds your passwords. Show it only to people you trust.",
            "Ce code contient vos mots de passe. Ne le montrez qu'à des personnes de confiance.",
        ),
        HomeOnly => ("Home network only", "Réseau de la maison uniquement"),
        HomeAndAway => ("At home and away", "À la maison et à distance"),
        Remote => ("Remote access", "Accès à distance"),
        GatewayAddress => ("Gateway address", "Adresse de la passerelle"),
        GatewayKey => ("Gateway key", "Clé de la passerelle"),
        KeyPlaceholder => ("64 hex characters", "64 caractères hexadécimaux"),
        Saved => ("Saved", "Enregistrée"),
        Save => ("Save", "Enregistrer"),
        Hint => (
            "Add the key to include remote access.",
            "Ajoutez la clé pour inclure l'accès à distance.",
        ),
    };
    if lang == Lang::Fr { fr } else { en }
}

/// The text table for the current language.
pub fn use_mobile_text() -> impl Fn(MobileText) -> &'static str + Copy + 'static {
    let lang = use_lang();
    move |k| mobile_text(lang(), k)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Adding a variant without listing it in `ALL` fails to compile here.
    const fn listed(k: MobileText) -> bool {
        use MobileText::*;
        match k {
            Title | Lead | Show | Hide | Warning | HomeOnly | HomeAndAway | Remote
            | GatewayAddress | GatewayKey | KeyPlaceholder | Saved | Save | Hint => true,
        }
    }

    const ALL: &[MobileText] = {
        use MobileText::*;
        &[
            Title,
            Lead,
            Show,
            Hide,
            Warning,
            HomeOnly,
            HomeAndAway,
            Remote,
            GatewayAddress,
            GatewayKey,
            KeyPlaceholder,
            Saved,
            Save,
            Hint,
        ]
    };

    #[test]
    fn mobile_copy_has_both_languages() {
        assert!(ALL.iter().all(|&k| listed(k)));
        crate::i18n::assert_table(ALL, mobile_text, &[]);
    }
}
