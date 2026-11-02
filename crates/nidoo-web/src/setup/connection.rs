//! The MQTT and Database steps. [`MqttForm`] and [`DatabaseForm`] are also the bodies of
//! Settings › Broker and Settings › Database, so they need no `House`, socket or wizard.
//! Next (or Save) stays off until the values as they stand have passed a test.

use leptos::prelude::*;
use nidoo_api::{DatabaseView, MqttView, SetupStatus, SetupStep, TestResult};
use serde_json::{Map, Value, json};

use super::wizard::WizardNav;
use super::{Applying, SetupState, setup_href};
use crate::api::setup::{SaveError, put_database, put_mqtt, test_database, test_mqtt};
use crate::format::date_only;
use crate::i18n::setup::{SetupText, setup_text, use_setup_text};
use crate::i18n::use_lang;

/// The MQTT fields as typed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MqttDraft {
    pub host: String,
    pub port: String,
    pub user: String,
    pub password: String,
    pub client_id: String,
    pub z2m_base: String,
}

/// A port field's text; it must be a number from 1 to 65535.
fn parse_port(text: &str) -> Result<u16, SetupText> {
    match text.trim().parse::<u16>() {
        Ok(p) if p >= 1 => Ok(p),
        _ => Err(SetupText::PortInvalid),
    }
}

impl MqttDraft {
    /// The stored values, or the local defaults when the hub has none. The password stays empty:
    /// the hub never sends it, and an empty one reads as "keep".
    pub fn from_view(v: Option<&MqttView>) -> MqttDraft {
        match v {
            Some(v) => MqttDraft {
                host: v.host.clone(),
                port: v.port.to_string(),
                user: v.user.clone().unwrap_or_default(),
                password: String::new(),
                client_id: v.client_id.clone().unwrap_or_default(),
                z2m_base: v.z2m_base.clone().unwrap_or_default(),
            },
            None => MqttDraft {
                host: "localhost".into(),
                port: "1883".into(),
                ..MqttDraft::default()
            },
        }
    }

    /// The request body. The password is always a string: `""` keeps the stored one, and the
    /// web never sends `null`, which would clear it. Blank optional fields are left out.
    pub fn body(&self) -> Result<Value, SetupText> {
        let host = self.host.trim();
        if host.is_empty() {
            return Err(SetupText::FieldRequired);
        }
        let mut m = Map::new();
        m.insert("host".into(), json!(host));
        m.insert("port".into(), json!(parse_port(&self.port)?));
        m.insert("password".into(), json!(self.password));
        for (key, text) in [
            ("user", &self.user),
            ("client_id", &self.client_id),
            ("z2m_base", &self.z2m_base),
        ] {
            if !text.trim().is_empty() {
                m.insert(key.into(), json!(text.trim()));
            }
        }
        Ok(Value::Object(m))
    }
}

/// The database fields as typed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DatabaseDraft {
    pub host: String,
    pub port: String,
    pub database: String,
    pub user: String,
    pub password: String,
}

impl DatabaseDraft {
    /// The stored values, or the local defaults when the hub has none.
    pub fn from_view(v: Option<&DatabaseView>) -> DatabaseDraft {
        match v {
            Some(v) => DatabaseDraft {
                host: v.host.clone(),
                port: v.port.to_string(),
                database: v.database.clone(),
                user: v.user.clone(),
                password: String::new(),
            },
            None => DatabaseDraft {
                host: "localhost".into(),
                port: "5432".into(),
                database: "nidoo".into(),
                user: "nidoo".into(),
                password: String::new(),
            },
        }
    }

    /// The request body; the password is always a string, as for [`MqttDraft::body`].
    pub fn body(&self) -> Result<Value, SetupText> {
        let (host, database, user) = (self.host.trim(), self.database.trim(), self.user.trim());
        if host.is_empty() || database.is_empty() || user.is_empty() {
            return Err(SetupText::FieldRequired);
        }
        Ok(json!({
            "host": host,
            "port": parse_port(&self.port)?,
            "database": database,
            "user": user,
            "password": self.password,
        }))
    }
}

