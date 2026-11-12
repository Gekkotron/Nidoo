//! The Restore step: on a fresh machine, pick a backup on an FTP server or upload one, check its
//! password, look at what it holds and restore it. The hub restarts into the running house when
//! the restore ends. Needs [`WizardNav`], [`Applying`] and [`SetupState`] in context.

use gloo_timers::future::TimeoutFuture;
use leptos::prelude::*;
use nidoo_api::{
    BackupState, BackupSummary, FtpSource, RemoteBackup, RestoreFromFtp, RestoreProgress, SetupStep,
};

use super::{Applying, setup_href};
use crate::api::setup::{
    FtpError, SaveError, restore_ftp, restore_ftp_inspect, restore_ftp_list, restore_pending,
    restore_progress, restore_upload, restore_upload_inspect,
};
use crate::i18n::format::{date_time, local_date_time};
use crate::i18n::setup::{SetupText, setup_text, use_setup_text};
use crate::i18n::{BackupText, Lang, backup_text, current_lang, use_lang};
use crate::screens::backup_settings::part_text;
use crate::screens::system_settings::size;

/// How often the page asks the hub how the restore is going, in milliseconds. A test provides a
/// shorter one.
#[derive(Clone, Copy)]
pub struct RestorePollMs(pub u32);

const DEFAULT_POLL_MS: u32 = 1_000;

/// This many progress polls in a row without an answer mean the hub is restarting.
const LOST_AFTER: u32 = 5;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    Source,
    /// FTP only: the backups the server holds.
    Pick,
    Password,
    Summary,
    Running,
    Failed,
}

/// Where the backup comes from.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Via {
    Ftp,
    Upload,
}

/// What went wrong: a sentence of ours (in the language in use) or the hub's own words.
#[derive(Clone, Debug, PartialEq)]
enum Problem {
    Known(SetupText),
    Hub(String),
}

fn problem_text(lang: Lang, p: &Problem) -> String {
    match p {
        Problem::Known(k) => setup_text(lang, *k).to_owned(),
        Problem::Hub(m) => m.clone(),
    }
}

fn ftp_problem(e: FtpError) -> Problem {
    match e {
        FtpError::Unreachable => Problem::Known(SetupText::RestoreFtpUnreachable),
        FtpError::Api(e) => Problem::Hub(e.error),
    }
}

/// The FTP fields as the hub's source. A blank port is left out (the hub uses 21), and so is a
/// blank folder.
fn ftp_source(
    host: &str,
    port: &str,
    user: &str,
    password: &str,
    folder: &str,
) -> Result<FtpSource, SetupText> {
    let host = host.trim();
    if host.is_empty() {
        return Err(SetupText::FieldRequired);
    }
    let port = match port.trim() {
        "" => None,
        text => match text.parse::<u16>() {
            Ok(p) if p >= 1 => Some(p),
            _ => return Err(SetupText::PortInvalid),
        },
    };
    let folder = folder.trim();
    Ok(FtpSource {
        host: host.to_owned(),
        port,
        user: user.trim().to_owned(),
        password: password.to_owned(),
        folder: (!folder.is_empty()).then(|| folder.to_owned()),
    })
}

/// A backup's date as the browser's local time.
fn when(lang: Lang, unix: i64) -> String {
    date_time(lang, local_date_time(unix.max(0) as u64))
}

/// A summary's RFC 3339 creation time as local time; the raw text when it does not parse.
fn summary_date(lang: Lang, created_at: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(created_at)
        .map(|t| when(lang, t.timestamp()))
        .unwrap_or_else(|_| created_at.to_owned())
}

/// Whole percent of `bytes` in `total`, at most 100.
fn percent(bytes: u64, total: u64) -> u64 {
    if total == 0 {
        return 0;
    }
    (u128::from(bytes) * 100 / u128::from(total)).min(100) as u64
}

