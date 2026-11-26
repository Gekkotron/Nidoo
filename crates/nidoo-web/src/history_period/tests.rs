use super::*;
use chrono::NaiveDate;

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn sp(a: NaiveDate, b: NaiveDate) -> Span {
    Span { from: a, to: b }
}

fn today() -> NaiveDate {
    d(2026, 11, 26)
}

fn state(unit: Unit, at: Option<NaiveDate>) -> PeriodState {
    PeriodState {
        unit,
        at,
        custom: None,
        vs: None,
        show: None,
    }
}

fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            out.push(u8::from_str_radix(&s[i + 1..i + 3], 16).unwrap());
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).unwrap()
}

fn parse_query(q: &str) -> PeriodState {
    let pairs: Vec<(String, String)> = q
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap();
            (k.to_string(), decode(v))
        })
        .collect();
    parse_state(|k| pairs.iter().find(|(pk, _)| pk == k).map(|(_, v)| v.clone()))
}

#[test]
fn parse_state_reads_legacy_ranges() {
    assert_eq!(parse_query("range=7d"), state(Unit::Week, None));
    assert_eq!(parse_query("range=1h"), state(Unit::Hour1, None));
    assert_eq!(parse_query("range=6h"), state(Unit::Hour6, None));
    assert_eq!(parse_query("range=24h"), state(Unit::Day, None));
    assert_eq!(parse_query("range=30d"), state(Unit::Month, None));
    assert_eq!(parse_query("range=1y"), state(Unit::Year, None));
    let s = parse_query("period=month&at=2025-11-12&vs=1");
    assert_eq!(s.unit, Unit::Month);
    assert_eq!(s.at, Some(d(2025, 11, 12)));
    assert_eq!(s.vs, Some(1));
    assert_eq!(parse_query(""), state(Unit::Day, None));
}

#[test]
fn parse_state_rulings() {
    assert_eq!(parse_query("period=bogus"), state(Unit::Day, None));
    assert_eq!(
        parse_query("period=custom&from=2026-02-01"),
        state(Unit::Day, None)
    );
    assert_eq!(
        parse_query("period=custom&from=2026-03-01&to=2026-02-01"),
        state(Unit::Day, None)
    );
    assert_eq!(parse_query("period=day&vs=0").vs, None);
    assert_eq!(parse_query("period=day&vs=x").vs, None);
    assert_eq!(parse_query("period=day&show=").show, None);
    assert_eq!(parse_query("period=day&at=nope").at, None);
    assert_eq!(parse_query("period=week&range=1y").unit, Unit::Week);
}

#[test]
fn state_path_round_trips() {
    let s = PeriodState {
        unit: Unit::Custom,
        at: None,
        custom: Some(sp(d(2025, 11, 1), d(2026, 3, 31))),
        vs: Some(2),
        show: Some("sensor.a,sensor.b".to_string()),
    };
    let path = state_path("sensor.t", &s);
    assert!(path.starts_with("/history/sensor.t?period=custom"));
    let (_, q) = path.split_once('?').unwrap();
    assert_eq!(parse_query(q), s);
    let m = state(Unit::Month, Some(d(2026, 10, 1)));
    assert_eq!(
        state_path("a b", &m),
        "/history/a%20b?period=month&at=2026-10-01"
    );
}

#[test]
fn span_of_iso_week() {
    assert_eq!(
        span_of(Unit::Week, d(2026, 11, 26)),
        sp(d(2026, 11, 23), d(2026, 11, 29))
    );
    assert_eq!(
        span_of(Unit::Week, d(2026, 1, 1)),
        sp(d(2025, 12, 29), d(2026, 1, 4))
    );
    assert_eq!(
        span_of(Unit::Month, d(2028, 2, 10)),
        sp(d(2028, 2, 1), d(2028, 2, 29))
    );
    assert_eq!(
        span_of(Unit::Year, d(2026, 6, 1)),
        sp(d(2026, 1, 1), d(2026, 12, 31))
    );
}

#[test]
fn step_moves_by_unit_and_custom_length() {
    let cur = state(Unit::Month, None);
    let back = step(&cur, -1, today());
    assert_eq!(back.at, Some(d(2026, 10, 1)));
    let fwd = step(&back, 1, today());
    assert_eq!(fwd.at, None);
    let c = PeriodState {
        custom: Some(sp(d(2025, 11, 1), d(2026, 3, 31))),
        ..state(Unit::Custom, None)
    };
    let c2 = step(&c, -1, today());
    assert_eq!(c2.unit, Unit::Custom);
    assert_eq!(c2.custom, Some(sp(d(2025, 6, 3), d(2025, 10, 31))));
    assert_eq!(c2.at, None);
    let h = state(Unit::Hour1, None);
    assert_eq!(step(&h, -1, today()), h);
}

#[test]
fn can_step_stops_at_first_date() {
    let jan = state(Unit::Month, Some(d(2022, 1, 1)));
    assert!(!can_step(&jan, -1, today(), Some(d(2022, 1, 1))));
    assert!(can_step(&jan, -1, today(), None));
    assert!(can_step(&jan, 1, today(), None));
    let cur = state(Unit::Month, None);
    assert!(!can_step(&cur, 1, today(), None));
    assert!(can_step(&cur, -1, today(), Some(d(2022, 1, 1))));
    assert!(!can_step(&state(Unit::Hour6, None), -1, today(), None));
}

