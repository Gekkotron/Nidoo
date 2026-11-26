use super::*;
use serde_json::json;

const NOW: u64 = 1_700_000_000;
const SRC: &str = "frigate";

fn n(payload: Value) -> Result<Alert, Rejected> {
    normalize(SRC, &payload, NOW)
}
fn ok(payload: Value) -> Alert {
    n(payload).expect("alert")
}
fn fcm_size(data: &BTreeMap<String, String>) -> usize {
    size(data)
}
fn act(id: &str, label: &str) -> Action {
    Action {
        id: id.into(),
        label: label.into(),
    }
}
fn alert(f: impl FnOnce(&mut Alert)) -> Alert {
    let mut a = Alert {
        id: "x".into(),
        title: "T".into(),
        ..Alert::default()
    };
    f(&mut a);
    a
}

// ---------- v1 ----------

#[test]
fn v1_every_field() {
    let a = ok(json!({
        "v": 1, "id": "frigate-front-1696", "channel": "camera", "severity": "critical",
        "title": "Front door", "body": "Person detected",
        "image_url": "http://192.168.1.20:5000/api/events/1696/snapshot.jpg",
        "video_url": "http://192.168.1.20:5000/api/events/1696/clip.mp4",
        "entity_id": "binary_sensor.front_door", "camera": "front_door",
        "actions": [{"id": "disarm", "label": "Disarm"}],
        "silent": true, "ttl": 600,
    }));
    assert_eq!(
        a,
        Alert {
            id: "frigate-front-1696".into(),
            channel: "camera".into(),
            severity: "critical".into(),
            title: "Front door".into(),
            body: "Person detected".into(),
            image_url: Some("http://192.168.1.20:5000/api/events/1696/snapshot.jpg".into()),
            video_url: Some("http://192.168.1.20:5000/api/events/1696/clip.mp4".into()),
            entity_id: Some("binary_sensor.front_door".into()),
            actions: vec![act("disarm", "Disarm")],
            silent: true,
            cancel: false,
            ttl: 600,
            sent_at: NOW,
            source: "frigate".into(),
            loc: None,
            camera: Some("front_door".into()),
        }
    );
}

#[test]
fn v1_defaults() {
    let a = ok(json!({"v": 1, "id": "x", "title": "T"}));
    assert_eq!(
        (
            a.channel.as_str(),
            a.severity.as_str(),
            a.body.as_str(),
            a.actions.len(),
            a.silent,
            a.ttl
        ),
        ("misc", "info", "", 0, false, DEFAULT_TTL)
    );
    assert_eq!((a.image_url, a.video_url, a.entity_id), (None, None, None));
    assert_eq!(a.camera, None);
}

#[test]
fn v1_unknown_channel_becomes_misc() {
    for channel in [json!("nope"), json!(null), json!(7), json!(["alarm"])] {
        assert_eq!(
            ok(json!({"v": 1, "id": "x", "title": "T", "channel": channel})).channel,
            "misc"
        );
    }
}

#[test]
fn v1_all_seven_channels_accepted() {
    for channel in CHANNELS {
        assert_eq!(
            ok(json!({"v": 1, "id": "x", "title": "T", "channel": channel})).channel,
            channel
        );
    }
}

#[test]
fn v1_unknown_severity_becomes_info() {
    for severity in [json!("urgent"), json!(null), json!(3)] {
        assert_eq!(
            ok(json!({"v": 1, "id": "x", "title": "T", "severity": severity})).severity,
            "info"
        );
    }
}

#[test]
fn v1_bad_ttl_uses_default() {
    for ttl in [json!(0), json!(-5), json!("600"), json!(null), json!(true)] {
        assert_eq!(
            ok(json!({"v": 1, "id": "x", "title": "T", "ttl": ttl})).ttl,
            DEFAULT_TTL
        );
    }
}

#[test]
fn v1_numeric_id_becomes_string() {
    assert_eq!(ok(json!({"v": 1, "id": 1696, "title": "T"})).id, "1696");
}

#[test]
fn v1_invalid_actions_entries_are_dropped() {
    let a = ok(json!({"v": 1, "id": "x", "title": "T", "actions": [
        {"id": "a", "label": "A"}, {"id": "b"}, {"label": "C"}, "junk", {"id": "", "label": "D"}]}));
    assert_eq!(a.actions, vec![act("a", "A")]);
}

