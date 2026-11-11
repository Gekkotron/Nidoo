//! Which retained MQTT topics a backup carries, and how they are read and written back.
//!
//! The list is one place in code ([`Family`]); `docs/reference/mqtt-topics.md` has a matching
//! **Backed up** column and a test keeps the two in step. Definitions and user data are backed
//! up; runtime state and discovery the hub republishes itself are not.

use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use bytes::Bytes;
use futures_util::future::BoxFuture;
use rumqttc::{
    AsyncClient, ConnectReturnCode, Event, EventLoop, MqttOptions, Packet, QoS, SubscribeFilter,
};
use tokio::time::Instant;

use crate::config::{Config, MqttConfig};

/// How long a collection waits without a new message before it ends.
pub const QUIET: Duration = Duration::from_secs(2);

/// Time allowed to reach the broker and to see each of its answers.
const STEP_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_PACKET: usize = 4 * 1024 * 1024;
const KEEP_ALIVE: Duration = Duration::from_secs(30);
/// Publishes sent before their acknowledgements are awaited.
const WINDOW: usize = 32;

/// The topic prefixes the hub is configured with (trailing `/` removed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub ui: String,
    pub automation: String,
    pub flow_labels: String,
    /// The first discovery prefix: the only one the hub publishes its own entities under.
    pub discovery: String,
    pub push: String,
    pub climate: String,
    pub alarm: String,
}

impl Layout {
    pub fn from_config(cfg: &Config) -> Layout {
        let clean = |s: &str| s.trim().trim_end_matches('/').to_owned();
        Layout {
            ui: clean(&cfg.topics.ui_prefix),
            automation: clean(&cfg.topics.automation_prefix),
            flow_labels: clean(&cfg.topics.flow_labels),
            discovery: cfg
                .topics
                .discovery_prefixes
                .first()
                .map(|p| clean(p))
                .unwrap_or_default(),
            push: clean(&cfg.push.prefix),
            climate: clean(&cfg.climate.prefix),
            alarm: clean(&cfg.alarm.prefix),
        }
    }
}

/// A kind of retained topic the hub owns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    UiOverride,
    Areas,
    Home,
    Types,
    Dashboard,
    Ambiance,
    Flow,
    FlowLabels,
    VirtualConfig,
    VirtualState,
    WeatherAny,
    SolarCache,
    SolarOther,
    GroupDefinition,
    GroupMigrated,
    GroupOther,
    NetworkDefinition,
    NetworkOther,
    PushTokens,
    PushHistory,
    ClimateHouse,
    ClimateZoneSettings,
    ClimateZoneRuntime,
    AlarmSettings,
    AlarmRuntime,
    AlarmDiscovery,
}

