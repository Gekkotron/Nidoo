//! The phone setup code: the `v:1` JSON the Android app reads from a QR, built from the hub's
//! own settings, and the checks on the optional gateway the phone uses away from home.

use std::fmt;
use std::net::IpAddr;

use nidoo_api::{MobileInput, MobileView, SecretView};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::config::MqttConfig;
use crate::setup::secret;
use crate::setup::sections::Sections;

const BAD_URL: &str = "The gateway address must start with https:// and name a host";
const BAD_KEY: &str = "The gateway key must be 64 hex characters";

/// What the phone needs beyond the other sections: the gateway it reaches the house through
/// when away.
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MobileSection {
    pub gateway_url: Option<String>,
    pub gateway_key: Option<String>,
}

impl fmt::Debug for MobileSection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let key = self.gateway_key.as_ref().map(|_| "***");
        f.debug_struct("MobileSection")
            .field("gateway_url", &self.gateway_url)
            .field("gateway_key", &key)
            .finish()
    }
}

/// A read of the section: whether a key is stored, never the key.
pub fn view(s: &MobileSection) -> MobileView {
    MobileView {
        gateway_url: s.gateway_url.clone(),
        gateway_key: SecretView {
            set: s.gateway_key.is_some(),
        },
        gateway_ready: s.gateway_url.is_some() && s.gateway_key.is_some(),
    }
}

/// The section after a `PUT`, or the fixed copy saying what is wrong.
pub fn apply_input(
    stored: &MobileSection,
    input: MobileInput,
) -> Result<MobileSection, &'static str> {
    let gateway_url = match input.gateway_url.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(text) => Some(normalize_gateway_url(text).ok_or(BAD_URL)?),
    };
    let gateway_key = secret::apply(stored.gateway_key.clone(), input.gateway_key);
    let gateway_key = match gateway_key {
        Some(k) => {
            let k = k.trim();
            if k.len() != 64 || !k.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(BAD_KEY);
            }
            Some(k.to_ascii_lowercase())
        }
        None => None,
    };
    Ok(MobileSection {
        gateway_url,
        gateway_key,
    })
}

/// The gateway address as the app's parser wants it, or `None` when it is not an https URL
/// with a host. The text is stored in its normalized form (lower-case scheme, punycode host,
/// escaped path), so a capitalised `Https://` or a sloppy `https:host` typed on a phone never
/// becomes a code the app refuses; a bare trailing `/` is dropped.
fn normalize_gateway_url(text: &str) -> Option<String> {
    let u = url::Url::parse(text).ok()?;
    let ok =
        u.scheme() == "https" && u.host_str().is_some_and(|h| !h.is_empty()) && u.port() != Some(0);
    if !ok {
        return None;
    }
    let s = u.as_str();
    let bare_root = u.path() == "/" && u.query().is_none() && u.fragment().is_none();
    Some(
        if bare_root {
            s.trim_end_matches('/')
        } else {
            s
        }
        .to_owned(),
    )
}

/// Whether the app takes `url` as a Frigate address: `http(s)://host[:port]` with an ASCII
/// host (letters, digits, `.`, `_`, `-`, or a bracketed IPv6) and at most a trailing `/`
/// (Nidoo-app `isHistoryUrl`). Anything else would make the app refuse the whole code.
fn app_takes_frigate_url(url: &str) -> bool {
    let url = url.trim();
    let Some(rest) = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
    else {
        return false;
    };
    let rest = rest.strip_suffix('/').unwrap_or(rest);
    let (host, port) = if let Some(v6) = rest.strip_prefix('[') {
        let Some((inside, after)) = v6.split_once(']') else {
            return false;
        };
        if inside.is_empty() || !inside.chars().all(|c| c.is_ascii_hexdigit() || c == ':') {
            return false;
        }
        match after {
            "" => ("v6", None),
            p => match p.strip_prefix(':') {
                Some(p) => ("v6", Some(p)),
                None => return false,
            },
        }
    } else {
        match rest.split_once(':') {
            Some((h, p)) => (h, Some(p)),
            None => (rest, None),
        }
    };
    let host_ok = !host.is_empty()
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    let port_ok = port.is_none_or(|p| {
        (1..=5).contains(&p.len())
            && p.bytes().all(|b| b.is_ascii_digit())
            && p.parse::<u32>().is_ok_and(|n| (1..=65535).contains(&n))
    });
    host_ok && port_ok
}

