//! Launch restore's spawn-and-register phase against a removed relay, using
//! the snapshot `apply_workspace` scheduled it with.

use super::spawn_and_register_restored_agents;
use crate::app_state::AppState;
use crate::managed_agents::admission_test_support::{app_with_keyless_agent, reached_spawn, RELAY};
use crate::managed_agents::{load_managed_agents, readd_relay, remove_relay, AdmissionSnapshot};
use std::sync::atomic::AtomicBool;
use tauri::Manager;

/// Runs restore's spawn phase with `scheduled` and returns the agent's recorded start error.
fn restore(
    test: &crate::managed_agents::admission_test_support::TestApp,
    scheduled: &AdmissionSnapshot,
) -> Option<String> {
    let app = test.app.handle();
    let agents = load_managed_agents(app).unwrap();
    spawn_and_register_restored_agents(
        app,
        &AtomicBool::new(false),
        scheduled,
        RELAY,
        &agents,
        None,
    )
    .unwrap();
    load_managed_agents(app).unwrap()[0].last_error.clone()
}

#[test]
fn restore_scheduled_before_a_remove_and_readd_starts_nothing() {
    let test = app_with_keyless_agent();
    let state = test.app.state::<AppState>();
    let scheduled = AdmissionSnapshot::capture(&state);
    remove_relay(&state, RELAY).unwrap();
    readd_relay(&state, RELAY).unwrap();
    assert_eq!(restore(&test, &scheduled), None);

    // A restore scheduled after the re-add reaches the spawn.
    let error = restore(&test, &AdmissionSnapshot::capture(&state)).unwrap();
    assert!(reached_spawn(&error), "{error}");
}