/// Everything the step remembers, and what it does. All fields are signals, so it is `Copy`.
#[derive(Clone, Copy)]
struct Flow {
    stage: RwSignal<Stage>,
    via: RwSignal<Via>,
    host: RwSignal<String>,
    port: RwSignal<String>,
    user: RwSignal<String>,
    ftp_password: RwSignal<String>,
    folder: RwSignal<String>,
    /// The source the typed fields made, kept for the later calls.
    source: RwSignal<Option<FtpSource>>,
    backups: RwSignal<Option<Vec<RemoteBackup>>>,
    selected: RwSignal<String>,
    file: RwSignal<Option<web_sys::File>, LocalStorage>,
    password: RwSignal<String>,
    summary: RwSignal<Option<BackupSummary>>,
    progress: RwSignal<Option<RestoreProgress>>,
    problem: RwSignal<Option<Problem>>,
    busy: RwSignal<bool>,
    /// Counts the list requests, so only the latest one's answer is used.
    lists: RwSignal<u32>,
    /// Bumped whenever a restore run ends or a new one starts, so an older run's tasks stop.
    run: RwSignal<u32>,
    poll_ms: u32,
    applying: Applying,
}

impl Flow {
    fn new() -> Self {
        Self {
            stage: RwSignal::new(Stage::Source),
            via: RwSignal::new(Via::Ftp),
            host: RwSignal::new(String::new()),
            port: RwSignal::new("21".to_owned()),
            user: RwSignal::new(String::new()),
            ftp_password: RwSignal::new(String::new()),
            folder: RwSignal::new(String::new()),
            source: RwSignal::new(None),
            backups: RwSignal::new(None),
            selected: RwSignal::new(String::new()),
            file: RwSignal::new_local(None),
            password: RwSignal::new(String::new()),
            summary: RwSignal::new(None),
            progress: RwSignal::new(None),
            problem: RwSignal::new(None),
            busy: RwSignal::new(false),
            lists: RwSignal::new(0),
            run: RwSignal::new(0),
            poll_ms: use_context::<RestorePollMs>().map_or(DEFAULT_POLL_MS, |p| p.0),
            applying: expect_context::<Applying>(),
        }
    }

    fn go(self, stage: Stage) {
        self.problem.set(None);
        self.stage.set(stage);
    }

    fn request(self) -> Option<RestoreFromFtp> {
        Some(RestoreFromFtp {
            source: self.source.get_untracked()?,
            file: self.selected.get_untracked(),
            password: self.password.get_untracked(),
        })
    }

    /// Starts a new run, or ends the current one when the signal is already gone.
    fn bump(self) -> u32 {
        self.run
            .try_update(|r| {
                *r += 1;
                *r
            })
            .unwrap_or(0)
    }

    fn is_current(self, run: u32) -> bool {
        self.run.try_get_untracked() == Some(run)
    }

    /// Source → Pick: reads the typed FTP fields, then lists the server's backups.
    fn find(self) {
        let source = ftp_source(
            &self.host.get_untracked(),
            &self.port.get_untracked(),
            &self.user.get_untracked(),
            &self.ftp_password.get_untracked(),
            &self.folder.get_untracked(),
        );
        match source {
            Ok(source) => {
                self.via.set(Via::Ftp);
                self.source.set(Some(source));
                self.go(Stage::Pick);
                self.load_list();
            }
            Err(p) => self.problem.set(Some(Problem::Known(p))),
        }
    }

    /// Asks the server for its backups, newest first with the newest selected.
    fn load_list(self) {
        let Some(source) = self.source.get_untracked() else {
            return;
        };
        // A newer request supersedes an older one still in flight, whose answer is dropped.
        let mine = self.lists.try_update(|n| {
            *n += 1;
            *n
        });
        self.busy.set(true);
        self.problem.set(None);
        self.backups.set(None);
        leptos::task::spawn_local(async move {
            let result = restore_ftp_list(&source).await;
            if self.lists.try_get_untracked() != mine {
                return;
            }
            match result {
                Ok(mut list) => {
                    list.sort_by_key(|b| std::cmp::Reverse(b.created_at));
                    self.selected
                        .try_set(list.first().map(|b| b.file.clone()).unwrap_or_default());
                    self.backups.try_set(Some(list));
                }
                Err(e) => {
                    self.problem.try_set(Some(ftp_problem(e)));
                }
            }
            self.busy.try_set(false);
        });
    }