impl Family {
    pub const ALL: &'static [Family] = &[
        Family::UiOverride,
        Family::Areas,
        Family::Home,
        Family::Types,
        Family::Dashboard,
        Family::Ambiance,
        Family::Flow,
        Family::FlowLabels,
        Family::VirtualConfig,
        Family::VirtualState,
        Family::WeatherAny,
        Family::SolarCache,
        Family::SolarOther,
        Family::GroupDefinition,
        Family::GroupMigrated,
        Family::GroupOther,
        Family::NetworkDefinition,
        Family::NetworkOther,
        Family::PushTokens,
        Family::PushHistory,
        Family::ClimateHouse,
        Family::ClimateZoneSettings,
        Family::ClimateZoneRuntime,
        Family::AlarmSettings,
        Family::AlarmRuntime,
        Family::AlarmDiscovery,
    ];

    /// Definitions and user data: yes. Runtime state and hub-republished discovery: no.
    pub fn backed_up(self) -> bool {
        use Family::*;
        matches!(
            self,
            UiOverride
                | Areas
                | Home
                | Dashboard
                | Ambiance
                | Flow
                | FlowLabels
                | VirtualConfig
                | SolarCache
                | GroupDefinition
                | GroupMigrated
                | NetworkDefinition
                | PushTokens
                | PushHistory
                | ClimateHouse
                | ClimateZoneSettings
                | AlarmSettings
        )
    }

    /// The first cell of this family's row in `docs/reference/mqtt-topics.md`, backticks removed.
    pub fn doc_topic(self) -> &'static str {
        use Family::*;
        match self {
            UiOverride => "nidoo/ui/<entity id>",
            Areas => "nidoo/ui/_areas",
            Home => "nidoo/ui/_home",
            Types => "nidoo/ui/_types",
            Dashboard => "nidoo/ui/dashboards/<id>",
            Ambiance => "nidoo/ui/ambiances/<id>",
            Flow => "nidoo/automation/<flow id>",
            FlowLabels => "nidoo/flow_labels",
            VirtualConfig => {
                "<first discovery prefix>/<switch|number|button|select>/virtual/<id>/config"
            }
            VirtualState => "<first discovery prefix>/virtual/<id>",
            WeatherAny => {
                "<first discovery prefix>/<sensor|binary_sensor>/weather/<key>/config, <first discovery prefix>/weather/<key>, <first discovery prefix>/weather/availability"
            }
            SolarCache => "<first discovery prefix>/solar/cache",
            SolarOther => {
                "<first discovery prefix>/sensor/solar/<key>/config, <first discovery prefix>/solar/<key>, <first discovery prefix>/solar/availability"
            }
            GroupDefinition => "<first discovery prefix>/group/<id>/definition",
            GroupMigrated => "<first discovery prefix>/group/_migrated",
            GroupOther => {
                "<first discovery prefix>/<component>/group/<id>/config, <first discovery prefix>/group/<id>, <first discovery prefix>/group/<id>/avail"
            }
            NetworkDefinition => "<first discovery prefix>/network/<id>/definition",
            NetworkOther => {
                "<first discovery prefix>/binary_sensor/network/<id>/config, <first discovery prefix>/network/<id>, <first discovery prefix>/network/<id>/avail"
            }
            PushTokens => "nidoo/push/tokens/<installId>",
            PushHistory => "nidoo/push/history",
            ClimateHouse => "nidoo/climate/house",
            ClimateZoneSettings => "nidoo/climate/zone/<zone>/settings",
            ClimateZoneRuntime => "nidoo/climate/zone/<zone>/<doc>",
            AlarmSettings => "<prefix>/settings (alarm)",
            AlarmRuntime => "<prefix>/state, <prefix>/panel, <prefix>/panel/attributes (alarm)",
            AlarmDiscovery => "<first discovery prefix>/alarm_control_panel/alarm/house/config",
        }
    }

    /// The MQTT filter that reads this family; `None` when it is not backed up. A filter may
    /// also match topics of other families (`<ui>/+`): [`classify`] tells them apart.
    pub fn filter(self, l: &Layout) -> Option<String> {
        use Family::*;
        if !self.backed_up() {
            return None;
        }
        Some(match self {
            UiOverride | Areas | Home => format!("{}/+", l.ui),
            Dashboard => format!("{}/dashboards/+", l.ui),
            Ambiance => format!("{}/ambiances/+", l.ui),
            Flow => format!("{}/+", l.automation),
            FlowLabels => l.flow_labels.clone(),
            VirtualConfig => format!("{}/+/virtual/+/config", l.discovery),
            SolarCache => format!("{}/solar/cache", l.discovery),
            GroupDefinition => format!("{}/group/+/definition", l.discovery),
            GroupMigrated => format!("{}/group/_migrated", l.discovery),
            NetworkDefinition => format!("{}/network/+/definition", l.discovery),
            PushTokens => format!("{}/tokens/+", l.push),
            PushHistory => format!("{}/history", l.push),
            ClimateHouse => format!("{}/house", l.climate),
            ClimateZoneSettings => format!("{}/zone/+/settings", l.climate),
            AlarmSettings => format!("{}/settings", l.alarm),
            _ => return None,
        })
    }
}

