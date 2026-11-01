//! The values a flow compares: a boolean, a number or a piece of text.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::model::Op;
use crate::model::{GenericState, GenericValue, StateValue};
use crate::types::{self, TypeDef};

/// A comparable value; the JSON form is the bare boolean, number or string.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(untagged)]
pub enum Scalar {
    Bool(bool),
    Number(f64),
    Text(String),
}

impl Scalar {
    /// The headline value of an entity state: on/off, a measurement or the state word.
    /// `None` when the entity has no such value (unknown, or a cover without a state yet).
    pub fn of_state(value: &StateValue) -> Option<Scalar> {
        match value {
            StateValue::OnOff(on) => Some(Scalar::Bool(*on)),
            StateValue::Number(n) => Some(Scalar::Number(*n)),
            StateValue::Text(text) | StateValue::Alarm(text) => Some(Scalar::Text(text.clone())),
            StateValue::Light(light) => Some(Scalar::Bool(light.on)),
            StateValue::Cover(cover) => cover.state.clone().map(Scalar::Text),
            StateValue::Climate(climate) => climate.mode.clone().map(Scalar::Text),
            StateValue::Vacuum(vacuum) => vacuum.state.clone().map(Scalar::Text),
            StateValue::WaterHeater(heater) => heater.mode.clone().map(Scalar::Text),
            // Needs the type table: see `Scalar::of_generic`.
            StateValue::Generic(_) => None,
            StateValue::Unknown => None,
        }
    }

    /// The primary field of a generic entity as a scalar.
    pub fn of_generic(def: &TypeDef, state: &GenericState) -> Option<Scalar> {
        Some(match types::state::primary(def, state)? {
            GenericValue::Bool(b) => Scalar::Bool(*b),
            GenericValue::Number(n) => Scalar::Number(*n),
            GenericValue::Text(t) => Scalar::Text(t.clone()),
        })
    }

    /// Reads a JSON boolean, finite number or string; null, arrays and objects are `None`.
    pub fn from_json(value: &Value) -> Option<Scalar> {
        match value {
            Value::Bool(b) => Some(Scalar::Bool(*b)),
            Value::Number(n) => n.as_f64().filter(|f| f.is_finite()).map(Scalar::Number),
            Value::String(s) => Some(Scalar::Text(s.clone())),
            _ => None,
        }
    }

    /// The JSON form; a non-finite number (never produced by [`Scalar::from_json`]) becomes null.
    pub fn to_json(&self) -> Value {
        match self {
            Scalar::Bool(b) => Value::Bool(*b),
            Scalar::Number(n) => {
                serde_json::Number::from_f64(*n).map_or(Value::Null, Value::Number)
            }
            Scalar::Text(s) => Value::String(s.clone()),
        }
    }
}

/// Why two scalars could not be ordered.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CompareError {
    /// An ordering operator needs numbers on both sides.
    #[error("both sides must be numbers")]
    NotNumeric,
}