    /// Password → Summary: the hub opens the backup's head with the password.
    fn inspect(self) {
        if self.busy.get_untracked() || self.password.with_untracked(|p| p.is_empty()) {
            return;
        }
        self.busy.set(true);
        self.problem.set(None);
        leptos::task::spawn_local(async move {
            let password = self.password.get_untracked();
            let result = match self.via.get_untracked() {
                Via::Ftp => match self.request() {
                    Some(request) => restore_ftp_inspect(&request).await.map_err(ftp_problem),
                    None => Err(Problem::Known(SetupText::RestoreFailed)),
                },
                Via::Upload => match self.file.get_untracked() {
                    Some(file) => restore_upload_inspect(&file, &password)
                        .await
                        .map_err(|e| Problem::Hub(e.error)),
                    None => Err(Problem::Known(SetupText::RestoreFailed)),
                },
            };
            match result {
                Ok(summary) => {
                    self.summary.try_set(Some(summary));
                    self.go(Stage::Summary);
                }
                Err(p) => {
                    self.problem.try_set(Some(p));
                }
            }
            self.busy.try_set(false);
        });
    }

    /// Summary → Running: starts the restore and follows it.
    fn start(self) {
        let run = self.bump();
        self.progress.set(None);
        self.go(Stage::Running);
        match self.via.get_untracked() {
            Via::Ftp => leptos::task::spawn_local(async move {
                let Some(request) = self.request() else {
                    return;
                };
                match restore_ftp(&request).await {
                    // The hub answered at once; its progress route tells the rest.
                    Ok(first) => {
                        if self.is_current(run) && !self.observe(run, first) {
                            self.follow(run);
                        }
                    }
                    Err(e) => self.fail(run, Problem::Hub(e.error)),
                }
            }),
            Via::Upload => {
                self.follow(run);
                leptos::task::spawn_local(async move {
                    let Some(file) = self.file.get_untracked() else {
                        return;
                    };
                    let password = self.password.get_untracked();
                    match restore_upload(&file, &password).await {
                        Ok(last) => {
                            if !last.is_some_and(|p| self.observe(run, p)) {
                                self.succeed(run);
                            }
                        }
                        // The hub can refuse before it reads the body, and the browser then
                        // sees a network error: the progress route has the hub's reason.
                        Err(e) => match restore_progress().await {
                            Ok(Some(p)) if p.state == BackupState::Done => self.succeed(run),
                            Ok(Some(p)) if p.state == BackupState::Failed && p.error.is_some() => {
                                self.fail(run, Problem::Hub(p.error.unwrap_or_default()));
                            }
                            _ => self.fail(run, Problem::Hub(e.error)),
                        },
                    }
                });
            }
        }
    }

    /// Acts on a progress reading; true when the run is over.
    fn observe(self, run: u32, p: RestoreProgress) -> bool {
        match p.state {
            BackupState::Running => {
                if self.is_current(run) {
                    self.progress.try_set(Some(p));
                }
                false
            }
            BackupState::Done => {
                self.succeed(run);
                true
            }
            BackupState::Failed | BackupState::Cancelled => {
                let problem = p
                    .error
                    .map_or(Problem::Known(SetupText::RestoreFailed), Problem::Hub);
                self.fail(run, problem);
                true
            }
        }
    }

    /// Asks the hub how the restore is going until it is over. A hub that stops answering is
    /// restarting into the running house.
    fn follow(self, run: u32) {
        leptos::task::spawn_local(async move {
            let mut lost = 0;
            loop {
                TimeoutFuture::new(self.poll_ms).await;
                if !self.is_current(run) {
                    return;
                }
                match restore_progress().await {
                    Ok(Some(p)) => {
                        lost = 0;
                        if self.observe(run, p) {
                            return;
                        }
                    }
                    Ok(None) => lost = 0,
                    Err(_) => {
                        lost += 1;
                        if lost >= LOST_AFTER {
                            self.succeed(run);
                            return;
                        }
                    }
                }
            }
        });
    }

    /// The restore is done: wait for the hub to restart, then reload into the house.
    fn succeed(self, run: u32) {
        if self.is_current(run) {
            self.bump();
            self.applying.start("/");
        }
    }

    fn fail(self, run: u32, problem: Problem) {
        if self.is_current(run) {
            self.bump();
            self.problem.try_set(Some(problem));
            self.stage.try_set(Stage::Failed);
        }
    }

    /// Failed → Retry: an FTP restore runs again with the same source; an upload needs its file
    /// again.
    fn retry(self) {
        match self.via.get_untracked() {
            Via::Ftp => self.start(),
            Via::Upload => {
                self.file.set(None);
                self.password.set(String::new());
                self.go(Stage::Source);
            }
        }
    }