#[test]
fn is_current_and_switch_unit() {
    assert!(is_current(&state(Unit::Month, None), today()));
    assert!(is_current(&state(Unit::Hour1, None), today()));
    assert!(!is_current(
        &state(Unit::Month, Some(d(2026, 10, 1))),
        today()
    ));
    let mut s = state(Unit::Month, Some(d(2026, 10, 1)));
    s.vs = Some(1);
    s.show = Some("x".into());
    let w = switch_unit(&s, Unit::Year, today());
    assert_eq!((w.unit, w.at), (Unit::Year, None));
    assert_eq!((w.vs, w.show.as_deref()), (Some(1), Some("x")));
    let w = switch_unit(&s, Unit::Week, today());
    assert_eq!(w.at, Some(d(2026, 9, 28)));
    let cur = switch_unit(&state(Unit::Year, None), Unit::Day, today());
    assert_eq!(cur.at, None);
    let c = switch_unit(&s, Unit::Custom, today());
    assert_eq!(c.custom, Some(sp(d(2026, 10, 1), d(2026, 10, 31))));
    let h = switch_unit(&s, Unit::Hour1, today());
    assert_eq!((h.at, h.custom), (None, None));
    let c = switch_unit(&state(Unit::Hour6, None), Unit::Custom, today());
    assert_eq!(c.custom, Some(sp(today(), today())));
}

#[test]
fn compare_span_maps_feb_29() {
    let day = sp(d(2028, 2, 29), d(2028, 2, 29));
    assert_eq!(
        compare_span(Unit::Day, day, 1),
        sp(d(2027, 2, 28), d(2027, 2, 28))
    );
    let feb = sp(d(2028, 2, 1), d(2028, 2, 29));
    assert_eq!(
        compare_span(Unit::Month, feb, 1),
        sp(d(2027, 2, 1), d(2027, 2, 28))
    );
}

#[test]
fn compare_span_week_is_iso_week() {
    assert_eq!(
        compare_span(Unit::Week, sp(d(2026, 11, 23), d(2026, 11, 29)), 1),
        sp(d(2025, 11, 24), d(2025, 11, 30))
    );
}

#[test]
fn compare_span_week_53_falls_back() {
    assert_eq!(
        compare_span(Unit::Week, sp(d(2026, 12, 28), d(2027, 1, 3)), 1),
        sp(d(2025, 12, 22), d(2025, 12, 28))
    );
}

#[test]
fn compare_years_from_first() {
    let nov = sp(d(2026, 11, 1), d(2026, 11, 30));
    assert_eq!(compare_years(nov, Some(d(2022, 1, 1))), vec![1, 2, 3, 4]);
    assert_eq!(compare_years(nov, Some(d(2026, 1, 15))), Vec::<u8>::new());
    assert_eq!(compare_years(nov, None), Vec::<u8>::new());
}

#[test]
fn per_for_and_zoom() {
    let day = sp(today(), today());
    assert_eq!(per_for(Unit::Day, day), DailyPer::Hour);
    assert_eq!(per_for(Unit::Week, day), DailyPer::Day);
    assert_eq!(per_for(Unit::Year, day), DailyPer::Month);
    let c151 = sp(d(2025, 11, 1), d(2026, 3, 31));
    assert_eq!(per_for(Unit::Custom, c151), DailyPer::Day);
    let c400 = sp(d(2025, 1, 1), d(2026, 2, 4));
    assert_eq!(c400.days(), 400);
    assert_eq!(per_for(Unit::Custom, c400), DailyPer::Month);
    assert_eq!(zoom(Unit::Year, DailyPer::Month), Some(Unit::Month));
    assert_eq!(zoom(Unit::Month, DailyPer::Day), Some(Unit::Day));
    assert_eq!(zoom(Unit::Day, DailyPer::Hour), None);
    assert_eq!(zoom(Unit::Custom, DailyPer::Month), None);
}

#[test]
fn quick_picks() {
    let winter = sp(d(2026, 11, 1), d(2027, 3, 31));
    assert_eq!(this_winter(d(2026, 11, 26)), winter);
    assert_eq!(this_winter(d(2026, 6, 10)), winter);
    assert_eq!(this_winter(d(2027, 2, 2)), winter);
    assert_eq!(
        last_12_months(d(2026, 11, 26)),
        sp(d(2025, 11, 27), d(2026, 11, 26))
    );
    assert_eq!(
        all_data(d(2026, 1, 15), today()),
        sp(d(2026, 1, 15), today())
    );
}

#[test]
fn slugs_round_trip() {
    for u in [
        Unit::Hour1,
        Unit::Hour6,
        Unit::Day,
        Unit::Week,
        Unit::Month,
        Unit::Year,
        Unit::Custom,
    ] {
        assert_eq!(Unit::from_slug(u.slug()), Some(u));
    }
    assert_eq!(Unit::from_slug("x"), None);
}
