use nidoo_watchdog::supervisor::{
    Baseline, Check, Ctx, Evidence, Problem, Reading, Supervisor, alerts,
};
use serde_json::Value;

const NOW: u64 = 1_800_000_000;
const H: u64 = 3600;
const D: u64 = 24 * H;

fn sensor(id: &str, name: &str) -> Reading {
    Reading {
        entity_id: id.into(),
        name: name.into(),
        device_class: Some("temperature".into()),
        unit: Some("°C".into()),
        value: Some(15.0),
        last_change: Some(NOW - 14 * H),
        last_report: Some(NOW - 60),
        unavailable_since: None,
        baseline: Some(Baseline {
            max_gap: 3 * H,
            changes: 400,
            rest: None,
        }),
    }
}

fn silent(id: &str, name: &str) -> Reading {
    Reading {
        last_change: Some(NOW - 10 * H),
        last_report: Some(NOW - 10 * H),
        ..sensor(id, name)
    }
}

fn ctx(now: u64) -> Ctx {
    Ctx {
        now,
        armed_at: NOW - 30 * D,
        factor: 2.0,
        silent_paused: false,
    }
}

fn key(alert: &Value) -> &str {
    alert["loc"]["key"].as_str().unwrap()
}

fn open_problem(id: &str, check: Check, opened_at: u64, usual: Option<u64>) -> Problem {
    Problem {
        entity_id: id.into(),
        name: "Tank temperature".into(),
        check,
        opened_at,
        evidence: Evidence {
            value: Some(15.0),
            unit: Some("°C".into()),
            for_secs: 8 * H,
            usual_secs: usual,
        },
        alert_id: Some(format!("supervisor-{}-{id}-{opened_at}", check.as_str())),
    }
}

#[test]
fn stuck_probe_lifecycle() {
    let mut sup = Supervisor::new(Vec::new());
    let probe = sensor("sensor.tank", "Tank temperature");

    let mut step = sup.step(std::slice::from_ref(&probe), &ctx(NOW));
    assert_eq!(step.opened.len(), 1);
    assert!(step.closed.is_empty());
    let sent = alerts(&mut step, true, true, NOW);
    assert_eq!(sent.len(), 1);
    let a = &sent[0];
    assert_eq!(key(a), "supervisor.stuck");
    assert_eq!(a["severity"], "warning");
    assert_eq!(a["channel"], "system");
    assert_eq!(a["ttl"], 86400);
    assert_eq!(a["entity_id"], "sensor.tank");
    assert_eq!(a["title"], "Tank temperature looks stuck");
    assert_eq!(
        a["body"],
        "Same value (15.0 °C) for 14 h, usually changes within 3 h. Check the sensor."
    );
    assert_eq!(a["loc"]["args"]["name"], "Tank temperature");
    assert_eq!(a["loc"]["args"]["hours"], "14");
    assert_eq!(a["loc"]["args"]["usual"], "3");
    let actions = a["actions"].as_array().unwrap();
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0]["id"], "mute");
    assert_eq!(actions[0]["label"], "Mute this sensor");
    let id = a["id"].as_str().unwrap();
    assert!(id.starts_with("supervisor-"));
    assert_eq!(step.opened[0].alert_id.as_deref(), Some(id));
    assert_eq!(sup.entity_for_alert(id), Some("sensor.tank"));

    let mut step = sup.step(std::slice::from_ref(&probe), &ctx(NOW + H));
    assert!(step.opened.is_empty() && step.closed.is_empty());
    assert!(alerts(&mut step, true, true, NOW + H).is_empty());

    let moved = Reading {
        value: Some(15.5),
        last_change: Some(NOW + H + 600),
        ..probe
    };
    let mut step = sup.step(&[moved], &ctx(NOW + 2 * H));
    assert_eq!(step.closed.len(), 1);
    assert!(!step.closed[0].silent);
    assert_eq!(step.closed[0].at, NOW + 2 * H);
    let sent = alerts(&mut step, true, true, NOW + 2 * H);
    assert_eq!(sent.len(), 1);
    assert_eq!(key(&sent[0]), "supervisor.recovered");
    assert_eq!(sent[0]["severity"], "info");
    assert_eq!(sent[0]["title"], "Tank temperature is back to normal");
    assert!(sent[0].get("actions").is_none());
    assert_eq!(sup.open().count(), 0);
}

