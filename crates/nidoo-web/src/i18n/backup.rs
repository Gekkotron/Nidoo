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
            "Notify me when a backup fails",
            "Me prévenir quand une sauvegarde échoue",
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
    };
    if lang == Lang::Fr { fr } else { en }
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
            | Cancel | PartSettings | PartRetained | PartHistory | PartUpload | PartDownload
            | LastTitle | LastNone | LastDate | LastDestination | DestinationFtp
            | DestinationDownload | LastFile | LastSize | LastOutcome | OutcomeOk
            | OutcomeFailed => true,
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
                BackupText::LastDate,
                BackupText::LastDestination,
                BackupText::OutcomeOk,
            ],
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
