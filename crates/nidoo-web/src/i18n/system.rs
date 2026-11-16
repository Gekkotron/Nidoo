//! FR/EN text: Settings › System, the hub's uptime, machine figures and services.

use super::*;

/// A fixed word or sentence of the system page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SystemText {
    Up,
    Down,
    Unknown,
    Since,
    UpOver30Days,
    Restart,
    Restarts,
    Cpu,
    Ram,
    Disk,
    Temperature,
    Devices,
    Entities,
    Retained,
    CommandsToday,
    Broker,
    Zigbee,
    Database,
    Hub,
    DatabaseSize,
    NotifyMe,
    NotifyHint,
    Loading,
    CouldNotLoad,
    UnitDay,
    UnitHour,
    UnitMinute,
    UnitGb,
    UnitMb,
    Back,
    Server,
    HubLine,
    RestartsHeading,
    NoRestarts,
    DownFor,
    StopClean,
    StopUnexpected,
    StopFirst,
    UnitSecond,
    SupervisorTitle,
    SupervisorHint,
    Supervise,
    SupervisorRecovered,
    Sensitivity,
    SensitivityLow,
    SensitivityNormal,
    SensitivityHigh,
    IgnoredSensors,
    AddSensor,
    Unignore,
    OpenProblems,
    NoProblems,
    Mute,
    NeedsDatabase,
    CheckStuck,
    CheckSilent,
    CheckImpossible,
    For,
}

/// A fixed word of the system page.
pub fn system_text(lang: Lang, k: SystemText) -> &'static str {
    use SystemText::*;
    let (en, fr) = match k {
        Up => ("Up", "En marche"),
        Down => ("Down", "Arrêté"),
        Unknown => ("unknown", "inconnu"),
        Since => ("since", "depuis"),
        UpOver30Days => ("up over 30 days", "de disponibilité sur 30 jours"),
        Restart => ("restart", "redémarrage"),
        Restarts => ("restarts", "redémarrages"),
        Cpu => ("CPU", "Processeur"),
        Ram => ("RAM", "Mémoire"),
        Disk => ("Disk", "Disque"),
        Temperature => ("Temperature", "Température"),
        Devices => ("Devices", "Appareils"),
        Entities => ("entities", "entités"),
        Retained => ("Retained", "Retenus"),
        CommandsToday => ("Commands today", "Commandes du jour"),
        Broker => ("Broker", "Broker MQTT"),
        Zigbee => ("Zigbee2MQTT", "Zigbee2MQTT"),
        Database => ("Database", "Base de données"),
        Hub => ("hub", "hub"),
        DatabaseSize => ("database", "base"),
        NotifyMe => ("Notify me", "Me prévenir"),
        NotifyHint => (
            "Send a push when the broker, Zigbee2MQTT or the database stays down, or the hub restarts unexpectedly.",
            "Envoie une alerte quand le broker, Zigbee2MQTT ou la base reste hors service, ou que le hub redémarre sans prévenir.",
        ),
        Loading => ("Loading…", "Chargement…"),
        CouldNotLoad => (
            "Could not load the system health",
            "Impossible de charger la santé du système",
        ),
        UnitDay => ("d", "j"),
        UnitHour => ("h", "h"),
        UnitMinute => ("min", "min"),
        UnitGb => ("GB", "Go"),
        UnitMb => ("MB", "Mo"),
        Back => ("Back", "Retour"),
        Server => ("Server", "Serveur"),
        HubLine => ("Hub", "Hub"),
        RestartsHeading => ("Restarts", "Redémarrages"),
        NoRestarts => ("No restart recorded", "Aucun redémarrage enregistré"),
        DownFor => ("down", "arrêt de"),
        StopClean => ("Clean stop", "Arrêt propre"),
        StopUnexpected => ("Unexpected stop", "Arrêt inattendu"),
        StopFirst => ("First start", "Premier démarrage"),
        UnitSecond => ("s", "s"),
        SupervisorTitle => ("Sensor supervisor", "Surveillance des capteurs"),
        SupervisorHint => (
            "Flags numeric sensors that look stuck, stop reporting or read an impossible value. Each sensor learns its own normal from 14 days of history.",
            "Repère les capteurs numériques qui semblent bloqués, ne donnent plus de nouvelles ou affichent une valeur impossible. Chaque capteur apprend son fonctionnement normal sur 14 jours d'historique.",
        ),
        Supervise => ("Supervise sensors", "Surveiller les capteurs"),
        SupervisorRecovered => (
            "Tell me when it is back to normal",
            "M'avertir du retour à la normale",
        ),
        Sensitivity => ("Sensitivity", "Sensibilité"),
        SensitivityLow => ("Low", "Faible"),
        SensitivityNormal => ("Normal", "Normale"),
        SensitivityHigh => ("High", "Élevée"),
        IgnoredSensors => ("Ignored sensors", "Capteurs ignorés"),
        AddSensor => ("Add", "Ajouter"),
        Unignore => ("Stop ignoring", "Ne plus ignorer"),
        OpenProblems => ("Open problems", "Problèmes en cours"),
        NoProblems => (
            "Every sensor looks fine",
            "Tous les capteurs semblent normaux",
        ),
        Mute => ("Mute", "Ignorer"),
        NeedsDatabase => ("Needs the database", "Nécessite la base de données"),
        CheckStuck => ("stuck", "bloquée"),
        CheckSilent => ("silent", "muette"),
        CheckImpossible => ("impossible value", "valeur impossible"),
        For => ("for", "depuis"),
    };
    if lang == Lang::Fr { fr } else { en }
}