#[test]
fn no_baseline_keeps_the_problem_open() {
    let mut sup = Supervisor::new(vec![open_problem(
        "sensor.tank",
        Check::Stuck,
        NOW - 2 * H,
        Some(3 * H),
    )]);
    let reading = Reading {
        baseline: None,
        ..sensor("sensor.tank", "Tank temperature")
    };
    let step = sup.step(&[reading], &ctx(NOW));
    assert!(step.opened.is_empty() && step.closed.is_empty());
    assert_eq!(sup.open().count(), 1);
}

#[test]
fn silent_paused_neither_opens_nor_closes() {
    let mut sup = Supervisor::new(vec![open_problem(
        "sensor.tank",
        Check::Silent,
        NOW - 2 * H,
        Some(3 * H),
    )]);
    let fresh = Reading {
        last_change: Some(NOW - 60),
        last_report: Some(NOW - 60),
        ..sensor("sensor.tank", "Tank temperature")
    };
    let paused = Ctx {
        silent_paused: true,
        ..ctx(NOW)
    };
    let step = sup.step(&[fresh.clone(), silent("sensor.b", "B")], &paused);
    assert!(step.opened.is_empty() && step.closed.is_empty());

    let step = sup.step(&[fresh], &ctx(NOW));
    assert_eq!(step.closed.len(), 1, "a report after opening closes it");
}

#[test]
fn offline_problem_closes_when_back_online() {
    let mut sup = Supervisor::new(vec![open_problem(
        "sensor.tank",
        Check::Silent,
        NOW - 2 * H,
        None,
    )]);
    let still_offline = Reading {
        last_change: Some(NOW - 60),
        unavailable_since: Some(NOW - 3 * H),
        ..sensor("sensor.tank", "Tank temperature")
    };
    assert!(
        sup.step(std::slice::from_ref(&still_offline), &ctx(NOW))
            .closed
            .is_empty()
    );

    let online = Reading {
        unavailable_since: None,
        ..still_offline
    };
    assert_eq!(sup.step(&[online], &ctx(NOW + H)).closed.len(), 1);
}

#[test]
fn impossible_closes_when_back_in_range() {
    let mut sup = Supervisor::new(Vec::new());
    let wet = Reading {
        device_class: Some("humidity".into()),
        unit: Some("%".into()),
        value: Some(140.0),
        baseline: None,
        ..sensor("sensor.hum", "Bathroom humidity")
    };
    let mut step = sup.step(std::slice::from_ref(&wet), &ctx(NOW));
    let sent = alerts(&mut step, true, true, NOW);
    assert_eq!(key(&sent[0]), "supervisor.impossible");
    assert_eq!(
        sent[0]["title"],
        "Bathroom humidity reads an impossible value"
    );
    assert_eq!(
        sent[0]["body"],
        "140.0 % is outside what this sensor can measure. Check the sensor."
    );

    let fine = Reading {
        value: Some(55.0),
        ..wet
    };
    assert_eq!(sup.step(&[fine], &ctx(NOW + H)).closed.len(), 1);
}

#[test]
fn vanished_entity_closes_silently() {
    let mut sup = Supervisor::new(vec![open_problem(
        "sensor.gone",
        Check::Stuck,
        NOW - 2 * H,
        Some(3 * H),
    )]);
    let mut step = sup.step(&[], &ctx(NOW));
    assert_eq!(step.closed.len(), 1);
    assert!(step.closed[0].silent);
    assert!(alerts(&mut step, true, true, NOW).is_empty());
    assert_eq!(sup.open().count(), 0);
}

