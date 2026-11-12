//! The Settings → Backup page: the backup password, the FTP server, how many backups to keep,
//! back up now or download a backup, and the last backup. Neither password is ever shown: the
//! hub says only whether one is stored.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use leptos::prelude::*;
use leptos::task::spawn_local;
use nidoo_api::{
    BackupDestination, BackupInput, BackupPart, BackupProgress, BackupState, BackupView, FtpCheck,
    FtpInput, FtpStep, LastBackup, Secret,
};

use crate::api::backup::{self as api, DOWNLOAD_PATH};
use crate::i18n::format::{date_time, local_date_time};
use crate::i18n::{BackupText, backup_text, use_backup_text, use_lang};
use crate::screens::alarm_settings::SwitchRow;
use crate::screens::system_settings::size;

/// How often the running backup is read again.
const POLL_MS: u32 = 1000;

/// The fewest characters a backup password may have.
const MIN_PASSWORD: usize = 8;

/// The fewest backups the hub keeps.
const MIN_KEEP: u32 = 1;

/// Whether `password` and `repeat` may be saved as the backup password.
fn password_ok(password: &str, repeat: &str) -> bool {
    password == repeat && password.chars().count() >= MIN_PASSWORD
}

/// A port field: `Some(None)` when empty (the hub's default), `Some(Some(n))` for a port, `None`
/// when it is not one.
fn parse_port(text: &str) -> Option<Option<u16>> {
    let text = text.trim();
    if text.is_empty() {
        return Some(None);
    }
    text.parse::<u16>().ok().filter(|p| *p > 0).map(Some)
}

/// A keep field: a whole number of backups, at least [`MIN_KEEP`].
fn parse_keep(text: &str) -> Option<u32> {
    text.trim().parse::<u32>().ok().filter(|k| *k >= MIN_KEEP)
}

/// The stored section as a `PUT` that changes nothing: both passwords kept.
fn base_input(view: &BackupView) -> BackupInput {
    BackupInput {
        password: Secret::Keep,
        ftp: FtpInput {
            host: view.ftp.host.clone(),
            port: Some(view.ftp.port),
            user: view.ftp.user.clone(),
            password: Secret::Keep,
            folder: Some(view.ftp.folder.clone()),
        },
        keep: Some(view.keep),
        notify_failure: view.notify_failure,
    }
}

/// A password field that is left empty keeps the stored one.
fn typed_secret(text: &str) -> Secret {
    if text.is_empty() {
        Secret::Keep
    } else {
        Secret::Set(text.to_owned())
    }
}

pub(crate) fn part_text(part: BackupPart) -> BackupText {
    match part {
        BackupPart::Settings => BackupText::PartSettings,
        BackupPart::Retained => BackupText::PartRetained,
        BackupPart::History => BackupText::PartHistory,
        BackupPart::Upload => BackupText::PartUpload,
        BackupPart::Download => BackupText::PartDownload,
    }
}

fn step_text(step: FtpStep) -> BackupText {
    match step {
        FtpStep::Connect => BackupText::StepConnect,
        FtpStep::Login => BackupText::StepLogin,
        FtpStep::Folder => BackupText::StepFolder,
        FtpStep::Write => BackupText::StepWrite,
    }
}

/// A labelled text field bound to `value`.
#[component]
fn TextField(
    id: &'static str,
    label: BackupText,
    value: RwSignal<String>,
    #[prop(default = "text")] kind: &'static str,
    #[prop(default = "off")] autocomplete: &'static str,
    #[prop(into, optional)] placeholder: Signal<String>,
) -> impl IntoView {
    let lang = use_lang();
    view! {
        <label class="field" for=id>
            <span class="field__caption">{move || backup_text(lang(), label)}</span>
            <input
                id=id
                class="field__input"
                type=kind
                autocomplete=autocomplete
                placeholder=move || placeholder.get()
                prop:value=move || value.get()
                on:input=move |ev| value.set(event_target_value(&ev))
            />
        </label>
    }
}

