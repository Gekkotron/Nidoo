//! FR/EN text: Settings › Network presence, the hosts the hub watches by ping.

use super::*;

/// A fixed word or sentence of the network presence page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetworkText {
    Title,
    WatchedTitle,
    WatchedHint,
    Empty,
    Loading,
    CouldNotLoad,
    AddTitle,
    AddHint,
    Name,
    Host,
    Every,
    Room,
    NoRoom,
    Add,
    Edit,
    Save,
    Cancel,
    Remove,
    WatchOnNetwork,
    DeviceTag,
    EveryPrefix,
    Connected,
    DisconnectedFor,
    Paused,
    Waiting,
    CannotPing,
    CannotPingHint,
    Watched,
    CouldNotSave,
    UnitSecond,
    UnitMinute,
    UnitHour,
    UnitDay,
    PanelTitle,
    PanelHintHead,
    PanelHintTail,
    State,
    LastReply,
    LastReplyAgo,
    UnitMillisecond,
    NameSuffix,
}

/// A fixed word of the network presence page.
pub fn network_text(lang: Lang, k: NetworkText) -> &'static str {
    use NetworkText::*;
    let (en, fr) = match k {
        Title => ("Network presence", "Présence réseau"),
        WatchedTitle => ("Watched hosts", "Hôtes surveillés"),
        WatchedHint => (
            "Each host is pinged on its own interval and shows up as a “network” entity you can use in flows.",
            "Chaque hôte reçoit un ping à son propre rythme et devient une entité «\u{A0}réseau\u{A0}» utilisable dans les automatisations.",
        ),
        Empty => ("No host watched yet.", "Aucun hôte surveillé."),
        Loading => ("Loading…", "Chargement…"),
        CouldNotLoad => (
            "Could not load the watched hosts",
            "Impossible de charger les hôtes surveillés",
        ),
        AddTitle => ("Add host", "Ajouter un hôte"),
        AddHint => (
            "A device the hub already knows is switched on from its own sheet.",
            "Un appareil que le hub connaît déjà s'active depuis sa propre fiche.",
        ),
        Name => ("Name", "Nom"),
        Host => ("IP or hostname", "IP ou nom d'hôte"),
        Every => ("Ping every", "Ping toutes les"),
        Room => ("Room", "Pièce"),
        NoRoom => ("No room", "Aucune pièce"),
        Add => ("Add", "Ajouter"),
        Edit => ("Edit", "Modifier"),
        Save => ("Save", "Enregistrer"),
        Cancel => ("Cancel", "Annuler"),
        Remove => ("Remove", "Supprimer"),
        WatchOnNetwork => ("Watch on network", "Surveiller sur le réseau"),
        DeviceTag => ("device", "appareil"),
        EveryPrefix => ("every", "toutes les"),
        Connected => ("connected", "connecté"),
        DisconnectedFor => ("disconnected for", "déconnecté depuis"),
        Paused => ("paused", "en pause"),
        Waiting => (
            "waiting for the first reply",
            "en attente de la première réponse",
        ),
        CannotPing => (
            "the hub cannot send pings",
            "le hub ne peut pas envoyer de ping",
        ),
        CannotPingHint => (
            "The hub cannot send pings. On the server, run: sysctl net.ipv4.ping_group_range=\"0 2147483647\"",
            "Le hub ne peut pas envoyer de ping. Sur le serveur, lancez\u{A0}: sysctl net.ipv4.ping_group_range=\"0 2147483647\"",
        ),
        Watched => ("watched", "surveillés"),
        CouldNotSave => ("Could not save", "Enregistrement impossible"),
        UnitSecond => ("s", "s"),
        UnitMinute => ("min", "min"),
        UnitHour => ("h", "h"),
        UnitDay => ("d", "j"),
        PanelTitle => ("Network", "Réseau"),
        PanelHintHead => (
            "Disconnected after 3 missed pings in a row",
            "Déconnecté après 3 pings manqués d'affilée",
        ),
        PanelHintTail => (
            "Flows see it as “{}”.",
            "Les automatisations le voient comme «\u{A0}{}\u{A0}».",
        ),
        State => ("State", "État"),
        LastReply => ("Last reply", "Dernière réponse"),
        LastReplyAgo => ("{} ago", "il y a {}"),
        UnitMillisecond => ("ms", "ms"),
        NameSuffix => ("network", "réseau"),
    };
    if lang == Lang::Fr { fr } else { en }
}

/// The text table for the current language.
pub fn use_network_text() -> impl Fn(NetworkText) -> &'static str + Copy + 'static {
    let lang = use_lang();
    move |k| network_text(lang(), k)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Adding a variant without listing it in `ALL` fails to compile here.
    const fn listed(k: NetworkText) -> bool {
        use NetworkText::*;
        match k {
            Title | WatchedTitle | WatchedHint | Empty | Loading | CouldNotLoad | AddTitle
            | AddHint | Name | Host | Every | Room | NoRoom | Add | Edit | Save | Cancel
            | Remove | WatchOnNetwork | DeviceTag | EveryPrefix | Connected | DisconnectedFor
            | Paused | Waiting | CannotPing | CannotPingHint | Watched | CouldNotSave
            | UnitSecond | UnitMinute | UnitHour | UnitDay | PanelTitle | PanelHintHead
            | PanelHintTail | State | LastReply | LastReplyAgo | UnitMillisecond | NameSuffix => {
                true
            }
        }
    }

    const ALL: &[NetworkText] = {
        use NetworkText::*;
        &[
            Title,
            WatchedTitle,
            WatchedHint,
            Empty,
            Loading,
            CouldNotLoad,
            AddTitle,
            AddHint,
            Name,
            Host,
            Every,
            Room,
            NoRoom,
            Add,
            Edit,
            Save,
            Cancel,
            Remove,
            WatchOnNetwork,
            DeviceTag,
            EveryPrefix,
            Connected,
            DisconnectedFor,
            Paused,
            Waiting,
            CannotPing,
            CannotPingHint,
            Watched,
            CouldNotSave,
            UnitSecond,
            UnitMinute,
            UnitHour,
            UnitDay,
            PanelTitle,
            PanelHintHead,
            PanelHintTail,
            State,
            LastReply,
            LastReplyAgo,
            UnitMillisecond,
            NameSuffix,
        ]
    };

    #[test]
    fn network_copy_has_both_languages() {
        assert!(ALL.iter().all(|&k| listed(k)));
        crate::i18n::assert_table(
            ALL,
            network_text,
            &[
                NetworkText::UnitSecond,
                NetworkText::UnitMinute,
                NetworkText::UnitHour,
                NetworkText::UnitMillisecond,
            ],
        );
    }
}