#[test]
fn four_opened_in_one_run_group() {
    let readings: Vec<Reading> = ["A", "B", "C", "D"]
        .iter()
        .map(|n| silent(&format!("sensor.{n}"), n))
        .collect();
    let mut sup = Supervisor::new(Vec::new());
    let mut step = sup.step(&readings, &ctx(NOW));
    assert_eq!(step.opened.len(), 4);
    let sent = alerts(&mut step, true, true, NOW);
    assert_eq!(sent.len(), 1);
    let g = &sent[0];
    assert_eq!(key(g), "supervisor.group");
    assert_eq!(g["severity"], "warning");
    assert!(g.get("actions").is_none());
    assert!(g.get("entity_id").is_none());
    assert!(g["id"].as_str().unwrap().starts_with("supervisor-group-"));
    assert_eq!(g["title"], "4 sensors look wrong");
    assert_eq!(g["body"], "A, B, C and 1 more");
    assert_eq!(sup.open().count(), 4, "each problem is still recorded");

    // Their recovery, in one run, groups the same way.
    let back: Vec<Reading> = readings
        .iter()
        .map(|r| Reading {
            last_report: Some(NOW + H),
            last_change: Some(NOW + H),
            ..r.clone()
        })
        .collect();
    let mut step = sup.step(&back, &ctx(NOW + 2 * H));
    let sent = alerts(&mut step, true, true, NOW + 2 * H);
    assert_eq!(sent.len(), 1);
    assert_eq!(key(&sent[0]), "supervisor.group");
    assert_eq!(sent[0]["severity"], "info");
    assert_eq!(sent[0]["title"], "4 sensors are back to normal");

    let mut sup = Supervisor::new(Vec::new());
    let mut step = sup.step(&readings[..3], &ctx(NOW));
    let sent = alerts(&mut step, true, true, NOW);
    assert_eq!(sent.len(), 3);
    assert!(sent.iter().all(|a| key(a) == "supervisor.silent"));
    assert_eq!(sent[0]["title"], "A stopped reporting");
    assert_eq!(
        sent[0]["body"],
        "No report for 10 h. Check the sensor or its battery."
    );
}

#[test]
fn notify_off_tracks_but_sends_nothing() {
    let probe = sensor("sensor.tank", "Tank temperature");
    let mut sup = Supervisor::new(Vec::new());
    let mut step = sup.step(std::slice::from_ref(&probe), &ctx(NOW));
    assert!(alerts(&mut step, false, true, NOW).is_empty());
    assert_eq!(step.opened[0].alert_id, None);
    assert_eq!(sup.open().count(), 1);

    let mut step = sup.step(&[probe], &ctx(NOW + H));
    assert!(alerts(&mut step, true, true, NOW + H).is_empty());
}

#[test]
fn recovered_suppressed() {
    let mut sup = Supervisor::new(vec![open_problem(
        "sensor.tank",
        Check::Stuck,
        NOW - 2 * H,
        Some(3 * H),
    )]);
    let moved = Reading {
        last_change: Some(NOW - 60),
        ..sensor("sensor.tank", "Tank temperature")
    };
    let mut step = sup.step(&[moved], &ctx(NOW));
    assert_eq!(step.closed.len(), 1);
    assert!(alerts(&mut step, true, false, NOW).is_empty());
}

#[test]
fn mute_removes_without_push() {
    let loaded = open_problem("sensor.tank", Check::Stuck, NOW - 2 * H, Some(3 * H));
    let id = loaded.alert_id.clone().unwrap();
    let mut sup = Supervisor::new(vec![
        loaded,
        open_problem("sensor.other", Check::Stuck, NOW - 2 * H, Some(3 * H)),
    ]);
    assert_eq!(sup.entity_for_alert(&id), Some("sensor.tank"));
    let muted = sup.mute("sensor.tank");
    assert_eq!(muted.len(), 1);
    assert_eq!(muted[0].entity_id, "sensor.tank");
    assert_eq!(sup.entity_for_alert(&id), None);
    assert_eq!(sup.open().count(), 1);
    assert!(sup.mute("sensor.tank").is_empty());
}

#[test]
fn clear_returns_everything() {
    let mut sup = Supervisor::new(vec![
        open_problem("sensor.a", Check::Stuck, NOW - 2 * H, Some(3 * H)),
        open_problem("sensor.b", Check::Silent, NOW - 2 * H, None),
    ]);
    assert_eq!(sup.clear().len(), 2);
    assert_eq!(sup.open().count(), 0);
}

#[test]
fn loc_args_stay_within_the_push_limits() {
    let long = "x".repeat(150);
    let readings: Vec<Reading> = (0..5)
        .map(|i| silent(&format!("sensor.{i}"), &format!("{long}{i}")))
        .collect();
    let mut sup = Supervisor::new(Vec::new());
    let mut step = sup.step(&readings, &ctx(NOW));
    let sent = alerts(&mut step, true, true, NOW);
    let mut step2 = Supervisor::new(Vec::new()).step(&readings[..1], &ctx(NOW));
    let single = alerts(&mut step2, true, true, NOW);
    for a in sent.iter().chain(&single) {
        let args = a["loc"]["args"].as_object().unwrap();
        assert!(args.len() <= 8);
        for (name, value) in args {
            let value = value.as_str().expect("string arg");
            assert!(value.chars().count() <= 200, "{name} too long");
        }
    }
}
