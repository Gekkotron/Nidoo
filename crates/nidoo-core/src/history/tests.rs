use super::*;
use crate::model::{
    Availability, ClimateState, CoverState, EntityState, LightState, StateValue, VacuumState,
    WaterHeaterState,
};

fn incoming(id: &str, at: Micros, value: StateValue, availability: Availability) -> Incoming {
    Incoming {
        at,
        id: id.into(),
        num: num_of(&value),
        state: EntityState {
            value,
            attributes: serde_json::Map::new(),
            availability,
            last_update: Some((at / 1_000_000) as u64),
        },
    }
}

#[test]
fn first_state_writes_value_and_availability() {
    let mut lw = LastWritten::default();
    let mut rows = Rows::default();
    lw.diff(
        &incoming("a", 1, StateValue::Number(21.5), Availability::Online),
        &mut rows,
    );
    assert_eq!(rows.values.len(), 1);
    assert_eq!(rows.availability.len(), 1);
    assert_eq!(rows.values[0].num, Some(21.5));
    assert_eq!(rows.availability[0].availability, Availability::Online);
}

#[test]
fn same_state_again_writes_nothing() {
    let mut lw = LastWritten::default();
    let mut rows = Rows::default();
    lw.diff(
        &incoming("a", 1, StateValue::Number(21.5), Availability::Online),
        &mut rows,
    );
    let mut rows2 = Rows::default();
    lw.diff(
        &incoming("a", 2, StateValue::Number(21.5), Availability::Online),
        &mut rows2,
    );
    assert!(rows2.is_empty());
}

#[test]
fn attributes_only_change_writes_nothing() {
    let mut lw = LastWritten::default();
    let mut rows = Rows::default();
    let base = incoming("a", 1, StateValue::Number(2.0), Availability::Online);
    lw.diff(&base, &mut rows);
    let mut rows2 = Rows::default();
    let mut later = incoming("a", 2, StateValue::Number(2.0), Availability::Online);
    later
        .state
        .attributes
        .insert("linkquality".into(), serde_json::json!(123));
    lw.diff(&later, &mut rows2);
    assert!(rows2.is_empty());
}

#[test]
fn availability_change_alone_writes_availability_row() {
    let mut lw = LastWritten::default();
    let mut rows = Rows::default();
    lw.diff(
        &incoming("a", 1, StateValue::Number(5.0), Availability::Online),
        &mut rows,
    );
    let mut rows2 = Rows::default();
    lw.diff(
        &incoming("a", 2, StateValue::Number(5.0), Availability::Offline),
        &mut rows2,
    );
    assert!(rows2.values.is_empty());
    assert_eq!(rows2.availability.len(), 1);
    assert_eq!(rows2.availability[0].availability, Availability::Offline);
}

#[test]
fn light_brightness_change_is_a_value_row_with_same_num() {
    let mut lw = LastWritten::default();
    let mut rows = Rows::default();
    let first = incoming(
        "l",
        1,
        StateValue::Light(LightState {
            on: true,
            brightness: Some(10),
            ..LightState::default()
        }),
        Availability::Online,
    );
    lw.diff(&first, &mut rows);
    assert_eq!(rows.values.len(), 1);
    assert_eq!(rows.values[0].num, Some(1.0));

    let mut rows2 = Rows::default();
    let second = incoming(
        "l",
        2,
        StateValue::Light(LightState {
            on: true,
            brightness: Some(200),
            ..LightState::default()
        }),
        Availability::Online,
    );
    lw.diff(&second, &mut rows2);
    assert_eq!(rows2.values.len(), 1);
    assert_eq!(rows2.values[0].num, Some(1.0));
}