/// The family of `topic`, or `None` when it is not a retained topic the hub owns.
pub fn classify(topic: &str, l: &Layout) -> Option<Family> {
    if topic == l.flow_labels {
        return Some(Family::FlowLabels);
    }
    if let Some(rest) = under(topic, &l.ui) {
        return classify_ui(rest);
    }
    if let Some(id) = under(topic, &l.automation) {
        return segment(id).then_some(Family::Flow);
    }
    if let Some(rest) = under(topic, &l.push) {
        return match rest.split('/').collect::<Vec<_>>()[..] {
            ["tokens", id] if segment(id) => Some(Family::PushTokens),
            ["history"] => Some(Family::PushHistory),
            _ => None,
        };
    }
    if let Some(rest) = under(topic, &l.climate) {
        return match rest.split('/').collect::<Vec<_>>()[..] {
            ["house"] => Some(Family::ClimateHouse),
            ["zone", zone, "settings"] if segment(zone) => Some(Family::ClimateZoneSettings),
            ["zone", zone, "proposal" | "override" | "decision" | "eco"] if segment(zone) => {
                Some(Family::ClimateZoneRuntime)
            }
            _ => None,
        };
    }
    if let Some(rest) = under(topic, &l.alarm) {
        return match rest {
            "settings" => Some(Family::AlarmSettings),
            "state" | "panel" | "panel/attributes" => Some(Family::AlarmRuntime),
            _ => None,
        };
    }
    classify_discovery(under(topic, &l.discovery)?)
}

fn classify_ui(rest: &str) -> Option<Family> {
    match rest.split('/').collect::<Vec<_>>()[..] {
        ["_areas"] => Some(Family::Areas),
        ["_home"] => Some(Family::Home),
        ["_types"] => Some(Family::Types),
        ["dashboards", id] if segment(id) => Some(Family::Dashboard),
        ["ambiances", id] if segment(id) => Some(Family::Ambiance),
        ["dashboards" | "ambiances"] => None,
        [id] if segment(id) && !id.starts_with('_') => Some(Family::UiOverride),
        _ => None,
    }
}

/// `rest` is what follows the first discovery prefix.
fn classify_discovery(rest: &str) -> Option<Family> {
    let parts: Vec<&str> = rest.split('/').collect();
    if parts.iter().any(|p| p.is_empty()) {
        return None;
    }
    match parts[..] {
        [
            "switch" | "number" | "button" | "select",
            "virtual",
            _,
            "config",
        ] => Some(Family::VirtualConfig),
        ["virtual", _] => Some(Family::VirtualState),
        ["sensor" | "binary_sensor", "weather", _, "config"] | ["weather", _] => {
            Some(Family::WeatherAny)
        }
        ["solar", "cache"] => Some(Family::SolarCache),
        ["sensor", "solar", _, "config"] | ["solar", _] => Some(Family::SolarOther),
        ["group", "_migrated"] => Some(Family::GroupMigrated),
        ["group", _, "definition"] => Some(Family::GroupDefinition),
        [_, "group", _, "config"] | ["group", _] | ["group", _, "avail"] => {
            Some(Family::GroupOther)
        }
        ["network", _, "definition"] => Some(Family::NetworkDefinition),
        ["binary_sensor", "network", _, "config"] | ["network", _] | ["network", _, "avail"] => {
            Some(Family::NetworkOther)
        }
        ["alarm_control_panel", "alarm", "house", "config"] => Some(Family::AlarmDiscovery),
        _ => None,
    }
}

/// What follows `prefix/` in `topic`, when it is non-empty.
fn under<'a>(topic: &'a str, prefix: &str) -> Option<&'a str> {
    if prefix.is_empty() {
        return None;
    }
    topic
        .strip_prefix(prefix)?
        .strip_prefix('/')
        .filter(|rest| !rest.is_empty())
}

