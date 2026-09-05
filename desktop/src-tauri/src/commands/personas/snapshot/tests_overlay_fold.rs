//! Overlay-fold resolution (kind:30179) tests for export.
//!
//! Kept in a sibling file so `snapshot/tests.rs` stays under the
//! 1000-line gate; `#[path]`-included from there as a child module,
//! so `super::*` still resolves to the shared test helpers.
//!
//! Export resolves against `resolved_records(load_managed_agents(..))`, not
//! raw disk. These tests pin the two behaviors that fold buys export:
//! a relay-only agent resolves before its first START on this device, and a
//! follower exports the effective relay config instead of stale disk values.

use super::*;
use crate::managed_agents::private_config_overlay::PrivateConfigOverlay;
use buzz_core_pkg::private_managed_agent::{
    Payload, PrivateConfig, PrivateIdentity, FORMAT, VERSION,
};
use serde_json::{json, Map};

fn relay_payload(pubkey: &str, name: &str, prompt: &str) -> Payload {
    Payload {
        format: FORMAT.into(),
        version: VERSION,
        agent_pubkey: pubkey.into(),
        owner_pubkey: "11".repeat(32),
        generation: 1,
        previous_event_id: None,
        updated_at: "2026-08-07T00:00:00Z".into(),
        identity: PrivateIdentity {
            private_key_nsec: "nsec-test".into(),
            auth_tag: None,
        },
        config: PrivateConfig {
            relay_url: "wss://relay.example".into(),
            name: name.into(),
            persona_id: None,
            runtime: Some("goose".into()),
            model: Some("relay-model".into()),
            provider: None,
            system_prompt: Some(prompt.into()),
            parallelism: None,
            respond_to: Some("allowlist".into()),
            respond_to_allowlist: vec!["ab".repeat(32)],
            agent_command_override: None,
            agent_args: vec![],
            idle_timeout_seconds: None,
            max_turn_duration_seconds: None,
            env_vars: BTreeMap::new(),
            backend: json!({"type":"local"}),
            backend_agent_id: None,
            team_id: None,
            persona_name_in_team: None,
            relay_mesh: None,
            effort_level: None,
            extra: Map::new(),
        },
        extensions: BTreeMap::new(),
        extra: Map::new(),
    }
}

/// Gap B: a relay-only agent (30179 head, no disk record — never started
/// on this device) must resolve for export once the disk list is folded
/// through the overlay. Raw disk alone returns "agent not found".
#[test]
fn relay_only_agent_resolves_for_export_after_overlay_fold() {
    let mut overlay = PrivateConfigOverlay::default();
    overlay
        .insert(relay_payload("relay-only-pk", "Relay Only", "relay prompt"))
        .unwrap();
    let disk: Vec<ManagedAgentRecord> = vec![];

    // Raw disk: not found (the pre-fix behavior).
    assert!(resolve_from_lists("relay-only-pk", &disk, &[]).is_err());

    // Folded: resolves, with the relay config as the effective record.
    let folded = overlay.resolved_records(&disk);
    let (record, is_def) = resolve_from_lists("relay-only-pk", &folded, &[]).unwrap();
    assert!(!is_def);
    assert_eq!(record.name, "Relay Only");
    assert_eq!(record.system_prompt.as_deref(), Some("relay prompt"));
}

/// Gap A: a follower device with a stale disk record must export the
/// effective 30179 values, not the disk snapshot — and the exported
/// manifest must advertise the enforced respond_to via the instance
/// fallback (the overlay patch only populates instance fields).
#[test]
fn follower_exports_effective_relay_values_not_stale_disk() {
    use crate::managed_agents::agent_snapshot::build_snapshot;

    let mut overlay = PrivateConfigOverlay::default();
    overlay
        .insert(relay_payload("follower-pk", "Fresh Name", "fresh prompt"))
        .unwrap();
    let mut stale = make_instance("follower-pk", "some-persona");
    stale.name = "Stale Name".into();
    stale.system_prompt = Some("stale prompt".into());

    let folded = overlay.resolved_records(std::slice::from_ref(&stale));
    let (record, _) = resolve_from_lists("follower-pk", &folded, &[]).unwrap();
    assert_eq!(record.name, "Fresh Name");
    assert_eq!(record.system_prompt.as_deref(), Some("fresh prompt"));
    assert_eq!(record.model.as_deref(), Some("relay-model"));

    let snapshot = build_snapshot(record, MemoryLevel::None, vec![], None);
    assert_eq!(
        snapshot.definition.system_prompt.as_deref(),
        Some("fresh prompt")
    );
    assert_eq!(snapshot.definition.respond_to.as_deref(), Some("allowlist"));
    assert_eq!(
        snapshot.definition.respond_to_allowlist,
        vec!["ab".repeat(32)]
    );
}

