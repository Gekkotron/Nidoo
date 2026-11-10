//! FR/EN text: flows (automations), their card sentences, schedules and the card library.

use super::*;
use nidoo_core::flow::SunWhich;
use nidoo_core::model::AlarmMode;

const NBSP: char = '\u{A0}';

/// Fixed words of the flows area. Sentences with values are the helper functions below.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlowText {
    // Verbs a card sentence starts with.
    TurnOn,
    TurnOff,
    Toggle,
    Open,
    Close,
    Stop,
    Press,
    Lock,
    Unlock,
    Disarm,
    Wait,
    Notify,
    Speak,
    SpeakAllRooms,
    Enable,
    Disable,
    // Words inside sentences.
    ThisFlow,
    ThisFlowCap,
    TriggerValue,
    IsDark,
    NoTrigger,
    NoRoom,
    GroupTag,
    // Colours of a light, lower case, read in a card title.
    ColourWhite,
    ColourRed,
    ColourOrange,
    ColourYellow,
    ColourGreen,
    ColourCyan,
    ColourBlue,
    ColourPurple,
    ColourMagenta,
    ColourPink,
    // Run state and what fired a run.
    ManualRun,
    ScheduleRun,
    SunRun,
    // Card library entries.
    Schedule,
    ScheduleSub,
    SunEvent,
    SunEventSub,
    WhenDark,
    WhenDarkSub,
    SwitchCard,
    SwitchSub,
    TimeCard,
    TimeSub,
    MathCard,
    MathSub,
    MathInputs,
    MathAddInput,
    MathRemoveInput,
    MathInputName,
    MathNow,
    MathAverageNote,
    MathFormula,
    MathResultName,
    MathResultHint,
    MathNameInvalid,
    MathSaveTo,
    MathNotSaved,
    MathFunctions,
    MathNames,
    MathSuggestions,
    Otherwise,
    Outside,
    WaitCard,
    WaitSub,
    NotifySub,
    SpeakSub,
    DefaultSpeakText,
    SpeakTextLabel,
    SpeakTextExample,
    SpeakRoomLabel,
    Ask,
    AskSub,
    DefaultAskText,
    AskTextLabel,
    AskAnswersLabel,
    AskAddAnswer,
    AskWordsLabel,
    AskNameLabel,
    AskDefaultLabel,
    AskNoAnswer,
    DefaultAnswerYes,
    DefaultAnswerNo,
    DisableSelfSub,
    EnableFlow,
    EnableFlowSub,
    DefaultNotifyTitle,
    SectionWhen,
    SectionAnd,
    SectionThen,
    SectionWait,
    SectionFlow,
    SearchCards,
    SearchCardsLabel,
    // List screen.
    FilterDeviceState,
    FilterButtonEvent,
    FilterManual,
    SearchFlows,
    SearchFlowsLabel,
    Sort,
    SortName,
    SortLastRun,
    GroupBy,
    GroupRoom,
    GroupNone,
    GroupLabel,
    NoLabel,
    Labels,
    AllOn,
    AllOff,
    NewLabel,
    NewChoice,
    AddLabelInChoice,
    TriggerMenu,
    ChoiceIncomplete,
    LabelPick,
    LabelOn,
    LabelOff,
    ManageLabels,
    ChoiceOneActive,
    StandaloneLabels,
    LabelNameField,
    LabelColorField,
    NoChoice,
    CreateLabelButton,
    EditLabel,
    ChoiceDeleteNote,
    AddLabelPlaceholder,
    ViewLabel,
    ViewList,
    ViewCards,
    StatusAll,
    StatusOn,
    StatusOff,
    NeedsAttention,
    RoomNeedsAttention,
    NoFlowsMatch,
    ClearFilters,
    NoFlowsYet,
    NoRunsYet,
    FixFlowFirst,
    FlowActions,
    OpenFlow,
    MoveToRoom,
    DeleteFlow,
    CantUndo,
    Cancel,
    // Canvas.
    PanelLibrary,
    PanelSettings,
    PanelRuns,
    CloseRuns,
    DeleteWire,
    CannotRun,
    UnknownFlow,
    BackToFlows,
    NodeEditor,
    CloseEditor,
    Name,
    Auto,
    DeleteCard,
    HintDelete,
    HintDuplicate,
    Required,
    NodeTrigger,
    NodeSun,
    NodeOffPeak,
    OffPeakSub,
    OffPeakStarts,
    OffPeakEnds,
    IsOffPeak,
    IsOffPeakSub,
    OpIsOffPeak,
    OffPeakEdge,
    NodeManual,
    ManualSub,
    ManualValue,
    ManualNoValue,
    ManualText,
    ManualPress,
    ManualSaveFirst,
    NodeSignal,
    NodeIf,
    NodeDelay,
    NodeSilent,
    SilentSub,
    NodeLimit,
    LimitSub,
    LimitMax,
    LimitPer,
    NodeCommand,
    NodeFlowSwitch,
    NodeFlowLabel,
    TitleLabel,
    TitleExample,
    BodyLabel,
    NoneParen,
    AddValue,
    TriggerValueRow,
    Preview,
    Silent,
    Entity,
    FiresWhen,
    OnlyAfter,
    AnySignal,
    EntityDefaults,
    ValueOf,
    MissingFail,
    MissingTrue,
    MissingFalse,
    Duration,
    UpToRandom,
    ExactDuration,
    CommandJson,
    OtherValue,
    ChooseValue,
    BackToList,
    Action,
    NoneChoice,
    ValueFromResult,
    ColourCustom,
    AdvancedJson,
    ParamPosition,
    ParamTemperature,
    ParamValue,
    ParamBrightness,
    ParamColour,
    ParamEffect,
    ParamCode,
    ParisFallback,
    SunRise,
    SunSet,
    SunDawn,
    SunDusk,
    TimingAt,
    TimingBefore,
    TimingAfter,
    NoEventWeek,
    Tidy,
    TidyTitle,
    Undo,
    CancelEditing,
    Saving,
    SavingEllipsis,
    Save,
    RunNow,
    EditLayout,
    FlowName,
    Runs,
    ZoomIn,
    ZoomOut,
    FitView,
    Minimap,
    // Schedule picker.
    EveryNMinutes,
    Hourly,
    Daily,
    Weekdays,
    Advanced,
    PickOneDay,
    // Screens (part B).
    PageTitle,
    ModeRestart,
    ModeSingle,
    ModeRestartOption,
    ModeSingleOption,
    Mode,
    ModeHint,
    Invalid,
    NextFires,
    Minutes,
    Zoom,
    AddCard,
    NothingMatches,
    Back,
    StatusLabel,
    TriggerType,
    DeleteFlowLabel,
    StepOk,
    StepFailed,
    StepSkipped,
    NodeCompare,
    Channel,
    Severity,
    NodeFlow,
    NodeAction,
    SolarEvent,
    Timing,
    WireYes,
    WireNo,
    ParisWarning,
    EveryMinutesLabel,
    AtMinuteLabel,
    AtTimeLabel,
    OnDays,
    CronLabel,
    SevInfo,
    SevWarning,
    SevCritical,
    OpIsDark,
    WhenUnknown,
    AverageOver,
    AverageHint,
    DarkShift,
    DarkShiftHint,
    OpBetween,
    AddRule,
    OtherwiseExit,
    FireAll,
    MoveRuleUp,
    MoveRuleDown,
    DeleteRule,
    RuleAnd,
    TimeFrom,
    TimeTo,
    RuleLabel,
}