/// Whether the last successful test (`last_ok`, the body it ran on as text) was of exactly `body`.
pub fn test_passed(last_ok: Option<&str>, body: &Value) -> bool {
    last_ok == Some(body.to_string().as_str())
}

/// Which service a form talks to.
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Mqtt,
    Database,
}

/// A test outcome and the body it ran on.
#[derive(Clone, PartialEq)]
struct Outcome {
    key: String,
    ok: bool,
    message: String,
    /// The date the database's history starts, formatted; only for a passing database test.
    history: Option<String>,
}

async fn run_test(kind: Kind, body: &Value) -> Outcome {
    let key = body.to_string();
    let result = match kind {
        Kind::Mqtt => test_mqtt(body).await,
        Kind::Database => test_database(body).await,
    };
    match result {
        Ok(TestResult {
            ok,
            message,
            detail,
        }) => Outcome {
            history: detail
                .as_ref()
                .and_then(|d| d.get("history_since"))
                .and_then(Value::as_str)
                .and_then(date_only),
            key,
            ok,
            message,
        },
        // A refusal before any connection (422) arrives as the hub's own message.
        Err(e) => Outcome {
            key,
            ok: false,
            message: e.error,
            history: None,
        },
    }
}

async fn run_save(kind: Kind, body: &Value) -> Result<SetupStatus, SaveError> {
    match kind {
        Kind::Mqtt => put_mqtt(body).await,
        Kind::Database => put_database(body).await,
    }
}

/// The Test button, its outcome, and Back / Next (Save). Shared by both forms.
#[component]
fn ConnectionActions(
    kind: Kind,
    body: Memo<Result<Value, SetupText>>,
    applying_target: String,
    on_saved: Callback<SetupStatus>,
    wizard: bool,
    on_back: Option<Callback<()>>,
) -> impl IntoView {
    let t = use_setup_text();
    let lang = use_lang();
    let applying = expect_context::<Applying>();
    let outcome = RwSignal::new(None::<Outcome>);
    let last_ok = RwSignal::new(None::<String>);
    let testing = RwSignal::new(false);
    let saving = RwSignal::new(false);
    let failure = RwSignal::new(None::<String>);

    // An outcome belongs to the values it ran on: edit a field and it no longer shows.
    let shown = Memo::new(move |_| {
        let current = body.get().ok()?.to_string();
        outcome.get().filter(|o| o.key == current)
    });
    let passed = Memo::new(move |_| {
        body.with(|b| {
            b.as_ref()
                .is_ok_and(|b| last_ok.with(|l| test_passed(l.as_deref(), b)))
        })
    });

    let test = move |_| {
        let Ok(b) = body.get_untracked() else { return };
        if testing.get_untracked() || saving.get_untracked() {
            return;
        }
        testing.set(true);
        failure.set(None);
        leptos::task::spawn_local(async move {
            let result = run_test(kind, &b).await;
            if result.ok {
                last_ok.try_set(Some(result.key.clone()));
            }
            outcome.try_set(Some(result));
            testing.try_set(false);
        });
    };

    let save = move |_| {
        let Ok(b) = body.get_untracked() else { return };
        if !passed.get_untracked() || saving.get_untracked() {
            return;
        }
        saving.set(true);
        failure.set(None);
        let target = applying_target.clone();
        leptos::task::spawn_local(async move {
            match run_save(kind, &b).await {
                // The hub restarts: the button stays off until the page reloads.
                Ok(s) if s.restarting => {
                    applying.start(&target);
                    return;
                }
                Err(SaveError::Restarting) => {
                    applying.start(&target);
                    return;
                }
                Ok(s) => on_saved.run(s),
                Err(SaveError::Failed(r)) => {
                    // The broker changed since the test: it no longer vouches for these values.
                    last_ok.try_set(None);
                    outcome.try_set(Some(Outcome {
                        key: b.to_string(),
                        ok: false,
                        message: r.message,
                        history: None,
                    }));
                }
                Err(e) => {
                    failure.try_set(Some(e.message(lang())));
                }
            }
            saving.try_set(false);
        });
    };

    view! {
        <p class="loc__problem" role="status">
            {move || body.get().err().map(t)}
        </p>
        <div class="conn__result" role="status">
            {move || {
                shown
                    .get()
                    .map(|o| {
                        let class = if o.ok {
                            "conn__line conn__line--ok"
                        } else {
                            "conn__line conn__line--bad"
                        };
                        let history = o
                            .history
                            .map(|d| {
                                setup_text(lang(), SetupText::HistorySince).replace("{date}", &d)
                            });
                        view! {
                            <p class=class>{o.message}</p>
                            {history.map(|h| view! { <p class="settings__hint">{h}</p> })}
                        }
                    })
            }}
        </div>
        <p class="loc__problem" role="alert">
            {move || failure.get()}
        </p>
        {matches!(kind, Kind::Database)
            .then(|| view! { <p class="settings__hint">{move || t(SetupText::NoSkipDatabase)}</p> })
            .filter(|_| wizard)}
        <div class="loc__actions conn__actions">
            {on_back
                .map(|back| {
                    view! {
                        <button type="button" class="btn" on:click=move |_| back.run(())>
                            {move || t(SetupText::Back)}
                        </button>
                    }
                })}
            <button
                type="button"
                class="btn conn__test"
                prop:disabled=move || body.with(Result::is_err) || testing.get() || saving.get()
                on:click=test
            >
                {move || t(if testing.get() { SetupText::Testing } else { SetupText::TestConnection })}
            </button>
            <button
                type="button"
                class="btn btn--primary loc__save"
                prop:disabled=move || !passed.get() || saving.get()
                on:click=save
            >
                {move || t(if wizard { SetupText::Next } else { SetupText::Save })}
            </button>
        </div>
    }
}

