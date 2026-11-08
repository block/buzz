//! Launch restore against a removed relay: scheduled through the production
//! `launch_restore_task`, run through the real restore wrapper, with only
//! Phase A's live process sweeps replaced by an inert stand-in.

use super::{launch_restore_task, spawn_and_register_restored_agents};
use crate::app_state::AppState;
use crate::managed_agents::admission_test_support::{
    app_with_keyless_agent, reached_spawn, TestApp, RELAY,
};
use crate::managed_agents::{load_managed_agents, readd_relay, remove_relay, AdmissionSnapshot};
use std::sync::atomic::AtomicBool;
use tauri::Manager;

fn last_error(test: &TestApp) -> Option<String> {
    load_managed_agents(test.app.handle()).unwrap()[0]
        .last_error
        .clone()
}

/// Schedules restore, runs `before_run` before the deferred task executes,
/// then runs it and returns the agent's recorded start error.
async fn schedule_and_restore(before_run: impl FnOnce(&AppState)) -> Option<String> {
    let test = app_with_keyless_agent();
    let restore = launch_restore_task(test.app.handle().clone(), |_, _| {});
    before_run(&test.app.state::<AppState>());
    restore.await.unwrap();
    last_error(&test)
}

#[tokio::test]
async fn restore_scheduled_before_a_remove_and_readd_starts_nothing() {
    let error = schedule_and_restore(|state| {
        remove_relay(state, RELAY).unwrap();
        readd_relay(state, RELAY).unwrap();
    })
    .await;
    assert_eq!(error, None);
}

#[tokio::test]
async fn restore_scheduled_with_no_removal_reaches_spawn() {
    let error = schedule_and_restore(|_| {}).await.unwrap();
    assert!(reached_spawn(&error), "{error}");
}

#[test]
fn restore_spawn_phase_refuses_a_stale_snapshot() {
    let test = app_with_keyless_agent();
    let state = test.app.state::<AppState>();
    let app = test.app.handle();
    let scheduled = AdmissionSnapshot::capture(&state);
    remove_relay(&state, RELAY).unwrap();
    readd_relay(&state, RELAY).unwrap();
    let agents = load_managed_agents(app).unwrap();
    spawn_and_register_restored_agents(
        app,
        &AtomicBool::new(false),
        &scheduled,
        RELAY,
        &agents,
        None,
    )
    .unwrap();
    assert_eq!(last_error(&test), None);
}