impl FlowText {
    pub const ALL: &'static [FlowText] = &[
        FlowText::TurnOn,
        FlowText::TurnOff,
        FlowText::Toggle,
        FlowText::Open,
        FlowText::Close,
        FlowText::Stop,
        FlowText::Press,
        FlowText::Lock,
        FlowText::Unlock,
        FlowText::Disarm,
        FlowText::Wait,
        FlowText::Notify,
        FlowText::Speak,
        FlowText::SpeakAllRooms,
        FlowText::Enable,
        FlowText::Disable,
        FlowText::ThisFlow,
        FlowText::ThisFlowCap,
        FlowText::TriggerValue,
        FlowText::IsDark,
        FlowText::NoTrigger,
        FlowText::NoRoom,
        FlowText::GroupTag,
        FlowText::ColourWhite,
        FlowText::ColourRed,
        FlowText::ColourOrange,
        FlowText::ColourYellow,
        FlowText::ColourGreen,
        FlowText::ColourCyan,
        FlowText::ColourBlue,
        FlowText::ColourPurple,
        FlowText::ColourMagenta,
        FlowText::ColourPink,
        FlowText::ManualRun,
        FlowText::ScheduleRun,
        FlowText::SunRun,
        FlowText::Schedule,
        FlowText::ScheduleSub,
        FlowText::SunEvent,
        FlowText::SunEventSub,
        FlowText::WhenDark,
        FlowText::WhenDarkSub,
        FlowText::SwitchCard,
        FlowText::SwitchSub,
        FlowText::TimeCard,
        FlowText::TimeSub,
        FlowText::MathCard,
        FlowText::MathSub,
        FlowText::MathInputs,
        FlowText::MathAddInput,
        FlowText::MathRemoveInput,
        FlowText::MathInputName,
        FlowText::MathNow,
        FlowText::MathAverageNote,
        FlowText::MathFormula,
        FlowText::MathResultName,
        FlowText::MathResultHint,
        FlowText::MathNameInvalid,
        FlowText::MathSaveTo,
        FlowText::MathNotSaved,
        FlowText::MathFunctions,
        FlowText::MathNames,
        FlowText::MathSuggestions,
        FlowText::Otherwise,
        FlowText::Outside,
        FlowText::WaitCard,
        FlowText::WaitSub,
        FlowText::NotifySub,
        FlowText::SpeakSub,
        FlowText::DefaultSpeakText,
        FlowText::SpeakTextLabel,
        FlowText::SpeakTextExample,
        FlowText::SpeakRoomLabel,
        FlowText::Ask,
        FlowText::AskSub,
        FlowText::DefaultAskText,
        FlowText::AskTextLabel,
        FlowText::AskAnswersLabel,
        FlowText::AskAddAnswer,
        FlowText::AskWordsLabel,
        FlowText::AskNameLabel,
        FlowText::AskDefaultLabel,
        FlowText::AskNoAnswer,
        FlowText::DefaultAnswerYes,
        FlowText::DefaultAnswerNo,
        FlowText::DisableSelfSub,
        FlowText::EnableFlow,
        FlowText::EnableFlowSub,
        FlowText::DefaultNotifyTitle,
        FlowText::SectionWhen,
        FlowText::SectionAnd,
        FlowText::SectionThen,
        FlowText::SectionWait,
        FlowText::SectionFlow,
        FlowText::SearchCards,
        FlowText::SearchCardsLabel,
        FlowText::FilterDeviceState,
        FlowText::FilterButtonEvent,
        FlowText::FilterManual,
        FlowText::SearchFlows,
        FlowText::SearchFlowsLabel,
        FlowText::Sort,
        FlowText::SortName,
        FlowText::SortLastRun,
        FlowText::GroupBy,
        FlowText::GroupRoom,
        FlowText::GroupNone,
        FlowText::GroupLabel,
        FlowText::NoLabel,
        FlowText::Labels,
        FlowText::AllOn,
        FlowText::AllOff,
        FlowText::NewLabel,
        FlowText::NewChoice,
        FlowText::AddLabelInChoice,
        FlowText::TriggerMenu,
        FlowText::ChoiceIncomplete,
        FlowText::LabelPick,
        FlowText::LabelOn,
        FlowText::LabelOff,
        FlowText::ManageLabels,
        FlowText::ChoiceOneActive,
        FlowText::StandaloneLabels,
        FlowText::LabelNameField,
        FlowText::LabelColorField,
        FlowText::NoChoice,
        FlowText::CreateLabelButton,
        FlowText::EditLabel,
        FlowText::ChoiceDeleteNote,
        FlowText::AddLabelPlaceholder,
        FlowText::ViewLabel,
        FlowText::ViewList,
        FlowText::ViewCards,
        FlowText::StatusAll,
        FlowText::StatusOn,
        FlowText::StatusOff,
        FlowText::NeedsAttention,
        FlowText::RoomNeedsAttention,
        FlowText::NoFlowsMatch,
        FlowText::ClearFilters,
        FlowText::NoFlowsYet,
        FlowText::NoRunsYet,
        FlowText::FixFlowFirst,
        FlowText::FlowActions,
        FlowText::OpenFlow,
        FlowText::MoveToRoom,
        FlowText::DeleteFlow,
        FlowText::CantUndo,
        FlowText::Cancel,
        FlowText::PanelLibrary,
        FlowText::PanelSettings,
        FlowText::PanelRuns,
        FlowText::CloseRuns,
        FlowText::DeleteWire,
        FlowText::CannotRun,
        FlowText::UnknownFlow,
        FlowText::BackToFlows,
        FlowText::NodeEditor,
        FlowText::CloseEditor,
        FlowText::Name,
        FlowText::Auto,
        FlowText::DeleteCard,
        FlowText::HintDelete,
        FlowText::HintDuplicate,
        FlowText::Required,
        FlowText::NodeTrigger,
        FlowText::NodeSun,
        FlowText::NodeOffPeak,
        FlowText::OffPeakSub,
        FlowText::OffPeakStarts,
        FlowText::OffPeakEnds,
        FlowText::IsOffPeak,
        FlowText::IsOffPeakSub,
        FlowText::OpIsOffPeak,
        FlowText::OffPeakEdge,
        FlowText::NodeManual,
        FlowText::ManualSub,
        FlowText::ManualValue,
        FlowText::ManualNoValue,
        FlowText::ManualText,
        FlowText::ManualPress,
        FlowText::ManualSaveFirst,
        FlowText::NodeSignal,
        FlowText::NodeIf,
        FlowText::NodeDelay,
        FlowText::NodeSilent,
        FlowText::SilentSub,
        FlowText::NodeLimit,
        FlowText::LimitSub,
        FlowText::LimitMax,
        FlowText::LimitPer,
        FlowText::NodeCommand,
        FlowText::NodeFlowSwitch,
        FlowText::NodeFlowLabel,
        FlowText::TitleLabel,
        FlowText::TitleExample,
        FlowText::BodyLabel,
        FlowText::NoneParen,
        FlowText::AddValue,
        FlowText::TriggerValueRow,
        FlowText::Preview,
        FlowText::Silent,
        FlowText::Entity,
        FlowText::FiresWhen,
        FlowText::OnlyAfter,
        FlowText::AnySignal,
        FlowText::EntityDefaults,
        FlowText::ValueOf,
        FlowText::MissingFail,
        FlowText::MissingTrue,
        FlowText::MissingFalse,
        FlowText::Duration,
        FlowText::UpToRandom,
        FlowText::ExactDuration,
        FlowText::CommandJson,
        FlowText::OtherValue,
        FlowText::ChooseValue,
        FlowText::BackToList,
        FlowText::Action,
        FlowText::NoneChoice,
        FlowText::ValueFromResult,
        FlowText::ColourCustom,
        FlowText::AdvancedJson,
        FlowText::ParamPosition,
        FlowText::ParamTemperature,
        FlowText::ParamValue,
        FlowText::ParamBrightness,
        FlowText::ParamColour,
        FlowText::ParamEffect,
        FlowText::ParamCode,
        FlowText::ParisFallback,
        FlowText::SunRise,
        FlowText::SunSet,
        FlowText::SunDawn,
        FlowText::SunDusk,
        FlowText::TimingAt,
        FlowText::TimingBefore,
        FlowText::TimingAfter,
        FlowText::NoEventWeek,
        FlowText::Tidy,
        FlowText::TidyTitle,
        FlowText::Undo,
        FlowText::CancelEditing,
        FlowText::Saving,
        FlowText::SavingEllipsis,
        FlowText::Save,
        FlowText::RunNow,
        FlowText::EditLayout,
        FlowText::FlowName,
        FlowText::Runs,
        FlowText::ZoomIn,
        FlowText::ZoomOut,
        FlowText::FitView,
        FlowText::Minimap,
        FlowText::EveryNMinutes,
        FlowText::Hourly,
        FlowText::Daily,
        FlowText::Weekdays,
        FlowText::Advanced,
        FlowText::PickOneDay,
        FlowText::PageTitle,
        FlowText::ModeRestart,
        FlowText::ModeSingle,
        FlowText::ModeRestartOption,
        FlowText::ModeSingleOption,
        FlowText::Mode,
        FlowText::ModeHint,
        FlowText::Invalid,
        FlowText::NextFires,
        FlowText::Minutes,
        FlowText::Zoom,
        FlowText::AddCard,
        FlowText::NothingMatches,
        FlowText::Back,
        FlowText::StatusLabel,
        FlowText::TriggerType,
        FlowText::DeleteFlowLabel,
        FlowText::StepOk,
        FlowText::StepFailed,
        FlowText::StepSkipped,
        FlowText::NodeCompare,
        FlowText::Channel,
        FlowText::Severity,
        FlowText::NodeFlow,
        FlowText::NodeAction,
        FlowText::SolarEvent,
        FlowText::Timing,
        FlowText::WireYes,
        FlowText::WireNo,
        FlowText::ParisWarning,
        FlowText::EveryMinutesLabel,
        FlowText::AtMinuteLabel,
        FlowText::AtTimeLabel,
        FlowText::OnDays,
        FlowText::CronLabel,
        FlowText::SevInfo,
        FlowText::SevWarning,
        FlowText::SevCritical,
        FlowText::OpIsDark,
        FlowText::WhenUnknown,
        FlowText::AverageOver,
        FlowText::AverageHint,
        FlowText::DarkShift,
        FlowText::DarkShiftHint,
        FlowText::OpBetween,
        FlowText::AddRule,
        FlowText::OtherwiseExit,
        FlowText::FireAll,
        FlowText::MoveRuleUp,
        FlowText::MoveRuleDown,
        FlowText::DeleteRule,
        FlowText::RuleAnd,
        FlowText::TimeFrom,
        FlowText::TimeTo,
        FlowText::RuleLabel,
    ];
}

