//! `start_pair` and the native pair restart against a removed relay, driven
//! through the production admission check on a mock app.

use super::{restart_pair, start_pair};
use crate::app_state::AppState;
use crate::managed_agents::admission_test_support::{
    app_with_keyless_agent, reached_spawn, refused, RELAY,
};
use crate::managed_agents::{readd_relay, remove_relay, AdmissionSnapshot};
use tauri::Manager;

fn start(
    test: &crate::managed_agents::admission_test_support::TestApp,
    admission: &AdmissionSnapshot,
) -> String {
    start_pair(
        test.pubkey.clone(),
        RELAY.into(),
        true,
        None,
        admission,
        test.app.handle().clone(),
    )
    .unwrap_err()
}

#[test]
fn start_pair_refuses_a_removed_relay_until_readded() {
    let test = app_with_keyless_agent();
    let state = test.app.state::<AppState>();
    let before = AdmissionSnapshot::capture(&state);
    remove_relay(&state, RELAY).unwrap();
    assert!(refused(&start(&test, &AdmissionSnapshot::capture(&state))));

    readd_relay(&state, RELAY).unwrap();
    // Work captured before the removal stays refused after the re-add.
    assert!(refused(&start(&test, &before)));
    // Fresh work after the re-add reaches the spawn.
    assert!(reached_spawn(&start(
        &test,
        &AdmissionSnapshot::capture(&state)
    )));
}

#[test]
fn restart_straddling_a_remove_and_readd_starts_nothing() {
    let test = app_with_keyless_agent();
    let app = test.app.handle().clone();
    let stop_app = app.clone();
    let error = restart_pair(test.pubkey.clone(), RELAY.into(), app.clone(), move || {
        let state = stop_app.state::<AppState>();
        remove_relay(&state, RELAY)?;
        readd_relay(&state, RELAY)
    })
    .unwrap_err();
    assert!(refused(&error), "{error}");

    // A restart that crosses no removal reaches the spawn.
    let error = restart_pair(test.pubkey.clone(), RELAY.into(), app, || Ok(())).unwrap_err();
    assert!(reached_spawn(&error), "{error}");
}
