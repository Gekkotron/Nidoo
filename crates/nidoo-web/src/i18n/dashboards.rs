//! FR/EN text: Dashboard widgets and group states.

use super::*;

/// What a group card or a room widget says about its members.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroupState {
    /// No member is online.
    Unavailable,
    Opening,
    Closing,
    AllOpen,
    AllClosed,
    /// Covers that disagree.
    Covers {
        open: usize,
        closed: usize,
    },
    AllOn,
    AllOff,
    /// Lights or switches that disagree.
    SomeOn {
        on: usize,
        total: usize,
    },
}

/// The text of a [`GroupState`]: "All closed" / "Tout fermé", "2 of 3 on" / "2 sur 3 allumés".
pub fn group_state(lang: Lang, s: GroupState) -> String {
    // French plural: 0 and 1 are singular.
    let fr_plural =
        |n: usize, one: &str, many: &str| format!("{n} {}", if n > 1 { many } else { one });
    match (lang, s) {
        (Lang::En, GroupState::Unavailable) => "Unavailable".into(),
        (Lang::Fr, GroupState::Unavailable) => "Indisponible".into(),
        (Lang::En, GroupState::Opening) => "Opening…".into(),
        (Lang::Fr, GroupState::Opening) => "Ouverture…".into(),
        (Lang::En, GroupState::Closing) => "Closing…".into(),
        (Lang::Fr, GroupState::Closing) => "Fermeture…".into(),
        (Lang::En, GroupState::AllOpen) => "All open".into(),
        (Lang::Fr, GroupState::AllOpen) => "Tout ouvert".into(),
        (Lang::En, GroupState::AllClosed) => "All closed".into(),
        (Lang::Fr, GroupState::AllClosed) => "Tout fermé".into(),
        (Lang::En, GroupState::Covers { open, closed }) => format!("{open} open · {closed} closed"),
        (Lang::Fr, GroupState::Covers { open, closed }) => format!(
            "{} · {}",
            fr_plural(open, "ouvert", "ouverts"),
            fr_plural(closed, "fermé", "fermés")
        ),
        (Lang::En, GroupState::AllOn) => "All on".into(),
        (Lang::Fr, GroupState::AllOn) => "Tout allumé".into(),
        (Lang::En, GroupState::AllOff) => "All off".into(),
        (Lang::Fr, GroupState::AllOff) => "Tout éteint".into(),
        (Lang::En, GroupState::SomeOn { on, total }) => format!("{on} of {total} on"),
        (Lang::Fr, GroupState::SomeOn { on, total }) => {
            format!(
                "{on} sur {total} {}",
                if on > 1 { "allumés" } else { "allumé" }
            )
        }
    }
}

/// Words of the dashboard screen, its edit mode, its sheets and the widget library.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DashText {
    LoadingDashboard,
    UnknownDashboard,
    BackToHome,
    // Edit toolbar.
    ToggleEdit,
    ToggleEditUnsaved,
    DashboardSettings,
    Discard,
    Save,
    // Tile tools.
    Move,
    Resize,
    // Widget sheet.
    Title,
    Size,
    Range,
    /// The config sheet's picker for what the widget shows.
    Shows,
    DeleteWidget,
    Close,
    // Dashboard sheet.
    Name,
    ShowHeader,
    DeleteDashboard,
    // Library.
    WidgetLibrary,
    AddWidget,
    SearchPlaceholder,
    SearchWidgets,
    NothingMatches,
    RoomGroupSub,
    FlowButtonSub,
    SectionLights,
    SectionCovers,
    SectionClimate,
    SectionSensors,
    SectionWaterHeaters,
    SectionLocks,
    SectionCameras,
    SectionRoomGroups,
    SectionFlowButtons,
    SectionEnergy,
    SectionHistory,
    // Camera widgets.
    AllCameras,
    CamerasLabel,
    AddCamera,
    MoveUp,
    MoveDown,
    RemoveCamera,
}