/// The supervisor card's learning line: "Learning: 3 sensors have less than 7 days of history".
pub fn learning_line(lang: Lang, n: u32) -> String {
    match (lang, n) {
        (Lang::Fr, 1) => "Apprentissage : 1 capteur a moins de 7 jours d'historique".to_owned(),
        (Lang::Fr, n) => format!("Apprentissage : {n} capteurs ont moins de 7 jours d'historique"),
        (_, 1) => "Learning: 1 sensor has less than 7 days of history".to_owned(),
        (_, n) => format!("Learning: {n} sensors have less than 7 days of history"),
    }
}

/// The text table for the current language.
pub fn use_system_text() -> impl Fn(SystemText) -> &'static str + Copy + 'static {
    let lang = use_lang();
    move |k| system_text(lang(), k)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Adding a variant without listing it in `ALL` fails to compile here.
    const fn listed(k: SystemText) -> bool {
        use SystemText::*;
        match k {
            Up | Down | Unknown | Since | UpOver30Days | Restart | Restarts | Cpu | Ram | Disk
            | Temperature | Devices | Entities | Retained | CommandsToday | Broker | Zigbee
            | Database | Hub | DatabaseSize | NotifyMe | NotifyHint | Loading | CouldNotLoad
            | UnitDay | UnitHour | UnitMinute | UnitGb | UnitMb | Back | Server | HubLine
            | RestartsHeading | NoRestarts | DownFor | StopClean | StopUnexpected | StopFirst
            | UnitSecond | SupervisorTitle | SupervisorHint | Supervise | SupervisorRecovered
            | Sensitivity | SensitivityLow | SensitivityNormal | SensitivityHigh
            | IgnoredSensors | AddSensor | Unignore | OpenProblems | NoProblems | Mute
            | NeedsDatabase | CheckStuck | CheckSilent | CheckImpossible | For => true,
        }
    }

    const ALL: &[SystemText] = {
        use SystemText::*;
        &[
            Up,
            Down,
            Unknown,
            Since,
            UpOver30Days,
            Restart,
            Restarts,
            Cpu,
            Ram,
            Disk,
            Temperature,
            Devices,
            Entities,
            Retained,
            CommandsToday,
            Broker,
            Zigbee,
            Database,
            Hub,
            DatabaseSize,
            NotifyMe,
            NotifyHint,
            Loading,
            CouldNotLoad,
            UnitDay,
            UnitHour,
            UnitMinute,
            UnitGb,
            UnitMb,
            Back,
            Server,
            HubLine,
            RestartsHeading,
            NoRestarts,
            DownFor,
            StopClean,
            StopUnexpected,
            StopFirst,
            UnitSecond,
            SupervisorTitle,
            SupervisorHint,
            Supervise,
            SupervisorRecovered,
            Sensitivity,
            SensitivityLow,
            SensitivityNormal,
            SensitivityHigh,
            IgnoredSensors,
            AddSensor,
            Unignore,
            OpenProblems,
            NoProblems,
            Mute,
            NeedsDatabase,
            CheckStuck,
            CheckSilent,
            CheckImpossible,
            For,
        ]
    };

    #[test]
    fn system_copy_has_both_languages() {
        assert!(ALL.iter().all(|&k| listed(k)));
        crate::i18n::assert_table(
            ALL,
            system_text,
            &[
                SystemText::Zigbee,
                SystemText::Hub,
                SystemText::UnitHour,
                SystemText::UnitMinute,
                SystemText::HubLine,
                SystemText::UnitSecond,
            ],
        );
    }
}
