//! Drive the production blocking list core, including its real store-lock wait.
use super::list_managed_agents_in_scope;
use crate::{
    app_state::{build_app_state, AppState},
    user_operation::UserOperationScope,
};
use tauri::Manager;

#[test]
fn queued_list_rejects_changed_workspace_before_touching_disk() {
    let _paths = TestPaths::new();
    let app = tauri::test::mock_builder()
        .manage(build_app_state())
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    let state = app.state::<AppState>();
    let operation = UserOperationScope::capture(&state).unwrap();
    let store = state.managed_agents_store_lock.lock().unwrap();
    // A retired request must fail before trying to load/create records,
    // personas, or runtime receipts in the isolated empty app store.
    std::thread::scope(|threads| {
        let handle = app.handle().clone();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let worker = threads.spawn(move || {
            let state = handle.state::<AppState>();
            started_tx.send(()).unwrap();
            list_managed_agents_in_scope(&handle, &state, &operation)
        });
        started_rx.recv().unwrap();
        let keys = state.local_identity_keys().unwrap();
        state
            .install_local_workspace("wss://changed.example".into(), Some(keys))
            .unwrap();
        drop(store);
        let result = worker.join().unwrap();
        assert_eq!(
            result.unwrap_err(),
            "owner authorization scope changed; retry operation"
        );
    });
}

