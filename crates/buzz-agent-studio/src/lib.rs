#![deny(unsafe_code)]
#![warn(missing_docs)]
//! `buzz-agent-studio` — Agent Studio backend for Buzz Hive.
//!
//! Port target: [Ngxba/claude-code-cli-ui](https://github.com/Ngxba/claude-code-cli-ui).
//! Kind registry: `buzz_core::kind` range 47200–47399.

pub mod events;
pub mod graph;
pub mod graph_events;
pub mod graph_loader;
pub mod monitor;
pub mod skill_import;