/// A labelled text input bound to `value`.
#[component]
pub(crate) fn TextField(
    id: &'static str,
    label: SetupText,
    value: RwSignal<String>,
    #[prop(optional)] password: bool,
    #[prop(optional)] wide: bool,
    /// Shows this text while the field is empty.
    #[prop(optional, into)]
    placeholder: Option<Signal<Option<&'static str>>>,
) -> impl IntoView {
    let t = use_setup_text();
    view! {
        <label class="field" class:loc__wide=wide for=id>
            <span class="field__caption">{move || t(label)}</span>
            <input
                id=id
                class="field__input"
                type=if password { "password" } else { "text" }
                autocomplete=if password { "new-password" } else { "off" }
                placeholder=move || placeholder.and_then(|p| p.get())
                prop:value=move || value.get()
                on:input=move |ev| value.set(event_target_value(&ev))
            />
        </label>
    }
}

/// Host, port and login of the MQTT broker, with a connection test. A save that restarts the
/// hub starts the "Applying…" overlay towards `applying_target`; any other save calls
/// `on_saved`. `wizard` labels the button Next instead of Save.
#[component]
pub fn MqttForm(
    initial: Option<MqttView>,
    applying_target: String,
    #[prop(into)] on_saved: Callback<SetupStatus>,
    #[prop(optional)] on_back: Option<Callback<()>>,
    #[prop(optional)] wizard: bool,
) -> impl IntoView {
    let t = use_setup_text();
    let secret_saved = initial.as_ref().is_some_and(|v| v.password.set);
    let d = MqttDraft::from_view(initial.as_ref());
    let host = RwSignal::new(d.host);
    let port = RwSignal::new(d.port);
    let user = RwSignal::new(d.user);
    let password = RwSignal::new(d.password);
    let client_id = RwSignal::new(d.client_id);
    let z2m_base = RwSignal::new(d.z2m_base);
    let body = Memo::new(move |_| {
        MqttDraft {
            host: host.get(),
            port: port.get(),
            user: user.get(),
            password: password.get(),
            client_id: client_id.get(),
            z2m_base: z2m_base.get(),
        }
        .body()
    });
    let placeholder = Signal::derive(move || secret_saved.then(|| t(SetupText::SecretSaved)));
    view! {
        <div class="loc">
            <div class="loc__grid">
                <TextField id="mqtt-host" label=SetupText::Host value=host />
                <TextField id="mqtt-port" label=SetupText::Port value=port />
                <TextField id="mqtt-user" label=SetupText::User value=user />
                <TextField
                    id="mqtt-password"
                    label=SetupText::Password
                    value=password
                    password=true
                    placeholder=placeholder
                />
            </div>
            <details class="loc__details">
                <summary class="sheet__summary">{move || t(SetupText::Advanced)}</summary>
                <div class="loc__grid">
                    <TextField id="mqtt-client" label=SetupText::ClientId value=client_id />
                    <TextField id="mqtt-z2m" label=SetupText::Z2mBase value=z2m_base />
                </div>
            </details>
            <ConnectionActions
                kind=Kind::Mqtt
                body=body
                applying_target=applying_target
                on_saved=on_saved
                wizard=wizard
                on_back=on_back
            />
        </div>
    }
}

