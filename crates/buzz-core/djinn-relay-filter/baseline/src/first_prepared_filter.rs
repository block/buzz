//! Comparison-only immutable first verified generated implementation.
#[path = "../../src/adapter.rs"]
mod adapter;
#[path = "../../src/input.rs"]
mod input;
#[path = "../../evidence/first/djinn/solution/mod.rs"]
mod solution;
pub use adapter::{prepare_event, prepare_filters};
pub use input::{EventInput, FilterInput};
pub use solution::filters_match;
