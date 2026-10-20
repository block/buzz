//! Mock-app fixture for the admission tests at the production spawn sites.
//! Its agent has no private key, so a start that passes admission reaches
//! `spawn_agent_child` and stops at the key refusal there — no process runs.
//! The keyring is swapped for keyring's in-memory mock, which holds no keys,
//! so the test never touches (or prompts for) the system keychain.

use super::{save_managed_agents, ManagedAgentRecord, RELAY_REMOVED_ERROR};
use tauri::test::MockRuntime;

pub const RELAY: &str = "wss://removed.example";

pub struct TestApp {
    pub app: tauri::App<MockRuntime>,
    pub pubkey: String,
    _data: tempfile::TempDir,
}

/// A mock app whose store holds one keyless local agent that starts on launch.
pub fn app_with_keyless_agent() -> TestApp {
    #[cfg(feature = "system-keyring")]
    keyring::set_default_credential_builder(keyring::mock::default_credential_builder());
    let data = tempfile::tempdir().unwrap();
    let mut context = tauri::test::mock_context(tauri::test::noop_assets());
    // An absolute identifier replaces the app data base dir.
    context.config_mut().identifier = data.path().to_str().unwrap().to_owned();
    let state = crate::app_state::build_app_state();
    *state.relay_url_override.lock().unwrap() = Some(RELAY.into());
    let app = tauri::test::mock_builder()
        .manage(state)
        .build(context)
        .unwrap();
    let pubkey = nostr::Keys::generate().public_key().to_hex();
    let record: ManagedAgentRecord = serde_json::from_value(serde_json::json!({
        "pubkey": pubkey, "name": "Admission Test Agent", "relay_url": RELAY,
        "acp_command": "", "agent_command": "", "agent_args": [], "mcp_command": "",
        "turn_timeout_seconds": 0, "system_prompt": null,
        "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-01T00:00:00Z",
        "last_started_at": null, "last_stopped_at": null, "last_exit_code": null,
        "last_error": null, "start_on_app_launch": true
    }))
    .unwrap();
    save_managed_agents(app.handle(), &[record]).unwrap();
    TestApp {
        app,
        pubkey,
        _data: data,
    }
}

/// A start error from the admission check rather than from spawning.
pub fn refused(error: &str) -> bool {
    error == RELAY_REMOVED_ERROR
}

/// A start error from `spawn_agent_child`'s key refusal: admission passed.
pub fn reached_spawn(error: &str) -> bool {
    error.contains("has no private key available")
}
