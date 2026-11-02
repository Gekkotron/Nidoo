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
    };
    if lang == Lang::Fr { fr } else { en }
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
            | UnitSecond => true,
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
