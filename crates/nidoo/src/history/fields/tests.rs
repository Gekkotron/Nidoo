use super::*;
use nidoo_core::flow::fields;
use nidoo_core::model::{
    Capabilities, GenericState, GenericValue, Kind, MediaPlayerState, VacuumState,
};
use serde_json::json;

const S: Micros = 1_000_000;
const FROM: Micros = 1_000_000 * S;
const WINDOW: Micros = 600 * S;
const TO: Micros = FROM + 3 * WINDOW;

fn vac(state: Option<&str>, battery: Option<u8>, fan: Option<&str>) -> StateValue {
    StateValue::Vacuum(VacuumState {
        state: state.map(str::to_owned),
        battery,
        fan_speed: fan.map(str::to_owned),
    })
}

fn at(secs: i64) -> Micros {
    FROM + secs * S
}

fn vacuum_tracks(carry: Option<&StateValue>, rows: &[(Micros, StateValue)]) -> Vec<FieldTrack> {
    let list = fields(Kind::Vacuum, "vacuum", &Capabilities::default(), None);
    build_tracks(&TrackInput {
        component: "vacuum",
        fields: &list,
        headline_num: &nidoo_core::history::num_of,
        carry,
        rows,
        from: FROM,
        to: TO,
        window: WINDOW,
    })
}

fn steps(track: &FieldTrack) -> &[(i64, Option<Scalar>)] {
    match &track.data {
        TrackData::Steps { changes } => changes,
        other => panic!("not steps: {other:?}"),
    }
}

fn text(s: &str) -> Option<Scalar> {
    Some(Scalar::Text(s.into()))
}

#[test]
fn a_field_the_main_chart_draws_gets_no_track() {
    // A cover's position is its recorded history number: the main chart already plots it.
    let list = fields(Kind::Cover, "cover", &Capabilities::default(), None);
    let cover = |position| {
        StateValue::Cover(nidoo_core::model::CoverState {
            state: Some("open".into()),
            position: Some(position),
        })
    };
    let tracks = build_tracks(&TrackInput {
        component: "cover",
        fields: &list,
        headline_num: &|_| Some(1.0),
        carry: Some(&cover(100)),
        rows: &[(at(60), cover(40))],
        from: FROM,
        to: TO,
        window: WINDOW,
    });
    assert!(tracks.is_empty(), "{tracks:?}");
}

#[test]
fn vacuum_has_headline_then_fields_in_order() {
    let tracks = vacuum_tracks(
        None,
        &[(at(10), vac(Some("docked"), Some(100), Some("max")))],
    );
    let names: Vec<_> = tracks.iter().map(|t| t.field.as_deref()).collect();
    assert_eq!(names, [None, Some("battery"), Some("fan_speed")]);
}

#[test]
fn headline_steps_collapse_and_carry() {
    let carry = vac(Some("docked"), Some(90), None);
    let rows = [
        (at(100), vac(Some("docked"), Some(95), None)),
        (at(200), vac(Some("cleaning"), Some(95), None)),
        (at(300), vac(Some("cleaning"), Some(80), None)),
        (at(400), vac(Some("docked"), Some(80), None)),
    ];
    let tracks = vacuum_tracks(Some(&carry), &rows);
    let f = FROM / S;
    assert_eq!(
        steps(&tracks[0]),
        [
            (f, text("docked")),
            (f + 200, text("cleaning")),
            (f + 400, text("docked"))
        ]
    );
}

#[test]
fn unknown_carry_starts_unknown() {
    let rows = [(at(100), vac(Some("docked"), Some(95), Some("max")))];
    let tracks = vacuum_tracks(None, &rows);
    let f = FROM / S;
    assert_eq!(steps(&tracks[0]), [(f, None), (f + 100, text("docked"))]);
    assert_eq!(steps(&tracks[2]), [(f, None), (f + 100, text("max"))]);
}