pub fn flow_text(lang: Lang, k: FlowText) -> &'static str {
    use FlowText as T;
    let (en, fr) = match k {
        T::TurnOn => ("Turn on", "Allumer"),
        T::TurnOff => ("Turn off", "Éteindre"),
        T::Toggle => ("Toggle", "Basculer"),
        T::Open => ("Open", "Ouvrir"),
        T::Close => ("Close", "Fermer"),
        T::Stop => ("Stop", "Arrêter"),
        T::Press => ("Press", "Appuyer sur"),
        T::Lock => ("Lock", "Verrouiller"),
        T::Unlock => ("Unlock", "Déverrouiller"),
        T::Disarm => ("Disarm", "Désactiver"),
        T::Wait => ("Wait", "Attendre"),
        T::Notify => ("Notify", "Notifier"),
        T::Speak => ("Speak", "Annoncer"),
        T::SpeakAllRooms => ("All rooms", "Toutes les pièces"),
        T::Enable => ("Enable", "Activer"),
        T::Disable => ("Disable", "Désactiver"),
        T::ThisFlow => ("this flow", "cette automatisation"),
        T::ThisFlowCap => ("This flow", "Cette automatisation"),
        T::TriggerValue => ("Trigger value", "Valeur du déclencheur"),
        T::IsDark => (
            "Is it dark now? (sun below \u{2212}6°)",
            "Fait-il nuit\u{A0}? (soleil sous \u{2212}6°)",
        ),
        T::NoTrigger => ("No trigger", "Aucun déclencheur"),
        T::NoRoom => ("No room", "Sans pièce"),
        T::GroupTag => ("(Group)", "(Groupe)"),
        T::ColourWhite => ("white", "blanc"),
        T::ColourRed => ("red", "rouge"),
        T::ColourOrange => ("orange", "orange"),
        T::ColourYellow => ("yellow", "jaune"),
        T::ColourGreen => ("green", "vert"),
        T::ColourCyan => ("cyan", "cyan"),
        T::ColourBlue => ("blue", "bleu"),
        T::ColourPurple => ("purple", "violet"),
        T::ColourMagenta => ("magenta", "magenta"),
        T::ColourPink => ("pink", "rose"),
        T::ManualRun => ("manual", "manuel"),
        T::ScheduleRun => ("schedule", "programmation"),
        T::SunRun => ("sun", "soleil"),
        T::Schedule => ("Schedule", "Programmation"),
        T::ScheduleSub => (
            "At a set time, or on a repeat",
            "À heure fixe ou de façon répétée",
        ),
        T::SunEvent => ("Sun event", "Événement solaire"),
        T::SunEventSub => (
            "Sunrise, sunset, dusk or dawn",
            "Lever ou coucher du soleil, crépuscule ou aube",
        ),
        T::WhenDark => ("When it's dark", "Quand il fait nuit"),
        T::NodeOffPeak => ("Off-peak", "Heures creuses"),
        T::OffPeakSub => (
            "When off-peak starts or ends",
            "Quand les heures creuses commencent ou finissent",
        ),
        T::OffPeakStarts => ("Off-peak starts", "Les heures creuses commencent"),
        T::OffPeakEnds => ("Off-peak ends", "Les heures creuses finissent"),
        T::IsOffPeak => ("Is it off-peak?", "Est-ce les heures creuses\u{A0}?"),
        T::IsOffPeakSub => ("Off-peak hours right now", "Heures creuses en ce moment"),
        T::OpIsOffPeak => ("is off-peak", "heures creuses"),
        T::OffPeakEdge => ("Fires when", "Se déclenche quand"),
        T::WhenDarkSub => ("Between dusk and dawn", "Entre le crépuscule et l'aube"),
        T::SwitchCard => ("Switch", "Commuter"),
        T::SwitchSub => (
            "Several exits, one per rule",
            "Plusieurs sorties, une par règle",
        ),
        T::TimeCard => ("Time of day", "Plage horaire"),
        T::MathCard => ("Math", "Calcul"),
        T::MathSub => (
            "Compute a number from your devices",
            "Calculer un nombre à partir de vos appareils",
        ),
        T::MathInputs => ("Inputs", "Entrées"),
        T::MathAddInput => ("＋ Input", "＋ Entrée"),
        T::MathRemoveInput => ("Remove input", "Retirer l'entrée"),
        T::MathInputName => ("Input name", "Nom de l'entrée"),
        T::MathNow => ("now", "maintenant"),
        T::MathAverageNote => (
            "\u{201c}now\u{201d} switches to an average (1m, 5m, 15m, 1h)",
            "« maintenant » passe à une moyenne (1m, 5m, 15m, 1h)",
        ),
        T::MathFormula => ("Formula", "Formule"),
        T::MathResultName => ("Result name", "Nom du résultat"),
        T::MathResultHint => (
            "Later cards use it: a Command's value, an If or {name} in a notification",
            "Les cartes suivantes l'utilisent : la valeur d'une Commande, un Si ou {name} dans une notification",
        ),
        T::MathNameInvalid => (
            "Lowercase letters, digits and _, starting with a letter",
            "Minuscules, chiffres et _, en commençant par une lettre",
        ),
        T::MathFunctions => ("ƒ Functions", "ƒ Fonctions"),
        T::MathNames => ("Names you can use", "Noms utilisables"),
        T::MathSuggestions => ("Suggestions", "Propositions"),
        T::MathSaveTo => ("Save to (optional)", "Enregistrer dans (facultatif)"),
        T::MathNotSaved => (
            "\u{2014} not saved \u{2014}",
            "\u{2014} non enregistré \u{2014}",
        ),
        T::TimeSub => (
            "Inside or outside two times",
            "Dans ou hors d'une plage horaire",
        ),
        T::Otherwise => ("otherwise", "sinon"),
        T::Outside => ("outside", "en dehors"),
        T::WaitCard => ("Wait", "Attente"),
        T::WaitSub => ("A fixed or random delay", "Une attente fixe ou aléatoire"),
        T::NotifySub => (
            "A push to your phone",
            "Une notification sur votre téléphone",
        ),
        T::DisableSelfSub => ("Stops it from running again", "L'empêche de se relancer"),
        T::EnableFlow => ("Enable a flow", "Activer une automatisation"),
        T::EnableFlowSub => (
            "Switches another flow on or off",
            "Active ou désactive une autre automatisation",
        ),
        T::DefaultNotifyTitle => ("Notification", "Notification"),
        T::SpeakSub => (
            "Said out loud on the speakers",
            "Dit à voix haute sur les haut-parleurs",
        ),
        T::DefaultSpeakText => ("Hello", "Bonjour"),
        T::SpeakTextLabel => ("Sentence", "Phrase"),
        T::SpeakTextExample => ("Dinner is ready", "Le dîner est prêt"),
        T::SpeakRoomLabel => ("Room", "Pièce"),
        T::Ask => ("Ask", "Demander"),
        T::AskSub => (
            "Asks out loud and waits for the answer",
            "Pose la question à voix haute et attend la réponse",
        ),
        T::DefaultAskText => ("Shall I turn the heating up?", "Je monte le chauffage ?"),
        T::AskTextLabel => ("Question", "Question"),
        T::AskAnswersLabel => ("Answers", "Réponses"),
        T::AskAddAnswer => ("Add an answer", "Ajouter une réponse"),
        T::AskWordsLabel => ("Also accepts", "Accepte aussi"),
        T::AskNameLabel => ("Keep the number as", "Garder le nombre sous"),
        T::AskDefaultLabel => ("If no number", "Sans nombre"),
        T::AskNoAnswer => ("No answer", "Pas de réponse"),
        T::DefaultAnswerYes => ("Yes", "Oui"),
        T::DefaultAnswerNo => ("No", "Non"),
        T::SectionWhen => ("WHEN · TRIGGER", "Déclencheurs"),
        T::SectionAnd => ("AND · CHECK", "Conditions"),
        T::SectionThen => ("THEN · ACTION", "Actions"),
        T::SectionWait => ("WAIT", "Attente"),
        T::SectionFlow => ("FLOW", "Automatisation"),
        T::SearchCards => (
            "Search a card or an entity",
            "Rechercher une carte ou une entité",
        ),
        T::SearchCardsLabel => ("Search cards", "Rechercher des cartes"),
        T::FilterDeviceState => ("Device state", "État d'un appareil"),
        T::FilterButtonEvent => ("Button / event", "Bouton / événement"),
        T::FilterManual => ("Manual", "Manuel"),
        T::SearchFlows => (
            "Search flows, devices, rooms…",
            "Rechercher une automatisation, un appareil, une pièce…",
        ),
        T::SearchFlowsLabel => ("Search flows", "Rechercher des automatisations"),
        T::Sort => ("Sort", "Tri"),
        T::SortName => ("Name", "Nom"),
        T::SortLastRun => ("Last run", "Dernier lancement"),
        T::GroupBy => ("Group", "Regroupement"),
        T::GroupRoom => ("Room", "Pièce"),
        T::GroupNone => ("None", "Aucun"),
        T::GroupLabel => ("Label", "Étiquette"),
        T::NoLabel => ("No label", "Sans étiquette"),
        T::Labels => ("Labels", "Étiquettes"),
        T::AllOn => ("Turn all on", "Tout activer"),
        T::AllOff => ("Turn all off", "Tout désactiver"),
        T::NewLabel => ("New label…", "Nouvelle étiquette…"),
        T::NewChoice => ("Choice", "Choix"),
        T::AddLabelInChoice => ("Label in this choice", "Étiquette dans ce choix"),
        T::TriggerMenu => ("Trigger", "Déclencheur"),
        T::ChoiceIncomplete => ("Needs two labels", "Il faut deux étiquettes"),
        T::LabelPick => ("Pick", "Choisir"),
        T::LabelOn => ("Turn on", "Activer"),
        T::LabelOff => ("Turn off", "Désactiver"),
        T::ManageLabels => ("Manage labels", "Gérer les étiquettes"),
        T::ChoiceOneActive => ("choice · one at a time", "choix · une seule active"),
        T::StandaloneLabels => ("Standalone labels", "Étiquettes seules"),
        T::LabelNameField => ("Name", "Nom"),
        T::LabelColorField => ("Colour", "Couleur"),
        T::NoChoice => ("No choice", "Aucun choix"),
        T::CreateLabelButton => ("Create", "Créer"),
        T::EditLabel => ("Edit", "Modifier"),
        T::ChoiceDeleteNote => (
            "Its labels stay, as standalone labels.",
            "Ses étiquettes restent, comme étiquettes seules.",
        ),
        T::AddLabelPlaceholder => ("＋ add…", "＋ ajouter…"),
        T::ViewLabel => ("View", "Affichage"),
        T::ViewList => ("List", "Liste"),
        T::ViewCards => ("Cards", "Cartes"),
        T::StatusAll => ("All", "Toutes"),
        T::StatusOn => ("On", "Actives"),
        T::StatusOff => ("Off", "Inactives"),
        T::NeedsAttention => ("⚠ Needs attention", "⚠ À vérifier"),
        T::RoomNeedsAttention => (
            "A flow in this room needs attention",
            "Une automatisation de cette pièce demande votre attention",
        ),
        T::NoFlowsMatch => ("No flows match", "Aucune automatisation ne correspond"),
        T::ClearFilters => ("Clear filters", "Effacer les filtres"),
        T::NoFlowsYet => (
            "No flows yet. Ask the assistant to create one.",
            "Aucune automatisation pour l'instant. Demandez à l'assistant d'en créer une.",
        ),
        T::NoRunsYet => ("No runs yet", "Aucun lancement pour l'instant"),
        T::FixFlowFirst => (
            "Fix this flow first",
            "Corrigez d'abord cette automatisation",
        ),
        T::FlowActions => ("Flow actions", "Actions de l'automatisation"),
        T::OpenFlow => ("Open", "Ouvrir"),
        T::MoveToRoom => ("Move to room", "Déplacer vers une pièce"),
        T::DeleteFlow => ("Delete flow…", "Supprimer l'automatisation…"),
        T::CantUndo => ("This can't be undone.", "Cette action est irréversible."),
        T::Cancel => ("Cancel", "Annuler"),
        T::PanelLibrary => ("Card library", "Bibliothèque de cartes"),
        T::PanelSettings => ("Card settings", "Réglages de la carte"),
        T::PanelRuns => ("Recent runs", "Lancements récents"),
        T::CloseRuns => ("Close runs", "Fermer les lancements"),
        T::DeleteWire => ("Delete wire", "Supprimer le lien"),
        T::CannotRun => (
            "This flow cannot run",
            "Cette automatisation ne peut pas se lancer",
        ),
        T::UnknownFlow => ("Unknown flow", "Automatisation inconnue"),
        T::BackToFlows => ("Back to flows", "Retour aux automatisations"),
        T::NodeEditor => ("Node editor", "Éditeur de carte"),
        T::CloseEditor => ("Close editor", "Fermer l'éditeur"),
        T::Name => ("Name", "Nom"),
        T::Auto => ("(auto)", "(auto)"),
        T::DeleteCard => ("Delete card", "Supprimer la carte"),
        T::HintDelete => ("delete", "supprimer"),
        T::HintDuplicate => ("duplicate", "dupliquer"),
        T::Required => ("required", "obligatoire"),
        T::NodeTrigger => ("Trigger", "Déclencheur"),
        T::NodeSun => ("Sun", "Soleil"),
        T::NodeManual => ("Manual", "Manuel"),
        T::ManualSub => (
            "Start by hand from the ▶ button",
            "Lancer à la main avec le bouton ▶",
        ),
        T::ManualValue => ("Value", "Valeur"),
        T::ManualNoValue => ("No value", "Aucune valeur"),
        T::ManualText => ("Text", "Texte"),
        T::ManualPress => ("Run from here", "Lancer depuis ici"),
        T::ManualSaveFirst => ("Save to run from here", "Enregistrez pour lancer d'ici"),
        T::NodeSignal => ("Signal", "Signal"),
        T::NodeIf => ("If", "Si"),
        T::NodeDelay => ("Delay", "Attente"),
        T::NodeSilent => ("Silent for", "Silencieux depuis"),
        T::SilentSub => (
            "When an entity stops reporting",
            "Quand une entité n'envoie plus rien",
        ),
        T::NodeLimit => ("Limit", "Limite"),
        T::LimitSub => (
            "Lets only so many runs through",
            "Ne laisse passer qu'un nombre de lancements",
        ),
        T::LimitMax => (
            "Runs allowed (1\u{2013}100)",
            "Lancements autorisés (1\u{2013}100)",
        ),
        T::LimitPer => (
            "Per window (1m\u{2013}168h)",
            "Par fenêtre (1m\u{2013}168h)",
        ),
        T::NodeCommand => ("Command", "Commande"),
        T::NodeFlowSwitch => ("Flow switch", "Interrupteur d'automatisation"),
        T::NodeFlowLabel => ("Pick a label", "Choisir une étiquette"),
        T::TitleLabel => ("Title", "Titre"),
        T::TitleExample => ("Door open", "Porte ouverte"),
        T::BodyLabel => ("Body", "Message"),
        T::NoneParen => ("(none)", "(aucun)"),
        T::AddValue => ("＋ Value", "＋ Valeur"),
        T::TriggerValueRow => ("Trigger value", "Valeur du déclencheur"),
        T::Preview => ("Preview", "Aperçu"),
        T::Silent => ("Silent", "Silencieuse"),
        T::Entity => ("Entity", "Entité"),
        T::FiresWhen => (
            "Fires when it becomes (JSON, optional)",
            "Se déclenche quand elle devient (JSON, facultatif)",
        ),
        T::OnlyAfter => (
            "Only after (previous, JSON, optional)",
            "Seulement après (valeur précédente, JSON, facultatif)",
        ),
        T::AnySignal => ("any", "n'importe lequel"),
        T::EntityDefaults => (
            "Entity (defaults to trigger value)",
            "Entité (par défaut\u{A0}: valeur du déclencheur)",
        ),
        T::MissingFail => ("Fail", "Échouer"),
        T::MissingTrue => ("Take Yes", "Prendre «\u{A0}Oui\u{A0}»"),
        T::MissingFalse => ("Take No", "Prendre «\u{A0}Non\u{A0}»"),
        T::Duration => ("Duration", "Durée"),
        T::UpToRandom => ("Up to (random)", "Jusqu'à (aléatoire)"),
        T::ExactDuration => ("(exact duration)", "(durée exacte)"),
        T::CommandJson => (
            r#"Command (JSON, e.g. {"type":"turn_on"})"#,
            r#"Commande (JSON, p. ex. {"type":"turn_on"})"#,
        ),
        T::OtherValue => ("Other value…", "Autre valeur…"),
        T::ChooseValue => ("Choose…", "Choisir…"),
        T::BackToList => ("Back to the list", "Revenir à la liste"),
        T::Action => ("Action", "Action"),
        T::NoneChoice => ("None", "Aucun"),
        T::ValueFromResult => ("Value from result", "Valeur d'un résultat"),
        T::ColourCustom => ("Custom", "Personnalisée"),
        T::AdvancedJson => ("Advanced (JSON)", "Avancé (JSON)"),
        T::ParamPosition => ("Position", "Position"),
        T::ParamTemperature => ("Temperature", "Température"),
        T::ParamValue => ("Value", "Valeur"),
        T::ParamBrightness => ("Brightness", "Luminosité"),
        T::ParamColour => ("Colour", "Couleur"),
        T::ParamEffect => ("Effect", "Effet"),
        T::ParamCode => ("Code", "Code"),
        T::ParisFallback => ("Paris fallback", "Paris par défaut"),
        T::SunRise => ("Sunrise", "Lever du soleil"),
        T::SunSet => ("Sunset", "Coucher du soleil"),
        T::SunDawn => ("Dawn", "Aube"),
        T::SunDusk => ("Dusk", "Crépuscule"),
        T::TimingAt => ("At the event", "À l'événement"),
        T::TimingBefore => ("Before", "Avant"),
        T::TimingAfter => ("After", "Après"),
        T::NoEventWeek => (
            "No event in the next week",
            "Aucun événement la semaine prochaine",
        ),
        T::Tidy => ("Tidy", "Ranger"),
        T::TidyTitle => (
            "Arrange the cards automatically",
            "Ranger les cartes automatiquement",
        ),
        T::Undo => ("Undo", "Défaire"),
        T::CancelEditing => ("Cancel editing", "Annuler la modification"),
        T::Saving => ("Saving", "Enregistrement"),
        T::SavingEllipsis => ("Saving…", "Enregistrement…"),
        T::Save => ("Save", "Enregistrer"),
        T::RunNow => ("Run now", "Lancer maintenant"),
        T::EditLayout => ("Edit layout", "Modifier la disposition"),
        T::FlowName => ("Flow name", "Nom de l'automatisation"),
        T::Runs => ("Runs", "Lancements"),
        T::ZoomIn => ("Zoom in", "Zoom avant"),
        T::ZoomOut => ("Zoom out", "Zoom arrière"),
        T::FitView => ("Fit view", "Ajuster la vue"),
        T::Minimap => ("Minimap", "Mini-carte"),
        T::EveryNMinutes => ("Every N minutes", "Toutes les N minutes"),
        T::Hourly => ("Hourly", "Toutes les heures"),
        T::Daily => ("Daily", "Tous les jours"),
        T::Weekdays => ("Weekdays", "Jours de la semaine"),
        T::Advanced => ("Advanced", "Avancé"),
        T::PickOneDay => ("Pick at least one day", "Choisissez au moins un jour"),
        T::PageTitle => ("Flows", "Automatisations"),
        T::ModeRestart => ("restart", "relance"),
        T::ModeSingle => ("single", "unique"),
        T::ModeRestartOption => ("Restart if running", "Relancer si en cours"),
        T::ModeSingleOption => ("Ignore if running", "Ignorer si en cours"),
        T::Mode => ("Mode", "Mode"),
        T::ModeHint => (
            "When the trigger fires during a run: single ignores it, restart starts the run over",
            "Si le déclencheur revient pendant un lancement\u{A0}: unique l'ignore, relance repart de zéro",
        ),
        T::Invalid => ("Invalid", "Invalide"),
        T::NextFires => ("Next fires", "Prochains déclenchements"),
        T::Minutes => ("Minutes", "Minutes"),
        T::Zoom => ("Zoom", "Zoom"),
        T::AddCard => ("Add a card", "Ajouter une carte"),
        T::NothingMatches => ("Nothing matches", "Aucun résultat"),
        T::Back => ("Back", "Retour"),
        T::StatusLabel => ("Status", "Statut"),
        T::TriggerType => ("Trigger type", "Type de déclencheur"),
        T::DeleteFlowLabel => ("Delete flow", "Supprimer l'automatisation"),
        T::StepOk => ("ok", "ok"),
        T::StepFailed => ("failed", "échec"),
        T::StepSkipped => ("skipped", "ignoré"),
        T::NodeCompare => ("Compare", "Comparer"),
        T::Channel => ("Channel", "Canal"),
        T::Severity => ("Severity", "Gravité"),
        T::NodeFlow => ("Flow", "Automatisation"),
        T::NodeAction => ("Action", "Action"),
        T::SolarEvent => ("Solar event", "Événement solaire"),
        T::Timing => ("Timing", "Moment"),
        T::WireYes => ("yes", "oui"),
        T::WireNo => ("no", "non"),
        T::ParisWarning => (
            "No latitude/longitude in [flows]: Paris is used by default",
            "Pas de latitude/longitude dans [flows]\u{A0}: Paris est utilisé par défaut",
        ),
        T::EveryMinutesLabel => ("Every (minutes, 1–59)", "Toutes les (minutes, 1–59)"),
        T::AtMinuteLabel => ("At minute (:MM, 0–59)", "À la minute (:MM, 0–59)"),
        T::AtTimeLabel => ("At (HH:MM)", "À (HH:MM)"),
        T::OnDays => ("On", "Les jours"),
        T::CronLabel => (
            "Cron (minute hour day month weekday)",
            "Cron (minute heure jour mois jour de la semaine)",
        ),
        T::SevInfo => ("info", "info"),
        T::SevWarning => ("warning", "avertissement"),
        T::SevCritical => ("critical", "critique"),
        T::OpIsDark => ("is dark", "fait nuit"),
        T::WhenUnknown => ("When the value is unknown", "Quand la valeur est inconnue"),
        T::ValueOf => ("Value of", "Valeur de"),
        T::AverageOver => ("Average over", "Moyenne sur"),
        T::AverageHint => (
            "empty = current value \u{b7} e.g. 15m",
            "vide = valeur actuelle \u{b7} ex. 15m",
        ),
        T::DarkShift => ("Shift (minutes)", "Décalage (minutes)"),
        T::DarkShiftHint => (
            "\u{2212}30 = dark 30\u{a0}min before civil dusk and until 30\u{a0}min after civil dawn",
            "\u{2212}30 = nuit 30\u{a0}min avant le crépuscule civil et jusqu'à 30\u{a0}min après l'aube civile",
        ),
        T::OpBetween => ("between", "entre"),
        T::AddRule => ("Add rule", "Ajouter une règle"),
        T::OtherwiseExit => ("Otherwise exit", "Sortie « sinon »"),
        T::FireAll => (
            "Fire every matching rule",
            "Déclencher toutes les règles vraies",
        ),
        T::MoveRuleUp => ("Move rule up", "Monter la règle"),
        T::MoveRuleDown => ("Move rule down", "Descendre la règle"),
        T::DeleteRule => ("Delete rule", "Supprimer la règle"),
        T::RuleAnd => ("and", "et"),
        T::TimeFrom => ("From", "De"),
        T::TimeTo => ("Until", "Jusqu'à"),
        T::RuleLabel => ("Rule", "Règle"),
    };
    match lang {
        Lang::En => en,
        Lang::Fr => fr,
    }
}