/// Settings → Backup.
#[component]
pub fn BackupSettingsPage() -> impl IntoView {
    let t = use_backup_text();
    let lang = use_lang();

    let stored = RwSignal::new(None::<BackupView>);
    let failed = RwSignal::new(None::<String>);
    let password = RwSignal::new(String::new());
    let repeat = RwSignal::new(String::new());
    let host = RwSignal::new(String::new());
    let port = RwSignal::new(String::new());
    let user = RwSignal::new(String::new());
    let ftp_password = RwSignal::new(String::new());
    let folder = RwSignal::new(String::new());
    let keep = RwSignal::new(String::new());
    let notify = RwSignal::new(true);
    let saving = RwSignal::new(false);
    let save_error = RwSignal::new(None::<String>);
    let checks = RwSignal::new(None::<Vec<FtpCheck>>);
    let testing = RwSignal::new(false);
    let progress = RwSignal::new(None::<BackupProgress>);
    let last = RwSignal::new(None::<LastBackup>);
    let run_error = RwSignal::new(None::<String>);

    // The form shows what the hub stored; typed passwords are cleared once they are sent.
    let show = move |v: BackupView| {
        host.try_set(v.ftp.host.clone());
        port.try_set(v.ftp.port.to_string());
        user.try_set(v.ftp.user.clone());
        folder.try_set(v.ftp.folder.clone());
        keep.try_set(v.keep.to_string());
        notify.try_set(v.notify_failure);
        password.try_set(String::new());
        repeat.try_set(String::new());
        ftp_password.try_set(String::new());
        stored.try_set(Some(v));
    };

    spawn_local(async move {
        match api::get_backup_settings().await {
            Ok(v) => show(v),
            Err(e) => {
                failed.try_set(Some(e.error));
            }
        }
    });

    let reload_last = move || {
        spawn_local(async move {
            if let Ok(l) = api::last_backup().await {
                last.try_set(l);
            }
        });
    };
    reload_last();

    // Reads the running backup every second until it ends, then the record it left.
    let stop = Arc::new(AtomicBool::new(false));
    let following = Arc::new(AtomicBool::new(false));
    let follow = {
        let stop = stop.clone();
        Callback::new(move |()| {
            if following.swap(true, Ordering::SeqCst) {
                return;
            }
            let stop = stop.clone();
            let following = following.clone();
            spawn_local(async move {
                loop {
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                    match api::backup_current().await {
                        Ok(Some(p)) if p.state == BackupState::Running => {
                            progress.try_set(Some(p));
                            gloo_timers::future::TimeoutFuture::new(POLL_MS).await;
                        }
                        _ => {
                            progress.try_set(None);
                            break;
                        }
                    }
                }
                following.store(false, Ordering::SeqCst);
                if !stop.load(Ordering::Relaxed) {
                    reload_last();
                }
            });
        })
    };
    follow.run(());
    on_cleanup(move || stop.store(true, Ordering::Relaxed));

    // Sends the stored section with `change` applied, and shows what the hub answers.
    let save = move |change: Box<dyn FnOnce(&mut BackupInput)>| {
        let Some(mut input) = stored.with_untracked(|v| v.as_ref().map(base_input)) else {
            return;
        };
        if saving.get_untracked() {
            return;
        }
        change(&mut input);
        saving.set(true);
        save_error.set(None);
        spawn_local(async move {
            match api::put_backup_settings(&input).await {
                Ok(v) => show(v),
                Err(e) => {
                    save_error.try_set(Some(e.error));
                }
            }
            saving.try_set(false);
        });
    };

    let loaded = move || stored.with(Option::is_some);
    let password_set = move || stored.with(|v| v.as_ref().is_some_and(|v| v.password.set));
    let host_set = move || stored.with(|v| v.as_ref().is_some_and(|v| !v.ftp.host.is_empty()));
    let running = move || progress.with(Option::is_some);

    let save_password = move |_| {
        let typed = password.get_untracked();
        if password_ok(&typed, &repeat.get_untracked()) {
            save(Box::new(move |i| i.password = Secret::Set(typed)));
        }
    };

    let ftp_form = move || -> Option<FtpInput> {
        Some(FtpInput {
            host: host.get_untracked().trim().to_owned(),
            port: parse_port(&port.get_untracked())?,
            user: user.get_untracked(),
            password: typed_secret(&ftp_password.get_untracked()),
            folder: Some(folder.get_untracked()),
        })
    };
    let ftp_valid = move || parse_port(&port.get()).is_some();
    let save_ftp = move |_| {
        if let Some(ftp) = ftp_form() {
            save(Box::new(move |i| i.ftp = ftp));
        }
    };
    let test_connection = move |_| {
        let Some(ftp) = ftp_form() else {
            return;
        };
        if testing.get_untracked() {
            return;
        }
        testing.set(true);
        save_error.set(None);
        spawn_local(async move {
            match api::test_ftp(Some(&ftp)).await {
                Ok(steps) => {
                    checks.try_set(Some(steps));
                }
                Err(e) => {
                    checks.try_set(None);
                    save_error.try_set(Some(e.error));
                }
            }
            testing.try_set(false);
        });
    };

    let keep_valid = move || parse_keep(&keep.get()).is_some();
    let save_keep = move |_| {
        if let Some(count) = parse_keep(&keep.get_untracked()) {
            let notify_failure = notify.get_untracked();
            save(Box::new(move |i| {
                i.keep = Some(count);
                i.notify_failure = notify_failure;
            }));
        }
    };

    let back_up_now = move |_| {
        run_error.set(None);
        spawn_local(async move {
            match api::start_backup().await {
                Ok(p) => {
                    progress.try_set(Some(p));
                    follow.run(());
                }
                Err(e) => {
                    run_error.try_set(Some(e.error));
                }
            }
        });
    };
    let cancel = move |_| {
        spawn_local(async move {
            if let Err(e) = api::cancel_backup().await {
                run_error.try_set(Some(e.error));
            }
        });
    };

    let run_blocked = move || !password_set() || !host_set() || running();
    let download_blocked = move || !password_set() || running();

    view! {
        <div class="backup">
            {move || {
                failed
                    .get()
                    .map(|e| {
                        view! {
                            <p class="home-settings__error">
                                {format!("{}: {e}", t(BackupText::CouldNotLoad))}
                            </p>
                        }
                    })
            }}
            <section class="sheet__panel backup__password">
                <h2 class="sheet__heading">{move || t(BackupText::PasswordTitle)}</h2>
                <Show when=password_set>
                    <p class="backup__set">{move || t(BackupText::PasswordIsSet)}</p>
                </Show>
                <TextField
                    id="backup-password"
                    label=BackupText::PasswordNew
                    value=password
                    kind="password"
                    autocomplete="new-password"
                />
                <TextField
                    id="backup-password-repeat"
                    label=BackupText::PasswordRepeat
                    value=repeat
                    kind="password"
                    autocomplete="new-password"
                />
                <p class="system__hint">{move || t(BackupText::PasswordRule)}</p>
                <p class="backup__warning">{move || t(BackupText::PasswordWarning)}</p>
                <button
                    type="button"
                    class="btn btn--primary backup__password-save"
                    disabled=move || {
                        !loaded() || saving.get() || !password_ok(&password.get(), &repeat.get())
                    }
                    on:click=save_password
                >
                    {move || t(BackupText::Save)}
                </button>
            </section>
            <section class="sheet__panel backup__ftp">
                <h2 class="sheet__heading">{move || t(BackupText::FtpTitle)}</h2>
                <TextField id="backup-ftp-host" label=BackupText::Host value=host />
                <TextField id="backup-ftp-port" label=BackupText::Port value=port kind="number" />
                <TextField id="backup-ftp-user" label=BackupText::User value=user />
                <TextField
                    id="backup-ftp-password"
                    label=BackupText::FtpPassword
                    value=ftp_password
                    kind="password"
                    autocomplete="new-password"
                    placeholder=Signal::derive(move || {
                        if stored.with(|v| v.as_ref().is_some_and(|v| v.ftp.password.set)) {
                            t(BackupText::FtpPasswordKept).to_owned()
                        } else {
                            String::new()
                        }
                    })
                />
                <TextField id="backup-ftp-folder" label=BackupText::Folder value=folder />
                <div class="backup__buttons">
                    <button
                        type="button"
                        class="btn backup__test"
                        disabled=move || !loaded() || testing.get() || !ftp_valid()
                        on:click=test_connection
                    >
                        {move || t(if testing.get() { BackupText::Testing } else { BackupText::TestConnection })}
                    </button>
                    <button
                        type="button"
                        class="btn btn--primary backup__ftp-save"
                        disabled=move || !loaded() || saving.get() || !ftp_valid()
                        on:click=save_ftp
                    >
                        {move || t(BackupText::Save)}
                    </button>
                </div>
                {move || {
                    checks
                        .get()
                        .map(|steps| {
                            view! {
                                <ul class="backup__checks">
                                    {steps
                                        .into_iter()
                                        .map(|c| {
                                            let ok = c.ok;
                                            view! {
                                                <li
                                                    class="backup__check"
                                                    class:backup__check--ok=ok
                                                    class:backup__check--bad=!ok
                                                >
                                                    <span class="backup__mark" aria-hidden="true">
                                                        {if ok { "✓" } else { "✗" }}
                                                    </span>
                                                    <span class="backup__step">
                                                        {move || t(step_text(c.step))}
                                                    </span>
                                                    {c
                                                        .detail
                                                        .map(|d| {
                                                            view! { <span class="backup__detail">{d}</span> }
                                                        })}
                                                </li>
                                            }
                                        })
                                        .collect_view()}
                                </ul>
                            }
                        })
                }}
            </section>
            <section class="sheet__panel backup__keep">
                <h2 class="sheet__heading">{move || t(BackupText::KeepTitle)}</h2>
                <TextField id="backup-keep" label=BackupText::Keep value=keep kind="number" />
                <SwitchRow
                    label=Signal::derive(move || t(BackupText::NotifyFailure).to_owned())
                    on=notify
                    on_toggle=Callback::new(move |on: bool| notify.set(on))
                />
                <button
                    type="button"
                    class="btn btn--primary backup__keep-save"
                    disabled=move || !loaded() || saving.get() || !keep_valid()
                    on:click=save_keep
                >
                    {move || t(BackupText::Save)}
                </button>
            </section>
            {move || {
                save_error
                    .get()
                    .map(|e| view! { <p class="home-settings__error" role="alert">{e}</p> })
            }}
            <section class="sheet__panel backup__actions">
                <div class="backup__buttons">
                    <button
                        type="button"
                        class="btn btn--primary backup__run"
                        disabled=run_blocked
                        on:click=back_up_now
                    >
                        {move || t(BackupText::BackUpNow)}
                    </button>
                    <a
                        class="btn backup__download"
                        href=move || (!download_blocked()).then_some(DOWNLOAD_PATH)
                        download=""
                        aria-disabled=move || download_blocked().to_string()
                    >
                        {move || t(BackupText::DownloadBackup)}
                    </a>
                </div>
                <Show when=move || loaded() && !password_set()>
                    <p class="system__hint">{move || t(BackupText::NeedPassword)}</p>
                </Show>
                <Show when=move || loaded() && password_set() && !host_set()>
                    <p class="system__hint">{move || t(BackupText::NeedHost)}</p>
                </Show>
                {move || {
                    progress
                        .get()
                        .map(|p| {
                            view! {
                                <div class="backup__progress" role="status">
                                    <span>
                                        {format!(
                                            "{} · {} · {}",
                                            t(BackupText::Running),
                                            t(part_text(p.part)),
                                            size(lang(), p.bytes),
                                        )}
                                    </span>
                                    <button type="button" class="btn backup__cancel" on:click=cancel>
                                        {move || t(BackupText::Cancel)}
                                    </button>
                                </div>
                            }
                        })
                }}
                {move || {
                    run_error
                        .get()
                        .map(|e| view! { <p class="home-settings__error" role="alert">{e}</p> })
                }}
            </section>
            <section class="sheet__panel backup__last">
                <h2 class="sheet__heading">{move || t(BackupText::LastTitle)}</h2>
                {move || match last.get() {
                    None => view! { <p class="system__hint">{move || t(BackupText::LastNone)}</p> }.into_any(),
                    Some(l) => last_rows(&l, lang()).into_any(),
                }}
            </section>
        </div>
    }
}