pub fn dash_text(lang: Lang, k: DashText) -> &'static str {
    use DashText as T;
    let (en, fr) = match k {
        T::LoadingDashboard => ("Loading…", "Chargement…"),
        T::UnknownDashboard => ("Unknown dashboard.", "Tableau de bord inconnu."),
        T::BackToHome => ("Back to House", "Retour à l'accueil"),
        T::ToggleEdit => ("Toggle edit mode", "Activer ou désactiver le mode édition"),
        T::ToggleEditUnsaved => (
            "Toggle edit mode (unsaved changes)",
            "Activer ou désactiver le mode édition (modifications non enregistrées)",
        ),
        T::DashboardSettings => ("Dashboard settings", "Réglages du tableau de bord"),
        T::Discard => ("Discard", "Annuler"),
        T::Save => ("Save", "Enregistrer"),
        T::Move => (
            "Move: drag, or use the arrow keys",
            "Déplacer : glisser, ou utiliser les flèches",
        ),
        T::Resize => ("Resize", "Redimensionner"),
        T::Title => ("Title", "Titre"),
        T::Size => ("Size", "Taille"),
        T::Range => ("Range", "Période"),
        T::Shows => ("Shows", "Affiche"),
        T::DeleteWidget => ("Delete widget", "Supprimer le widget"),
        T::Close => ("Close", "Fermer"),
        T::Name => ("Name", "Nom"),
        T::ShowHeader => ("Show header", "Afficher l'en-tête"),
        T::DeleteDashboard => ("Delete dashboard", "Supprimer le tableau de bord"),
        T::WidgetLibrary => ("Widget library", "Bibliothèque de widgets"),
        T::AddWidget => ("Add a widget", "Ajouter un widget"),
        T::SearchPlaceholder => (
            "Search a widget, a device or a room",
            "Rechercher un widget, un appareil ou une pièce",
        ),
        T::SearchWidgets => ("Search widgets", "Rechercher des widgets"),
        T::NothingMatches => ("Nothing matches", "Aucun résultat"),
        T::RoomGroupSub => (
            "Lights and covers of the room",
            "Éclairages et volets de la pièce",
        ),
        T::FlowButtonSub => ("Runs the flow", "Lance l'automatisation"),
        T::SectionLights => ("LIGHTS", "ÉCLAIRAGES"),
        T::SectionCovers => ("COVERS", "VOLETS"),
        T::SectionClimate => ("CLIMATE", "CLIMAT"),
        T::SectionSensors => ("SENSORS", "CAPTEURS"),
        T::SectionWaterHeaters => ("WATER HEATERS", "CHAUFFE-EAU"),
        T::SectionLocks => ("LOCKS", "SERRURES"),
        T::SectionCameras => ("CAMERAS", "CAMÉRAS"),
        T::SectionRoomGroups => ("ROOM GROUPS", "PIÈCES"),
        T::SectionFlowButtons => ("FLOW BUTTONS", "BOUTONS D'AUTOMATISATION"),
        T::SectionEnergy => ("ENERGY", "ÉNERGIE"),
        T::SectionHistory => ("HISTORY CHARTS", "GRAPHIQUES D'HISTORIQUE"),
        T::AllCameras => ("All cameras", "Toutes les caméras"),
        T::CamerasLabel => ("Cameras", "Caméras"),
        T::AddCamera => ("Add camera", "Ajouter une caméra"),
        T::MoveUp => ("Move up", "Monter"),
        T::MoveDown => ("Move down", "Descendre"),
        T::RemoveCamera => ("Remove camera", "Retirer la caméra"),
    };
    match lang {
        Lang::En => en,
        Lang::Fr => fr,
    }
}

/// What a widget is: "Light" / "Lumière". Read by the config sheet title and the placeholder.
pub fn widget_kind_word(lang: Lang, kind: nidoo_core::dashboard::WidgetKind) -> &'static str {
    use nidoo_core::dashboard::WidgetKind as K;
    let (en, fr) = match kind {
        K::Light => ("Light", "Lumière"),
        K::Cover => ("Cover", "Volet"),
        K::Climate => ("Climate", "Thermostat"),
        K::Lock => ("Lock", "Verrou"),
        K::Sensor => ("Sensor", "Capteur"),
        K::RoomGroup => ("Room group", "Groupe de la pièce"),
        K::FlowButton => ("Flow button", "Bouton d'automatisation"),
        K::EnergyStat => ("Energy stat", "Statistique d'énergie"),
        K::HistoryChart => ("History chart", "Graphique d'historique"),
        K::Camera => ("Camera", "Caméra"),
        K::WaterHeater => ("Water heater", "Chauffe-eau"),
    };
    match lang {
        Lang::En => en,
        Lang::Fr => fr,
    }
}

/// The title of a widget's config sheet: "Widget: Light" / "Widget : Lumière".
pub fn widget_sheet_title(lang: Lang, kind: nidoo_core::dashboard::WidgetKind) -> String {
    let word = widget_kind_word(lang, kind);
    match lang {
        Lang::En => format!("Widget: {word}"),
        Lang::Fr => format!("Widget\u{A0}: {word}"),
    }
}

/// The placeholder of a widget with nothing to show: "Lock: lock.door" / "Verrou : lock.door".
pub fn widget_placeholder(lang: Lang, kind: nidoo_core::dashboard::WidgetKind, id: &str) -> String {
    let word = widget_kind_word(lang, kind);
    match lang {
        Lang::En => format!("{word}: {id}"),
        Lang::Fr => format!("{word}\u{A0}: {id}"),
    }
}

/// "Delete Kitchen?" / "Supprimer Kitchen ?"; the name is the user's.
pub fn delete_question(lang: Lang, name: &str) -> String {
    match lang {
        Lang::En => format!("Delete {name}?"),
        Lang::Fr => format!("Supprimer {name}\u{A0}?"),
    }
}