/// What a label switch did, in a sentence: « Été choisi : 5 activées, 12 désactivées. »,
/// « Vacances : 3 désactivées. » or « … : rien à changer. », then « 1 non modifiée : Nom
/// (invalide). » when some flows were skipped. The label reads as its catalog name, or its id
/// when the catalog no longer has it.
pub fn label_switch_result(
    lang: Lang,
    r: &nidoo_api::LabelSwitchResult,
    catalog: &nidoo_core::flow::FlowLabelCatalog,
) -> String {
    use nidoo_core::flow::LabelAction;
    let fr = lang == Lang::Fr;
    let name = catalog
        .label(&r.label)
        .map_or(r.label.as_str(), |l| l.name.as_str());
    // Spec wording: French plural for every count but 1 (« 0 désactivées »).
    let n_fr = |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    let on = |n| {
        if fr {
            n_fr(n, "activée", "activées")
        } else {
            format!("{n} on")
        }
    };
    let off = |n| {
        if fr {
            n_fr(n, "désactivée", "désactivées")
        } else {
            format!("{n} off")
        }
    };
    let (enabled, disabled) = (r.enabled.len(), r.disabled.len());
    let counts = if enabled + disabled == 0 {
        (if fr {
            "rien à changer"
        } else {
            "nothing to change"
        })
        .to_owned()
    } else {
        match r.action {
            LabelAction::Pick => format!("{}, {}", on(enabled), off(disabled)),
            LabelAction::On => on(enabled),
            LabelAction::Off => off(disabled),
        }
    };
    let head = match (r.action, fr) {
        (LabelAction::Pick, true) => format!("{name} choisi"),
        (LabelAction::Pick, false) => format!("{name} picked"),
        _ => name.to_owned(),
    };
    let colon = if fr { "\u{a0}:" } else { ":" };
    let mut out = format!("{head}{colon} {counts}.");
    if !r.skipped.is_empty() {
        let names: Vec<String> = r
            .skipped
            .iter()
            .map(|s| {
                let reason = match (s.reason.as_str(), fr) {
                    ("invalid", true) => "invalide",
                    (other, _) => other,
                };
                format!("{} ({reason})", s.name)
            })
            .collect();
        let n = r.skipped.len();
        let lead = if fr {
            n_fr(n, "non modifiée", "non modifiées")
        } else {
            format!("{n} not changed")
        };
        out.push_str(&format!(" {lead}{colon} {}.", names.join(", ")));
    }
    out
}

