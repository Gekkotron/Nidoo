//! FR/EN text: the Assistant page and its floating button. The assistant's own replies are
//! French and come from the hub.

use super::*;

/// A fixed word or sentence of the Assistant page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssistantText {
    /// The page title, the sidebar entry and the breadcrumb.
    Title,
    /// The floating button's label.
    Open,
    Placeholder,
    Send,
    Simulation,
    SimulationHint,
    Hint,
    Yes,
    No,
    HowUnderstood,
    Words,
    Intent,
    Room,
    Entities,
    Value,
    Gate,
    Actions,
    Elapsed,
    /// The button that marks a turn as wrongly understood.
    Wrong,
    MarkedWrong,
    NotKept,
    NoTrace,
    Failed,
    /// The button that empties the conversation on this page.
    Clear,
    ClearHint,
}

/// A fixed word or sentence of the Assistant page.
pub fn assistant_text(lang: Lang, k: AssistantText) -> &'static str {
    use AssistantText::*;
    let (en, fr) = match k {
        Title => ("Assistant", "Assistant"),
        Open => ("Open the assistant", "Ouvrir l'assistant"),
        Placeholder => (
            "Type a sentence in French, e.g. « allume la lumière du salon »",
            "Écrivez une phrase, par exemple « allume la lumière du salon »",
        ),
        Send => ("Send", "Envoyer"),
        Simulation => ("Simulation", "Simulation"),
        SimulationHint => (
            "Plan and reply without touching the house",
            "Comprendre et répondre sans rien envoyer à la maison",
        ),
        Hint => (
            "Lights on or off, heating setpoints and temperatures, in French.",
            "Lumières, consignes de chauffage et températures.",
        ),
        Yes => ("Yes", "Oui"),
        No => ("No", "Non"),
        HowUnderstood => ("How it understood", "Ce qui a été compris"),
        Words => ("Words", "Mots"),
        Intent => ("Intent", "Intention"),
        Room => ("Room", "Pièce"),
        Entities => ("Devices", "Appareils"),
        Value => ("Value", "Valeur"),
        Gate => ("Safety check", "Contrôle de sécurité"),
        Actions => ("Actions", "Actions"),
        Elapsed => ("Time", "Durée"),
        Wrong => ("Wrong", "Faux"),
        MarkedWrong => ("Marked wrong", "Marqué faux"),
        NotKept => (
            "History is not kept: the hub has no database, so past turns are lost on restart.",
            "L'historique n'est pas conservé : sans base de données, le hub oublie les échanges à son redémarrage.",
        ),
        NoTrace => (
            "Send a sentence to see how it is understood.",
            "Envoyez une phrase pour voir ce qui est compris.",
        ),
        Failed => ("The hub did not answer.", "Le hub n'a pas répondu."),
        Clear => ("Clear", "Effacer"),
        ClearHint => (
            "Empty this conversation. The hub keeps it, and ↑ still recalls your sentences.",
            "Vider cette conversation. Le hub la garde, et ↑ rappelle toujours vos phrases.",
        ),
    };
    match lang {
        Lang::En => en,
        Lang::Fr => fr,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_text_has_both_languages() {
        use AssistantText::*;
        assert_table(
            &[
                Title,
                Open,
                Placeholder,
                Send,
                Simulation,
                SimulationHint,
                Hint,
                Yes,
                No,
                HowUnderstood,
                Words,
                Intent,
                Room,
                Entities,
                Value,
                Gate,
                Actions,
                Elapsed,
                Wrong,
                MarkedWrong,
                NotKept,
                NoTrace,
                Failed,
                Clear,
                ClearHint,
            ],
            assistant_text,
            &[Title, Simulation, Actions],
        );
    }
}