    /// Leaves the restore path: the hub restarts into the setup by hand.
    fn abandon(self) {
        if self.busy.get_untracked() {
            return;
        }
        self.busy.set(true);
        self.problem.set(None);
        leptos::task::spawn_local(async move {
            match restore_pending(false).await {
                Ok(_) | Err(SaveError::Restarting) => {
                    self.applying.start(&setup_href(SetupStep::Devices));
                    return;
                }
                Err(e) => {
                    self.problem
                        .try_set(Some(Problem::Hub(e.message(current_lang()))));
                }
            }
            self.busy.try_set(false);
        });
    }
}

/// A field's caption: a word of the wizard or of the Backup page.
#[derive(Clone, Copy)]
enum Caption {
    Setup(SetupText),
    Backup(BackupText),
}

#[component]
fn Field(
    id: &'static str,
    caption: Caption,
    value: RwSignal<String>,
    #[prop(default = "text")] kind: &'static str,
) -> impl IntoView {
    let lang = use_lang();
    view! {
        <label class="field" for=id>
            <span class="field__caption">
                {move || match caption {
                    Caption::Setup(k) => setup_text(lang(), k),
                    Caption::Backup(k) => backup_text(lang(), k),
                }}
            </span>
            <input
                id=id
                class="field__input"
                type=kind
                autocomplete="off"
                prop:value=move || value.get()
                on:input=move |ev| value.set(event_target_value(&ev))
            />
        </label>
    }
}

/// The problem line under a form.
#[component]
fn ProblemLine(flow: Flow) -> impl IntoView {
    let lang = use_lang();
    view! {
        <p class="loc__problem restore__problem" role="alert">
            {move || flow.problem.get().map(|p| problem_text(lang(), &p))}
        </p>
    }
}

fn source_view(flow: Flow) -> impl IntoView {
    let t = use_setup_text();
    let on_file = move |ev: leptos::ev::Event| {
        let input = event_target::<web_sys::HtmlInputElement>(&ev);
        if let Some(file) = input.files().and_then(|f| f.get(0)) {
            flow.via.set(Via::Upload);
            flow.file.set(Some(file));
            flow.password.set(String::new());
            flow.go(Stage::Password);
        }
    };
    view! {
        <section class="sheet__panel restore__panel">
            <h2 class="sheet__heading">{move || t(SetupText::RestoreFromFtp)}</h2>
            <div class="loc__grid">
                <Field id="restore-host" caption=Caption::Setup(SetupText::Host) value=flow.host />
                <Field id="restore-port" caption=Caption::Setup(SetupText::Port) value=flow.port />
                <Field id="restore-user" caption=Caption::Setup(SetupText::User) value=flow.user />
                <Field
                    id="restore-ftp-password"
                    caption=Caption::Setup(SetupText::Password)
                    value=flow.ftp_password
                    kind="password"
                />
                <Field
                    id="restore-folder"
                    caption=Caption::Backup(BackupText::Folder)
                    value=flow.folder
                />
            </div>
            <ProblemLine flow=flow />
            <div class="loc__actions">
                <button type="button" class="btn btn--primary restore__find" on:click=move |_| flow.find()>
                    {move || t(SetupText::RestoreFindBackups)}
                </button>
            </div>
        </section>
        <section class="sheet__panel restore__panel">
            <h2 class="sheet__heading">{move || t(SetupText::RestoreFromUpload)}</h2>
            <label class="field" for="restore-file">
                <span class="field__caption">{move || t(SetupText::RestoreFile)}</span>
                <input id="restore-file" class="field__input" type="file" on:change=on_file />
            </label>
        </section>
    }
}