/// Text of the dashboard widgets that have their own card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WidgetText {
    /// The flow button.
    Run,
    /// A room with neither lights nor covers.
    NoLightsOrCovers,
    // Energy stat labels: short enough for a 1x1 card, the value carries the unit.
    Today,
    ThisWeek,
    ThisMonth,
    GridPowerNow,
    SolarPowerNow,
    SelfConsumption,
    // Camera widget.
    Live,
    LiveOff,
    NoEvents,
    CameraNotFound,
    CameraUnavailable,
    PrevCamera,
    NextCamera,
}

pub fn widget_text(lang: Lang, t: WidgetText) -> &'static str {
    let (en, fr) = match t {
        WidgetText::Run => ("Run", "Lancer"),
        WidgetText::NoLightsOrCovers => ("No lights or covers", "Ni lumière ni volet"),
        WidgetText::Today => ("Today", "Aujourd'hui"),
        WidgetText::ThisWeek => ("Last 7 days", "7 derniers jours"),
        WidgetText::ThisMonth => ("Last 30 days", "30 derniers jours"),
        WidgetText::GridPowerNow => ("Grid now", "Réseau"),
        WidgetText::SolarPowerNow => ("Solar now", "Solaire"),
        WidgetText::SelfConsumption => ("Self-use", "Autoconso"),
        WidgetText::Live => ("Live", "En direct"),
        WidgetText::LiveOff => ("Live view off", "Direct désactivé"),
        WidgetText::NoEvents => ("No events yet", "Aucun événement"),
        WidgetText::CameraNotFound => ("Camera not found", "Caméra introuvable"),
        WidgetText::CameraUnavailable => ("Camera unavailable", "Caméra indisponible"),
        WidgetText::PrevCamera => ("Previous camera", "Caméra précédente"),
        WidgetText::NextCamera => ("Next camera", "Caméra suivante"),
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
    fn group_states_in_french_agree_in_number() {
        assert_eq!(group_state(Lang::Fr, GroupState::AllClosed), "Tout fermé");
        assert_eq!(
            group_state(Lang::Fr, GroupState::Covers { open: 1, closed: 2 }),
            "1 ouvert · 2 fermés"
        );
        assert_eq!(
            group_state(Lang::En, GroupState::Covers { open: 1, closed: 2 }),
            "1 open · 2 closed"
        );
        assert_eq!(
            group_state(Lang::Fr, GroupState::Unavailable),
            "Indisponible"
        );
    }
    #[test]
    fn every_dash_text_has_two_languages() {
        use DashText as T;
        const ALL: &[DashText] = &[
            T::LoadingDashboard,
            T::UnknownDashboard,
            T::BackToHome,
            T::ToggleEdit,
            T::ToggleEditUnsaved,
            T::DashboardSettings,
            T::Discard,
            T::Save,
            T::Move,
            T::Resize,
            T::Title,
            T::Size,
            T::Range,
            T::Shows,
            T::DeleteWidget,
            T::Close,
            T::Name,
            T::ShowHeader,
            T::DeleteDashboard,
            T::WidgetLibrary,
            T::AddWidget,
            T::SearchPlaceholder,
            T::SearchWidgets,
            T::NothingMatches,
            T::RoomGroupSub,
            T::FlowButtonSub,
            T::SectionLights,
            T::SectionCovers,
            T::SectionClimate,
            T::SectionSensors,
            T::SectionWaterHeaters,
            T::SectionLocks,
            T::SectionRoomGroups,
            T::SectionFlowButtons,
            T::SectionEnergy,
            T::SectionHistory,
        ];
        crate::i18n::assert_table(ALL, dash_text, &[]);
    }

    #[test]
    fn sentences_follow_the_language() {
        use nidoo_core::dashboard::WidgetKind;
        assert_eq!(
            widget_sheet_title(Lang::En, WidgetKind::Light),
            "Widget: Light"
        );
        assert_eq!(
            widget_sheet_title(Lang::Fr, WidgetKind::Cover),
            "Widget\u{A0}: Volet"
        );
        assert_eq!(
            widget_sheet_title(Lang::Fr, WidgetKind::Light),
            "Widget\u{A0}: Lumière"
        );
        assert_eq!(
            widget_sheet_title(Lang::Fr, WidgetKind::RoomGroup),
            "Widget\u{A0}: Groupe de la pièce"
        );
        assert_eq!(widget_kind_word(Lang::Fr, WidgetKind::Lock), "Verrou");
        assert_eq!(delete_question(Lang::En, "Den"), "Delete Den?");
        assert_eq!(delete_question(Lang::Fr, "Den"), "Supprimer Den\u{A0}?");
    }

    #[test]
    fn every_dashboards_text_has_two_languages() {
        const ALL: &[WidgetText] = &[
            WidgetText::Run,
            WidgetText::NoLightsOrCovers,
            WidgetText::Today,
            WidgetText::ThisWeek,
            WidgetText::ThisMonth,
            WidgetText::GridPowerNow,
            WidgetText::SolarPowerNow,
            WidgetText::SelfConsumption,
        ];
        crate::i18n::assert_table(ALL, widget_text, &[]);
    }
}
