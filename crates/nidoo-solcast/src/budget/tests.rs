use super::*;
use std::collections::{BTreeMap, BTreeSet};

fn utc(s: &str) -> DateTime<Utc> {
    s.parse().unwrap()
}

fn day(s: &str) -> NaiveDate {
    s.parse().unwrap()
}

fn ledger(d: Option<&str>, calls: u32, last: Option<i64>, block: Option<i64>) -> Ledger {
    Ledger {
        utc_day: d.map(day),
        calls,
        last_attempt: last,
        blocked_until: block,
    }
}

#[test]
fn merge_same_day_keeps_max_calls_and_later_times() {
    let a = ledger(Some("2026-10-24"), 3, Some(100), None);
    let b = ledger(Some("2026-10-24"), 5, Some(50), Some(900));
    let expected = ledger(Some("2026-10-24"), 5, Some(100), Some(900));
    assert_eq!(a.merge(&b), expected);
    assert_eq!(b.merge(&a), expected);
}

#[test]
fn merge_different_days_keeps_the_later_day_with_its_calls() {
    let old = ledger(Some("2026-10-23"), 9, Some(10), Some(5));
    let new = ledger(Some("2026-10-24"), 1, Some(20), None);
    let expected = ledger(Some("2026-10-24"), 1, Some(20), Some(5));
    assert_eq!(old.merge(&new), expected);
    assert_eq!(new.merge(&old), expected);
}

#[test]
fn merge_with_default_takes_the_other() {
    let a = ledger(Some("2026-10-24"), 2, Some(7), Some(8));
    assert_eq!(Ledger::default().merge(&a), a);
    assert_eq!(a.merge(&Ledger::default()), a);
}

#[test]
fn refreshes_per_day_values() {
    assert_eq!(refreshes_per_day(10, 1), 8);
    assert_eq!(refreshes_per_day(10, 2), 4);
    assert_eq!(refreshes_per_day(8, 1), 6);
    assert_eq!(refreshes_per_day(3, 2), 1);
    assert_eq!(refreshes_per_day(1, 1), 1);
}

#[test]
fn slots_paris_october_eight() {
    let s = slots(
        Some(utc("2026-10-24T06:20:00Z")),
        Some(utc("2026-10-24T16:42:00Z")),
        day("2026-10-24"),
        chrono_tz::Europe::Paris,
        8,
    );
    assert_eq!(s.len(), 8);
    assert_eq!(s[0], utc("2026-10-24T05:50:00Z"));
    assert_eq!(s[7], utc("2026-10-24T15:42:00Z"));
    assert!(s.windows(2).all(|w| w[0] < w[1]));
}

#[test]
fn slots_one_refresh_is_morning_only() {
    let s = slots(
        Some(utc("2026-10-24T06:20:00Z")),
        Some(utc("2026-10-24T16:42:00Z")),
        day("2026-10-24"),
        chrono_tz::Europe::Paris,
        1,
    );
    assert_eq!(s, vec![utc("2026-10-24T05:50:00Z")]);
}

#[test]
fn slots_polar_fallback_uses_7_and_19_local() {
    let s = slots(None, None, day("2026-12-21"), chrono_tz::Europe::Paris, 2);
    // 07:00 CET = 06:00Z, first = 05:30Z; last = 19:00 CET - 1 h = 17:00Z.
    assert_eq!(
        s,
        vec![utc("2026-12-21T05:30:00Z"), utc("2026-12-21T17:00:00Z")]
    );
}

#[test]
fn slots_sunset_before_sunrise_falls_back() {
    let s = slots(
        Some(utc("2026-12-21T17:00:00Z")),
        Some(utc("2026-12-21T06:00:00Z")),
        day("2026-12-21"),
        chrono_tz::Europe::Paris,
        2,
    );
    assert_eq!(
        s,
        vec![utc("2026-12-21T05:30:00Z"), utc("2026-12-21T17:00:00Z")]
    );
}

