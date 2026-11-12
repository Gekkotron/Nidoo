//! FR/EN text: Settings › Backup, the backup password, the FTP server and the backup job.

use super::*;

/// A fixed word or sentence of the Backup page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackupText {
    CouldNotLoad,
    Loading,
    PasswordTitle,
    PasswordNew,
    PasswordRepeat,
    PasswordRule,
    PasswordWarning,
    PasswordIsSet,
    Save,
    Saved,
    FtpTitle,
    Host,
    Port,
    User,
    FtpPassword,
    FtpPasswordKept,
    Folder,
    TestConnection,
    Testing,
    StepConnect,
    StepLogin,
    StepFolder,
    StepWrite,
    KeepTitle,
    Keep,
    NotifyFailure,
    BackUpNow,
    DownloadBackup,
    NeedPassword,
    NeedHost,
    Running,
    Cancel,
    PartSettings,
    PartRetained,
    PartHistory,
    PartZ2m,
    PartUpload,
    PartDownload,
    LastTitle,
    LastNone,
    LastDate,
    LastDestination,
    DestinationFtp,
    DestinationDownload,
    LastFile,
    LastSize,
    LastOutcome,
    OutcomeOk,
    OutcomeFailed,
    IncludeZ2m,
    IncludeZ2mHelp,
    OutcomePartial,
    Download,
    Delete,
    RestoredPassword,
    Confirm,
    WrongPassword,
    RestoredZ2mTitle,
    RestoredZ2mHint,
}

impl BackupText {
    pub const ALL: &'static [BackupText] = &[
        BackupText::CouldNotLoad,
        BackupText::Loading,
        BackupText::PasswordTitle,
        BackupText::PasswordNew,
        BackupText::PasswordRepeat,
        BackupText::PasswordRule,
        BackupText::PasswordWarning,
        BackupText::PasswordIsSet,
        BackupText::Save,
        BackupText::Saved,
        BackupText::FtpTitle,
        BackupText::Host,
        BackupText::Port,
        BackupText::User,
        BackupText::FtpPassword,
        BackupText::FtpPasswordKept,
        BackupText::Folder,
        BackupText::TestConnection,
        BackupText::Testing,
        BackupText::StepConnect,
        BackupText::StepLogin,
        BackupText::StepFolder,
        BackupText::StepWrite,
        BackupText::KeepTitle,
        BackupText::Keep,
        BackupText::NotifyFailure,
        BackupText::BackUpNow,
        BackupText::DownloadBackup,
        BackupText::NeedPassword,
        BackupText::NeedHost,
        BackupText::Running,
        BackupText::Cancel,
        BackupText::PartSettings,
        BackupText::PartRetained,
        BackupText::PartHistory,
        BackupText::PartZ2m,
        BackupText::PartUpload,
        BackupText::PartDownload,
        BackupText::LastTitle,
        BackupText::LastNone,
        BackupText::LastDate,
        BackupText::LastDestination,
        BackupText::DestinationFtp,
        BackupText::DestinationDownload,
        BackupText::LastFile,
        BackupText::LastSize,
        BackupText::LastOutcome,
        BackupText::OutcomeOk,
        BackupText::OutcomeFailed,
        BackupText::IncludeZ2m,
        BackupText::IncludeZ2mHelp,
        BackupText::OutcomePartial,
        BackupText::Download,
        BackupText::Delete,
        BackupText::RestoredPassword,
        BackupText::Confirm,
        BackupText::WrongPassword,
        BackupText::RestoredZ2mTitle,
        BackupText::RestoredZ2mHint,
    ];
}