fn pick_view(flow: Flow) -> impl IntoView {
    let t = use_setup_text();
    let lang = use_lang();
    let body = move || {
        if flow.busy.get() {
            let loading = backup_text(lang(), BackupText::Loading);
            return view! { <p class="setup__lead">{loading}</p> }.into_any();
        }
        if flow.problem.with(Option::is_some) {
            return view! {
                <ProblemLine flow=flow />
                <div class="loc__actions">
                    <button type="button" class="btn restore__retry" on:click=move |_| flow.load_list()>
                        {move || t(SetupText::Retry)}
                    </button>
                </div>
            }
            .into_any();
        }
        match flow.backups.get() {
            Some(list) if list.is_empty() => {
                view! { <p class="setup__lead">{move || t(SetupText::RestoreNoBackups)}</p> }
                    .into_any()
            }
            Some(list) => list
                .into_iter()
                .map(|b| {
                    let label =
                        format!("{} · {}", when(lang(), b.created_at), size(lang(), b.size));
                    let file = b.file.clone();
                    let chosen = b.file.clone();
                    view! {
                        <label class="restore__item">
                            <input
                                type="radio"
                                name="restore-file"
                                value=b.file
                                prop:checked=move || flow.selected.with(|s| *s == chosen)
                                on:change=move |_| flow.selected.set(file.clone())
                            />
                            <span>{label}</span>
                        </label>
                    }
                })
                .collect_view()
                .into_any(),
            None => ().into_any(),
        }
    };
    view! {
        <h2 class="sheet__heading">{move || t(SetupText::RestorePick)}</h2>
        {body}
        <div class="loc__actions restore__actions">
            <button type="button" class="btn restore__back" on:click=move |_| flow.go(Stage::Source)>
                {move || t(SetupText::Back)}
            </button>
            <button
                type="button"
                class="btn btn--primary restore__next"
                prop:disabled=move || {
                    flow.busy.get() || flow.selected.with(String::is_empty)
                        || flow.backups.with(Option::is_none)
                }
                on:click=move |_| flow.go(Stage::Password)
            >
                {move || t(SetupText::Next)}
            </button>
        </div>
    }
}

fn password_view(flow: Flow) -> impl IntoView {
    let t = use_setup_text();
    let lang = use_lang();
    let back = move |_| {
        flow.go(if flow.via.get_untracked() == Via::Ftp {
            Stage::Pick
        } else {
            Stage::Source
        });
    };
    view! {
        <h2 class="sheet__heading">
            {move || backup_text(lang(), BackupText::PasswordTitle)}
        </h2>
        <p class="setup__lead">{move || t(SetupText::RestorePasswordLead)}</p>
        <Field
            id="restore-password"
            caption=Caption::Setup(SetupText::Password)
            value=flow.password
            kind="password"
        />
        <ProblemLine flow=flow />
        <div class="loc__actions restore__actions">
            <button type="button" class="btn restore__back" on:click=back>
                {move || t(SetupText::Back)}
            </button>
            <button
                type="button"
                class="btn btn--primary restore__next"
                prop:disabled=move || flow.busy.get() || flow.password.with(String::is_empty)
                on:click=move |_| flow.inspect()
            >
                {move || if flow.busy.get() { t(SetupText::Testing) } else { t(SetupText::Next) }}
            </button>
        </div>
    }
}

fn summary_view(flow: Flow) -> impl IntoView {
    let t = use_setup_text();
    let lang = use_lang();
    let rows = move || {
        flow.summary.get().map(|s| {
            let row = |caption: SetupText, value: String| {
                view! {
                    <div class="restore__row">
                        <dt>{move || t(caption)}</dt>
                        <dd>{value}</dd>
                    </div>
                }
            };
            view! {
                <dl class="restore__summary">
                    {row(SetupText::RestoreSummaryDate, summary_date(lang(), &s.created_at))}
                    {row(SetupText::RestoreSummaryVersion, s.nidoo_version.clone())}
                    {row(SetupText::RestoreSummaryFlows, s.flows.to_string())}
                    {row(SetupText::RestoreSummaryDashboards, s.dashboards.to_string())}
                    {row(SetupText::RestoreSummaryHistory, size(lang(), s.history_bytes))}
                </dl>
            }
        })
    };
    view! {
        {rows}
        <ProblemLine flow=flow />
        <div class="loc__actions restore__actions">
            <button type="button" class="btn restore__back" on:click=move |_| flow.go(Stage::Password)>
                {move || t(SetupText::Back)}
            </button>
            <button
                type="button"
                class="btn btn--primary restore__go"
                on:click=move |_| flow.start()
            >
                {move || t(SetupText::RestoreNow)}
            </button>
        </div>
    }
}