#[test]
fn ledger_resets_on_new_utc_day() {
    let mut l = Ledger::default();
    l.record_attempt(utc("2026-10-24T23:59:00Z"));
    l.record_attempt(utc("2026-10-24T23:59:30Z"));
    assert_eq!(l.calls_on(utc("2026-10-24T23:59:59Z")), 2);
    assert_eq!(l.calls_on(utc("2026-10-25T00:00:00Z")), 0);
    l.record_attempt(utc("2026-10-25T00:01:00Z"));
    assert_eq!(l.calls, 1);
    assert_eq!(l.utc_day, Some(day("2026-10-25")));
    assert_eq!(
        l.last_attempt,
        Some(utc("2026-10-25T00:01:00Z").timestamp())
    );
}

#[test]
fn quota_exceeded_blocks_until_next_utc_midnight_plus_5() {
    let mut l = Ledger::default();
    let now = utc("2026-10-24T10:00:00Z");
    l.record_quota_exceeded(now, 10);
    assert_eq!(l.calls_on(now), 10);
    let until = utc("2026-10-25T00:05:00Z");
    assert_eq!(l.blocked_until, Some(until.timestamp()));
    assert_eq!(
        decide(now, &l, false, &[], 10, 1),
        Decision::SleepUntil(until)
    );
}

#[test]
fn decide_calls_when_no_forecast_and_budget_left() {
    let l = Ledger::default();
    assert_eq!(
        decide(utc("2026-10-24T22:00:00Z"), &l, false, &[], 10, 1),
        Decision::CallNow
    );
}

#[test]
fn decide_never_calls_over_limit_even_without_forecast() {
    let now = utc("2026-10-24T10:00:00Z");
    let l = Ledger {
        utc_day: Some(now.date_naive()),
        calls: 9,
        ..Ledger::default()
    };
    // 9 + 2 sites > 10.
    assert_eq!(
        decide(now, &l, false, &[], 10, 2),
        Decision::SleepUntil(utc("2026-10-25T00:05:00Z"))
    );
    // 9 + 1 == 10 is allowed.
    assert_eq!(decide(now, &l, false, &[], 10, 1), Decision::CallNow);
}

#[test]
fn decide_missed_slots_coalesce_into_one_call() {
    let slots = [
        utc("2026-10-24T06:00:00Z"),
        utc("2026-10-24T08:00:00Z"),
        utc("2026-10-24T10:00:00Z"),
        utc("2026-10-24T12:00:00Z"),
    ];
    let mut l = Ledger::default();
    l.record_attempt(utc("2026-10-24T05:00:00Z"));
    let now = utc("2026-10-24T10:30:00Z");
    assert_eq!(decide(now, &l, true, &slots, 10, 1), Decision::CallNow);
    l.record_attempt(now);
    assert_eq!(
        decide(now, &l, true, &slots, 10, 1),
        Decision::SleepUntil(slots[3])
    );
}

#[test]
fn decide_sleeps_at_night_until_morning_slot() {
    let slots = [utc("2026-10-24T06:00:00Z"), utc("2026-10-25T06:00:00Z")];
    let mut l = Ledger::default();
    l.record_attempt(utc("2026-10-24T06:00:00Z"));
    let now = utc("2026-10-24T22:00:00Z");
    assert_eq!(
        decide(now, &l, true, &slots, 10, 1),
        Decision::SleepUntil(slots[1])
    );
    // No future slot known: look again in an hour.
    assert_eq!(
        decide(now, &l, true, &slots[..1], 10, 1),
        Decision::SleepUntil(utc("2026-10-24T23:00:00Z"))
    );
}

#[test]
fn failed_first_call_waits_for_next_slot() {
    let now = utc("2026-10-24T10:00:00Z");
    let mut l = Ledger::default();
    l.record_attempt(now - Duration::minutes(1));
    let next = utc("2026-10-24T12:00:00Z");
    let early = [utc("2026-10-24T08:00:00Z"), next];
    assert_eq!(
        decide(now, &l, false, &early, 10, 1),
        Decision::SleepUntil(next)
    );
    let due = [utc("2026-10-24T09:59:30Z"), next];
    assert_eq!(decide(now, &l, false, &due, 10, 1), Decision::CallNow);
}

