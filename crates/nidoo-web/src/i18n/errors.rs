//! FR/EN text: the web's own errors (network, HTTP status, broker offline), the connection texts
//! and the shared toast words. The hub's own messages are never in here: they are shown as received.

use super::*;

const NBSP: char = '\u{a0}';

/// A failure the web itself found, as opposed to a message the hub wrote. It is turned into text
/// when it happens (see the `From<WebError> for ApiError` in `api::core`), in the language then
/// active.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WebError {
    /// The request never got an answer; the payload is the browser's own message.
    Network(String),
    /// The hub answered with this status and a body that is not its own error JSON.
    Http(u16),
    /// A reply that could not be read; the payload is the decoder's own message.
    Decode(String),
    /// A 503 on a command: the hub cannot reach the broker.
    BrokerOffline,
    /// A 503 on a climate route: the engine is off on the hub.
    ClimateDisabled,
}

/// The text of a web error: a translated lead, then the browser's own message as received.
pub fn web_error_text(lang: Lang, e: &WebError) -> String {
    match (lang, e) {
        (Lang::En, WebError::Network(m)) => format!("Network error: {m}"),
        (Lang::Fr, WebError::Network(m)) => format!("Erreur réseau{NBSP}: {m}"),
        (Lang::En, WebError::Decode(m)) => format!("Unreadable reply: {m}"),
        (Lang::Fr, WebError::Decode(m)) => format!("Réponse illisible{NBSP}: {m}"),
        (Lang::En, WebError::Http(status)) => format!("The hub answered HTTP {status}"),
        (Lang::Fr, WebError::Http(status)) => format!("Le hub a répondu HTTP {status}"),
        (lang, WebError::BrokerOffline) => error_text(lang, ErrorText::BrokerOffline).to_owned(),
        (lang, WebError::ClimateDisabled) => {
            error_text(lang, ErrorText::ClimateDisabled).to_owned()
        }
    }
}

/// Fixed words of the errors area.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorText {
    BrokerOffline,
    ClimateDisabled,
    /// A command the hub refused without giving a reason.
    CommandRefused,
    /// The close button of a toast.
    Dismiss,
    /// A sheet's back link.
    Back,
    /// The connection pill's tooltip when the socket closed.
    ConnectionClosed,
    /// The connection pill's tooltip when the page has no address to open a socket against.
    NoAddress,
}

/// The text of a fixed word.
pub fn error_text(lang: Lang, k: ErrorText) -> &'static str {
    use ErrorText as T;
    let (en, fr) = match k {
        T::BrokerOffline => (
            "Broker offline — command refused",
            "Broker hors ligne — commande refusée",
        ),
        T::ClimateDisabled => ("Climate is disabled", "Le climat est désactivé"),
        T::CommandRefused => ("Command refused", "Commande refusée"),
        T::Dismiss => ("Dismiss", "Fermer"),
        T::Back => ("Back", "Retour"),
        T::ConnectionClosed => ("Connection closed", "Connexion fermée"),
        T::NoAddress => (
            "Page address unavailable",
            "Adresse de la page indisponible",
        ),
    };
    match lang {
        Lang::En => en,
        Lang::Fr => fr,
    }
}

/// "Could not load dashboards: …": the lead is translated, the hub's text follows as received.
pub fn dashboards_load_failed(lang: Lang, detail: &str) -> String {
    match lang {
        Lang::En => format!("Could not load dashboards: {detail}"),
        Lang::Fr => format!("Impossible de charger les tableaux de bord{NBSP}: {detail}"),
    }
}

/// A failure toast about a named thing: "Lamp: Command refused" / "Lamp : Commande refusée".
/// The name is the user's own and is not translated.
pub fn named_failure(lang: Lang, name: &str, failure: &str) -> String {
    match lang {
        Lang::En => format!("{name}: {failure}"),
        Lang::Fr => format!("{name}{NBSP}: {failure}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: &[ErrorText] = &[
        ErrorText::BrokerOffline,
        ErrorText::ClimateDisabled,
        ErrorText::CommandRefused,
        ErrorText::Dismiss,
        ErrorText::Back,
        ErrorText::ConnectionClosed,
        ErrorText::NoAddress,
    ];

    #[test]
    fn every_error_text_has_two_languages() {
        assert_table(ALL, error_text, &[]);
    }

    #[test]
    fn web_errors_lead_in_the_active_language() {
        let net = WebError::Network("Failed to fetch".into());
        assert_eq!(
            web_error_text(Lang::En, &net),
            "Network error: Failed to fetch"
        );
        assert_eq!(
            web_error_text(Lang::Fr, &net),
            "Erreur réseau\u{a0}: Failed to fetch"
        );
        assert_eq!(
            web_error_text(Lang::En, &WebError::Http(500)),
            "The hub answered HTTP 500"
        );
        assert_eq!(
            web_error_text(Lang::Fr, &WebError::Http(500)),
            "Le hub a répondu HTTP 500"
        );
        assert_eq!(
            web_error_text(Lang::Fr, &WebError::BrokerOffline),
            "Broker hors ligne — commande refusée"
        );
        assert_eq!(
            web_error_text(Lang::En, &WebError::BrokerOffline),
            "Broker offline — command refused"
        );
        assert_eq!(
            web_error_text(Lang::Fr, &WebError::Decode("eof".into())),
            "Réponse illisible\u{a0}: eof"
        );
        assert_eq!(
            web_error_text(Lang::En, &WebError::ClimateDisabled),
            "Climate is disabled"
        );
    }

    #[test]
    fn sentences_follow_french_typography() {
        assert_eq!(
            dashboards_load_failed(Lang::Fr, "boom"),
            "Impossible de charger les tableaux de bord\u{a0}: boom"
        );
        assert_eq!(
            dashboards_load_failed(Lang::En, "boom"),
            "Could not load dashboards: boom"
        );
        assert_eq!(
            named_failure(Lang::Fr, "Lampe", "Commande refusée"),
            "Lampe\u{a0}: Commande refusée"
        );
        assert_eq!(named_failure(Lang::En, "Lamp", "No"), "Lamp: No");
    }
}
