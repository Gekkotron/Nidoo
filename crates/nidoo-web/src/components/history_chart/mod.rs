//! History chart: pure geometry and label formatting; the Leptos component wraps both.

pub mod format;
pub mod geom;
pub mod view;

pub use format::paris_offset;
pub use view::{Flavor, HistoryChart};
