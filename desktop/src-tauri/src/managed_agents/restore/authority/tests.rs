use super::*;
use crate::managed_agents::{private_config_overlay::test_relay_payload, BackendKind};
use tauri::Manager;

#[test]
fn restore_reloads_authority_and_membership_and_holds_scope_until_registration() {
    let _env = crate::managed_agents::lock_path_mutex();
    let temp = tempfile::tempdir().unwrap();
    struct Env(&'static str, Option<std::ffi::OsString>);
    impl Drop for Env {
        fn drop(&mut self) {
            match self.1.take() {
                Some(value) => std::env::set_var(self.0, value),
                None => std::env::remove_var(self.0),
            }
        }
    }
    let _vars: Vec<_> = ["HOME", "XDG_DATA_HOME"]
        .into_iter()
        .map(|key| {
            let guard = Env(key, std::env::var_os(key));
            std::env::set_var(key, temp.path());
            guard
        })
        .collect();
    let app = tauri::test::mock_builder()
        .manage(crate::app_state::build_app_state())
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    let state = app.state::<AppState>();
    state
        .managed_agent_authority_ready
        .store(true, std::sync::atomic::Ordering::Release);
    let scope = retention::active_retention_scope(app.handle(), &state)
        .unwrap()
        .db_path;
    let pubkey = "ab".repeat(32);
    let payload = test_relay_payload(&pubkey);
    let mut disk = {
        let mut overlay = state.private_managed_agent_overlay.lock().unwrap();
        overlay.insert(payload.clone()).unwrap();
        overlay.materialize_relay_only_record(&pubkey, &[]).unwrap()
    };
    disk.start_on_app_launch = true;
    disk.backend = BackendKind::Provider {
        id: "old-provider".into(),
        config: serde_json::json!({}),
    };
    crate::managed_agents::save_managed_agents(app.handle(), &[disk.clone()]).unwrap();
    assert_eq!(restore_candidate_pubkeys(&[disk.clone()]), vec![pubkey]);
    let prepared = vec![disk.clone()];
    let _transition = state.managed_agent_runtime_transition.lock().unwrap();
    {
        let authority = lock_restore_authority(app.handle(), &state, &scope, &prepared).unwrap();
        assert_eq!(authority.candidates.len(), 1);
        assert_eq!(authority.candidates[0].backend, BackendKind::Local);
        assert!(
            state.managed_agents_store_lock.try_lock().is_err(),
            "writers must wait until registration"
        );
        assert_eq!(
            authority.owner_hex,
            state.signing_keys().unwrap().public_key().to_hex()
        );
    }
    let mut changed = payload.clone();
    changed.config.name = "latest config at launch".into();
    state
        .private_managed_agent_overlay
        .lock()
        .unwrap()
        .insert(changed.clone())
        .unwrap();
    assert_eq!(
        lock_restore_authority(app.handle(), &state, &scope, &prepared)
            .unwrap()
            .candidates[0]
            .name,
        changed.config.name
    );
    changed.config.backend = serde_json::json!({"type":"provider","id":"new-provider","config":{}});
    state
        .private_managed_agent_overlay
        .lock()
        .unwrap()
        .insert(changed)
        .unwrap();
    assert!(
        lock_restore_authority(app.handle(), &state, &scope, &prepared)
            .unwrap()
            .candidates
            .is_empty()
    );
    state
        .private_managed_agent_overlay
        .lock()
        .unwrap()
        .insert(payload)
        .unwrap();
    disk.start_on_app_launch = false;
    crate::managed_agents::save_managed_agents(app.handle(), &[disk]).unwrap();
    assert!(
        lock_restore_authority(app.handle(), &state, &scope, &prepared)
            .unwrap()
            .candidates
            .is_empty()
    );
    crate::managed_agents::save_managed_agents(app.handle(), &[]).unwrap();
    assert!(
        lock_restore_authority(app.handle(), &state, &scope, &prepared)
            .unwrap()
            .candidates
            .is_empty(),
        "deleted row must not launch from a prepared clone"
    );
    state
        .managed_agent_authority_ready
        .store(false, std::sync::atomic::Ordering::Release);
    assert!(
        lock_restore_authority(app.handle(), &state, &scope, &prepared)
            .err()
            .unwrap()
            .contains("authority is unavailable")
    );
    state
        .managed_agent_authority_ready
        .store(true, std::sync::atomic::Ordering::Release);
    *state.keys.lock().unwrap() = nostr::Keys::generate();
    assert!(
        lock_restore_authority(app.handle(), &state, &scope, &prepared)
            .err()
            .unwrap()
            .contains("workspace changed")
    );
}