/// One non-empty topic level.
fn segment(s: &str) -> bool {
    !s.is_empty() && !s.contains('/')
}

/// The deduped filters of the backed-up families, in family order.
pub fn filters(l: &Layout) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for filter in Family::ALL.iter().filter_map(|f| f.filter(l)) {
        if !out.contains(&filter) {
            out.push(filter);
        }
    }
    out
}

/// MQTT wildcard matching (`+` one level, `#` the rest, last).
#[doc(hidden)]
pub fn matches(filter: &str, topic: &str) -> bool {
    let mut f = filter.split('/');
    let mut t = topic.split('/');
    loop {
        match (f.next(), t.next()) {
            (Some("#"), _) => return true,
            (Some("+"), Some(_)) => {}
            (Some(a), Some(b)) if a == b => {}
            (None, None) => return true,
            _ => return false,
        }
    }
}

/// Reads and rewrites retained documents.
pub trait RetainedBroker: Send + Sync {
    /// Every retained message matching `filters`, once `quiet` passes without a new one.
    fn collect(
        &self,
        filters: Vec<String>,
        quiet: Duration,
    ) -> BoxFuture<'_, Result<Vec<(String, Bytes)>, String>>;

    /// Publishes each document retained; an empty payload clears the topic.
    fn publish_retained(&self, docs: Vec<(String, Bytes)>) -> BoxFuture<'_, Result<(), String>>;
}

/// The broker the hub is configured with, over its own `<client_id>-backup` connection.
pub struct MqttRetained {
    cfg: MqttConfig,
}

impl MqttRetained {
    pub fn new(cfg: MqttConfig) -> Self {
        Self { cfg }
    }

    /// A clean-session client, connected once the broker accepted it.
    async fn connect(&self) -> Result<(AsyncClient, EventLoop), String> {
        let mut options = MqttOptions::new(
            format!("{}-backup", self.cfg.client_id),
            self.cfg.host.clone(),
            self.cfg.port,
        );
        if let Some(user) = &self.cfg.user {
            options.set_credentials(user, self.cfg.password.as_deref().unwrap_or(""));
        }
        options
            .set_keep_alive(KEEP_ALIVE)
            .set_clean_session(true)
            .set_max_packet_size(MAX_PACKET, MAX_PACKET);
        let (client, mut eventloop) = AsyncClient::new(options, WINDOW * 2);
        loop {
            match poll(&mut eventloop).await? {
                Event::Incoming(Packet::ConnAck(ack)) if ack.code == ConnectReturnCode::Success => {
                    return Ok((client, eventloop));
                }
                Event::Incoming(Packet::ConnAck(_)) => {
                    return Err("the broker refused the connection".to_owned());
                }
                _ => {}
            }
        }
    }
}

async fn poll(eventloop: &mut EventLoop) -> Result<Event, String> {
    match tokio::time::timeout(STEP_TIMEOUT, eventloop.poll()).await {
        Ok(Ok(event)) => Ok(event),
        Ok(Err(err)) => Err(format!("broker connection failed: {err}")),
        Err(_) => Err("the broker did not answer in time".to_owned()),
    }
}

