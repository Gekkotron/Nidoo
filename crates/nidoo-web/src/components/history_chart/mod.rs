//! History chart: pure geometry and label formatting; the Leptos component wraps both.

pub mod fetch;
pub mod format;
pub mod geom;
pub mod periods;
pub mod plot;
pub mod selection;
pub mod view;
pub mod window;

pub use format::paris_offset;
pub use view::{Flavor, HistoryChart};
pub use window::ChartWindow;
