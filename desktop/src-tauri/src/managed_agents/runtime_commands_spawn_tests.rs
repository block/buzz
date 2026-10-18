//! Production-seam regression: `start_pair` must spawn the harness with the
//! relay spelling the community was configured with.
//!
//! Runtime-pair keys canonicalize loopback spellings (`localhost` →
//! `127.0.0.1`), but the relay binds a community to the literal connection
//! authority. Dialing the canonical key lands the harness in a different,
//! usually empty tenant (`discovered 0 channel(s)`). This drives the real
//! `start_pair` path with a fake ACP harness and reads the env the child
//! actually received, so reverting the call site to `key.relay_url` fails.

use tauri::Manager as _;

use super::start_pair;
use crate::app_state::AppState;
use crate::managed_agents::{save_managed_agents, AdmissionSnapshot, ManagedAgentRecord};

/// Wait for the fake harness to write its relay URL (or fail the test).
fn read_dialed_url(path: &std::path::Path) -> String {
    for _ in 0..100 {
        if let Ok(value) = std::fs::read_to_string(path) {
            if !value.is_empty() {
                return value;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    panic!("fake harness never recorded BUZZ_RELAY_URL");
}

#[test]
fn start_pair_dials_the_configured_loopback_spelling() {
    use std::io::Write as _;

    let dir = tempfile::tempdir().expect("tempdir");
    let dump = dir.path().join("relay-url.txt");
    let harness = dir.path().join("fake-acp");
    {
        let mut file = std::fs::File::create(&harness).expect("fake harness");
        writeln!(file, "#!/bin/sh").expect("shebang");
        writeln!(
            file,
            "printf '%s' \"$BUZZ_RELAY_URL\" > \"$BUZZ_TEST_DUMP\""
        )
        .expect("body");
    }
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&harness, std::fs::Permissions::from_mode(0o755))
            .expect("chmod fake harness");
    }

    #[cfg(feature = "system-keyring")]
    keyring::set_default_credential_builder(keyring::mock::default_credential_builder());

    let mut context = tauri::test::mock_context(tauri::test::noop_assets());
    context.config_mut().identifier = dir.path().to_str().expect("utf-8 tempdir").to_owned();
    let state = crate::app_state::build_app_state();
    *state.relay_url_override.lock().expect("relay override") = Some("ws://localhost:3000".into());
    let app = tauri::test::mock_builder()
        .manage(state)
        .build(context)
        .expect("mock app");

    let pubkey = nostr::Keys::generate().public_key().to_hex();
    let private_key = nostr::Keys::generate().secret_key().to_secret_hex();
    let record: ManagedAgentRecord = serde_json::from_value(serde_json::json!({
        "pubkey": pubkey,
        "name": "spawn seam agent",
        "private_key_nsec": private_key,
        "relay_url": "ws://localhost:3000",
        "acp_command": harness.to_string_lossy(),
        "agent_command": "",
        "agent_args": [],
        "mcp_command": "",
        "turn_timeout_seconds": 0,
        "system_prompt": null,
        "env_vars": { "BUZZ_TEST_DUMP": dump.to_string_lossy() },
        "start_on_app_launch": false,
        "created_at": "2026-01-01T00:00:00Z",
        "updated_at": "2026-01-01T00:00:00Z",
        "last_started_at": null,
        "last_stopped_at": null,
        "last_exit_code": null,
        "last_error": null
    }))
    .expect("record fixture");
    save_managed_agents(app.handle(), &[record]).expect("save record");

    let admission = AdmissionSnapshot::capture(&app.state::<AppState>());
    let status = start_pair(
        pubkey,
        "ws://localhost:3000".into(),
        true,
        None,
        &admission,
        app.handle().clone(),
    )
    .expect("start pair");

    // Identity stays canonical; only the dialed spelling is preserved.
    assert_eq!(status.relay_url, "ws://127.0.0.1:3000");
    assert_eq!(read_dialed_url(&dump), "ws://localhost:3000");
}