#[test]
fn v1_cancel_needs_only_v_and_id() {
    let a = ok(json!({"v": 1, "id": "frigate-front-1696", "cancel": true}));
    assert_eq!(
        a,
        Alert {
            id: "frigate-front-1696".into(),
            cancel: true,
            sent_at: NOW,
            source: "frigate".into(),
            ..Alert::default()
        }
    );
}

#[test]
fn v1_cancel_drops_other_content() {
    let a = ok(
        json!({"v": 1, "id": "x", "cancel": true, "title": "T", "body": "B",
                      "image_url": "http://x/y.jpg"}),
    );
    assert_eq!(
        (a.title.as_str(), a.body.as_str(), a.image_url, a.cancel),
        ("", "", None, true)
    );
}

// ---------- rejections ----------

#[test]
fn rejected_v1_payloads() {
    for payload in [
        json!({"v": 1, "title": "T"}),
        json!({"v": 1, "id": "", "title": "T"}),
        json!({"v": 1, "id": "x"}),
        json!({"v": 1, "id": "x", "title": "   "}),
        json!({"v": 1, "cancel": true}),
        json!({"v": 2, "id": "x", "title": "T"}),
        json!({"v": "1", "id": "x", "title": "T"}),
        json!({"v": true, "id": "x", "title": "T"}),
        json!({"v": null, "id": "x", "title": "T"}),
    ] {
        assert!(n(payload.clone()).is_err(), "{payload}");
    }
}

#[test]
fn non_object_payloads_are_rejected() {
    for payload in [
        json!([1, 2]),
        json!("text"),
        json!(42),
        json!(null),
        json!(true),
    ] {
        let r = n(payload).unwrap_err();
        assert!(!r.reason.is_empty());
    }
}

// ---------- legacy ----------

#[test]
fn legacy_channel_mapping() {
    for (channel_id, expected) in [
        (json!("alarme"), "alarm"),
        (json!("nvr"), "camera"),
        (json!("pv"), "energy"),
        (json!("surveillance"), "climate"),
        (json!("Supervisor"), "system"),
        (json!("anything-else"), "misc"),
        (json!(null), "misc"),
        (json!(12), "misc"),
    ] {
        let a =
            ok(json!({"title": "T", "body": "B", "data": {"id": "1", "channelId": channel_id}}));
        assert_eq!(a.channel, expected);
    }
}

#[test]
fn legacy_full_example() {
    let payload = json!({
        "title": "Alarme", "body": "Intrusion salon", "priority": "high", "ttl": 900,
        "data": {
            "id": 1696, "channelId": "alarme", "image_url": "http://cam/snap.jpg",
            "video_url": "http://cam/clip.mp4", "silent": "true",
            "actions": "[{\"btn\":\"Désarmer\",\"action\":\"disarm\"},{\"btn\":\"Ignorer\",\"action\":\"ignore\"}]",
            "something_else": "dropped", "color": "red",
        },
    });
    assert_eq!(
        normalize("jeedom", &payload, NOW).unwrap(),
        Alert {
            id: "1696".into(),
            channel: "alarm".into(),
            severity: "warning".into(),
            title: "Alarme".into(),
            body: "Intrusion salon".into(),
            image_url: Some("http://cam/snap.jpg".into()),
            video_url: Some("http://cam/clip.mp4".into()),
            entity_id: None,
            actions: vec![act("disarm", "Désarmer"), act("ignore", "Ignorer")],
            silent: true,
            cancel: false,
            ttl: 900,
            sent_at: NOW,
            source: "jeedom".into(),
            loc: None,
            camera: None,
        }
    );
}

#[test]
fn legacy_id_number_or_string() {
    for (raw, expected) in [
        (json!("abc"), "abc"),
        (json!(42), "42"),
        (json!(42.0), "42"),
    ] {
        assert_eq!(ok(json!({"title": "T", "data": {"id": raw}})).id, expected);
    }
}