/// Restarts every 10 minutes with the forecast lost and the ledger persisted.
fn storm(start: DateTime<Utc>, hours: i64, sites: u32, slots: &[DateTime<Utc>]) {
    let mut l = Ledger::default();
    for step in 0..(hours * 6) {
        let now = start + Duration::minutes(10 * step);
        let prev = l.last_attempt;
        if decide(now, &l, false, slots, 10, sites) == Decision::CallNow {
            if let Some(p) = prev {
                assert!(
                    slots.iter().any(|s| s.timestamp() > p && *s <= now),
                    "call at {now} without a due slot"
                );
            }
            for _ in 0..sites {
                l.record_attempt(now);
            }
        }
        assert!(l.calls_on(now) <= 10);
    }
    assert!(l.calls >= 1);
}

#[test]
fn restart_storm_never_exceeds_limit() {
    let start = utc("2026-10-24T00:00:00Z");
    let slots: Vec<_> = (0..16).map(|k| start + Duration::minutes(90 * k)).collect();
    storm(start, 24, 1, &slots);
}

#[test]
fn restart_storm_across_utc_midnight_with_two_sites() {
    let start = utc("2026-10-24T23:00:00Z");
    let slots: Vec<_> = (0..8)
        .map(|k| start + Duration::minutes(30 + 30 * k))
        .collect();
    storm(start, 4, 2, &slots);
}

fn week_within_limit(zone: Tz) {
    const SITES: u32 = 2;
    const LIMIT: u32 = 10;
    let refreshes = refreshes_per_day(LIMIT, SITES);
    let start = utc("2026-06-01T00:00:00Z");
    let end = start + Duration::days(7);
    let sun = |d: NaiveDate| {
        let at = |h, m| {
            let naive = d.and_hms_opt(h, m, 0).unwrap();
            zone.from_local_datetime(&naive)
                .earliest()
                .unwrap()
                .with_timezone(&Utc)
        };
        (at(6, 30), at(18, 30))
    };

    let mut ledger = Ledger::default();
    let mut has_forecast = false;
    let mut per_utc_day: BTreeMap<NaiveDate, u32> = BTreeMap::new();
    let mut local_days_called: BTreeSet<NaiveDate> = BTreeSet::new();
    let mut now = start;
    while now < end {
        let today = now.with_timezone(&zone).date_naive();
        let mut all: Vec<_> = [-1i64, 0, 1]
            .iter()
            .flat_map(|d| {
                let date = today + Duration::days(*d);
                let (rise, set) = sun(date);
                slots(Some(rise), Some(set), date, zone, refreshes)
            })
            .collect();
        all.sort();
        if decide(now, &ledger, has_forecast, &all, LIMIT, SITES) == Decision::CallNow {
            if has_forecast {
                assert!(
                    all.iter()
                        .any(|s| *s <= now && now - *s <= Duration::minutes(5)),
                    "call at {now} not right after a slot"
                );
            }
            for _ in 0..SITES {
                ledger.record_attempt(now);
                *per_utc_day.entry(now.date_naive()).or_default() += 1;
            }
            has_forecast = true;
            local_days_called.insert(today);
        }
        assert!(ledger.calls_on(now) <= LIMIT);
        now += Duration::minutes(5);
    }

    assert!(per_utc_day.values().all(|c| *c <= LIMIT), "{per_utc_day:?}");
    let planned = refreshes * SITES;
    assert!(
        per_utc_day.iter().skip(1).all(|(_, c)| *c <= planned),
        "{per_utc_day:?}"
    );
    let midnight = |d: NaiveDate| {
        zone.from_local_datetime(&d.and_time(NaiveTime::MIN))
            .earliest()
            .unwrap()
            .with_timezone(&Utc)
    };
    let mut d = start.with_timezone(&zone).date_naive();
    let mut checked = 0;
    while midnight(d + Duration::days(1)) <= end {
        if midnight(d) >= start {
            assert!(local_days_called.contains(&d), "no call on local day {d}");
            checked += 1;
        }
        d += Duration::days(1);
    }
    assert!(checked >= 5, "checked only {checked} local days");
}

#[test]
fn week_within_limit_in_paris() {
    week_within_limit(chrono_tz::Europe::Paris);
}

#[test]
fn week_within_limit_in_los_angeles() {
    week_within_limit(chrono_tz::America::Los_Angeles);
}

#[test]
fn week_within_limit_in_sydney() {
    week_within_limit(chrono_tz::Australia::Sydney);
}
