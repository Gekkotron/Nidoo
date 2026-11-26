//! The phone setup code: the `v:1` JSON the Android app reads from a QR, built from the hub's
//! own settings, and the checks on the optional gateway the phone uses away from home.

use std::fmt;
use std::net::IpAddr;

use nidoo_api::{MobileInput, MobileView, SecretView};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::config::MqttConfig;
use crate::setup::bundled::Bundled;
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

/// The broker settings the phone gets: the bundled app login, which the broker ACL keeps away
/// from alerts, alarm settings and flow labels, when the hub itself uses the bundled login;
/// the hub's own settings otherwise (an own broker, or a `bundled.json` from before the app
/// login).
pub fn phone_mqtt(mqtt: &MqttConfig, bundled: Option<&Bundled>) -> MqttConfig {
    let mut out = mqtt.clone();
    let Some(b) = bundled else {
        return out;
    };
    let on_bundled = mqtt.user.as_deref() == Some(b.mqtt_user.as_str())
        && mqtt.password.as_deref() == Some(b.mqtt_password.as_str());
    if let (true, Some((user, password))) = (on_bundled, b.app_login()) {
        out.user = Some(user.to_owned());
        out.password = Some(password.to_owned());
    }
    out
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
mod tests;