// Persona loading can persist built-ins even for an empty list. Keep both the
// regression and its mutation-test failure path away from the real app store.
struct TestPaths {
    old_home: Option<std::ffi::OsString>,
    old_xdg: Option<std::ffi::OsString>,
    _temp: tempfile::TempDir,
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl TestPaths {
    fn new() -> Self {
        let lock = crate::managed_agents::lock_path_mutex();
        let temp = tempfile::tempdir().unwrap();
        let old_home = std::env::var_os("HOME");
        let old_xdg = std::env::var_os("XDG_DATA_HOME");
        std::env::set_var("HOME", temp.path());
        std::env::set_var("XDG_DATA_HOME", temp.path());
        Self {
            old_home,
            old_xdg,
            _temp: temp,
            _lock: lock,
        }
    }
}

impl Drop for TestPaths {
    fn drop(&mut self) {
        for (name, value) in [("HOME", &self.old_home), ("XDG_DATA_HOME", &self.old_xdg)] {
            match value {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
    }
}

#[test]
fn current_recovery_scope_still_lists_independent_agents() {
    let _paths = TestPaths::new();
    let state = build_app_state();
    state
        .identity_lost
        .store(true, std::sync::atomic::Ordering::Release);
    let app = tauri::test::mock_builder()
        .manage(state)
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    let state = app.state::<AppState>();
    let operation = UserOperationScope::capture(&state).unwrap();
    assert!(
        list_managed_agents_in_scope(app.handle(), &state, &operation)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn registered_list_ipc_preserves_recovery_housekeeping() {
    for command in ["list_managed_agents", "list_personas", "list_teams"] {
        assert_registered_list_preserves_recovery(command);
    }
}

fn assert_registered_list_preserves_recovery(command: &str) {
    let _paths = TestPaths::new();
    let state = build_app_state();
    state
        .identity_lost
        .store(true, std::sync::atomic::Ordering::Release);
    let app = tauri::test::mock_builder()
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            super::list_managed_agents,
            crate::commands::list_personas,
            crate::commands::list_teams
        ])
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .unwrap();
    let response = tauri::test::get_ipc_response(
        &webview,
        tauri::webview::InvokeRequest {
            cmd: command.into(),
            callback: tauri::ipc::CallbackFn(0),
            error: tauri::ipc::CallbackFn(1),
            url: if cfg!(windows) {
                "http://tauri.localhost"
            } else {
                "tauri://localhost"
            }
            .parse()
            .unwrap(),
            body: tauri::ipc::InvokeBody::Json(serde_json::json!({})),
            headers: Default::default(),
            invoke_key: tauri::test::INVOKE_KEY.to_string(),
        },
    )
    .unwrap();
    let value = response.deserialize::<serde_json::Value>().unwrap();
    assert!(value.is_array(), "{command} failed recovery listing");
    if command == "list_managed_agents" {
        assert_eq!(value, serde_json::json!([]));
    }
}

#[test]
fn registered_list_ipc_cannot_commit_after_workspace_change() {
    let _paths = TestPaths::new();
    let app = tauri::test::mock_builder()
        .manage(build_app_state())
        .invoke_handler(tauri::generate_handler![super::list_managed_agents])
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .unwrap();
    let state = app.state::<AppState>();
    let store = state.managed_agents_store_lock.lock().unwrap();
    let relay = state.relay_url_override.lock().unwrap();
    std::thread::scope(|threads| {
        let worker = threads.spawn(move || {
            tauri::test::get_ipc_response(
                &webview,
                tauri::webview::InvokeRequest {
                    cmd: "list_managed_agents".into(),
                    callback: tauri::ipc::CallbackFn(0),
                    error: tauri::ipc::CallbackFn(1),
                    url: if cfg!(windows) {
                        "http://tauri.localhost"
                    } else {
                        "tauri://localhost"
                    }
                    .parse()
                    .unwrap(),
                    body: tauri::ipc::InvokeBody::Json(serde_json::json!({})),
                    headers: Default::default(),
                    invoke_key: tauri::test::INVOKE_KEY.to_string(),
                },
            )
        });
        // Capture takes operation admission before reading the relay. Pin that read
        // until the actual registered async handler has begun capture: this
        // does not depend on sleeping long enough for the IPC worker to run.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let entered = loop {
            if matches!(
                state.operation_generation.try_lock(),
                Err(std::sync::TryLockError::WouldBlock)
            ) {
                break true;
            }
            if std::time::Instant::now() >= deadline {
                break false;
            }
            std::thread::yield_now();
        };
        // Release both locks even on timeout, so failure cannot hang the worker.
        drop(relay);
        if entered {
            state
                .install_local_workspace("wss://changed.example".into(), None)
                .unwrap();
        }
        drop(store);
        let response = worker.join().unwrap();
        assert!(
            entered,
            "registered handler did not enter operation capture"
        );
        assert_eq!(
            response.err(),
            Some(serde_json::json!(
                "owner authorization scope changed; retry operation"
            ))
        );
    });
}

#[test]
fn defaults_save_rejects_stale_scope_without_replacing_machine_preference() {
    use crate::commands::global_agent_config::save_defaults_in_scope;
    use crate::managed_agents::{load_global_agent_config, GlobalAgentConfig};
    let _paths = TestPaths::new();
    let app = tauri::test::mock_builder()
        .manage(build_app_state())
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    let state = app.state::<AppState>();
    let operation = UserOperationScope::capture(&state).unwrap();
    let first: GlobalAgentConfig =
        serde_json::from_value(serde_json::json!({"model": "first-model"})).unwrap();
    let second: GlobalAgentConfig =
        serde_json::from_value(serde_json::json!({"model": "second-model"})).unwrap();
    save_defaults_in_scope(app.handle(), &state, &first, &operation).unwrap();
    let before = serde_json::to_value(load_global_agent_config(app.handle()).unwrap()).unwrap();
    state
        .install_local_workspace("wss://changed.example".into(), None)
        .unwrap();
    let result = save_defaults_in_scope(app.handle(), &state, &second, &operation);
    assert_eq!(
        result.err().as_deref(),
        Some("owner authorization scope changed; retry operation")
    );
    assert_eq!(
        serde_json::to_value(load_global_agent_config(app.handle()).unwrap()).unwrap(),
        before
    );
    let current = UserOperationScope::capture(&state).unwrap();
    save_defaults_in_scope(app.handle(), &state, &second, &current).unwrap();
    assert_ne!(
        serde_json::to_value(load_global_agent_config(app.handle()).unwrap()).unwrap(),
        before
    );
}