/// Host, port, database and login of TimescaleDB, with a connection test. Same contract as
/// [`MqttForm`]; the wizard step has no Skip.
#[component]
pub fn DatabaseForm(
    initial: Option<DatabaseView>,
    applying_target: String,
    #[prop(into)] on_saved: Callback<SetupStatus>,
    #[prop(optional)] on_back: Option<Callback<()>>,
    #[prop(optional)] wizard: bool,
) -> impl IntoView {
    let t = use_setup_text();
    let secret_saved = initial.as_ref().is_some_and(|v| v.password.set);
    let d = DatabaseDraft::from_view(initial.as_ref());
    let host = RwSignal::new(d.host);
    let port = RwSignal::new(d.port);
    let database = RwSignal::new(d.database);
    let user = RwSignal::new(d.user);
    let password = RwSignal::new(d.password);
    let body = Memo::new(move |_| {
        DatabaseDraft {
            host: host.get(),
            port: port.get(),
            database: database.get(),
            user: user.get(),
            password: password.get(),
        }
        .body()
    });
    let placeholder = Signal::derive(move || secret_saved.then(|| t(SetupText::SecretSaved)));
    view! {
        <div class="loc">
            <div class="loc__grid">
                <TextField id="db-host" label=SetupText::Host value=host />
                <TextField id="db-port" label=SetupText::Port value=port />
                <TextField id="db-name" label=SetupText::DatabaseName value=database wide=true />
                <TextField id="db-user" label=SetupText::User value=user />
                <TextField
                    id="db-password"
                    label=SetupText::Password
                    value=password
                    password=true
                    placeholder=placeholder
                />
            </div>
            <ConnectionActions
                kind=Kind::Database
                body=body
                applying_target=applying_target
                on_saved=on_saved
                wizard=wizard
                on_back=on_back
            />
        </div>
    }
}

/// The status the wizard last heard from the hub, which pre-fills the steps.
fn known_status() -> Option<SetupStatus> {
    use_context::<SetupState>().and_then(|s| s.0.get_untracked())
}

/// The MQTT step. Needs [`WizardNav`] and [`Applying`] in context.
#[component]
pub fn MqttStep() -> impl IntoView {
    let t = use_setup_text();
    let nav = expect_context::<WizardNav>();
    // Captured here: a spawned task has no reactive owner, so context is gone after an await.
    let state = use_context::<SetupState>();
    view! {
        <div class="setup__body">
            <h1 class="setup__title">{move || t(SetupText::MqttTitle)}</h1>
            <p class="setup__lead">{move || t(SetupText::MqttLead)}</p>
            <MqttForm
                initial=known_status().and_then(|s| s.mqtt)
                applying_target=setup_href(SetupStep::Database)
                on_saved=move |s: SetupStatus| {
                    if let Some(st) = state {
                        st.0.try_set(Some(s));
                    }
                    nav.step.set(SetupStep::Database);
                }
                on_back=Callback::new(move |_| nav.step.set(SetupStep::Welcome))
                wizard=true
            />
        </div>
    }
}

