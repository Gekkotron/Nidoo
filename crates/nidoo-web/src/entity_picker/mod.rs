//! The searchable entity picker: a pure model (what is listed, grouped and capped), the
//! predicates fields use to say what they accept, and the texts.

pub mod fields;
pub mod model;
pub mod needs;
pub mod view;

use std::sync::Arc;

use nidoo_api::EntityView;

use crate::i18n::picker::PickerText;

/// A yes/no question about an entity, shared between a field and the picker it opens.
pub type Keep = Arc<dyn Fn(&EntityView) -> bool + Send + Sync>;

/// The soft filter behind the picker's "Suggested" chip: its label and its predicate.
#[derive(Clone)]
pub struct Suggest {
    pub text: PickerText,
    pub keep: Keep,
}

pub use fields::{EntityAddButton, EntityPickField, EntityPickListField};
pub use view::{EntityPicker, Mode, PickerBody, Pinned};