/// Compares the entity's current value (`left`) with the node's value (`right`).
///
/// `eq` and `ne` are typed: values of different types are never equal, and an absent left side
/// equals nothing (so `ne` holds). The ordering operators need two numbers: an absent left side
/// is `Ok(false)` (there is nothing to order), while two present sides that are not both numbers
/// are [`CompareError::NotNumeric`].
pub fn compare(left: Option<&Scalar>, op: Op, right: &Scalar) -> Result<bool, CompareError> {
    match op {
        Op::Eq => Ok(left == Some(right)),
        Op::Ne => Ok(left != Some(right)),
        Op::IsDark => unreachable!("is_dark is handled in the engine before compare"),
        Op::IsOffPeak => unreachable!("is_off_peak is handled in the engine before compare"),
        Op::Gt | Op::Lt | Op::Gte | Op::Lte => match (left, right) {
            (None, _) => Ok(false),
            (Some(Scalar::Number(l)), Scalar::Number(r)) => Ok(match op {
                Op::Gt => l > r,
                Op::Lt => l < r,
                Op::Gte => l >= r,
                _ => l <= r,
            }),
            _ => Err(CompareError::NotNumeric),
        },
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn scalar_of_generic_is_its_primary() {
        use crate::model::{GenericState, GenericValue};
        let def = crate::types::type_def("lock").unwrap();
        let mut gs = GenericState::default();
        assert_eq!(Scalar::of_generic(def, &gs), None);
        gs.fields
            .insert("state".into(), GenericValue::Text("locked".into()));
        assert_eq!(
            Scalar::of_generic(def, &gs),
            Some(Scalar::Text("locked".into()))
        );
        assert_eq!(Scalar::of_state(&StateValue::Generic(gs)), None);
    }

    use serde_json::json;

    use super::*;
    use crate::model::{ClimateState, CoverState, LightState, VacuumState, WaterHeaterState};

    #[test]
    fn scalar_of_every_state_value() {
        let text = |s: &str| Some(Scalar::Text(s.to_owned()));
        assert_eq!(
            Scalar::of_state(&StateValue::OnOff(true)),
            Some(Scalar::Bool(true))
        );
        assert_eq!(
            Scalar::of_state(&StateValue::Number(2.5)),
            Some(Scalar::Number(2.5))
        );
        assert_eq!(Scalar::of_state(&StateValue::Text("a".into())), text("a"));
        let light = LightState {
            on: false,
            ..Default::default()
        };
        assert_eq!(
            Scalar::of_state(&StateValue::Light(light)),
            Some(Scalar::Bool(false))
        );
        let cover = |state: Option<&str>| {
            StateValue::Cover(CoverState {
                state: state.map(str::to_owned),
                position: None,
            })
        };
        assert_eq!(Scalar::of_state(&cover(Some("open"))), text("open"));
        assert_eq!(Scalar::of_state(&cover(None)), None);
        let climate = ClimateState {
            mode: Some("heat".into()),
            ..Default::default()
        };
        assert_eq!(
            Scalar::of_state(&StateValue::Climate(climate)),
            text("heat")
        );
        assert_eq!(
            Scalar::of_state(&StateValue::Alarm("armed_away".into())),
            text("armed_away")
        );
        let vacuum = VacuumState {
            state: Some("docked".into()),
            ..Default::default()
        };
        assert_eq!(
            Scalar::of_state(&StateValue::Vacuum(vacuum)),
            text("docked")
        );
        let heater = WaterHeaterState {
            mode: Some("eco".into()),
            ..Default::default()
        };
        assert_eq!(
            Scalar::of_state(&StateValue::WaterHeater(heater)),
            text("eco")
        );
        assert_eq!(Scalar::of_state(&StateValue::Unknown), None);
    }

    #[test]
    fn compare_is_typed() {
        let t = Scalar::Bool(true);
        let from = |v: Value| Scalar::from_json(&v).unwrap();
        assert_eq!(compare(Some(&t), Op::Eq, &from(json!(true))), Ok(true));
        assert_eq!(
            compare(Some(&t), Op::Eq, &Scalar::Text("true".into())),
            Ok(false)
        );
        assert_eq!(compare(None, Op::Eq, &t), Ok(false));
        assert_eq!(compare(None, Op::Ne, &t), Ok(true));
        assert_eq!(
            compare(Some(&Scalar::Number(3.0)), Op::Gt, &Scalar::Number(2.0)),
            Ok(true)
        );
        assert_eq!(
            compare(Some(&t), Op::Gt, &Scalar::Number(2.0)),
            Err(CompareError::NotNumeric)
        );
        assert_eq!(
            compare(
                Some(&Scalar::Text("a".into())),
                Op::Lt,
                &Scalar::Text("b".into())
            ),
            Err(CompareError::NotNumeric)
        );
        assert_eq!(compare(None, Op::Gt, &Scalar::Number(2.0)), Ok(false));
    }

    #[test]
    fn from_json_rejects_null_arrays_and_objects() {
        assert_eq!(Scalar::from_json(&json!(null)), None);
        assert_eq!(Scalar::from_json(&json!([1])), None);
        assert_eq!(Scalar::from_json(&json!({})), None);
        assert_eq!(Scalar::from_json(&json!(3)), Some(Scalar::Number(3.0)));
        assert_eq!(Scalar::Number(1.5).to_json(), json!(1.5));
    }
}