/// The broker host a phone can reach: the configured one, unless it only means something
/// inside the hub's machine (loopback or a bare Docker service name) and the browser's own
/// host is known.
pub fn phone_host<'a>(broker_host: &'a str, request_host: Option<&'a str>) -> &'a str {
    let parsed = broker_host.parse::<IpAddr>();
    let unreachable = broker_host == "localhost"
        || match &parsed {
            Ok(ip) => ip.is_loopback(),
            Err(_) => !broker_host.contains('.') && broker_host != "::1",
        }
        || broker_host == "::1";
    if !unreachable {
        return broker_host;
    }
    let Some(raw) = request_host.filter(|h| !h.is_empty()) else {
        return broker_host;
    };
    let host = if let Some(rest) = raw.strip_prefix('[') {
        rest.split(']').next().unwrap_or(rest)
    } else if raw.matches(':').count() == 1 {
        raw.split(':').next().unwrap_or(raw)
    } else {
        raw
    };
    if host.is_empty() { broker_host } else { host }
}

/// The JSON the QR encodes, compact.
pub fn build_payload(mqtt: &MqttConfig, sections: &Sections, request_host: Option<&str>) -> String {
    let host = phone_host(&mqtt.host, request_host);
    let mut local = Map::new();
    local.insert("host".into(), json!(host));
    local.insert("port".into(), json!(mqtt.port));
    local.insert("tls".into(), json!(false));
    local.insert("ws".into(), json!(false));
    if let Some(user) = mqtt.user.as_deref().filter(|u| !u.is_empty()) {
        local.insert("user".into(), json!(user));
        if let Some(pass) = &mqtt.password {
            local.insert("pass".into(), json!(pass));
        }
    }
    let mut root = Map::new();
    root.insert("v".into(), json!(1));
    root.insert("local".into(), Value::Object(local));
    let m = &sections.mobile;
    if let (Some(url), Some(key)) = (&m.gateway_url, &m.gateway_key) {
        root.insert(
            "gateway".into(),
            json!({"url": url, "key": key, "broker_host": host}),
        );
    }
    let f = &sections.frigate;
    if let Some(url) = f.url.as_deref().filter(|u| app_takes_frigate_url(u)) {
        let mut frigate = Map::new();
        frigate.insert("url".into(), json!(url));
        if let Some(user) = f.user.as_deref().filter(|u| !u.is_empty()) {
            frigate.insert("user".into(), json!(user));
            if let Some(pass) = &f.password {
                frigate.insert("pass".into(), json!(pass));
            }
        }
        root.insert("frigate".into(), Value::Object(frigate));
    }
    root.insert(
        "push".into(),
        json!({"token_topic": format!("{}/tokens/{{installId}}", sections.push.prefix)}),
    );
    serde_json::to_string(&Value::Object(root)).expect("a payload serializes")
}

#[cfg(test)]
mod tests {
    use super::*;
    use nidoo_api::Secret;

    const KEY: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn mqtt() -> MqttConfig {
        MqttConfig {
            host: "192.0.2.7".into(),
            port: 1884,
            user: Some("u".into()),
            password: Some("p".into()),
            ..MqttConfig::default()
        }
    }

    fn payload(m: &MqttConfig, s: &Sections, host: Option<&str>) -> Value {
        serde_json::from_str(&build_payload(m, s, host)).unwrap()
    }

    fn gateway_sections() -> Sections {
        let mut s = Sections::default();
        s.mobile.gateway_url = Some("https://house.example.ts.net".into());
        s.mobile.gateway_key = Some(KEY.into());
        s
    }