#[cfg(not(target_os = "windows"))]
#[test]
fn deleted_config_is_not_exported_or_selected_for_persona_cascade() {
    use crate::app_state::{build_app_state, AppState};
    use crate::commands::personas::{
        collect_cascade_pubkeys, snapshot::load_effective_managed_agents,
    };
    use crate::managed_agents::{load_managed_agents, save_managed_agents};
    use std::sync::atomic::Ordering;
    use tauri::Manager;

    let _env_lock = crate::managed_agents::lock_path_mutex();
    let temp = tempfile::tempdir().unwrap();
    struct EnvGuard(&'static str, Option<std::ffi::OsString>);
    impl EnvGuard {
        fn set(key: &'static str, value: &std::path::Path) -> Self {
            let prior = std::env::var_os(key);
            std::env::set_var(key, value);
            Self(key, prior)
        }
    }
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match self.1.take() {
                Some(value) => std::env::set_var(self.0, value),
                None => std::env::remove_var(self.0),
            }
        }
    }
    let _home = EnvGuard::set("HOME", temp.path());
    let _xdg = EnvGuard::set("XDG_DATA_HOME", temp.path());
    let app = tauri::test::mock_builder()
        .manage(build_app_state())
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    let state = app.state::<AppState>();
    let _store = state.managed_agents_store_lock.lock().unwrap();
    state
        .managed_agent_authority_ready
        .store(true, Ordering::Release);
    let denied = make_instance(&"aa".repeat(32), "persona");
    let permitted = make_instance(&"bb".repeat(32), "persona");
    let disk = vec![denied.clone(), permitted.clone()];
    save_managed_agents(app.handle(), &disk).unwrap();
    state
        .private_managed_agent_overlay
        .lock()
        .unwrap()
        .deny_deleted_config(&denied.pubkey);

    // Exercise the production loader shared by export and cascade, not a
    // test-side copy of its filter. The denied identity remains on disk for Stop.
    let selected = load_effective_managed_agents(app.handle(), &state).unwrap();
    assert!(resolve_from_lists(&denied.pubkey, &selected, &[]).is_err());
    assert_eq!(
        resolve_from_lists(&permitted.pubkey, &selected, &[])
            .unwrap()
            .0
            .pubkey,
        permitted.pubkey
    );
    assert_eq!(
        collect_cascade_pubkeys(&selected, "persona"),
        vec![permitted.pubkey.clone()]
    );
    assert_eq!(load_managed_agents(app.handle()).unwrap(), disk);

    // A newer validated head restores selection without replacing local identity.
    let mut restored = relay_payload(&denied.pubkey, "Restored", "new prompt");
    restored.config.persona_id = Some("persona".into());
    state
        .private_managed_agent_overlay
        .lock()
        .unwrap()
        .insert(restored)
        .unwrap();
    let selected = load_effective_managed_agents(app.handle(), &state).unwrap();
    assert_eq!(
        resolve_from_lists(&denied.pubkey, &selected, &[])
            .unwrap()
            .0
            .system_prompt
            .as_deref(),
        Some("new prompt")
    );
    assert_eq!(collect_cascade_pubkeys(&selected, "persona").len(), 2);
    assert_eq!(load_managed_agents(app.handle()).unwrap(), disk);

    state
        .managed_agent_authority_ready
        .store(false, Ordering::Release);
    assert!(load_effective_managed_agents(app.handle(), &state)
        .unwrap_err()
        .contains("authority is unavailable"));
}