/// The Database step. Saving it restarts a hub that is still in setup mode, so the overlay
/// reloads into Devices in running mode.
#[component]
pub fn DatabaseStep() -> impl IntoView {
    let t = use_setup_text();
    let nav = expect_context::<WizardNav>();
    let state = use_context::<SetupState>();
    view! {
        <div class="setup__body">
            <h1 class="setup__title">{move || t(SetupText::DatabaseTitle)}</h1>
            <p class="setup__lead">{move || t(SetupText::DatabaseLead)}</p>
            <DatabaseForm
                initial=known_status().and_then(|s| s.database)
                applying_target=setup_href(SetupStep::Devices)
                on_saved=move |s: SetupStatus| {
                    // Only reached when the hub did not restart. A hub still in setup mode has
                    // no house to show Devices with, so it goes back to the broker.
                    let next = if s.mode == "setup" {
                        SetupStep::Mqtt
                    } else {
                        SetupStep::Devices
                    };
                    if let Some(st) = state {
                        st.0.try_set(Some(s));
                    }
                    nav.step.set(next);
                }
                on_back=Callback::new(move |_| nav.step.set(SetupStep::Mqtt))
                wizard=true
            />
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mqtt() -> MqttDraft {
        MqttDraft::from_view(None)
    }

    #[test]
    fn an_empty_password_is_sent_as_an_empty_string() {
        let body = MqttDraft {
            password: "".into(),
            ..mqtt()
        }
        .body()
        .unwrap();
        assert_eq!(body["password"], json!(""));
        assert_ne!(body.get("password"), Some(&Value::Null));
        let db = DatabaseDraft::from_view(None).body().unwrap();
        assert_eq!(db["password"], json!(""));
        assert_ne!(db.get("password"), Some(&Value::Null));
    }

    #[test]
    fn blank_optional_mqtt_fields_are_left_out() {
        let body = mqtt().body().unwrap();
        assert_eq!(
            body,
            json!({"host": "localhost", "port": 1883, "password": ""})
        );
        let full = MqttDraft {
            user: "nidoo".into(),
            client_id: "hub".into(),
            z2m_base: "zigbee2mqtt".into(),
            ..mqtt()
        }
        .body()
        .unwrap();
        assert_eq!(full["user"], json!("nidoo"));
        assert_eq!(full["client_id"], json!("hub"));
        assert_eq!(full["z2m_base"], json!("zigbee2mqtt"));
    }

    #[test]
    fn a_changed_field_needs_a_new_test() {
        let b1 = mqtt().body().unwrap();
        let b2 = MqttDraft {
            port: "1884".into(),
            ..mqtt()
        }
        .body()
        .unwrap();
        assert!(test_passed(Some(&b1.to_string()), &b1));
        assert!(!test_passed(Some(&b1.to_string()), &b2));
        assert!(!test_passed(None, &b1));
    }

    #[test]
    fn a_bad_port_is_named() {
        for port in ["abc", "0", "", "70000"] {
            let m = MqttDraft {
                port: port.into(),
                ..mqtt()
            };
            assert_eq!(m.body(), Err(SetupText::PortInvalid), "{port}");
            let d = DatabaseDraft {
                port: port.into(),
                ..DatabaseDraft::from_view(None)
            };
            assert_eq!(d.body(), Err(SetupText::PortInvalid), "{port}");
        }
    }

    #[test]
    fn the_database_needs_a_name_and_a_user() {
        for blank in ["database", "user", "host"] {
            let mut d = DatabaseDraft::from_view(None);
            match blank {
                "database" => d.database = " ".into(),
                "user" => d.user.clear(),
                _ => d.host.clear(),
            }
            assert_eq!(d.body(), Err(SetupText::FieldRequired), "{blank}");
        }
    }

    #[test]
    fn defaults_without_a_stored_view() {
        let d = DatabaseDraft::from_view(None);
        assert_eq!(
            (
                d.host.as_str(),
                d.port.as_str(),
                d.database.as_str(),
                d.user.as_str()
            ),
            ("localhost", "5432", "nidoo", "nidoo")
        );
    }
}
