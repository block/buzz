//! Bounded completion of an already-admitted stop, using real disk storage.
use super::{mark_restart_pending, persist_last_error};
use crate::{
    app_state::{build_app_state, AppState},
    managed_agents::{
        load_managed_agents, save_managed_agents, ManagedAgentRecord, ManagedAgentRuntimeKey,
    },
    user_operation::UserOperationScope,
};
use tauri::Manager;

struct TestPaths {
    home: Option<std::ffi::OsString>,
    xdg: Option<std::ffi::OsString>,
    _temp: tempfile::TempDir,
    _lock: std::sync::MutexGuard<'static, ()>,
}
impl TestPaths {
    fn new() -> Self {
        let lock = crate::managed_agents::lock_path_mutex();
        let temp = tempfile::tempdir().unwrap();
        let home = std::env::var_os("HOME");
        let xdg = std::env::var_os("XDG_DATA_HOME");
        std::env::set_var("HOME", temp.path());
        std::env::set_var("XDG_DATA_HOME", temp.path());
        Self {
            home,
            xdg,
            _temp: temp,
            _lock: lock,
        }
    }
}
impl Drop for TestPaths {
    fn drop(&mut self) {
        for (key, value) in [("HOME", &self.home), ("XDG_DATA_HOME", &self.xdg)] {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }
}

#[test]
fn retired_restart_leaves_diagnostic_only_on_its_exact_stopped_record() {
    let _paths = TestPaths::new();
    let app = tauri::test::mock_builder()
        .manage(build_app_state())
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    let state = app.state::<AppState>();
    let operation = UserOperationScope::capture(&state).unwrap();
    let pubkey = nostr::Keys::generate().public_key().to_hex();
    let mut record: ManagedAgentRecord = serde_json::from_value(serde_json::json!({
        "pubkey": pubkey, "name": "Restart", "relay_url": "",
        "acp_command": "", "agent_command": "", "agent_args": [],
        "mcp_command": "", "turn_timeout_seconds": 0,
        "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-01T00:00:00Z"
    }))
    .unwrap();
    let keys = vec![
        ManagedAgentRuntimeKey::new(pubkey.clone(), "wss://first.example").unwrap(),
        ManagedAgentRuntimeKey::new(pubkey.clone(), "wss://second.example").unwrap(),
    ];
    let stopped = mark_restart_pending(&mut record, &keys).unwrap();
    save_managed_agents(app.handle(), &[record]).unwrap();
    // Cancellation before any completion still leaves both original destinations.
    let pending = load_managed_agents(app.handle()).unwrap()[0]
        .last_error
        .clone()
        .unwrap();
    assert!(pending.contains("first.example") && pending.contains("second.example"));
    // A successful first pair clears the marker and changes runtime metadata;
    // a later pair failure must still be recorded under the current scope.
    let mut partial = load_managed_agents(app.handle()).unwrap();
    partial[0].last_error = None;
    partial[0].updated_at = "2026-01-02T00:00:00Z".into();
    save_managed_agents(app.handle(), &partial).unwrap();
    persist_last_error(
        app.handle(),
        &pubkey,
        "second pair failed",
        &stopped,
        &operation,
    )
    .unwrap();
    let completed = load_managed_agents(app.handle()).unwrap();
    assert!(completed[0]
        .last_error
        .as_deref()
        .unwrap()
        .contains("second pair failed"));
    assert!(completed[0]
        .last_error
        .as_deref()
        .unwrap()
        .contains("second.example"));
    save_managed_agents(
        app.handle(),
        &[serde_json::from_value(stopped.clone()).unwrap()],
    )
    .unwrap();
    state
        .install_local_workspace("wss://other.example".into(), Some(nostr::Keys::generate()))
        .unwrap();
    assert!(operation.admit(&state).is_err());
    persist_last_error(app.handle(), &pubkey, "scope retired", &stopped, &operation).unwrap();
    let mut records = load_managed_agents(app.handle()).unwrap();
    let diagnostic = records[0].last_error.as_deref().unwrap();
    assert!(diagnostic.contains("scope retired"));
    assert!(diagnostic.contains("first.example") && diagnostic.contains("second.example"));
    assert!(!diagnostic.contains("other.example"));
    // Completion cannot overwrite subsequent edits, including a manual restart.
    records[0].name = "Changed".into();
    records[0].last_error = None;
    save_managed_agents(app.handle(), &records).unwrap();
    persist_last_error(app.handle(), &pubkey, "late failure", &stopped, &operation).unwrap();
    let current = load_managed_agents(app.handle()).unwrap();
    assert_eq!(current[0].name, "Changed");
    assert!(current[0].last_error.is_none());
    save_managed_agents(app.handle(), &[]).unwrap();
    assert!(
        persist_last_error(app.handle(), &pubkey, "late failure", &stopped, &operation).is_err()
    );
    assert!(load_managed_agents(app.handle()).unwrap().is_empty());
}