    #[test]
    fn phone_host_keeps_a_lan_address() {
        assert_eq!(phone_host("192.0.2.7", Some("192.0.2.5:8081")), "192.0.2.7");
        assert_eq!(phone_host("broker.lan", None), "broker.lan");
    }

    #[test]
    fn phone_host_swaps_loopback_and_docker_names() {
        for h in ["localhost", "127.0.0.1", "127.1.2.3", "::1", "mosquitto"] {
            assert_eq!(phone_host(h, Some("192.0.2.5:8081")), "192.0.2.5");
        }
        assert_eq!(
            phone_host("localhost", Some("[2001:db8::5]:8081")),
            "2001:db8::5"
        );
        assert_eq!(phone_host("localhost", None), "localhost");
        assert_eq!(phone_host("localhost", Some("")), "localhost");
    }

    #[test]
    fn payload_broker_only() {
        let v = payload(&mqtt(), &Sections::default(), None);
        assert_eq!(
            v,
            json!({"v":1,"local":{"host":"192.0.2.7","port":1884,"tls":false,"ws":false,"user":"u","pass":"p"},
                "push":{"token_topic":"nidoo/push/tokens/{installId}"}})
        );
    }

    #[test]
    fn payload_is_compact() {
        assert!(!build_payload(&mqtt(), &Sections::default(), None).contains(' '),);
    }

    #[test]
    fn payload_without_login_leaves_user_and_pass_out() {
        let m = MqttConfig {
            user: None,
            password: None,
            ..mqtt()
        };
        let v = payload(&m, &Sections::default(), None);
        assert!(v["local"].get("user").is_none());
        assert!(v["local"].get("pass").is_none());
    }

    #[test]
    fn payload_with_gateway() {
        let v = payload(&mqtt(), &gateway_sections(), None);
        assert_eq!(
            v["gateway"],
            json!({"url":"https://house.example.ts.net","key":KEY,"broker_host":"192.0.2.7"})
        );
    }

    #[test]
    fn payload_needs_both_url_and_key() {
        let mut s = Sections::default();
        s.mobile.gateway_url = Some("https://house.example.ts.net".into());
        assert!(payload(&mqtt(), &s, None).get("gateway").is_none());
        let mut s = Sections::default();
        s.mobile.gateway_key = Some(KEY.into());
        assert!(payload(&mqtt(), &s, None).get("gateway").is_none());
    }

    #[test]
    fn gateway_broker_host_follows_phone_host() {
        let m = MqttConfig {
            host: "localhost".into(),
            ..mqtt()
        };
        let v = payload(&m, &gateway_sections(), Some("192.0.2.5:8081"));
        assert_eq!(v["local"]["host"], "192.0.2.5");
        assert_eq!(v["gateway"]["broker_host"], "192.0.2.5");
    }

    #[test]
    fn frigate_with_login() {
        let mut s = Sections::default();
        s.frigate.url = Some("http://192.0.2.8:8971".into());
        s.frigate.user = Some("f".into());
        s.frigate.password = Some("fp".into());
        assert_eq!(
            payload(&mqtt(), &s, None)["frigate"],
            json!({"url":"http://192.0.2.8:8971","user":"f","pass":"fp"})
        );
    }

    #[test]
    fn frigate_pass_without_user_is_dropped() {
        let mut s = Sections::default();
        s.frigate.url = Some("http://192.0.2.8:8971".into());
        s.frigate.password = Some("fp".into());
        assert_eq!(
            payload(&mqtt(), &s, None)["frigate"],
            json!({"url":"http://192.0.2.8:8971"})
        );
    }

    #[test]
    fn no_frigate_when_url_empty() {
        let mut s = Sections::default();
        assert!(payload(&mqtt(), &s, None).get("frigate").is_none());
        s.frigate.url = Some(String::new());
        assert!(payload(&mqtt(), &s, None).get("frigate").is_none());
    }