impl RetainedBroker for MqttRetained {
    fn collect(
        &self,
        filters: Vec<String>,
        quiet: Duration,
    ) -> BoxFuture<'_, Result<Vec<(String, Bytes)>, String>> {
        Box::pin(async move {
            let (client, mut eventloop) = self.connect().await?;
            let subscriptions = filters
                .into_iter()
                .map(|path| SubscribeFilter::new(path, QoS::AtLeastOnce));
            client
                .subscribe_many(subscriptions)
                .await
                .map_err(|e| format!("could not subscribe: {e}"))?;
            let mut docs = BTreeMap::new();
            // The quiet period starts once the broker acknowledged the subscription.
            let mut deadline: Option<Instant> = None;
            loop {
                let event = match deadline {
                    None => poll(&mut eventloop).await?,
                    Some(at) => match tokio::time::timeout_at(at, eventloop.poll()).await {
                        Err(_) => break,
                        Ok(Ok(event)) => event,
                        Ok(Err(err)) => return Err(format!("broker connection failed: {err}")),
                    },
                };
                match event {
                    Event::Incoming(Packet::SubAck(_)) => deadline = Some(Instant::now() + quiet),
                    Event::Incoming(Packet::Publish(p)) if p.retain && !p.payload.is_empty() => {
                        docs.insert(p.topic.clone(), p.payload.clone());
                        deadline = Some(Instant::now() + quiet);
                    }
                    _ => {}
                }
            }
            let _ = client.try_disconnect();
            Ok(docs.into_iter().collect())
        })
    }

    fn publish_retained(&self, docs: Vec<(String, Bytes)>) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            let (client, mut eventloop) = self.connect().await?;
            let mut pending = docs.into_iter();
            let mut unacknowledged = 0usize;
            let mut more = true;
            while more || unacknowledged > 0 {
                while more && unacknowledged < WINDOW {
                    match pending.next() {
                        Some((topic, payload)) => {
                            client
                                .try_publish(topic, QoS::AtLeastOnce, true, payload)
                                .map_err(|e| format!("could not publish: {e}"))?;
                            unacknowledged += 1;
                        }
                        None => more = false,
                    }
                }
                if let Event::Incoming(Packet::PubAck(_)) = poll(&mut eventloop).await? {
                    unacknowledged -= 1;
                }
            }
            let _ = client.try_disconnect();
            Ok(())
        })
    }
}

/// An in-memory broker for tests (also used by the integration tests).
#[doc(hidden)]
#[derive(Default)]
pub struct MemBroker(pub Mutex<BTreeMap<String, Bytes>>);