#[test]
fn legacy_missing_id_falls_back_to_source_and_sha1() {
    // sha1("Door openSalon")[:8]
    let expected = format!("frigate-{}", sha1_prefix("Door openSalon"));
    assert_eq!(expected.len(), "frigate-".len() + 8);
    for data in [
        json!({}),
        json!({"id": ""}),
        json!({"id": null}),
        json!(null),
        json!("junk"),
    ] {
        let a = ok(json!({"title": "Door open", "body": "Salon", "data": data}));
        assert_eq!(a.id, expected);
    }
}

#[test]
fn sha1_prefix_matches_known_digest() {
    // sha1("abc") = a9993e364706816aba3e25717850c26c9cd0d89d
    assert_eq!(sha1_prefix("abc"), "a9993e36");
}

#[test]
fn legacy_same_content_gives_same_fallback_id() {
    let p = json!({"title": "Door open", "body": "Salon"});
    assert_eq!(ok(p.clone()).id, ok(p).id);
}

#[test]
fn legacy_empty_or_missing_title_is_cancel() {
    for title in [Some(""), None, Some("   ")] {
        let mut payload = json!({"body": "x", "data": {"id": "77"}});
        if let Some(t) = title {
            payload["title"] = json!(t);
        }
        let a = ok(payload);
        assert_eq!(
            (a.id.as_str(), a.cancel, a.title.as_str(), a.body.as_str()),
            ("77", true, "", "")
        );
    }
}

#[test]
fn legacy_priority_maps_to_severity() {
    for (priority, severity) in [
        (Some("high"), "warning"),
        (Some("normal"), "info"),
        (None, "info"),
    ] {
        let mut payload = json!({"title": "T"});
        if let Some(p) = priority {
            payload["priority"] = json!(p);
        }
        assert_eq!(ok(payload).severity, severity);
    }
}

#[test]
fn legacy_bad_actions_json_gives_no_actions() {
    assert!(
        ok(json!({"title": "T", "data": {"id": "1", "actions": "not json"}}))
            .actions
            .is_empty()
    );
}

#[test]
fn legacy_actions_may_already_be_a_list() {
    let a = ok(
        json!({"title": "T", "data": {"id": "1", "actions": [{"btn": "Go", "action": "go"}, {"btn": "x"}]}}),
    );
    assert_eq!(a.actions, vec![act("go", "Go")]);
}

#[test]
fn legacy_defaults() {
    let a = ok(json!({"title": "T"}));
    assert_eq!(
        (a.ttl, a.silent, a.entity_id, a.image_url),
        (DEFAULT_TTL, false, None, None)
    );
}

// ---------- to_fcm_data ----------

fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[test]
fn to_fcm_data_full_map_is_all_strings() {
    let a = Alert {
        id: "frigate-front-1696".into(),
        channel: "camera".into(),
        severity: "critical".into(),
        title: "Front door".into(),
        body: "Person detected".into(),
        image_url: Some("http://i/s.jpg".into()),
        video_url: Some("http://i/c.mp4".into()),
        entity_id: Some("binary_sensor.front_door".into()),
        actions: vec![act("disarm", "Disarm")],
        silent: false,
        sent_at: NOW,
        source: "frigate".into(),
        ..Alert::default()
    };
    assert_eq!(
        to_fcm_data(&a),
        map(&[
            ("v", "1"),
            ("id", "frigate-front-1696"),
            ("channel", "camera"),
            ("severity", "critical"),
            ("title", "Front door"),
            ("body", "Person detected"),
            ("image_url", "http://i/s.jpg"),
            ("video_url", "http://i/c.mp4"),
            ("entity_id", "binary_sensor.front_door"),
            ("actions", r#"[{"id":"disarm","label":"Disarm"}]"#),
            ("silent", "false"),
            ("sent_at", &NOW.to_string()),
            ("source", "frigate"),
        ])
    );
}

#[test]
fn to_fcm_data_leaves_out_absent_optionals_and_has_empty_actions() {
    let data = to_fcm_data(&alert(|a| {
        a.sent_at = NOW;
        a.source = "s".into();
    }));
    assert!(
        !data.contains_key("image_url")
            && !data.contains_key("video_url")
            && !data.contains_key("entity_id")
    );
    assert!(!data.contains_key("cancel"));
    assert_eq!(data["actions"], "[]");
    assert_eq!(data["silent"], "false");
}

#[test]
fn to_fcm_data_cancel_and_silent() {
    let data = to_fcm_data(&Alert {
        id: "x".into(),
        cancel: true,
        silent: true,
        sent_at: NOW,
        source: "s".into(),
        ..Alert::default()
    });
    assert_eq!(data["cancel"], "true");
    assert_eq!(data["silent"], "true");
    assert!(data["title"].is_empty() && data["body"].is_empty());
}

#[test]
fn to_fcm_data_keeps_non_ascii_unescaped() {
    let data = to_fcm_data(&alert(|a| a.actions = vec![act("a", "Désarmer")]));
    assert!(data["actions"].contains("Désarmer"));
}

#[test]
fn small_body_is_not_truncated() {
    let data = to_fcm_data(&alert(|a| a.body = "hello".into()));
    assert_eq!(data["body"], "hello");
}

#[test]
fn truncation_ascii_body_fits_under_limit() {
    let data = to_fcm_data(&alert(|a| a.body = "a".repeat(10_000)));
    assert!(fcm_size(&data) <= FCM_MAX_BYTES);
    assert!(data["body"].ends_with('…'));
    assert!(data["body"].starts_with(&"a".repeat(100)));
}

#[test]
fn truncation_multibyte_body_stays_valid_utf8_and_under_limit() {
    for body in [
        "é".repeat(5000),
        "🔥".repeat(3000),
        format!("é{}{}", "a".repeat(5000), "漢".repeat(100)),
    ] {
        let data = to_fcm_data(&alert(|a| {
            a.title = "Titre é".into();
            a.body = body.clone();
        }));
        assert!(fcm_size(&data) <= FCM_MAX_BYTES);
        assert!(data["body"].ends_with('…'));
        assert!(body.starts_with(data["body"].trim_end_matches('…')));
    }
}

#[test]
fn ttl_constants() {
    assert_eq!((DEFAULT_TTL, CANCEL_TTL, FCM_MAX_BYTES), (43200, 300, 4096));
    assert_eq!(MAX_TTL, 2_419_200);
}

#[test]
fn fit_truncates_a_huge_title_after_the_body() {
    let data = to_fcm_data(&alert(|a| {
        a.title = "é".repeat(3000);
        a.body = "b".repeat(3000);
    }));
    assert!(fcm_size(&data) <= FCM_MAX_BYTES);
    assert!(data["title"].ends_with('…') && data["body"].is_empty());
}

#[test]
fn fit_truncates_a_title_alone_when_body_is_empty() {
    let data = to_fcm_data(&alert(|a| a.title = "é".repeat(3000)));
    assert!(fcm_size(&data) <= FCM_MAX_BYTES);
    assert!(data["title"].ends_with('…'));
}

#[test]
fn fit_drops_actions_then_optional_fields_as_last_resort() {
    let data = to_fcm_data(&alert(|a| {
        a.actions = (0..200)
            .map(|i| act(&i.to_string(), &"L".repeat(50)))
            .collect();
        a.entity_id = Some("e".repeat(3000));
        a.video_url = Some("v".repeat(3000));
        a.image_url = Some("i".repeat(3000));
    }));
    assert!(fcm_size(&data) <= FCM_MAX_BYTES);
    assert_eq!(data["actions"], "[]");
    assert!(!data.contains_key("entity_id") && !data.contains_key("video_url"));
}

#[test]
fn fit_guarantees_the_limit_with_everything_oversized() {
    let data = to_fcm_data(&alert(|a| {
        a.title = "é".repeat(3000);
        a.body = "é".repeat(3000);
        a.entity_id = Some("e".repeat(5000));
        a.video_url = Some("v".repeat(5000));
        a.image_url = Some("i".repeat(5000));
    }));
    assert!(fcm_size(&data) <= FCM_MAX_BYTES);
}

#[test]
fn ttl_is_clamped_to_fcm_max_four_weeks() {
    assert_eq!(
        ok(json!({"v": 1, "id": "x", "title": "T", "ttl": 1_000_000_000u64})).ttl,
        2_419_200
    );
}

#[test]
fn non_finite_ttl_uses_default() {
    // JSON cannot carry inf/nan; serde_json turns them into null, which is the default too.
    for ttl in [f64::INFINITY, f64::NEG_INFINITY, f64::NAN] {
        assert_eq!(
            ok(json!({"v": 1, "id": "x", "title": "T", "ttl": ttl})).ttl,
            DEFAULT_TTL
        );
    }
}

// ---------- loc ----------

fn loc_value() -> Value {
    json!({"key": "frigate.object", "args": {"label": "car", "zone": "Driveway"}})
}

fn v1(extra: (&str, Value)) -> Value {
    let mut p = json!({"v": 1, "id": "a1", "channel": "camera", "title": "Driveway", "body": "Car in Driveway"});
    p[extra.0] = extra.1;
    p
}

#[test]
fn loc_is_kept_and_sent_as_compact_json() {
    let a = ok(v1(("loc", loc_value())));
    assert_eq!(a.loc, Some(loc_value()));
    assert_eq!(
        to_fcm_data(&a)["loc"],
        r#"{"key":"frigate.object","args":{"label":"car","zone":"Driveway"}}"#
    );
}

#[test]
fn loc_args_may_be_missing() {
    assert_eq!(
        ok(v1(("loc", json!({"key": "x.y"})))).loc,
        Some(json!({"key": "x.y", "args": {}}))
    );
}

#[test]
fn no_loc_means_no_fcm_key() {
    let a = ok(json!({"v": 1, "id": "a1", "title": "T"}));
    assert!(a.loc.is_none());
    assert!(!to_fcm_data(&a).contains_key("loc"));
}

#[test]
fn loc_wrong_shapes_are_dropped_but_the_alert_is_delivered() {
    let many: Map<String, Value> = (0..9).map(|i| (format!("a{i}"), json!("x"))).collect();
    for loc in [
        json!("{\"key\":\"frigate.object\"}"),
        json!({"args": {"label": "car"}}),
        json!({"key": "Frigate.Object"}),
        json!({"key": "x".repeat(65)}),
        json!({"key": "k", "args": ["car"]}),
        json!({"key": "k", "args": {"label": 3}}),
        json!({"key": "k", "args": {"Label": "car"}}),
        json!({"key": "k", "args": {"v": "x".repeat(201)}}),
        json!({"key": "k", "args": many}),
    ] {
        let a = ok(v1(("loc", loc.clone())));
        assert!(a.loc.is_none(), "{loc}");
        assert_eq!(a.body, "Car in Driveway");
    }
}

#[test]
fn legacy_payloads_never_carry_loc() {
    let a = normalize(
        "nvr",
        &json!({"title": "Cam", "body": "x", "loc": loc_value()}),
        NOW,
    )
    .unwrap();
    assert!(a.loc.is_none());
}

#[test]
fn fit_drops_loc_before_emptying_the_body() {
    let args: Map<String, Value> = (0..8)
        .map(|i| (format!("a{i}"), json!("x".repeat(200))))
        .collect();
    let mut p = v1(("loc", json!({"key": "k", "args": args})));
    p["body"] = json!("b".repeat(3000));
    let data = to_fcm_data(&ok(p));
    assert!(fcm_size(&data) <= FCM_MAX_BYTES);
    assert!(!data.contains_key("loc"));
    assert!(data["body"].starts_with(&"b".repeat(2000)));
}

// ---------- priority and ttl (from fcm_sender tests) ----------

#[test]
fn android_priority_table() {
    use Priority::{High, Normal};
    for (channel, severity, expected) in [
        ("misc", "warning", High),
        ("misc", "critical", High),
        ("energy", "info", Normal),
        ("climate", "info", Normal),
        ("system", "info", Normal),
        ("misc", "info", Normal),
        ("alarm", "info", High),
        ("security", "info", High),
        ("camera", "info", High),
        ("energy", "warning", High),
    ] {
        let a = alert(|a| {
            a.channel = channel.into();
            a.severity = severity.into();
        });
        assert_eq!(android_priority(&a), expected, "{channel}/{severity}");
    }
}

#[test]
fn ttl_is_payload_ttl_with_default() {
    assert_eq!(ttl_seconds(&alert(|_| {})), 43200);
    assert_eq!(ttl_seconds(&alert(|a| a.ttl = 600)), 600);
}

#[test]
fn cancel_ttl_is_300_whatever_the_payload_ttl() {
    assert_eq!(
        ttl_seconds(&alert(|a| {
            a.cancel = true;
            a.ttl = 43200;
        })),
        300
    );
}