#[test]
fn field_missing_in_some_states_goes_unknown_then_back() {
    let carry = vac(Some("docked"), None, Some("max"));
    let rows = [
        (at(100), vac(Some("cleaning"), None, None)),
        (at(200), vac(Some("cleaning"), None, Some("max"))),
        (at(300), StateValue::Unknown),
    ];
    let tracks = vacuum_tracks(Some(&carry), &rows);
    let f = FROM / S;
    assert_eq!(
        steps(&tracks[2]),
        [
            (f, text("max")),
            (f + 100, None),
            (f + 200, text("max")),
            (f + 300, None)
        ]
    );
}

#[test]
fn row_at_from_replaces_the_carry_entry() {
    let carry = vac(Some("docked"), None, None);
    let rows = [(at(0), vac(Some("cleaning"), None, None))];
    let tracks = vacuum_tracks(Some(&carry), &rows);
    assert_eq!(steps(&tracks[0]), [(FROM / S, text("cleaning"))]);
}

#[test]
fn row_at_from_equal_to_the_carry_collapses() {
    let carry = vac(Some("docked"), None, None);
    let rows = [
        (at(0), vac(Some("cleaning"), None, None)),
        (at(5), vac(Some("docked"), None, None)),
    ];
    let tracks = vacuum_tracks(Some(&carry), &rows);
    let f = FROM / S;
    assert_eq!(
        steps(&tracks[0]),
        [(f, text("cleaning")), (f + 5, text("docked"))]
    );
    let again = [(at(0), vac(Some("docked"), None, None))];
    let tracks = vacuum_tracks(Some(&carry), &again);
    assert_eq!(steps(&tracks[0]), [(f, text("docked"))]);
}

#[test]
fn battery_is_a_number_track_with_buckets() {
    let carry = vac(Some("docked"), Some(100), None);
    let rows = [
        (at(300), vac(Some("cleaning"), Some(80), None)),
        (at(900), vac(Some("cleaning"), Some(60), None)),
    ];
    let tracks = vacuum_tracks(Some(&carry), &rows);
    assert_eq!(tracks[1].unit.as_deref(), Some("%"));
    let TrackData::Number { window, points } = &tracks[1].data else {
        panic!("not a number track");
    };
    let f = FROM / S;
    assert_eq!(*window, 600);
    // [0,600): 100 for 300 s then 80 for 300 s; [600,1200): 80 for 300 s then 60;
    // [1200,1800): 60 carried.
    assert_eq!(
        points,
        &[
            (f, 90.0, 80.0, 100.0),
            (f + 600, 70.0, 60.0, 80.0),
            (f + 1200, 60.0, 60.0, 60.0),
        ]
    );
}

#[test]
fn number_windows_with_no_known_value_are_left_out() {
    let rows = [(at(700), vac(Some("docked"), Some(50), None))];
    let tracks = vacuum_tracks(None, &rows);
    let TrackData::Number { points, .. } = &tracks[1].data else {
        panic!("not a number track");
    };
    let f = FROM / S;
    assert_eq!(
        points.iter().map(|p| p.0).collect::<Vec<_>>(),
        [f + 600, f + 1200]
    );
    assert!(points.iter().all(|p| p.1.is_finite()));
}

#[test]
fn number_field_holding_a_non_number_is_skipped() {
    let list = fields(Kind::Generic, "fan", &Capabilities::default(), None);
    let generic = |pct: GenericValue| {
        StateValue::Generic(GenericState {
            fields: [
                ("state".to_owned(), GenericValue::Bool(true)),
                ("percentage".to_owned(), pct),
            ]
            .into(),
        })
    };
    let rows = [
        (at(0), generic(GenericValue::Number(40.0))),
        (at(300), generic(GenericValue::Text("n/a".into()))),
    ];
    let tracks = build_tracks(&TrackInput {
        component: "fan",
        fields: &list,
        headline_num: &|_| Some(1.0),
        carry: None,
        rows: &rows,
        from: FROM,
        to: FROM + WINDOW,
        window: WINDOW,
    });
    let pct = tracks
        .iter()
        .find(|t| t.field.as_deref() == Some("percentage"))
        .unwrap();
    let TrackData::Number { points, .. } = &pct.data else {
        panic!("not a number track");
    };
    assert_eq!(points, &[(FROM / S, 40.0, 40.0, 40.0)]);
}

