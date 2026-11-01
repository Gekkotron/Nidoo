//! FR/EN text: the native alarm's Settings page, the Security hero's countdown and cause lines,
//! and the device sheet's "In alarm" line.

use super::*;

/// A fixed word or sentence of the alarm screens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlarmText {
    Title,
    Hint,
    Loading,
    CouldNotLoad,
    EnableLabel,
    EnableHint,
    SensorsTitle,
    SensorsHint,
    SensorsEmpty,
    AddSensor,
    Opening,
    Motion,
    ModeAway,
    ModeHome,
    ModeNight,
    Delayed,
    DelayedHint,
    Instant,
    Remove,
    SirensTitle,
    SirensHint,
    AddSiren,
    AddMqttSiren,
    MqttSiren,
    Topic,
    PayloadOn,
    PayloadOff,
    PayloadChirp,
    ChirpMs,
    TestChirp,
    TestChirpUnsaved,
    TestChirpSent,
    NoSirens,
    RemotesTitle,
    RemotesEmpty,
    AddRemote,
    Signal,
    AddSignal,
    ActionAway,
    ActionHome,
    ActionNight,
    ActionDisarm,
    ActionSos,
    PresenceTitle,
    Trackers,
    PresenceArm,
    PresenceDisarm,
    ArmAfter,
    TimingTitle,
    EntryDelay,
    ExitDelay,
    SirenDuration,
    MotionCount,
    MotionWindow,
    Save,
    Saved,
    CouldNotSave,
    LastRefused,
    Warnings,
    StatusOff,
    StateDisarmed,
    StateArming,
    StatePending,
    StateTriggered,
    StateArmed,
    InAlarm,
    RoleSiren,
    RoleRemote,
    RolePresence,
    ArmsIn,
    SirenIn,
    TriggeredBy,
    Sos,
}