/// A fixed word or sentence of the Backup page.
pub fn backup_text(lang: Lang, k: BackupText) -> &'static str {
    use BackupText::*;
    let (en, fr) = match k {
        CouldNotLoad => (
            "Could not load the backup settings",
            "Impossible de charger les réglages de sauvegarde",
        ),
        Loading => ("Loading…", "Chargement…"),
        PasswordTitle => ("Backup password", "Mot de passe de sauvegarde"),
        PasswordNew => ("Password", "Mot de passe"),
        PasswordRepeat => ("Repeat the password", "Répétez le mot de passe"),
        PasswordRule => (
            "At least 8 characters, typed the same twice.",
            "Au moins 8 caractères, saisis à l'identique deux fois.",
        ),
        PasswordWarning => (
            "If you lose this password, your backups cannot be opened.",
            "Si vous perdez ce mot de passe, vos sauvegardes ne pourront pas être ouvertes.",
        ),
        PasswordIsSet => ("Set", "Défini"),
        Save => ("Save", "Enregistrer"),
        Saved => ("Saved", "Enregistré"),
        FtpTitle => ("FTP server", "Serveur FTP"),
        Host => ("Host", "Hôte"),
        Port => ("Port", "Port"),
        User => ("User", "Utilisateur"),
        FtpPassword => ("Password", "Mot de passe"),
        FtpPasswordKept => ("Saved", "Enregistré"),
        Folder => ("Folder", "Dossier"),
        TestConnection => ("Test connection", "Tester la connexion"),
        Testing => ("Testing…", "Test en cours…"),
        StepConnect => ("Connect to the server", "Connexion au serveur"),
        StepLogin => ("Log in", "Identification"),
        StepFolder => ("Open the folder", "Ouverture du dossier"),
        StepWrite => ("Write a test file", "Écriture d'un fichier test"),
        KeepTitle => ("Retention", "Conservation"),
        Keep => ("Backups to keep", "Sauvegardes à conserver"),
        NotifyFailure => (
            "Notify me when a backup fails or is incomplete",
            "Me prévenir quand une sauvegarde échoue ou est incomplète",
        ),
        BackUpNow => ("Back up now", "Sauvegarder maintenant"),
        DownloadBackup => ("Download a backup", "Télécharger une sauvegarde"),
        NeedPassword => (
            "Set a backup password first.",
            "Définissez d'abord un mot de passe de sauvegarde.",
        ),
        NeedHost => (
            "Enter and save the FTP server to back up to it.",
            "Saisissez et enregistrez le serveur FTP pour y sauvegarder.",
        ),
        Running => ("Backing up", "Sauvegarde en cours"),
        Cancel => ("Cancel", "Annuler"),
        PartSettings => ("Settings", "Réglages"),
        PartRetained => ("Retained", "Configuration"),
        PartHistory => ("History", "Historique"),
        PartZ2m => ("Zigbee2MQTT", "Zigbee2MQTT"),
        PartUpload => ("Upload", "Envoi"),
        PartDownload => ("Download", "Téléchargement"),
        LastTitle => ("Last backup", "Dernière sauvegarde"),
        LastNone => ("No backup yet.", "Aucune sauvegarde pour l'instant."),
        LastDate => ("Date", "Date"),
        LastDestination => ("Destination", "Destination"),
        DestinationFtp => ("FTP server", "Serveur FTP"),
        DestinationDownload => ("Browser download", "Téléchargement navigateur"),
        LastFile => ("File", "Fichier"),
        LastSize => ("Size", "Taille"),
        LastOutcome => ("Result", "Résultat"),
        OutcomeOk => ("OK", "OK"),
        OutcomeFailed => ("Failed", "Échec"),
        IncludeZ2m => (
            "Include Zigbee2MQTT data",
            "Inclure les données Zigbee2MQTT",
        ),
        IncludeZ2mHelp => (
            "Keeps your Zigbee network (key, paired devices) so a new Zigbee2MQTT doesn't need re-pairing.",
            "Conserve votre réseau Zigbee (clé, appareils appairés) : un nouveau Zigbee2MQTT n'a pas besoin de tout ré-appairer.",
        ),
        OutcomePartial => ("Partial", "Partielle"),
        Download => ("Download", "Télécharger"),
        Delete => ("Delete", "Supprimer"),
        RestoredPassword => ("Backup password", "Mot de passe de la sauvegarde"),
        Confirm => ("Confirm", "Confirmer"),
        WrongPassword => ("Wrong password", "Mot de passe incorrect"),
        RestoredZ2mTitle => (
            "Zigbee2MQTT data from the restored backup",
            "Données Zigbee2MQTT de la sauvegarde restaurée",
        ),
        RestoredZ2mHint => (
            "It holds your Zigbee network key: delete it once Zigbee2MQTT is back.",
            "L'archive contient la clé de votre réseau Zigbee : supprimez-la une fois Zigbee2MQTT de retour.",
        ),
    };
    if lang == Lang::Fr { fr } else { en }
}

