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

fn bundled(app: bool) -> Bundled {
    Bundled {
        mqtt_user: "u".into(),
        mqtt_password: "p".into(),
        db_user: "nidoo".into(),
        db_password: "d".into(),
        db_name: "nidoo".into(),
        app_mqtt_user: app.then(|| "nidoo-app".into()),
        app_mqtt_password: app.then(|| "ap".into()),
    }
}

#[test]
fn phone_gets_the_app_login_on_the_bundled_broker() {
    let b = bundled(true);
    let m = phone_mqtt(&mqtt(), Some(&b));
    assert_eq!(m.user.as_deref(), Some("nidoo-app"));
    assert_eq!(m.password.as_deref(), Some("ap"));
    assert_eq!(m.host, "192.0.2.7");
    assert_eq!(m.port, 1884);
    let v = payload(&m, &Sections::default(), None);
    assert_eq!(v["local"]["user"], "nidoo-app");
    assert_eq!(v["local"]["pass"], "ap");
}

#[test]
fn phone_keeps_the_hub_login_otherwise() {
    let hub = mqtt();
    assert_eq!(phone_mqtt(&hub, None), hub);
    assert_eq!(phone_mqtt(&hub, Some(&bundled(false))), hub);
    let own = MqttConfig {
        user: Some("me".into()),
        ..mqtt()
    };
    assert_eq!(phone_mqtt(&own, Some(&bundled(true))), own);
    let changed = MqttConfig {
        password: Some("other".into()),
        ..mqtt()
    };
    assert_eq!(phone_mqtt(&changed, Some(&bundled(true))), changed);
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