/// The live preview of a "pick a label" card: « Active les 5 automatisations « Été », désactive
/// les 12 « Hiver ». ». The off part is left out when nothing would be turned off.
pub fn label_preview(lang: Lang, on: usize, label: &str, off: usize, others: &[&str]) -> String {
    let quote = |s: &str| match lang {
        Lang::Fr => format!("«{NBSP}{s}{NBSP}»"),
        Lang::En => format!("\"{s}\""),
    };
    let label = quote(label);
    let others: Vec<String> = others.iter().map(|o| quote(o)).collect();
    let others = others.join(" / ");
    let mut out = match (lang, on) {
        (Lang::Fr, 0) => format!("Aucune automatisation {label} à activer"),
        (Lang::Fr, 1) => format!("Active l'automatisation {label}"),
        (Lang::Fr, n) => format!("Active les {n} automatisations {label}"),
        (Lang::En, 0) => format!("No {label} flow to turn on"),
        (Lang::En, 1) => format!("Turns on the {label} flow"),
        (Lang::En, n) => format!("Turns on the {n} {label} flows"),
    };
    if off > 0 && !others.is_empty() {
        out.push_str(&match (lang, off) {
            (Lang::Fr, 1) => format!(", désactive l'automatisation {others}"),
            (Lang::Fr, n) => format!(", désactive les {n} {others}"),
            (Lang::En, 1) => format!(", turns off the {others} flow"),
            (Lang::En, n) => format!(", turns off the {n} {others} flows"),
        });
    }
    out.push('.');
    out
}

/// A translator for [`FlowText`] bound to the language in context (reactive).
pub fn use_flow_text() -> impl Fn(FlowText) -> &'static str + Copy + 'static {
    let lang = use_lang();
    move |k| flow_text(lang(), k)
}

/// The word on a run's badge: "Running" / "En cours".
pub fn run_status(lang: Lang, status: nidoo_api::RunStatus) -> &'static str {
    use nidoo_api::RunStatus as S;
    let (en, fr) = match status {
        S::Running => ("Running", "En cours"),
        S::Done => ("Done", "Terminé"),
        S::Failed => ("Failed", "Échec"),
        S::Cancelled => ("Cancelled", "Annulé"),
        S::Skipped => ("Skipped", "Ignoré"),
    };
    match lang {
        Lang::En => en,
        Lang::Fr => fr,
    }
}

fn pick(lang: Lang, en: String, fr: String) -> String {
    match lang {
        Lang::En => en,
        Lang::Fr => fr,
    }
}

/// A question: "…?" in English, "…\u{A0}?" in French.
pub fn ask(lang: Lang, s: &str) -> String {
    match lang {
        Lang::En => format!("{s}?"),
        Lang::Fr => format!("{s}{NBSP}?"),
    }
}

/// The name of a value inside an entity's state ("Battery" / "Batterie"); a type-table field
/// without a translation reads as its name with spaces, capitalised.
pub fn field_label(lang: Lang, name: &str) -> String {
    let [en, fr] = match name {
        "battery" => ["Battery", "Batterie"],
        "fan_speed" => ["Fan speed", "Vitesse d'aspiration"],
        "brightness" => ["Brightness", "Luminosité"],
        "color_temp" => ["Colour temperature", "Température de couleur"],
        "effect" => ["Effect", "Effet"],
        "position" => ["Position", "Position"],
        "current" => ["Current temperature", "Température actuelle"],
        "target" => ["Target temperature", "Consigne"],
        "fan_mode" => ["Fan mode", "Mode de ventilation"],
        "action" => ["Activity", "Activité"],
        other => {
            let spaced = other.replace('_', " ");
            let mut chars = spaced.chars();
            return chars
                .next()
                .map(|first| first.to_uppercase().chain(chars).collect())
                .unwrap_or_default();
        }
    };
    match lang {
        Lang::En => en,
        Lang::Fr => fr,
    }
    .to_owned()
}

/// "Temperature avg 15 min" / "Temperature moy. 15 min": the entity's average over a window.
pub fn average_of(lang: Lang, e: &str, window: &str) -> String {
    pick(
        lang,
        format!("{e} avg {window}"),
        format!("{e} moy. {window}"),
    )
}

/// "When Door changes" / "Quand Door change".
pub fn when_changes(lang: Lang, e: &str) -> String {
    pick(
        lang,
        format!("When {e} changes"),
        format!("Quand {e} change"),
    )
}

/// "When Lamp turns on" / "Quand Lamp s'allume". `word` is the French state word for anything
/// but a light ("activé", "ouvert"); English always says on/off.
pub fn when_turns(lang: Lang, e: &str, on: bool, light: bool, word: &str) -> String {
    let en = format!("When {e} turns {}", if on { "on" } else { "off" });
    let fr = match (light, on) {
        (true, true) => format!("Quand {e} s'allume"),
        (true, false) => format!("Quand {e} s'éteint"),
        (false, _) => format!("Quand {e} devient {word}"),
    };
    pick(lang, en, fr)
}

/// "When Door becomes open" / "Quand Door devient ouvert".
pub fn when_becomes(lang: Lang, e: &str, v: &str) -> String {
    pick(
        lang,
        format!("When {e} becomes {v}"),
        format!("Quand {e} devient {v}"),
    )
}

/// A remote's press signal as words, "button 1 pressed 2×" / "bouton 1 appuyé 2×": `<digits>_<press>`
/// or a bare `<press>` with press one of single, double, triple, hold, long or release. Anything
/// else is `None`.
pub fn press_label(lang: Lang, signal: &str) -> Option<String> {
    let (button, press) = match signal.split_once('_') {
        Some((n, press)) if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => {
            (Some(n), press)
        }
        _ => (None, signal),
    };
    let (en, fr) = match press {
        "single" => ("pressed", "appuyé"),
        "double" => ("pressed 2×", "appuyé 2×"),
        "triple" => ("pressed 3×", "appuyé 3×"),
        "hold" | "long" => ("held", "maintenu"),
        "release" => ("released", "relâché"),
        _ => return None,
    };
    Some(match button {
        Some(n) => pick(lang, format!("button {n} {en}"), format!("bouton {n} {fr}")),
        None => pick(lang, en.to_owned(), fr.to_owned()),
    })
}

/// `press_label` with a capital, the signal chip's text; other signals stay raw.
pub fn signal_chip_label(lang: Lang, signal: &str) -> String {
    let Some(label) = press_label(lang, signal) else {
        return signal.to_owned();
    };
    let mut chars = label.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or(label)
}

/// "When Remote sends on" / "Quand Remote envoie on"; no `signal` reads "sends a signal". A press
/// signal drops the entity, the card's second line names the device: "When button 1 held".
pub fn when_sends(lang: Lang, e: &str, signal: Option<&str>) -> String {
    if let Some(label) = signal.and_then(|s| press_label(lang, s)) {
        return pick(lang, format!("When {label}"), format!("Quand {label}"));
    }
    match signal {
        Some(s) => pick(
            lang,
            format!("When {e} sends {s}"),
            format!("Quand {e} envoie {s}"),
        ),
        None => pick(
            lang,
            format!("When {e} sends a signal"),
            format!("Quand {e} envoie un signal"),
        ),
    }
}

