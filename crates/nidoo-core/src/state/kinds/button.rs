//! Button: write-only, it never reads a state topic.

use crate::model::{Entity, EntityState};
use crate::state::{Applied, StateError};
use crate::template::Templates;

pub(super) fn apply(
    _entity: &Entity,
    _state: &mut EntityState,
    _topic: &str,
    _payload: &str,
    _templates: &Templates,
) -> Result<Applied, StateError> {
    Ok(Applied::NotForThisEntity)
}