/// A fixed word of the alarm screens.
pub fn alarm_text(lang: Lang, k: AlarmText) -> &'static str {
    use AlarmText::*;
    let (en, fr) = match k {
        Title => ("Alarm", "Alarme"),
        Hint => (
            "Sensors, sirens, remotes and presence",
            "Capteurs, sirènes, télécommandes et présence",
        ),
        Loading => ("Loading…", "Chargement…"),
        CouldNotLoad => ("Could not load the alarm", "Impossible de charger l'alarme"),
        EnableLabel => ("Enable the alarm", "Activer l'alarme"),
        EnableHint => (
            "Off: nothing is watched and the alarm panel is not shown.",
            "Désactivée : rien n'est surveillé et le panneau d'alarme est masqué.",
        ),
        SensorsTitle => ("Sensors", "Capteurs"),
        SensorsHint => (
            "Tick \"home\" on the outside doors, and set entry doors to delayed.",
            "Cochez «\u{A0}présent\u{A0}» sur les portes extérieures et réglez les portes d'entrée sur «\u{A0}temporisée\u{A0}».",
        ),
        SensorsEmpty => ("No sensor watched yet.", "Aucun capteur surveillé."),
        AddSensor => ("Add a sensor…", "Ajouter un capteur…"),
        Opening => ("opening", "ouverture"),
        Motion => ("motion", "mouvement"),
        ModeAway => ("away", "absent"),
        ModeHome => ("home", "présent"),
        ModeNight => ("night", "nuit"),
        Delayed => ("delayed", "temporisée"),
        DelayedHint => (
            "Waits the entry delay (Timing) before the sirens ring.",
            "Attend le délai d'entrée (Délais) avant de faire sonner les sirènes.",
        ),
        Instant => ("instant", "immédiate"),
        Remove => ("Remove", "Retirer"),
        SirensTitle => ("Sirens", "Sirènes"),
        SirensHint => (
            "Every siren rings on a trigger. A chirp sounds when the alarm arms.",
            "Toutes les sirènes sonnent au déclenchement. Un bip retentit à l'armement.",
        ),
        AddSiren => ("Add a siren…", "Ajouter une sirène…"),
        AddMqttSiren => ("Add an MQTT siren", "Ajouter une sirène MQTT"),
        MqttSiren => ("MQTT topic", "Topic MQTT"),
        Topic => ("Topic", "Topic"),
        PayloadOn => ("On payload", "Payload d'allumage"),
        PayloadOff => ("Off payload", "Payload d'arrêt"),
        PayloadChirp => ("Chirp payload (optional)", "Payload du bip (facultatif)"),
        ChirpMs => ("Chirp (ms, optional)", "Bip (ms, facultatif)"),
        TestChirp => ("Test chirp", "Tester le bip"),
        TestChirpUnsaved => (
            "Save the siren changes to test the chirp.",
            "Enregistre les changements de sirènes pour tester le bip.",
        ),
        TestChirpSent => ("Chirp sent", "Bip envoyé"),
        NoSirens => ("No siren yet.", "Aucune sirène."),
        RemotesTitle => ("Remotes", "Télécommandes"),
        RemotesEmpty => (
            "A remote shows up here after its first button press.",
            "Une télécommande apparaît ici après son premier appui.",
        ),
        AddRemote => ("Add a remote…", "Ajouter une télécommande…"),
        Signal => ("Signal", "Signal"),
        AddSignal => ("Add a signal", "Ajouter un signal"),
        ActionAway => ("away", "absent"),
        ActionHome => ("home", "présent"),
        ActionNight => ("night", "nuit"),
        ActionDisarm => ("disarm", "désarmer"),
        ActionSos => ("SOS", "SOS"),
        PresenceTitle => ("Presence", "Présence"),
        Trackers => ("Phones (trackers)", "Téléphones (trackers)"),
        PresenceArm => (
            "Arm when everyone is away",
            "Armer quand tout le monde est parti",
        ),
        PresenceDisarm => (
            "Disarm when someone comes home",
            "Désarmer quand quelqu'un rentre",
        ),
        ArmAfter => ("Arm after (s)", "Armer après (s)"),
        TimingTitle => ("Timing", "Délais"),
        EntryDelay => ("Entry delay (s)", "Délai d'entrée (s)"),
        ExitDelay => ("Exit delay (s)", "Délai de sortie (s)"),
        SirenDuration => ("Siren rings for (s)", "Durée de la sirène (s)"),
        MotionCount => ("Motions to trigger", "Mouvements pour déclencher"),
        MotionWindow => ("Within (s)", "En (s)"),
        Save => ("Save", "Enregistrer"),
        Saved => ("Saved", "Enregistré"),
        CouldNotSave => ("Could not save", "Enregistrement impossible"),
        LastRefused => (
            "The hub refused the last settings and kept the previous ones",
            "Le hub a refusé les derniers réglages et garde les précédents",
        ),
        Warnings => ("The house no longer has", "La maison n'a plus"),
        StatusOff => ("Off", "Désactivée"),
        StateDisarmed => ("Disarmed", "Désarmée"),
        StateArming => ("Arming", "Armement"),
        StatePending => ("Entry delay", "Délai d'entrée"),
        StateTriggered => ("Triggered", "Déclenchée"),
        StateArmed => ("Armed", "Armée"),
        InAlarm => ("In alarm: ", "Dans l'alarme : "),
        RoleSiren => ("siren", "sirène"),
        RoleRemote => ("remote", "télécommande"),
        RolePresence => ("presence tracker", "tracker de présence"),
        ArmsIn => ("Armed in", "Armée dans"),
        SirenIn => ("Siren in", "Sirène dans"),
        TriggeredBy => ("Triggered by", "Déclenchée par"),
        Sos => ("SOS", "SOS"),
    };
    if lang == Lang::Fr { fr } else { en }
}