/// "Remote sends on" / "Remote envoie on", the list row's trigger text.
pub fn sends(lang: Lang, e: &str, signal: Option<&str>) -> String {
    match signal {
        Some(s) => pick(lang, format!("{e} sends {s}"), format!("{e} envoie {s}")),
        None => pick(
            lang,
            format!("{e} signals"),
            format!("{e} envoie un signal"),
        ),
    }
}

/// "Lamp is on?" / "Lamp est allumée\u{A0}?".
pub fn is_value(lang: Lang, e: &str, v: &str) -> String {
    ask(
        lang,
        &pick(lang, format!("{e} is {v}"), format!("{e} est {v}")),
    )
}

/// "Lamp is not on?" / "Lamp n'est pas allumée\u{A0}?".
pub fn is_not_value(lang: Lang, e: &str, v: &str) -> String {
    ask(
        lang,
        &pick(
            lang,
            format!("{e} is not {v}"),
            format!("{e} n'est pas {v}"),
        ),
    )
}

/// "Wait 5 min" / "Attendre 5 min"; with `up_to`, "Wait 1 min to 2 h" / "Attendre de 1 min à 2 h".
pub fn wait(lang: Lang, for_: &str, up_to: Option<&str>) -> String {
    let verb = flow_text(lang, FlowText::Wait);
    match up_to {
        Some(b) => pick(
            lang,
            format!("{verb} {for_} to {b}"),
            format!("{verb} de {for_} à {b}"),
        ),
        None => format!("{verb} {for_}"),
    }
}

/// "Door silent for 50m" / "Door silencieux depuis 50m"; with no entity yet, "Silent for 50m".
pub fn silent_for(lang: Lang, e: &str, for_: &str) -> String {
    let head = flow_text(lang, FlowText::NodeSilent);
    if e.is_empty() {
        return format!("{head} {for_}");
    }
    pick(
        lang,
        format!("{e} silent for {for_}"),
        format!("{e} silencieux depuis {for_}"),
    )
}

/// "At most 4 every 24h" / "Au plus 4 fois par 24h".
pub fn at_most(lang: Lang, max: u32, per: &str) -> String {
    pick(
        lang,
        format!("At most {max} every {per}"),
        format!("Au plus {max} fois par {per}"),
    )
}

/// `Speak "Dinner is ready" — Kitchen` / `Annoncer « Le dîner est prêt » — Cuisine`; `room` is
/// the room's name, or `None` for every room. No text reads "Speak" / "Annoncer".
pub fn speak(lang: Lang, text: &str, room: Option<&str>) -> String {
    let room = room.unwrap_or_else(|| flow_text(lang, FlowText::SpeakAllRooms));
    if text.trim().is_empty() {
        return format!("{} — {room}", flow_text(lang, FlowText::Speak));
    }
    pick(
        lang,
        format!("Speak \"{text}\" — {room}"),
        format!("Annoncer «{NBSP}{text}{NBSP}» — {room}"),
    )
}

/// `Ask "Heat up?" in Kitchen: Yes / No / no answer` / `Demande « Chauffer ? » dans Cuisine : Oui
/// / Non / pas de réponse`; `room` is the room's name, or `None` for every room, worded as
/// Speak words it (`Ask "Heat up?" — All rooms: Yes / no answer`).
pub fn ask_card(lang: Lang, text: &str, room: Option<&str>, answers: &[&str]) -> String {
    let (en_room, fr_room) = match room {
        Some(r) => (format!("in {r}"), format!("dans {r}")),
        None => (
            format!("— {}", flow_text(Lang::En, FlowText::SpeakAllRooms)),
            format!("— {}", flow_text(Lang::Fr, FlowText::SpeakAllRooms)),
        ),
    };
    let mut parts: Vec<&str> = answers.to_vec();
    parts.push(match lang {
        Lang::En => "no answer",
        Lang::Fr => "pas de réponse",
    });
    let list = parts.join(" / ");
    let quoted = text.trim();
    pick(
        lang,
        if quoted.is_empty() {
            format!("Ask {en_room}: {list}")
        } else {
            format!("Ask \"{text}\" {en_room}: {list}")
        },
        if quoted.is_empty() {
            format!("Demande {fr_room}{NBSP}: {list}")
        } else {
            format!("Demande «{NBSP}{text}{NBSP}» {fr_room}{NBSP}: {list}")
        },
    )
}

/// `Notify "Door open"` / `Notifier « Porte ouverte »`; no title reads "Notify" / "Notifier".
pub fn notify(lang: Lang, title: &str) -> String {
    if title.trim().is_empty() {
        return flow_text(lang, FlowText::Notify).to_owned();
    }
    pick(
        lang,
        format!("Notify \"{title}\""),
        format!("Notifier «{NBSP}{title}{NBSP}»"),
    )
}

/// "Set Lamp to 50 %" / "Régler Lamp sur 50 %".
pub fn set_to(lang: Lang, e: &str, v: &str) -> String {
    pick(
        lang,
        format!("Set {e} to {v}"),
        format!("Régler {e} sur {v}"),
    )
}

fn alarm_mode_words(mode: AlarmMode) -> (&'static str, &'static str) {
    match mode {
        AlarmMode::Home => ("home", "présent"),
        AlarmMode::Away => ("away", "absent"),
        AlarmMode::Night => ("night", "nuit"),
        AlarmMode::Vacation => ("vacation", "vacances"),
        AlarmMode::CustomBypass => ("custom bypass", "personnalisée"),
    }
}

/// "Arm Alarm (away)" / "Activer Alarme (absent)": the security screen's arm words.
pub fn arm(lang: Lang, e: &str, mode: AlarmMode) -> String {
    let (en, fr) = alarm_mode_words(mode);
    pick(
        lang,
        format!("Arm {e} ({en})"),
        format!("Activer {e} ({fr})"),
    )
}

/// "Arm (away)" / "Activer (absent)": an arm action in a list, without the entity.
pub fn arm_action(lang: Lang, mode: AlarmMode) -> String {
    let (en, fr) = alarm_mode_words(mode);
    pick(lang, format!("Arm ({en})"), format!("Activer ({fr})"))
}

/// "Fan: Low" / "Ventilation : Low": a fan mode in an action list.
pub fn fan_mode_action(lang: Lang, mode: &str) -> String {
    pick(
        lang,
        format!("Fan: {mode}"),
        format!("Ventilation{NBSP}: {mode}"),
    )
}

/// "Fan speed: Max" / "Vitesse : Max": a vacuum fan speed in an action list.
pub fn fan_speed_action(lang: Lang, speed: &str) -> String {
    pick(
        lang,
        format!("Fan speed: {speed}"),
        format!("Vitesse{NBSP}: {speed}"),
    )
}

/// "Mode: On" / "Mode : Marche": a control set to a value in an action list.
pub fn control_action(lang: Lang, control: &str, value: &str) -> String {
    pick(
        lang,
        format!("{control}: {value}"),
        format!("{control}{NBSP}: {value}"),
    )
}

/// "Set Heater mode to heat" / "Régler Heater en mode chauffage". The entity comes right after
/// the verb, so a caller that cuts the title out still reads as a sentence.
pub fn set_mode(lang: Lang, e: &str, mode: &str) -> String {
    pick(
        lang,
        format!("Set {e} mode to {mode}"),
        format!("Régler {e} en mode {mode}"),
    )
}

/// "Set Fan percentage to 60" / "Régler Fan sur 60 (percentage)".
pub fn set_control(lang: Lang, e: &str, control: &str, v: &str) -> String {
    pick(
        lang,
        format!("Set {e} {control} to {v}"),
        format!("Régler {e} sur {v} ({control})"),
    )
}

/// " at 39 %" / " à 39 %", appended to a "Turn on" sentence (no-break space before the unit).
pub fn at_brightness(lang: Lang, percent: f64) -> String {
    pick(
        lang,
        format!(" at {percent}{NBSP}%"),
        format!(" à {percent}{NBSP}%"),
    )
}

/// " in magenta" / " en magenta".
pub fn in_colour(lang: Lang, colour: FlowText) -> String {
    let w = flow_text(lang, colour);
    pick(lang, format!(" in {w}"), format!(" en {w}"))
}

/// " with breathe" / " avec breathe"; the effect is the device's own word, never translated.
pub fn with_effect(lang: Lang, effect: &str) -> String {
    pick(lang, format!(" with {effect}"), format!(" avec {effect}"))
}

/// "Enable Heating" / "Activer Heating"; `target` is a flow name or [`FlowText::ThisFlow`].
pub fn flow_switch(lang: Lang, enabled: bool, target: &str) -> String {
    let verb = flow_text(
        lang,
        if enabled {
            FlowText::Enable
        } else {
            FlowText::Disable
        },
    );
    format!("{verb} {target}")
}

/// `(English name, French "at" sentence, French name with its article)`.
fn sun_ref(which: SunWhich) -> (&'static str, &'static str, &'static str) {
    match which {
        SunWhich::Sunrise => ("sunrise", "Au lever du soleil", "le lever du soleil"),
        SunWhich::Sunset => ("sunset", "Au coucher du soleil", "le coucher du soleil"),
        SunWhich::Dusk => ("civil dusk", "Au crépuscule civil", "le crépuscule civil"),
        SunWhich::Dawn => ("civil dawn", "À l'aube civile", "l'aube civile"),
    }
}

/// "At sunset", "15 min after sunrise", "15 min before civil dusk" / "Au coucher du soleil",
/// "15 min après le lever du soleil", "15 min avant le crépuscule civil".
pub fn sun_sentence(lang: Lang, which: SunWhich, offset_minutes: i32) -> String {
    let (en, fr_at, fr_ref) = sun_ref(which);
    let n = offset_minutes.unsigned_abs();
    match (lang, offset_minutes) {
        (Lang::En, 0) => format!("At {en}"),
        (Lang::Fr, 0) => fr_at.to_owned(),
        (Lang::En, o) if o > 0 => format!("{n}{NBSP}min after {en}"),
        (Lang::Fr, o) if o > 0 => format!("{n}{NBSP}min après {fr_ref}"),
        (Lang::En, _) => format!("{n}{NBSP}min before {en}"),
        (Lang::Fr, _) => format!("{n}{NBSP}min avant {fr_ref}"),
    }
}

/// The `switch` card: "Switch on Temperature" / "Commuter selon Temperature".
pub fn switch_on(lang: Lang, e: &str) -> String {
    pick(
        lang,
        format!("Switch on {e}"),
        format!("Commuter selon {e}"),
    )
}

