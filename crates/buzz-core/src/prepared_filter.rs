//! Complete generated structural matcher for prepared live subscriptions.
//!
//! The exact generated source and data-only projection are shared with the isolated
//! Djinn experiment. Normal Buzz builds do not depend on Djinn, Lean or its toolchain.
//! Scope, authentication, search and query limits remain at their existing owners.
#[path = "../djinn-relay-filter/src/adapter.rs"]
mod adapter;
#[path = "../djinn-relay-filter/src/input.rs"]
mod input;
#[path = "../djinn-relay-filter/djinn/solution/mod.rs"]
mod solution;

pub use adapter::{prepare_event, prepare_filters};
pub use input::{EventInput, FilterInput};
pub use solution::filters_match;
