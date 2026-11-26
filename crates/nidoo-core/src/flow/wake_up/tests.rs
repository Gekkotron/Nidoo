use super::*;
use serde_json::json;

fn alarm() -> WakeUp {
    WakeUp {
        hour: 7,
        minute: 5,
        player: "music-player".to_owned(),
        ..WakeUp::default()
    }
}

fn paris(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<chrono_tz::Tz> {
    chrono_tz::Europe::Paris
        .with_ymd_and_hms(y, mo, d, h, mi, 0)
        .unwrap()
}

fn sched(expr: &str) -> Schedule {
    Schedule::parse(expr).unwrap()
}

#[test]
fn pre_ring_crosses_midnight() {
    // 2026-11-22 is a Sunday; the Monday 00:15 alarm pre-rings 45 min earlier.
    let got = next_pre_ring(&[sched("15 0 * * 1")], 45, &paris(2026, 11, 22, 22, 0));
    assert_eq!(got, Some(paris(2026, 11, 22, 23, 30)));
}

#[test]
fn ring_within_lead_only() {
    let alarms = [sched("0 8 * * *"), sched("0 9 * * *")];
    let now = paris(2026, 11, 25, 7, 30);
    assert_eq!(
        ring_within(&alarms, 45, &now),
        Some(paris(2026, 11, 25, 8, 0))
    );
    assert_eq!(ring_within(&alarms, 20, &now), None);
    // The boundary is inclusive, and a ring at `now` is not ahead.
    assert!(ring_within(&alarms, 30, &now).is_some());
    assert_eq!(ring_within(&alarms, 45, &paris(2026, 11, 25, 8, 0)), None);
    assert_eq!(ring_within(&[], 45, &now), None);
}

#[test]
fn pre_ring_two_alarms_same_time_once() {
    let two = [sched("0 7 * * *"), sched("0 7 * * *")];
    let got = next_pre_ring(&two, 30, &paris(2026, 11, 25, 6, 0));
    assert_eq!(got, Some(paris(2026, 11, 25, 6, 30)));
    let latest = latest_pre_ring_in(
        &two,
        30,
        &paris(2026, 11, 25, 6, 0),
        &paris(2026, 11, 25, 7, 0),
    );
    assert_eq!(latest, Some(paris(2026, 11, 25, 6, 30)));
}

#[test]
fn pre_ring_dst_day() {
    // Spring forward 2026-03-29: 02:00 becomes 03:00, so 07:00 CEST is 06:00 UTC.
    let got = next_pre_ring(&[sched("0 7 * * *")], 45, &paris(2026, 3, 29, 0, 0)).unwrap();
    assert_eq!(got, paris(2026, 3, 29, 7, 0) - TimeDelta::minutes(45));
    assert_eq!(got.format("%H:%M").to_string(), "06:15");
}

#[test]
fn pre_ring_after_inside_window_goes_next_week() {
    // Sunday 23:45 is already inside the 45 min before Monday 00:15, so the next pre-ring is
    // the following Sunday 23:30 (2026-11-29), strictly after `after`.
    let after = paris(2026, 11, 22, 23, 45);
    let got = next_pre_ring(&[sched("15 0 * * 1")], 45, &after).unwrap();
    assert_eq!(got, paris(2026, 11, 29, 23, 30));
    assert!(got > after);
}

#[test]
fn pre_ring_dst_absolute() {
    // 03:30 CEST on 2026-03-29 is 01:30 UTC; 45 absolute minutes earlier is 01:45 CET.
    let got = next_pre_ring(&[sched("30 3 * * *")], 45, &paris(2026, 3, 29, 0, 0)).unwrap();
    assert_eq!(got, paris(2026, 3, 29, 1, 45));
    assert_eq!(got, paris(2026, 3, 29, 3, 30) - TimeDelta::minutes(45));
}

#[test]
fn alarm_schedules_sorted_by_id() {
    let w = WakeUp {
        player: "p".to_owned(),
        ..WakeUp::default()
    };
    let flows = [
        to_flow("wake-up-2", "b", true, &w),
        to_flow("wake-up-1", "a", true, &w),
    ];
    let ids: Vec<String> = alarm_schedules(&flows, &[])
        .into_iter()
        .map(|e| e.0)
        .collect();
    assert_eq!(ids, ["wake-up-1", "wake-up-2"]);
}

#[test]
fn alarm_schedules_skips_disabled_and_unknown() {
    let w = WakeUp {
        player: "p".to_owned(),
        ..WakeUp::default()
    };
    let on = to_flow("wake-up-1", "a", true, &w);
    let off = to_flow("wake-up-2", "b", false, &w);
    let plain = Flow {
        id: "other".to_owned(),
        enabled: true,
        ..Flow::default()
    };
    let flows = [plain, off, on];
    let got = alarm_schedules(&flows, &[]);
    assert_eq!(got.len(), 1);
    assert_eq!(
        (got[0].0.as_str(), got[0].1.as_str()),
        ("wake-up-1", "0 7 * * 1-5")
    );
    assert!(alarm_schedules(&flows, &["nope".to_owned()]).is_empty());
    assert_eq!(alarm_schedules(&flows, &["wake-up-1".to_owned()]).len(), 1);
}

#[test]
fn cron_for_weekdays() {
    assert_eq!(cron(&alarm()), "5 7 * * 1-5");
    let weekend = WakeUp {
        days: vec![0, 6],
        ..alarm()
    };
    assert_eq!(cron(&weekend), "5 7 * * 0,6");
    let all = WakeUp {
        days: (0..=6).collect(),
        ..alarm()
    };
    assert_eq!(cron(&all), "5 7 * * 0-6");
}

fn full() -> WakeUp {
    WakeUp {
        source: Some("kitchen".to_owned()),
        speakers: vec![
            WakeSpeaker {
                volume_entity: "number.a_music".to_owned(),
                mute_entity: Some("switch.a_mute".to_owned()),
                volume: 25.0,
            },
            WakeSpeaker {
                volume_entity: "number.b_music".to_owned(),
                mute_entity: None,
                volume: 25.0,
            },
        ],
        muted: vec!["switch.c_mute".to_owned()],
        ..alarm()
    }
}

#[test]
fn to_flow_fans_out_speakers_then_play() {
    let flow = to_flow("wake-up-1", "Wake-up 07:05", true, &full());
    assert!(flow.enabled);
    assert_eq!(flow.labels, [LABEL]);
    let ids: Vec<&str> = flow.nodes.iter().map(|n| n.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "trigger", "volume-0", "volume-1", "unmute-0", "mute-0", "play"
        ]
    );
    let NodeKind::TriggerSchedule(trigger) = &flow.nodes[0].kind else {
        panic!("first node is the schedule");
    };
    assert_eq!(trigger.cron, "5 7 * * 1-5");
    let command_of = |i: usize| match &flow.nodes[i].kind {
        NodeKind::Command(c) => (c.entity.clone(), c.command.clone()),
        other => panic!("not a command: {other:?}"),
    };
    assert_eq!(
        command_of(1),
        ("number.a_music".into(), Command::SetNumber { value: 25.0 })
    );
    assert_eq!(
        command_of(2),
        ("number.b_music".into(), Command::SetNumber { value: 25.0 })
    );
    assert_eq!(command_of(3), ("switch.a_mute".into(), Command::TurnOff));
    assert!(matches!(
        command_of(4),
        (e, Command::TurnOn { .. }) if e == "switch.c_mute"
    ));
    assert_eq!(
        command_of(5),
        (
            "music-player".into(),
            Command::PlayPlaylist {
                playlist: "favorites".into(),
                shuffle: true,
                source: Some("kitchen".into()),
                fade_in_s: 60,
            }
        )
    );
    let chain: Vec<(&str, &str)> = flow
        .edges
        .iter()
        .map(|e| (e.from.as_str(), e.to.as_str()))
        .collect();
    // One branch per step, the playlist last: the engine runs them in edge order, and a
    // speaker that does not answer ends only its own branch, never the music.
    assert_eq!(
        chain,
        [
            ("trigger", "volume-0"),
            ("trigger", "volume-1"),
            ("trigger", "unmute-0"),
            ("trigger", "mute-0"),
            ("trigger", "play"),
        ]
    );
    let at = |i: usize| {
        let p = flow.nodes[i].position.expect("position");
        (p.x, p.y)
    };
    assert_eq!(at(0), (0.0, 0.0));
    for i in 1..=4 {
        assert_eq!(at(i), (280.0, 120.0 * (i - 1) as f64));
    }
    assert_eq!(at(5), (560.0, 0.0));
    assert_eq!(read(&flow), Some(Ok(full())));
    let off = to_flow("wake-up-1", "x", false, &full());
    assert!(!off.enabled);
    // The generated flow survives the wire and still reads back.
    let again = Flow::decode(flow.encode().as_bytes()).expect("decode");
    assert_eq!(read(&again), Some(Ok(full())));
    assert!(is_managed(&again));
}