/// The last backup's rows: date, destination, file, size and result.
fn last_rows(l: &LastBackup, lang: crate::i18n::Lang) -> impl IntoView + use<> {
    let when = date_time(lang, local_date_time(l.ended_at.max(0) as u64));
    let destination = backup_text(
        lang,
        match l.destination {
            BackupDestination::Ftp => BackupText::DestinationFtp,
            BackupDestination::Download => BackupText::DestinationDownload,
        },
    );
    let outcome = backup_text(
        lang,
        if l.ok {
            BackupText::OutcomeOk
        } else {
            BackupText::OutcomeFailed
        },
    );
    let row = |label: BackupText, value: String, class: &'static str| {
        view! {
            <div class="backup__row">
                <span class="backup__label">{backup_text(lang, label)}</span>
                <span class=class>{value}</span>
            </div>
        }
    };
    view! {
        {row(BackupText::LastDate, when, "backup__last-date")}
        {row(BackupText::LastDestination, destination.to_owned(), "backup__last-destination")}
        {row(BackupText::LastFile, l.file.clone(), "backup__last-file")}
        {row(BackupText::LastSize, size(lang, l.size), "backup__last-size")}
        {row(BackupText::LastOutcome, outcome.to_owned(), "backup__last-outcome")}
        {l
            .error
            .clone()
            .map(|e| view! { <p class="home-settings__error">{e}</p> })}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nidoo_api::FtpView;
    use nidoo_api::SecretView;

    #[test]
    fn a_password_needs_two_equal_entries_of_eight_characters() {
        assert!(!password_ok("", ""));
        assert!(!password_ok("short", "short"));
        assert!(!password_ok("long-enough", "long-enough!"));
        assert!(password_ok("exactly8", "exactly8"));
        // Eight characters, not eight bytes.
        assert!(password_ok("éééééééé", "éééééééé"));
        assert!(!password_ok("ééé", "ééé"));
    }

    #[test]
    fn ports_and_keep_counts_parse() {
        assert_eq!(parse_port(""), Some(None));
        assert_eq!(parse_port(" 2121 "), Some(Some(2121)));
        assert_eq!(parse_port("0"), None);
        assert_eq!(parse_port("70000"), None);
        assert_eq!(parse_port("ftp"), None);
        assert_eq!(parse_keep("7"), Some(7));
        assert_eq!(parse_keep("0"), None);
        assert_eq!(parse_keep("-1"), None);
        assert_eq!(parse_keep(""), None);
    }

    #[test]
    fn the_base_input_changes_nothing() {
        let view = BackupView {
            password: SecretView { set: true },
            ftp: FtpView {
                host: "nas.local".into(),
                port: 21,
                user: "nidoo".into(),
                password: SecretView { set: true },
                folder: "/backups".into(),
            },
            keep: 7,
            notify_failure: false,
        };
        let input = base_input(&view);
        assert_eq!(input.password, Secret::Keep);
        assert_eq!(input.ftp.password, Secret::Keep);
        assert_eq!(input.ftp.host, "nas.local");
        assert_eq!(input.ftp.port, Some(21));
        assert_eq!(input.ftp.folder.as_deref(), Some("/backups"));
        assert_eq!(input.keep, Some(7));
        assert!(!input.notify_failure);
        assert_eq!(typed_secret(""), Secret::Keep);
        assert_eq!(typed_secret("x"), Secret::Set("x".into()));
    }
}