fn running_view(flow: Flow) -> impl IntoView {
    let t = use_setup_text();
    let lang = use_lang();
    let total = move || flow.progress.with(|p| p.as_ref().and_then(|p| p.total));
    let label = move || {
        flow.progress.get().map(|p| {
            let part = backup_text(lang(), part_text(p.part));
            match p.total {
                Some(total) => format!("{part} · {} %", percent(p.bytes, total)),
                None => format!("{part} · {}", size(lang(), p.bytes)),
            }
        })
    };
    view! {
        <h2 class="sheet__heading">{move || t(SetupText::Restoring)}</h2>
        <progress
            class="restore__bar"
            max=move || total().map(|t| t.to_string())
            value=move || {
                flow.progress.with(|p| p.as_ref().and_then(|p| total().map(|_| p.bytes.to_string())))
            }
        ></progress>
        <p class="setup__lead" role="status">
            {label}
        </p>
    }
}

fn failed_view(flow: Flow) -> impl IntoView {
    let t = use_setup_text();
    view! {
        <h2 class="sheet__heading">{move || t(SetupText::RestoreFailed)}</h2>
        <ProblemLine flow=flow />
        <div class="loc__actions restore__actions">
            <button type="button" class="btn btn--primary restore__retry" on:click=move |_| flow.retry()>
                {move || t(SetupText::Retry)}
            </button>
        </div>
    }
}

/// The Restore step.
#[component]
pub fn RestoreStep() -> impl IntoView {
    let t = use_setup_text();
    let flow = Flow::new();
    // Leaving the step ends a run's polling.
    on_cleanup(move || {
        flow.bump();
    });
    view! {
        <div class="setup__body restore">
            <h1 class="setup__title">{move || t(SetupText::RestoreTitle)}</h1>
            <p class="setup__lead">{move || t(SetupText::RestoreLead)}</p>
            {move || match flow.stage.get() {
                Stage::Source => source_view(flow).into_any(),
                Stage::Pick => pick_view(flow).into_any(),
                Stage::Password => password_view(flow).into_any(),
                Stage::Summary => summary_view(flow).into_any(),
                Stage::Running => running_view(flow).into_any(),
                Stage::Failed => failed_view(flow).into_any(),
            }}
            <Show when=move || flow.stage.get() != Stage::Running>
                <button
                    type="button"
                    class="restore__without"
                    prop:disabled=move || flow.busy.get()
                    on:click=move |_| flow.abandon()
                >
                    {move || t(SetupText::RestoreWithoutBackup)}
                </button>
            </Show>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_typed_fields_make_the_source() {
        let s = ftp_source(" nas.local ", "21", " nidoo ", " pw ", " /backups ").unwrap();
        assert_eq!(s.host, "nas.local");
        assert_eq!(s.port, Some(21));
        assert_eq!(s.user, "nidoo");
        // A password is sent as typed.
        assert_eq!(s.password, " pw ");
        assert_eq!(s.folder.as_deref(), Some("/backups"));
    }

    #[test]
    fn a_blank_port_and_folder_are_left_out() {
        let s = ftp_source("nas.local", " ", "", "", "").unwrap();
        assert_eq!((s.port, s.folder), (None, None));
    }

    #[test]
    fn a_host_is_required_and_the_port_must_be_a_port() {
        assert_eq!(
            ftp_source(" ", "21", "", "", "").unwrap_err(),
            SetupText::FieldRequired
        );
        for bad in ["0", "70000", "ftp"] {
            assert_eq!(
                ftp_source("nas.local", bad, "", "", "").unwrap_err(),
                SetupText::PortInvalid
            );
        }
    }

    #[test]
    fn percent_stays_within_zero_and_a_hundred() {
        assert_eq!(percent(50, 100), 50);
        assert_eq!(percent(1, 3), 33);
        assert_eq!(percent(200, 100), 100);
        assert_eq!(percent(5, 0), 0);
        assert_eq!(percent(u64::MAX, u64::MAX), 100);
    }

    #[test]
    fn an_unparsable_summary_date_is_shown_as_received() {
        assert_eq!(summary_date(Lang::En, "yesterday"), "yesterday");
    }

    #[test]
    fn our_problems_follow_the_language_and_the_hubs_do_not() {
        let own = Problem::Known(SetupText::RestoreFtpUnreachable);
        assert_eq!(
            problem_text(Lang::En, &own),
            "Can't reach the FTP server. Is it switched on?"
        );
        assert_eq!(
            problem_text(Lang::Fr, &own),
            "Impossible de joindre le serveur FTP. Est-il allumé ?"
        );
        let hub = Problem::Hub("wrong password".into());
        assert_eq!(problem_text(Lang::Fr, &hub), "wrong password");
    }
}
