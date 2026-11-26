use super::*;

const URL: &str = "http://192.168.1.20:5000";

fn event(kind: &str, after: Value) -> Value {
    let mut base = json!({"id": "1696.5-abc", "camera": "front_door", "label": "person",
        "has_clip": false, "false_positive": false, "entered_zones": []});
    for (k, v) in after.as_object().expect("object") {
        base[k] = v.clone();
    }
    json!({"type": kind, "before": {}, "after": base})
}

fn new_event(after: Value) -> Value {
    event("new", after)
}

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}

fn bridge(tweak: impl FnOnce(&mut FrigateSettings)) -> FrigateBridge {
    let mut settings = FrigateSettings::new(URL);
    tweak(&mut settings);
    FrigateBridge::new(settings)
}

fn handle(b: &mut FrigateBridge, message: Value) -> Option<Value> {
    b.handle(&message, 1000)
}

#[test]
fn new_event_sends_an_audible_alert_with_the_snapshot() {
    let mut b = bridge(|_| {});
    let a = handle(&mut b, new_event(json!({})));
    assert_eq!(
        a,
        Some(json!({
            "v": 1, "id": "frigate-1696.5-abc", "channel": "camera", "severity": "warning",
            "title": "Front Door", "body": "Person detected", "camera": "front_door",
            "image_url": format!("{URL}/api/events/1696.5-abc/snapshot.jpg"),
            "silent": false, "ttl": 43200,
            "loc": {"key": "frigate.object", "args": {"label": "person"}},
        }))
    );
}

#[test]
fn end_with_clip_updates_the_same_alert_silently() {
    let mut b = bridge(|_| {});
    handle(&mut b, new_event(json!({})));
    let a = handle(&mut b, event("end", json!({"has_clip": true}))).expect("alert");
    assert_eq!(a["id"], "frigate-1696.5-abc");
    assert_eq!(
        a["video_url"],
        format!("{URL}/api/events/1696.5-abc/clip.mp4")
    );
    assert_eq!(a["silent"], true);
}

#[test]
fn updates_and_clipless_end_send_nothing_more() {
    let mut b = bridge(|_| {});
    handle(&mut b, new_event(json!({})));
    assert_eq!(handle(&mut b, event("update", json!({}))), None);
    assert_eq!(
        handle(&mut b, event("end", json!({"has_clip": false}))),
        None
    );
}

#[test]
fn end_of_an_event_never_announced_is_ignored_when_filtered_out() {
    let mut b = bridge(|s| s.labels = set(&["car"]));
    assert_eq!(
        handle(&mut b, event("end", json!({"has_clip": true}))),
        None
    );
}

#[test]
fn filters() {
    type Tweak = Box<dyn FnOnce(&mut FrigateSettings)>;
    let cases: Vec<(Tweak, Value)> = vec![
        (Box::new(|s| s.labels = set(&["car"])), json!({})),
        (Box::new(|s| s.cameras = set(&["garden"])), json!({})),
        (
            Box::new(|s| s.zones = set(&["porch"])),
            json!({"entered_zones": ["driveway"]}),
        ),
        (Box::new(|_| {}), json!({"false_positive": true})),
    ];
    for (tweak, after) in cases {
        let mut b = bridge(tweak);
        assert_eq!(handle(&mut b, new_event(after.clone())), None, "{after}");
    }
}

#[test]
fn zone_entered_on_update_triggers_then_body_names_it() {
    let mut b = bridge(|s| s.zones = set(&["porch"]));
    assert_eq!(handle(&mut b, new_event(json!({}))), None);
    let a = handle(
        &mut b,
        event("update", json!({"entered_zones": ["driveway", "porch"]})),
    )
    .expect("alert");
    assert_eq!(a["body"], "Person in Porch");
    assert_eq!(a["silent"], false);
}

#[test]
fn event_qualifying_only_at_end_sends_one_audible_alert_with_clip() {
    let mut b = bridge(|s| s.zones = set(&["porch"]));
    let a = handle(
        &mut b,
        event("end", json!({"has_clip": true, "entered_zones": ["porch"]})),
    )
    .expect("alert");
    assert_eq!(a["silent"], false);
    assert!(a["video_url"].as_str().expect("url").ends_with("/clip.mp4"));
}

#[test]
fn sub_label_names_the_person() {
    let mut b = bridge(|_| {});
    let a = handle(&mut b, new_event(json!({"sub_label": ["Alice", 0.92]}))).expect("alert");
    assert_eq!(a["body"], "Alice (person) detected");
}

#[test]
fn cooldown_suppresses_new_events_on_the_same_camera() {
    let mut b = bridge(|s| s.cooldown = 60);
    let at = |b: &mut FrigateBridge, m: Value, t: u64| b.handle(&m, t);
    assert!(at(&mut b, new_event(json!({"id": "a"})), 1000).is_some());
    assert!(at(&mut b, new_event(json!({"id": "b"})), 1030).is_none());
    assert!(
        at(
            &mut b,
            new_event(json!({"id": "c", "camera": "garden"})),
            1030
        )
        .is_some()
    );
    assert!(at(&mut b, new_event(json!({"id": "d"})), 1061).is_some());
    // a suppressed event ending while the camera still cools down stays silent too
    let end = event("end", json!({"id": "b", "has_clip": true}));
    assert!(at(&mut b, end, 1061).is_none());
}

#[test]
fn on_event_parses_bytes_and_ignores_junk() {
    let mut b = bridge(|_| {});
    assert_eq!(b.on_event(b"not json", 1000), None);
    assert_eq!(b.on_event(b"[]", 1000), None);
    assert_eq!(b.on_event(br#"{"type":"new","after":{}}"#, 1000), None);
    assert_eq!(
        b.on_event(br#"{"type":"gone","after":{"id":"x"}}"#, 1000),
        None
    );
    let good = new_event(json!({})).to_string();
    assert!(b.on_event(good.as_bytes(), 1000).is_some());
}

#[test]
fn loc_names_the_raw_label_the_zone_and_the_person() {
    let mut b = bridge(|_| {});
    let a = handle(
        &mut b,
        new_event(json!({"label": "car", "entered_zones": ["drive_way"],
            "sub_label": ["Alice", 0.9]})),
    )
    .expect("alert");
    assert_eq!(
        a["loc"],
        json!({"key": "frigate.object",
            "args": {"label": "car", "zone": "Drive Way", "who": "Alice"}})
    );
    assert_eq!(a["body"], "Alice (car) in Drive Way");
}

#[test]
fn no_label_means_no_loc() {
    let mut b = bridge(|_| {});
    let a = handle(&mut b, new_event(json!({"label": null}))).expect("alert");
    assert!(a.get("loc").is_none());
    assert_eq!(a["body"], "Object detected");
}

#[test]
fn media_url_loses_its_trailing_slash() {
    let mut b = FrigateBridge::new(FrigateSettings::new(format!("{URL}/")));
    let a = handle(&mut b, new_event(json!({}))).expect("alert");
    assert!(
        a["image_url"]
            .as_str()
            .expect("url")
            .starts_with(&format!("{URL}/api"))
    );
}

#[test]
fn remembers_at_most_the_last_events() {
    let mut b = bridge(|_| {});
    for i in 0..SEEN_CAP + 10 {
        handle(&mut b, new_event(json!({"id": format!("e{i}")})));
    }
    assert_eq!(b.sent.len(), SEEN_CAP);
}
