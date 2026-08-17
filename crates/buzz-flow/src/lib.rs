#![deny(unsafe_code)]
#![warn(missing_docs)]
//! `buzz-flow` — Flow Studio backend for Buzz Hive.
//!
//! Port target: [simstudioai/sim](https://github.com/simstudioai/sim).
//! See `docs/BUZZ_HIVE_MERGE_SPEC.md` for kind range 46200–46399.

pub mod blocks;
pub mod event_payloads;
pub mod events;
pub mod files;
pub mod knowledge;
pub mod projector;
pub mod tables;
pub mod tools;
pub mod workflow_bridge;
