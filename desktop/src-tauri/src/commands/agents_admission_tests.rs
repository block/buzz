//! The ordinary local start against a removed relay, through the production
//! `start_local_agent_with_preflight` on a mock app, and across its preflight
//! await.

use super::{start_local_agent_after_preflight, start_local_agent_with_preflight};
use crate::app_state::AppState;
use crate::managed_agents::admission_test_support::{
    app_with_keyless_agent, reached_spawn, refused, RELAY,
};
use crate::managed_agents::{readd_relay, remove_relay};
use tauri::Manager;

#[tokio::test]
async fn ordinary_start_refuses_a_removed_relay_until_readded() {
    let test = app_with_keyless_agent();
    let (app, state) = (test.app.handle(), test.app.state::<AppState>());
    let start =
        || start_local_agent_with_preflight(app, &state, &test.pubkey, false, None, None, None);

    remove_relay(&state, RELAY).unwrap();
    let error = start().await.unwrap_err();
    assert!(refused(&error), "{error}");

    readd_relay(&state, RELAY).unwrap();
    let error = start().await.unwrap_err();
    assert!(reached_spawn(&error), "{error}");
}

/// Runs the ordinary start with its preflight held open while `during` runs.
async fn start_across_preflight(during: impl FnOnce(&AppState)) -> String {
    let test = app_with_keyless_agent();
    let (app, state) = (test.app.handle(), test.app.state::<AppState>());
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
    let start = start_local_agent_after_preflight(
        app,
        &state,
        &test.pubkey,
        None,
        None,
        None,
        |_| async move {
            entered_tx.send(()).unwrap();
            release_rx.await.unwrap();
            Ok(())
        },
    );
    let control = async {
        entered_rx.await.unwrap();
        during(&state);
        release_tx.send(()).unwrap();
    };
    let (result, ()) = tokio::join!(start, control);
    result.unwrap_err()
}

#[tokio::test]
async fn ordinary_start_removed_and_readded_during_preflight_is_refused() {
    let error = start_across_preflight(|state| {
        remove_relay(state, RELAY).unwrap();
        readd_relay(state, RELAY).unwrap();
    })
    .await;
    assert!(refused(&error), "{error}");
}

#[tokio::test]
async fn ordinary_start_with_no_removal_during_preflight_reaches_spawn() {
    let error = start_across_preflight(|_| {}).await;
    assert!(reached_spawn(&error), "{error}");
}
