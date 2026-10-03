//! MIKE-49 checkpoint 1: a fixture-only, simulated-data-only Desktop audit
//! command and local audit view.
//!
//! Scope, deliberately narrow: this module proves the Desktop UI entry
//! point end to end against an in-process, disposable-key fixture — real
//! relay/key wiring, merge into `main`, and any live validation run are
//! explicitly held for a later, separately-approved step (see the PR
//! description this module ships with).
//!
//! This module and its submodules never reference `AppState`, `archive`,
//! or `native_relay_client` — the [`command::mike49_run_fixture_audit`]
//! Tauri command takes no `AppState` parameter, so there is no code path by
//! which it could reach the app's real signing key
//! (`desktop/src-tauri/src/app_state.rs`).

mod command;
mod fixture;
mod sanitize;

pub use command::mike49_run_fixture_audit;