/// The "delayed" entry button with the entry delay it waits: "delayed (30 s)".
pub fn delayed_with(lang: Lang, secs: u32) -> String {
    format!("{} ({secs}\u{A0}s)", alarm_text(lang, AlarmText::Delayed))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Adding a variant without listing it in `ALL` fails to compile here.
    const fn listed(k: AlarmText) -> bool {
        use AlarmText::*;
        match k {
            Title | Hint | Loading | CouldNotLoad | EnableLabel | EnableHint | SensorsTitle
            | SensorsHint | SensorsEmpty | AddSensor | Opening | Motion | ModeAway | ModeHome
            | ModeNight | Delayed | DelayedHint | Instant | Remove | SirensTitle | SirensHint
            | AddSiren | AddMqttSiren | MqttSiren | Topic | PayloadOn | PayloadOff
            | PayloadChirp | ChirpMs | TestChirp | TestChirpUnsaved | TestChirpSent | NoSirens
            | RemotesTitle | RemotesEmpty | AddRemote | Signal | AddSignal | ActionAway
            | ActionHome | ActionNight | ActionDisarm | ActionSos | PresenceTitle | Trackers
            | PresenceArm | PresenceDisarm | ArmAfter | TimingTitle | EntryDelay | ExitDelay
            | SirenDuration | MotionCount | MotionWindow | Save | Saved | CouldNotSave
            | LastRefused | Warnings | StatusOff | StateDisarmed | StateArming | StatePending
            | StateTriggered | StateArmed | InAlarm | RoleSiren | RoleRemote | RolePresence
            | ArmsIn | SirenIn | TriggeredBy | Sos => true,
        }
    }

    const ALL: &[AlarmText] = {
        use AlarmText::*;
        &[
            Title,
            Hint,
            Loading,
            CouldNotLoad,
            EnableLabel,
            EnableHint,
            SensorsTitle,
            SensorsHint,
            SensorsEmpty,
            AddSensor,
            Opening,
            Motion,
            ModeAway,
            ModeHome,
            ModeNight,
            Delayed,
            DelayedHint,
            Instant,
            Remove,
            SirensTitle,
            SirensHint,
            AddSiren,
            AddMqttSiren,
            MqttSiren,
            Topic,
            PayloadOn,
            PayloadOff,
            PayloadChirp,
            ChirpMs,
            TestChirp,
            TestChirpUnsaved,
            TestChirpSent,
            NoSirens,
            RemotesTitle,
            RemotesEmpty,
            AddRemote,
            Signal,
            AddSignal,
            ActionAway,
            ActionHome,
            ActionNight,
            ActionDisarm,
            ActionSos,
            PresenceTitle,
            Trackers,
            PresenceArm,
            PresenceDisarm,
            ArmAfter,
            TimingTitle,
            EntryDelay,
            ExitDelay,
            SirenDuration,
            MotionCount,
            MotionWindow,
            Save,
            Saved,
            CouldNotSave,
            LastRefused,
            Warnings,
            StatusOff,
            StateDisarmed,
            StateArming,
            StatePending,
            StateTriggered,
            StateArmed,
            InAlarm,
            RoleSiren,
            RoleRemote,
            RolePresence,
            ArmsIn,
            SirenIn,
            TriggeredBy,
            Sos,
        ]
    };

    #[test]
    fn alarm_copy_has_both_languages() {
        assert!(ALL.iter().all(|&k| listed(k)));
        crate::i18n::assert_table(
            ALL,
            alarm_text,
            &[
                AlarmText::Topic,
                AlarmText::Signal,
                AlarmText::ActionSos,
                AlarmText::Sos,
            ],
        );
    }

    #[test]
    fn delayed_button_names_the_entry_delay() {
        assert_eq!(delayed_with(Lang::En, 30), "delayed (30\u{A0}s)");
        assert_eq!(delayed_with(Lang::Fr, 45), "temporisée (45\u{A0}s)");
    }
}
