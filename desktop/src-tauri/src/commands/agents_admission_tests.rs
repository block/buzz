//! The ordinary local start against a removed relay, through the production
//! `start_local_agent_with_preflight` on a mock app.

use super::start_local_agent_with_preflight;
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