#[test]
fn flow_keeps_stored_speaker_ids() {
    let w = WakeUp {
        speakers: vec![WakeSpeaker {
            volume_entity: "number.gone_music".to_owned(),
            mute_entity: Some("switch.gone_mute".to_owned()),
            volume: 10.0,
        }],
        muted: vec!["switch.deleted".to_owned()],
        ..alarm()
    };
    let flow = to_flow("wake-up-2", "x", true, &w);
    let entities: Vec<&str> = flow
        .nodes
        .iter()
        .filter_map(|n| match &n.kind {
            NodeKind::Command(c) => Some(c.entity.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        entities,
        [
            "number.gone_music",
            "switch.gone_mute",
            "switch.deleted",
            "music-player"
        ]
    );
}

#[test]
fn read_none_for_plain_flow() {
    let flow = Flow::default();
    assert!(read(&flow).is_none());
    assert!(!is_managed(&flow));
}

#[test]
fn unparsable_settings_are_an_error() {
    let mut flow = Flow::default();
    flow.extra
        .insert(EXTRA_KEY.to_owned(), json!({"hour": "x"}));
    assert!(matches!(read(&flow), Some(Err(_))));
    assert!(is_managed(&flow));
}

#[test]
fn validate_rejects_no_day_and_blank_playlist() {
    assert_eq!(validate(&alarm()), Ok(()));
    let no_day = WakeUp {
        days: vec![],
        ..alarm()
    };
    assert_eq!(validate(&no_day), Err(WakeUpError::NoDay));
    let blank = WakeUp {
        playlist: "  ".to_owned(),
        ..alarm()
    };
    assert_eq!(validate(&blank), Err(WakeUpError::BlankPlaylist));
    let late = WakeUp {
        hour: 24,
        ..alarm()
    };
    assert_eq!(validate(&late), Err(WakeUpError::BadTime));
    assert_eq!(validate(&WakeUp::default()), Err(WakeUpError::NoPlayer));
}

#[test]
fn next_id_takes_lowest_free() {
    assert_eq!(next_id(&["wake-up-1", "wake-up-3"]), "wake-up-2");
    assert_eq!(next_id(&[]), "wake-up-1");
}

#[test]
fn default_name_pads_the_time() {
    assert_eq!(default_name(true, 7, 0), "Réveil 07:00");
    assert_eq!(default_name(false, 6, 45), "Wake-up 06:45");
}

#[test]
fn retime_sets_time_and_days() {
    let f = to_flow("wake-up-1", "Morning", true, &full());
    let r = retime(&f, 6, 15, &[5, 1, 1]).expect("retime");
    let w = read(&r).expect("alarm").expect("readable");
    assert_eq!((w.hour, w.minute, w.days.clone()), (6, 15, vec![1, 5]));
    let NodeKind::TriggerSchedule(t) = &r.nodes[0].kind else {
        panic!("first node is the schedule");
    };
    assert_eq!(t.cron, "15 6 * * 1,5");
}

#[test]
fn retime_keeps_everything_else() {
    let mut f = to_flow("wake-up-1", "Morning", true, &full());
    f.area = Some("bedroom".to_owned());
    f.labels.push("mine".to_owned());
    f.enabled = false;
    let r = retime(&f, 6, 15, &[1]).expect("retime");
    assert_eq!(r.id, f.id);
    assert_eq!(r.area, f.area);
    assert_eq!(r.labels, f.labels);
    assert_eq!(r.enabled, f.enabled);
    let (old, new) = (read(&f).unwrap().unwrap(), read(&r).unwrap().unwrap());
    assert_eq!(
        new,
        WakeUp {
            hour: 6,
            minute: 15,
            days: vec![1],
            ..old
        }
    );
    let ids = |f: &Flow| f.nodes.iter().map(|n| n.id.clone()).collect::<Vec<_>>();
    assert_eq!(ids(&r), ids(&f));
    assert_eq!(r.nodes[1..], f.nodes[1..]);
}

#[test]
fn retime_keeps_a_typed_name() {
    let f = to_flow("wake-up-1", "Morning", true, &full());
    assert_eq!(retime(&f, 6, 15, &[1]).unwrap().name, "Morning");
}

#[test]
fn retime_moves_a_default_name() {
    for (old, new) in [
        ("Wake-up 07:05", "Wake-up 06:15"),
        ("Réveil 07:05", "Réveil 06:15"),
    ] {
        let f = to_flow("wake-up-1", old, true, &full());
        assert_eq!(retime(&f, 6, 15, &[1]).unwrap().name, new);
    }
}

#[test]
fn retime_refuses_bad_input() {
    let f = to_flow("wake-up-1", "Morning", true, &full());
    assert_eq!(
        retime(&f, 6, 15, &[]),
        Err(RetimeError::Invalid(WakeUpError::NoDay))
    );
    assert_eq!(
        retime(&f, 6, 15, &[7]),
        Err(RetimeError::Invalid(WakeUpError::NoDay))
    );
    assert_eq!(
        retime(&f, 24, 0, &[1]),
        Err(RetimeError::Invalid(WakeUpError::BadTime))
    );
}

#[test]
fn retime_refuses_other_flows() {
    assert_eq!(
        retime(&Flow::default(), 6, 15, &[1]),
        Err(RetimeError::NotAnAlarm)
    );
    let mut f = Flow::default();
    f.extra.insert(EXTRA_KEY.to_owned(), json!("x"));
    assert!(matches!(
        retime(&f, 6, 15, &[1]),
        Err(RetimeError::Unreadable(_))
    ));
}