    #[test]
    fn frigate_the_app_refuses_is_left_out() {
        let mut s = Sections::default();
        for bad in [
            "http://192.0.2.8:5000/frigate",
            "http://caméra.example:5000",
            "ftp://192.0.2.8",
            "http://192.0.2.8:0",
            "http://192.0.2.8:70000",
        ] {
            s.frigate.url = Some(bad.into());
            assert!(payload(&mqtt(), &s, None).get("frigate").is_none(), "{bad}");
        }
        for good in [
            "http://192.0.2.8:5000/",
            "https://nvr.example:8971",
            "http://[2001:db8::8]:5000",
        ] {
            s.frigate.url = Some(good.into());
            assert_eq!(payload(&mqtt(), &s, None)["frigate"]["url"], good, "{good}");
        }
    }

    #[test]
    fn gateway_url_is_stored_as_the_app_reads_it() {
        let none = MobileSection::default();
        for (typed, stored) in [
            (
                "Https://house.example.ts.net",
                "https://house.example.ts.net",
            ),
            (
                "HTTPS://house.example.ts.net/",
                "https://house.example.ts.net",
            ),
            ("https:house.example.ts.net", "https://house.example.ts.net"),
            (
                "https://house.example.ts.net/a b",
                "https://house.example.ts.net/a%20b",
            ),
        ] {
            let got = apply_input(&none, input(Some(typed), Secret::Keep)).unwrap();
            assert_eq!(got.gateway_url.as_deref(), Some(stored), "{typed}");
        }
    }

    #[test]
    fn push_topic_follows_prefix() {
        let mut s = Sections::default();
        s.push.prefix = "house/push".into();
        assert_eq!(
            payload(&mqtt(), &s, None)["push"]["token_topic"],
            "house/push/tokens/{installId}"
        );
    }

    #[test]
    fn never_writes_prefix_or_history() {
        let v = payload(&mqtt(), &gateway_sections(), None);
        assert!(v.get("prefix").is_none());
        assert!(v.get("history").is_none());
    }

    fn input(url: Option<&str>, key: Secret) -> MobileInput {
        MobileInput {
            gateway_url: url.map(str::to_owned),
            gateway_key: key,
        }
    }

    #[test]
    fn apply_input_rules() {
        let none = MobileSection::default();
        for bad in ["http://x", "https://", "https://h:0", "not a url"] {
            assert_eq!(
                apply_input(&none, input(Some(bad), Secret::Keep)),
                Err(BAD_URL)
            );
        }
        let ok = apply_input(
            &none,
            input(Some("https://house.example.ts.net:8443"), Secret::Keep),
        )
        .unwrap();
        assert_eq!(
            ok.gateway_url.as_deref(),
            Some("https://house.example.ts.net:8443")
        );
        for bad in ["a".repeat(63), "a".repeat(65), "z".repeat(64)] {
            assert_eq!(
                apply_input(&none, input(None, Secret::Set(bad))),
                Err(BAD_KEY)
            );
        }
        let up = apply_input(&none, input(None, Secret::Set("A".repeat(64)))).unwrap();
        assert_eq!(up.gateway_key, Some("a".repeat(64)));
        let stored = MobileSection {
            gateway_url: None,
            gateway_key: Some(KEY.into()),
        };
        assert_eq!(
            apply_input(&stored, input(Some("  "), Secret::Keep)).unwrap(),
            stored
        );
        assert_eq!(
            apply_input(&stored, input(None, Secret::Clear)).unwrap(),
            MobileSection::default()
        );
    }

    #[test]
    fn view_never_carries_the_key() {
        let mut s = MobileSection {
            gateway_url: Some("https://house.example.ts.net".into()),
            gateway_key: None,
        };
        assert!(!view(&s).gateway_ready);
        s.gateway_key = Some(KEY.into());
        let v = view(&s);
        assert!(v.gateway_ready && v.gateway_key.set);
        assert!(!serde_json::to_string(&v).unwrap().contains(KEY));
    }

    #[test]
    fn debug_masks_the_key() {
        let s = MobileSection {
            gateway_url: None,
            gateway_key: Some(KEY.into()),
        };
        assert!(!format!("{s:?}").contains(KEY));
    }
}