impl RetainedBroker for MemBroker {
    fn collect(
        &self,
        filters: Vec<String>,
        _quiet: Duration,
    ) -> BoxFuture<'_, Result<Vec<(String, Bytes)>, String>> {
        Box::pin(async move {
            let docs = self.0.lock().unwrap_or_else(PoisonError::into_inner);
            Ok(docs
                .iter()
                .filter(|(topic, _)| filters.iter().any(|f| matches(f, topic)))
                .map(|(t, p)| (t.clone(), p.clone()))
                .collect())
        })
    }

    fn publish_retained(&self, docs: Vec<(String, Bytes)>) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            let mut store = self.0.lock().unwrap_or_else(PoisonError::into_inner);
            for (topic, payload) in docs {
                if payload.is_empty() {
                    store.remove(&topic);
                } else {
                    store.insert(topic, payload);
                }
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> Layout {
        Layout::from_config(&crate::config::Config::default())
    }

    /// One concrete topic per family under the default layout.
    const EXAMPLES: &[(&str, Family)] = &[
        ("nidoo/ui/light_kitchen", Family::UiOverride),
        ("nidoo/ui/_areas", Family::Areas),
        ("nidoo/ui/_home", Family::Home),
        ("nidoo/ui/_types", Family::Types),
        ("nidoo/ui/dashboards/main", Family::Dashboard),
        ("nidoo/ui/ambiances/movie", Family::Ambiance),
        ("nidoo/automation/night", Family::Flow),
        ("nidoo/flow_labels", Family::FlowLabels),
        ("nidoo/switch/virtual/guest/config", Family::VirtualConfig),
        ("nidoo/number/virtual/level/config", Family::VirtualConfig),
        ("nidoo/virtual/guest", Family::VirtualState),
        (
            "nidoo/sensor/weather/temperature/config",
            Family::WeatherAny,
        ),
        ("nidoo/weather/temperature", Family::WeatherAny),
        ("nidoo/weather/availability", Family::WeatherAny),
        ("nidoo/solar/cache", Family::SolarCache),
        ("nidoo/sensor/solar/today/config", Family::SolarOther),
        ("nidoo/solar/today", Family::SolarOther),
        ("nidoo/solar/availability", Family::SolarOther),
        ("nidoo/group/lights/definition", Family::GroupDefinition),
        ("nidoo/group/_migrated", Family::GroupMigrated),
        ("nidoo/light/group/lights/config", Family::GroupOther),
        ("nidoo/group/lights", Family::GroupOther),
        ("nidoo/group/lights/avail", Family::GroupOther),
        ("nidoo/network/router/definition", Family::NetworkDefinition),
        (
            "nidoo/binary_sensor/network/router/config",
            Family::NetworkOther,
        ),
        ("nidoo/network/router", Family::NetworkOther),
        ("nidoo/network/router/avail", Family::NetworkOther),
        ("nidoo/push/tokens/abc", Family::PushTokens),
        ("nidoo/push/history", Family::PushHistory),
        ("nidoo/climate/house", Family::ClimateHouse),
        (
            "nidoo/climate/zone/living/settings",
            Family::ClimateZoneSettings,
        ),
        (
            "nidoo/climate/zone/living/proposal",
            Family::ClimateZoneRuntime,
        ),
        (
            "nidoo/climate/zone/living/override",
            Family::ClimateZoneRuntime,
        ),
        (
            "nidoo/climate/zone/living/decision",
            Family::ClimateZoneRuntime,
        ),
        ("nidoo/climate/zone/living/eco", Family::ClimateZoneRuntime),
        ("nidoo/alarm/settings", Family::AlarmSettings),
        ("nidoo/alarm/state", Family::AlarmRuntime),
        ("nidoo/alarm/panel", Family::AlarmRuntime),
        ("nidoo/alarm/panel/attributes", Family::AlarmRuntime),
        (
            "nidoo/alarm_control_panel/alarm/house/config",
            Family::AlarmDiscovery,
        ),
    ];

    #[test]
    fn classify_examples() {
        let l = layout();
        for (topic, family) in EXAMPLES {
            assert_eq!(classify(topic, &l), Some(*family), "{topic}");
        }
        for family in Family::ALL {
            assert!(
                EXAMPLES.iter().any(|(_, f)| f == family),
                "no example for {family:?}"
            );
        }
        for topic in [
            "nidoo/sensor/kitchen_temp/config",
            "nidoo/climate/zone/living/hold/set",
            "nidoo/alarm/panel/set",
            "nidoo/push/responses",
            "nidoo/push/notification/frigate",
            "nidoo/ui/dashboards",
            "nidoo/ui/_other",
            "nidoo/ui/a/b",
            "zigbee2mqtt/bridge/devices",
            "homeassistant/switch/virtual/guest/config",
            "nidoo-sound/s1/volume",
        ] {
            assert_eq!(classify(topic, &l), None, "{topic}");
        }
    }

    #[test]
    fn classify_follows_a_custom_layout() {
        let mut cfg = crate::config::Config::default();
        cfg.topics.ui_prefix = "home/ui/".into();
        cfg.topics.discovery_prefixes = vec!["ha".into(), "other".into()];
        cfg.climate.prefix = "home/climate".into();
        let l = Layout::from_config(&cfg);
        assert_eq!(classify("home/ui/light_a", &l), Some(Family::UiOverride));
        assert_eq!(
            classify("ha/switch/virtual/a/config", &l),
            Some(Family::VirtualConfig)
        );
        assert_eq!(classify("other/switch/virtual/a/config", &l), None);
        assert_eq!(classify("nidoo/ui/light_a", &l), None);
        assert_eq!(
            classify("home/climate/house", &l),
            Some(Family::ClimateHouse)
        );
    }

    #[test]
    fn filters_catch_every_backed_up_example() {
        let l = layout();
        let filters = filters(&l);
        let mut seen = filters.clone();
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), filters.len(), "filters are deduped");
        for (topic, family) in EXAMPLES.iter().filter(|(_, f)| f.backed_up()) {
            assert!(
                filters.iter().any(|f| matches(f, topic)),
                "{topic} ({family:?}) matches no filter"
            );
        }
        for family in Family::ALL {
            assert_eq!(
                family.filter(&l).is_some(),
                family.backed_up(),
                "{family:?}"
            );
        }
    }

    #[test]
    fn wildcard_matching() {
        assert!(matches("a/+/c", "a/b/c"));
        assert!(!matches("a/+/c", "a/b/c/d"));
        assert!(matches("a/#", "a/b/c"));
        assert!(matches("a/#", "a"));
        assert!(!matches("a/b", "a/b/c"));
    }

    /// The first cells of the table rows, the *Retained* and *Backed up* cells beside them.
    fn doc_rows() -> Vec<(String, String, String)> {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../docs/reference/mqtt-topics.md"
        );
        let text = std::fs::read_to_string(path).expect("read mqtt-topics.md");
        let mut rows = Vec::new();
        for line in text.lines().filter(|l| l.starts_with("| ")) {
            let cells = split_row(line);
            if cells.len() < 2 || cells[0] == "Topic" || cells[0].starts_with("---") {
                continue;
            }
            assert_eq!(cells.len(), 6, "a row needs six cells: {line}");
            rows.push((
                cells[0].replace('`', ""),
                cells[2].clone(),
                cells[5].clone(),
            ));
        }
        rows
    }

    /// Splits on unescaped `|`; `\|` becomes `|`.
    fn split_row(line: &str) -> Vec<String> {
        let mut cells = vec![String::new()];
        let mut chars = line.trim().trim_start_matches('|').chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '\\' if chars.peek() == Some(&'|') => {
                    chars.next();
                    cells.last_mut().unwrap().push('|');
                }
                '|' => cells.push(String::new()),
                c => cells.last_mut().unwrap().push(c),
            }
        }
        cells.pop();
        cells.into_iter().map(|c| c.trim().to_owned()).collect()
    }

    #[test]
    fn doc_column_matches_the_code() {
        let rows = doc_rows();
        assert!(rows.len() > 30, "the table was not read");
        for (topic, retained, backed_up) in &rows {
            let owned =
                retained == "yes" || retained == "yes (state)" || retained.starts_with("yes for");
            if !owned {
                assert_eq!(backed_up, "—", "{topic}: not a retained topic of ours");
                continue;
            }
            let family = Family::ALL
                .iter()
                .find(|f| f.doc_topic() == topic)
                .unwrap_or_else(|| panic!("no Family for the row `{topic}`"));
            let expected = if family.backed_up() { "yes" } else { "no" };
            assert_eq!(backed_up, expected, "{topic}");
        }
        for family in Family::ALL {
            assert!(
                rows.iter().any(|(t, _, _)| t == family.doc_topic()),
                "{family:?} has no row `{}`",
                family.doc_topic()
            );
        }
    }

    #[tokio::test]
    async fn mem_broker_collects_and_clears() {
        let broker = MemBroker::default();
        broker
            .publish_retained(vec![
                ("nidoo/automation/a".into(), Bytes::from_static(b"1")),
                ("nidoo/automation/b".into(), Bytes::from_static(b"2")),
                ("nidoo/other/c".into(), Bytes::from_static(b"3")),
            ])
            .await
            .unwrap();
        let got = broker
            .collect(vec!["nidoo/automation/+".into()], QUIET)
            .await
            .unwrap();
        assert_eq!(got.len(), 2);
        assert!(
            got.iter()
                .any(|(t, p)| t == "nidoo/automation/a" && p == "1")
        );
        broker
            .publish_retained(vec![("nidoo/automation/a".into(), Bytes::new())])
            .await
            .unwrap();
        let got = broker
            .collect(vec!["nidoo/automation/+".into()], QUIET)
            .await
            .unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, "nidoo/automation/b");
    }
}