#[test]
fn unknown_and_non_finite_write_nothing() {
    let mut lw = LastWritten::default();
    let mut rows = Rows::default();
    lw.diff(
        &incoming("a", 1, StateValue::Number(20.0), Availability::Online),
        &mut rows,
    );
    assert_eq!(rows.values.len(), 1);

    let mut rows2 = Rows::default();
    lw.diff(
        &incoming("a", 2, StateValue::Unknown, Availability::Online),
        &mut rows2,
    );
    assert!(rows2.is_empty());

    let mut rows3 = Rows::default();
    lw.diff(
        &incoming("a", 3, StateValue::Number(20.0), Availability::Online),
        &mut rows3,
    );
    assert!(rows3.is_empty(), "cache was disturbed by Unknown");

    let mut rows4 = Rows::default();
    lw.diff(
        &incoming("a", 4, StateValue::Number(f64::NAN), Availability::Online),
        &mut rows4,
    );
    assert!(rows4.is_empty());

    let mut rows5 = Rows::default();
    lw.diff(
        &incoming(
            "a",
            5,
            StateValue::Number(f64::INFINITY),
            Availability::Online,
        ),
        &mut rows5,
    );
    assert!(rows5.is_empty());

    let mut rows6 = Rows::default();
    lw.diff(
        &incoming("a", 6, StateValue::Number(20.0), Availability::Unknown),
        &mut rows6,
    );
    assert!(rows6.availability.is_empty());
}

#[test]
fn num_of_each_kind() {
    assert_eq!(num_of(&StateValue::OnOff(true)), Some(1.0));
    assert_eq!(num_of(&StateValue::OnOff(false)), Some(0.0));
    assert_eq!(
        num_of(&StateValue::Cover(CoverState {
            state: None,
            position: Some(40),
        })),
        Some(40.0)
    );
    assert_eq!(
        num_of(&StateValue::Climate(ClimateState {
            current: Some(19.5),
            ..ClimateState::default()
        })),
        Some(19.5)
    );
    assert_eq!(
        num_of(&StateValue::WaterHeater(WaterHeaterState {
            current: Some(55.0),
            ..WaterHeaterState::default()
        })),
        Some(55.0)
    );
    assert_eq!(num_of(&StateValue::Text("hello".into())), None);
    assert_eq!(num_of(&StateValue::Alarm("armed".into())), None);
    assert_eq!(num_of(&StateValue::Vacuum(VacuumState::default())), None);
    assert_eq!(num_of(&StateValue::Number(f64::NAN)), None);
    assert_eq!(num_of(&StateValue::Number(f64::INFINITY)), None);
}

#[test]
fn pending_drops_oldest_past_capacity() {
    let mut p = Pending::new(2);
    let a = incoming("a", 1, StateValue::Number(1.0), Availability::Online);
    let b = incoming("b", 2, StateValue::Number(2.0), Availability::Online);
    let c = incoming("c", 3, StateValue::Number(3.0), Availability::Online);
    assert!(p.push(a));
    assert!(p.push(b.clone()));
    assert!(!p.push(c.clone()));
    let taken = p.take(10);
    assert_eq!(taken, vec![b, c]);
}

#[test]
fn requeue_front_keeps_order() {
    let mut p = Pending::new(10);
    let a = incoming("a", 1, StateValue::Number(1.0), Availability::Online);
    let b = incoming("b", 2, StateValue::Number(2.0), Availability::Online);
    let c = incoming("c", 3, StateValue::Number(3.0), Availability::Online);
    p.push(a.clone());
    p.push(b.clone());
    p.push(c.clone());
    let batch = p.take(2);
    p.requeue_front(batch);
    let all = p.take(3);
    assert_eq!(all, vec![a, b, c]);
}

fn generic(fields: &[(&str, &str)], num: Option<f64>) -> Incoming {
    use crate::model::{GenericState, GenericValue};
    let mut state = GenericState::default();
    for (k, v) in fields {
        state
            .fields
            .insert((*k).to_owned(), GenericValue::Text((*v).to_owned()));
    }
    let mut i = incoming(
        "g",
        1_000_000,
        StateValue::Generic(state),
        Availability::Online,
    );
    i.num = num;
    i
}

#[test]
fn lock_change_is_recorded_as_one_and_zero() {
    let mut last = LastWritten::default();
    let mut rows = Rows::default();
    last.diff(&generic(&[("state", "locked")], Some(1.0)), &mut rows);
    last.diff(&generic(&[("state", "unlocked")], Some(0.0)), &mut rows);
    let nums: Vec<_> = rows.values.iter().map(|r| r.num).collect();
    assert_eq!(nums, vec![Some(1.0), Some(0.0)]);
}

#[test]
fn fan_preset_change_is_not_recorded() {
    let mut last = LastWritten::default();
    let mut rows = Rows::default();
    last.diff(&generic(&[("preset", "eco")], None), &mut rows);
    assert_eq!(rows.values.len(), 1);
    assert_eq!(rows.values[0].num, None);
}