/// The `time.between` card: "Between 07:00 and 22:30" / "Entre 07:00 et 22:30".
pub fn time_between(lang: Lang, from: &str, to: &str) -> String {
    pick(
        lang,
        format!("Between {from} and {to}"),
        format!("Entre {from} et {to}"),
    )
}

/// The value after "=" under the formula, in the language's number form. Keeps up to three
/// decimals, so a result like 47.6 is not rounded to 48.
pub fn math_value(lang: Lang, value: f64) -> String {
    let rounded = (value * 1000.0).round() / 1000.0;
    if rounded.fract() == 0.0 || rounded.abs() >= 1000.0 {
        crate::i18n::format::number(lang, rounded)
    } else {
        let text = format!("{rounded}");
        match lang {
            Lang::Fr => text.replace('.', ","),
            Lang::En => text,
        }
    }
}

/// The note beside the preview of a formula whose input averages over `window`: it reads the
/// current value, not the average.
pub fn math_averaged_note(lang: Lang, window: &str) -> String {
    match lang {
        Lang::En => format!("now, not the {window} average"),
        Lang::Fr => format!("maintenant, pas la moyenne sur {window}"),
    }
}

/// The one-line description of formula function `name` in the Functions menu.
pub fn function_help(lang: Lang, name: &str) -> &'static str {
    let (en, fr) = match name {
        "min" => ("smallest of the values", "la plus petite des valeurs"),
        "max" => ("largest of the values", "la plus grande des valeurs"),
        "clamp" => ("keep x between low and high", "garde x entre bas et haut"),
        "abs" => ("value without its sign", "valeur sans son signe"),
        "round" => (
            "nearest whole number, or n decimals",
            "entier le plus proche, ou n décimales",
        ),
        "floor" => ("round down", "arrondi par défaut"),
        "ceil" => ("round up", "arrondi par excès"),
        "if" => (
            "a when the condition is true, else b",
            "a si la condition est vraie, sinon b",
        ),
        "interp" => (
            "lookup table, straight lines between points",
            "table de correspondance, droites entre les points",
        ),
        _ => ("", ""),
    };
    match lang {
        Lang::En => en,
        Lang::Fr => fr,
    }
}

/// The warning when the formula fails with the readings as they are now.
pub fn math_fails(lang: Lang, reason: &str) -> String {
    match lang {
        Lang::En => format!("Fails with the readings now: {reason}"),
        Lang::Fr => format!("Échoue avec les valeurs actuelles : {reason}"),
    }
}

/// Why the formula shows no preview: `name` has no value in the editor.
pub fn math_unknown(lang: Lang, name: &str) -> String {
    match lang {
        Lang::En => format!("No preview: {name} has no value here"),
        Lang::Fr => format!("Pas d'aperçu : {name} n'a pas de valeur ici"),
    }
}

/// The note under the rule list after a delete or an Otherwise toggle took wires with it:
/// "Removed 2 wires" / "2 fils supprimés".
pub fn removed_wires(lang: Lang, n: usize) -> String {
    match (lang, n) {
        (Lang::En, 1) => "Removed 1 wire".to_owned(),
        (Lang::En, n) => format!("Removed {n} wires"),
        (Lang::Fr, 1) => "1 fil supprimé".to_owned(),
        (Lang::Fr, n) => format!("{n} fils supprimés"),
    }
}

/// The `is_dark` card: the plain question without a shift, else where the dark starts:
/// "Is it dark? (from 30 min before civil dusk)" / "Fait-il nuit\u{A0}? (dès 30 min avant le
/// crépuscule civil)".
pub fn is_dark_sentence(lang: Lang, offset_minutes: i32) -> String {
    let n = offset_minutes.unsigned_abs();
    match (lang, offset_minutes) {
        (_, 0) => flow_text(lang, FlowText::IsDark).to_owned(),
        (Lang::En, o) if o < 0 => format!("Is it dark? (from {n}{NBSP}min before civil dusk)"),
        (Lang::En, _) => format!("Is it dark? (from {n}{NBSP}min after civil dusk)"),
        (Lang::Fr, o) if o < 0 => {
            format!("Fait-il nuit{NBSP}? (dès {n}{NBSP}min avant le crépuscule civil)")
        }
        (Lang::Fr, _) => {
            format!("Fait-il nuit{NBSP}? (dès {n}{NBSP}min après le crépuscule civil)")
        }
    }
}

/// The name of a sun event as a select option: "Sunrise" / "Lever du soleil".
pub fn sun_name(lang: Lang, which: SunWhich) -> &'static str {
    flow_text(
        lang,
        match which {
            SunWhich::Sunrise => FlowText::SunRise,
            SunWhich::Sunset => FlowText::SunSet,
            SunWhich::Dusk => FlowText::SunDusk,
            SunWhich::Dawn => FlowText::SunDawn,
        },
    )
}

/// "Delete “Name”?" / "Supprimer «\u{A0}Name\u{A0}»\u{A0}?".
pub fn delete_flow_prompt(lang: Lang, name: &str) -> String {
    pick(
        lang,
        format!("Delete “{name}”?"),
        format!("Supprimer «{NBSP}{name}{NBSP}»{NBSP}?"),
    )
}

/// "12 flows" / "12 automatisations": how many flows carry a label, in the label manager.
pub fn label_flow_count(lang: Lang, n: usize) -> String {
    super::format::count(
        lang,
        n,
        ("flow", "automatisation"),
        ("flows", "automatisations"),
    )
}

/// The warning under a label's delete prompt when label actions name it: "Used by the label
/// action of: A, B. Those actions will fail until fixed."
pub fn label_used_by(lang: Lang, flows: &[String]) -> String {
    let names = flows.join(", ");
    pick(
        lang,
        format!("Used by the label action of: {names}. Those actions will fail until fixed."),
        format!(
            "Utilisée par l'action d'étiquette de{NBSP}: {names}. Ces actions échoueront tant qu'elles ne sont pas corrigées."
        ),
    )
}

/// "Remove “Été”" / "Retirer «\u{A0}Été\u{A0}»": the ✕ of a label chip in the editor.
pub fn remove_label(lang: Lang, name: &str) -> String {
    pick(
        lang,
        format!("Remove “{name}”"),
        format!("Retirer «{NBSP}{name}{NBSP}»"),
    )
}

/// A label colour's name, for its swatch in the label manager.
pub fn label_color_word(lang: Lang, color: nidoo_core::flow::LabelColor) -> &'static str {
    use nidoo_core::flow::LabelColor as C;
    let (en, fr) = match color {
        C::Blue => ("Blue", "Bleu"),
        C::Amber => ("Amber", "Ambre"),
        C::Green => ("Green", "Vert"),
        C::Red => ("Red", "Rouge"),
        C::Purple => ("Purple", "Violet"),
        C::Teal => ("Teal", "Bleu canard"),
        C::Pink => ("Pink", "Rose"),
        C::Slate => ("Slate", "Ardoise"),
    };
    match lang {
        Lang::En => en,
        Lang::Fr => fr,
    }
}

/// "Couldn't save: <hub text>" / "Enregistrement impossible\u{A0}: <hub text>"; the hub's own
/// text comes as received.
pub fn save_failed(lang: Lang, hub: &str) -> String {
    pick(
        lang,
        format!("Couldn't save: {hub}"),
        format!("Enregistrement impossible{NBSP}: {hub}"),
    )
}

/// A span of seconds as `45 s`, `2 min 10 s` or `1 h 03 min`.
pub fn short_span(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}\u{a0}s")
    } else if secs < 3_600 {
        format!("{}\u{a0}min {:02}\u{a0}s", secs / 60, secs % 60)
    } else {
        format!(
            "{}\u{a0}h {:02}\u{a0}min",
            secs / 3_600,
            (secs % 3_600) / 60
        )
    }
}

/// "Invalid JSON: <parser text>" / "JSON invalide\u{A0}: <parser text>"; the parser's own text
/// comes as received.
pub fn invalid_json(lang: Lang, detail: &str) -> String {
    pick(
        lang,
        format!("Invalid JSON: {detail}"),
        format!("JSON invalide{NBSP}: {detail}"),
    )
}

/// "Next runs (Europe/Paris)" / "Prochains lancements (Europe/Paris)".
pub fn next_runs(lang: Lang, zone: &str) -> String {
    pick(
        lang,
        format!("Next runs ({zone})"),
        format!("Prochains lancements ({zone})"),
    )
}

/// "2:10 left" / "2:10 restant": the countdown of a parked delay.
pub fn time_left(lang: Lang, clock: &str) -> String {
    pick(lang, format!("{clock} left"), format!("{clock} restant"))
}

/// "Every minute" / "Toutes les minutes".
pub fn every_minute(lang: Lang) -> String {
    pick(lang, "Every minute".into(), "Toutes les minutes".into())
}

/// "Every 10 minutes" / "Toutes les 10 minutes".
pub fn every_minutes(lang: Lang, n: u8) -> String {
    pick(
        lang,
        format!("Every {n} minutes"),
        format!("Toutes les {n} minutes"),
    )
}

/// "Every hour at hh:15" / "Toutes les heures à hh:15".
pub fn every_hour_at(lang: Lang, minute: u8) -> String {
    pick(
        lang,
        format!("Every hour at hh:{minute:02}"),
        format!("Toutes les heures à hh:{minute:02}"),
    )
}

/// Which days a weekly schedule runs on, when that has a name of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DaysText {
    Weekdays,
    Weekends,
    EveryDay,
}

pub fn days_text(lang: Lang, d: DaysText) -> &'static str {
    let (en, fr) = match d {
        DaysText::Weekdays => ("Weekdays", "Du lundi au vendredi"),
        DaysText::Weekends => ("Weekends", "Le week-end"),
        DaysText::EveryDay => ("Every day", "Tous les jours"),
    };
    match lang {
        Lang::En => en,
        Lang::Fr => fr,
    }
}

/// "Weekdays at 07:00" / "Du lundi au vendredi à 07:00".
pub fn days_at(lang: Lang, who: &str, time: &str) -> String {
    pick(lang, format!("{who} at {time}"), format!("{who} à {time}"))
}