/// The last-backup line of a partial backup: it saved, without the Zigbee2MQTT data, for `reason`.
pub fn saved_without_z2m(lang: Lang, reason: &str) -> String {
    match lang {
        Lang::En => format!("Saved without Zigbee2MQTT data: {reason}"),
        Lang::Fr => format!("Sauvegardée sans les données Zigbee2MQTT : {reason}"),
    }
}

/// The text table for the current language.
pub fn use_backup_text() -> impl Fn(BackupText) -> &'static str + Copy + 'static {
    let lang = use_lang();
    move |k| backup_text(lang(), k)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Adding a variant without listing it in `ALL` fails to compile here.
    const fn listed(k: BackupText) -> bool {
        use BackupText::*;
        match k {
            CouldNotLoad | Loading | PasswordTitle | PasswordNew | PasswordRepeat
            | PasswordRule | PasswordWarning | PasswordIsSet | Save | Saved | FtpTitle | Host
            | Port | User | FtpPassword | FtpPasswordKept | Folder | TestConnection | Testing
            | StepConnect | StepLogin | StepFolder | StepWrite | KeepTitle | Keep
            | NotifyFailure | BackUpNow | DownloadBackup | NeedPassword | NeedHost | Running
            | Cancel | PartSettings | PartRetained | PartHistory | PartZ2m | PartUpload
            | PartDownload | LastTitle | LastNone | LastDate | LastDestination | DestinationFtp
            | DestinationDownload | LastFile | LastSize | LastOutcome | OutcomeOk
            | OutcomeFailed | IncludeZ2m | IncludeZ2mHelp | OutcomePartial | Download | Delete
            | RestoredZ2mTitle | RestoredZ2mHint | RestoredPassword | Confirm | WrongPassword => {
                true
            }
        }
    }

    #[test]
    fn every_backup_text_has_both_languages() {
        assert!(BackupText::ALL.iter().all(|&k| listed(k)));
        crate::i18n::assert_table(
            BackupText::ALL,
            backup_text,
            &[
                BackupText::Port,
                BackupText::PartZ2m,
                BackupText::LastDate,
                BackupText::LastDestination,
                BackupText::OutcomeOk,
            ],
        );
    }

    #[test]
    fn a_partial_backup_names_its_reason() {
        assert_eq!(
            saved_without_z2m(Lang::En, "Zigbee2MQTT is offline"),
            "Saved without Zigbee2MQTT data: Zigbee2MQTT is offline"
        );
        assert_eq!(
            saved_without_z2m(Lang::Fr, "x"),
            "Sauvegardée sans les données Zigbee2MQTT : x"
        );
    }

    #[test]
    fn password_warning_reads_the_same_in_each_language() {
        assert_eq!(
            backup_text(Lang::En, BackupText::PasswordWarning),
            "If you lose this password, your backups cannot be opened."
        );
        assert_eq!(
            backup_text(Lang::Fr, BackupText::PasswordWarning),
            "Si vous perdez ce mot de passe, vos sauvegardes ne pourront pas être ouvertes."
        );
    }
}