#[test]
fn numeric_headline_gets_no_track_and_generic_fields_follow() {
    let list = fields(Kind::Generic, "fan", &Capabilities::default(), None);
    let names: Vec<&str> = list.iter().map(|f| f.name.as_str()).collect();
    assert!(names.contains(&"percentage") && !names.contains(&"state"));
    let state = |on: bool, osc: bool| {
        StateValue::Generic(GenericState {
            fields: [
                ("state".to_owned(), GenericValue::Bool(on)),
                ("oscillation".to_owned(), GenericValue::Bool(osc)),
            ]
            .into(),
        })
    };
    let rows = [(at(10), state(true, false)), (at(20), state(false, false))];
    let tracks = build_tracks(&TrackInput {
        component: "fan",
        fields: &list,
        headline_num: &|v| match v {
            StateValue::Generic(g) => match g.fields.get("state") {
                Some(GenericValue::Bool(b)) => Some(f64::from(u8::from(*b))),
                _ => None,
            },
            _ => None,
        },
        carry: None,
        rows: &rows,
        from: FROM,
        to: TO,
        window: WINDOW,
    });
    assert!(tracks.iter().all(|t| t.field.is_some()));
    let osc = tracks
        .iter()
        .find(|t| t.field.as_deref() == Some("oscillation"))
        .unwrap();
    assert_eq!(
        steps(osc),
        [(FROM / S, None), (FROM / S + 10, Some(Scalar::Bool(false)))]
    );
}

#[test]
fn word_headline_of_a_media_player_gets_a_track() {
    let list = fields(
        Kind::MediaPlayer,
        "media_player",
        &Capabilities::default(),
        None,
    );
    let player = |title: &str| {
        StateValue::MediaPlayer(MediaPlayerState {
            state: Some("playing".into()),
            title: Some(title.into()),
            ..MediaPlayerState::default()
        })
    };
    let rows = [(at(5), player("a")), (at(9), player("b"))];
    let tracks = build_tracks(&TrackInput {
        component: "media_player",
        fields: &list,
        headline_num: &nidoo_core::history::num_of,
        carry: None,
        rows: &rows,
        from: FROM,
        to: TO,
        window: WINDOW,
    });
    // A media player's headline is a word, so it has one.
    assert_eq!(tracks[0].field, None);
    assert_eq!(tracks[1].field.as_deref(), Some("title"));
    assert_eq!(
        steps(&tracks[1]),
        [
            (FROM / S, None),
            (FROM / S + 5, text("a")),
            (FROM / S + 9, text("b"))
        ]
    );
}

#[test]
fn vacuum_reply_json() {
    let carry = vac(Some("docked"), Some(100), Some("balanced"));
    let rows = [
        (at(300), vac(Some("cleaning"), Some(80), Some("balanced"))),
        (at(900), vac(Some("cleaning"), Some(60), Some("max"))),
    ];
    let tracks = vacuum_tracks(Some(&carry), &rows);
    let reply = nidoo_api::HistoryFields {
        from: FROM / S,
        to: TO / S,
        tracks,
    };
    let f = FROM / S;
    assert_eq!(
        serde_json::to_value(&reply).unwrap(),
        json!({
            "from": f,
            "to": f + 1800,
            "tracks": [
                {"data": {"type": "steps", "changes": [
                    [f, "docked"], [f + 300, "cleaning"]]}},
                {"field": "battery", "unit": "%", "data": {"type": "number",
                    "window": 600, "points": [
                        [f, 90.0, 80.0, 100.0],
                        [f + 600, 70.0, 60.0, 80.0],
                        [f + 1200, 60.0, 60.0, 60.0]]}},
                {"field": "fan_speed", "data": {"type": "steps", "changes": [
                    [f, "balanced"], [f + 900, "max"]]}},
            ]
        })
    );
}