/// "Custom: 0 */2 1 * *" / "Personnalisé\u{A0}: 0 */2 1 * *".
pub fn custom_schedule(lang: Lang, cron: &str) -> String {
    pick(
        lang,
        format!("Custom: {cron}"),
        format!("Personnalisé{NBSP}: {cron}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Same word in both languages.
    const SAME: &[FlowText] = &[
        FlowText::Action,
        FlowText::ParamPosition,
        FlowText::ParamCode,
        FlowText::ColourOrange,
        FlowText::ColourCyan,
        FlowText::ColourMagenta,
        FlowText::DefaultNotifyTitle,
        FlowText::AskTextLabel,
        FlowText::Auto,
        FlowText::NodeSignal,
        FlowText::Minutes,
        FlowText::Zoom,
        FlowText::Mode,
        FlowText::StepOk,
        FlowText::SevInfo,
        FlowText::NodeAction,
    ];

    #[test]
    fn ask_sentences() {
        assert_eq!(
            ask_card(Lang::En, "Heat up?", Some("Kitchen"), &["Yes", "No"]),
            "Ask \"Heat up?\" in Kitchen: Yes / No / no answer"
        );
        assert_eq!(
            ask_card(Lang::Fr, "Chauffer ?", Some("Cuisine"), &["Oui", "Non"]),
            "Demande «\u{a0}Chauffer ?\u{a0}» dans Cuisine\u{a0}: Oui / Non / pas de réponse"
        );
        assert_eq!(
            ask_card(Lang::En, "Heat up?", None, &["Yes"]),
            "Ask \"Heat up?\" — All rooms: Yes / no answer"
        );
        assert_eq!(
            ask_card(Lang::Fr, "Q", None, &[]),
            "Demande «\u{a0}Q\u{a0}» — Toutes les pièces\u{a0}: pas de réponse"
        );
    }

    #[test]
    fn speak_sentences() {
        assert_eq!(
            speak(Lang::En, "Dinner", Some("Kitchen")),
            "Speak \"Dinner\" — Kitchen"
        );
        assert_eq!(
            speak(Lang::En, "Dinner", None),
            "Speak \"Dinner\" — All rooms"
        );
        assert_eq!(
            speak(Lang::Fr, "Dîner", None),
            "Annoncer «\u{a0}Dîner\u{a0}» — Toutes les pièces"
        );
        assert_eq!(speak(Lang::En, " ", None), "Speak — All rooms");
    }

    #[test]
    fn removed_wires_counts() {
        assert_eq!(removed_wires(Lang::En, 1), "Removed 1 wire");
        assert_eq!(removed_wires(Lang::En, 3), "Removed 3 wires");
        assert_eq!(removed_wires(Lang::Fr, 1), "1 fil supprimé");
        assert_eq!(removed_wires(Lang::Fr, 2), "2 fils supprimés");
    }

    #[test]
    fn switch_and_time_sentences() {
        assert_eq!(switch_on(Lang::En, "Lamp"), "Switch on Lamp");
        assert_eq!(switch_on(Lang::Fr, "Lampe"), "Commuter selon Lampe");
        assert_eq!(
            time_between(Lang::En, "07:00", "22:30"),
            "Between 07:00 and 22:30"
        );
        assert_eq!(
            time_between(Lang::Fr, "07:00", "22:30"),
            "Entre 07:00 et 22:30"
        );
    }

    fn switched(
        action: nidoo_core::flow::LabelAction,
        label: &str,
        on: usize,
        off: usize,
        skipped: &[(&str, &str)],
    ) -> nidoo_api::LabelSwitchResult {
        nidoo_api::LabelSwitchResult {
            label: label.into(),
            action,
            enabled: (0..on).map(|i| format!("on{i}")).collect(),
            disabled: (0..off).map(|i| format!("off{i}")).collect(),
            skipped: skipped
                .iter()
                .map(|(name, reason)| nidoo_api::SkippedFlow {
                    id: name.to_lowercase(),
                    name: (*name).into(),
                    reason: (*reason).into(),
                })
                .collect(),
        }
    }

    #[test]
    fn label_switch_result_sentences() {
        use nidoo_core::flow::{FlowLabelCatalog, LabelAction::*};
        let catalog: FlowLabelCatalog = serde_json::from_value(serde_json::json!({
            "labels": [
                {"id": "ete", "name": "Été", "choice": "saison"},
                {"id": "vacances", "name": "Vacances"}
            ],
            "choices": [{"id": "saison", "name": "Saison"}]
        }))
        .unwrap();
        let say = |lang, r| label_switch_result(lang, &r, &catalog);
        assert_eq!(
            say(Lang::Fr, switched(Pick, "ete", 5, 12, &[])),
            "Été choisi\u{a0}: 5 activées, 12 désactivées."
        );
        assert_eq!(
            say(Lang::En, switched(Pick, "ete", 5, 12, &[])),
            "Été picked: 5 on, 12 off."
        );
        assert_eq!(
            say(Lang::Fr, switched(Pick, "ete", 2, 0, &[])),
            "Été choisi\u{a0}: 2 activées, 0 désactivées."
        );
        assert_eq!(
            say(Lang::Fr, switched(Pick, "ete", 1, 1, &[])),
            "Été choisi\u{a0}: 1 activée, 1 désactivée."
        );
        assert_eq!(
            say(Lang::Fr, switched(Off, "vacances", 0, 3, &[])),
            "Vacances\u{a0}: 3 désactivées."
        );
        assert_eq!(
            say(Lang::En, switched(Off, "vacances", 0, 3, &[])),
            "Vacances: 3 off."
        );
        assert_eq!(
            say(Lang::Fr, switched(On, "vacances", 3, 0, &[])),
            "Vacances\u{a0}: 3 activées."
        );
        assert_eq!(
            say(Lang::Fr, switched(Pick, "ete", 0, 0, &[])),
            "Été choisi\u{a0}: rien à changer."
        );
        assert_eq!(
            say(Lang::En, switched(Pick, "ete", 0, 0, &[])),
            "Été picked: nothing to change."
        );
        assert_eq!(
            say(
                Lang::Fr,
                switched(Pick, "ete", 5, 12, &[("Nom", "invalid")])
            ),
            "Été choisi\u{a0}: 5 activées, 12 désactivées. 1 non modifiée\u{a0}: Nom (invalide)."
        );
        assert_eq!(
            say(
                Lang::En,
                switched(
                    Off,
                    "vacances",
                    0,
                    1,
                    &[("A", "invalid"), ("B", "publish: broker offline")]
                )
            ),
            "Vacances: 1 off. 2 not changed: A (invalid), B (publish: broker offline)."
        );
        // A label gone from the catalog reads as its id.
        assert_eq!(
            say(Lang::En, switched(On, "ghost", 1, 0, &[])),
            "ghost: 1 on."
        );
    }

    #[test]
    fn label_manager_sentences() {
        assert_eq!(label_flow_count(Lang::En, 1), "1 flow");
        assert_eq!(label_flow_count(Lang::En, 12), "12 flows");
        assert_eq!(label_flow_count(Lang::Fr, 12), "12 automatisations");
        let names = ["Summer switch".to_owned(), "Morning".to_owned()];
        assert_eq!(
            label_used_by(Lang::En, &names),
            "Used by the label action of: Summer switch, Morning. Those actions will fail until fixed."
        );
        assert!(
            label_used_by(Lang::Fr, &names)
                .starts_with("Utilisée par l'action d'étiquette de\u{a0}: Summer switch, Morning.")
        );
        assert_eq!(remove_label(Lang::En, "Été"), "Remove “Été”");
        assert_eq!(remove_label(Lang::Fr, "Été"), "Retirer «\u{a0}Été\u{a0}»");
        assert_eq!(
            label_color_word(Lang::Fr, nidoo_core::flow::LabelColor::Slate),
            "Ardoise"
        );
    }

    #[test]
    fn label_preview_sentences() {
        assert_eq!(
            label_preview(Lang::Fr, 5, "Été", 12, &["Hiver"]),
            "Active les 5 automatisations «\u{a0}Été\u{a0}», désactive les 12 «\u{a0}Hiver\u{a0}»."
        );
        assert_eq!(
            label_preview(Lang::En, 5, "Été", 12, &["Hiver"]),
            "Turns on the 5 \"Été\" flows, turns off the 12 \"Hiver\" flows."
        );
        assert_eq!(
            label_preview(Lang::Fr, 1, "Été", 1, &["Hiver", "Printemps"]),
            "Active l'automatisation «\u{a0}Été\u{a0}», désactive l'automatisation «\u{a0}Hiver\u{a0}» / «\u{a0}Printemps\u{a0}»."
        );
        assert_eq!(
            label_preview(Lang::En, 0, "Été", 0, &["Hiver"]),
            "No \"Été\" flow to turn on."
        );
        assert_eq!(
            label_preview(Lang::Fr, 0, "Été", 0, &[]),
            "Aucune automatisation «\u{a0}Été\u{a0}» à activer."
        );
        assert_eq!(
            label_preview(Lang::En, 1, "Été", 0, &[]),
            "Turns on the \"Été\" flow."
        );
    }

    #[test]
    fn every_word_has_both_languages() {
        assert_table(FlowText::ALL, flow_text, SAME);
    }

    #[test]
    fn french_questions_take_a_no_break_space() {
        assert_eq!(ask(Lang::En, "Lamp is on"), "Lamp is on?");
        assert_eq!(ask(Lang::Fr, "Lamp est allumée"), "Lamp est allumée\u{A0}?");
    }

    #[test]
    fn is_dark_sentences_name_the_shift() {
        assert_eq!(
            is_dark_sentence(Lang::En, 0),
            flow_text(Lang::En, FlowText::IsDark)
        );
        assert_eq!(
            is_dark_sentence(Lang::En, -30),
            "Is it dark? (from 30\u{A0}min before civil dusk)"
        );
        assert_eq!(
            is_dark_sentence(Lang::Fr, 15),
            "Fait-il nuit\u{A0}? (dès 15\u{A0}min après le crépuscule civil)"
        );
    }

    #[test]
    fn sun_sentences_in_both_languages() {
        assert_eq!(
            sun_sentence(Lang::Fr, SunWhich::Sunset, 0),
            "Au coucher du soleil"
        );
        assert_eq!(
            sun_sentence(Lang::Fr, SunWhich::Sunrise, 15),
            "15\u{A0}min après le lever du soleil"
        );
        assert_eq!(
            sun_sentence(Lang::Fr, SunWhich::Dawn, -15),
            "15\u{A0}min avant l'aube civile"
        );
        assert_eq!(
            sun_sentence(Lang::Fr, SunWhich::Dusk, 0),
            "Au crépuscule civil"
        );
    }
}
