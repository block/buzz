//! Epoch-level tests for `commands/global_agent_config.rs`.
//!
//! Included inside `mod tests` via `#[path]` from `global_agent_config_tests.rs`.
//! Heavy async epoch tests split here to keep each file under 1000 lines.

use super::*;
/// Full tail test: production epoch core with injected stop/spawn/receipt
/// closures. Verifies that the core calls stop, then spawn with captured
/// context (relay, owner, scope), then receipt, registers the runtime with
/// the captured scope_id, and saves the record.
///
/// Thufir's test 5: "call production restart_under_captured_epoch_for via
/// mock app with injected spawn_fn and write_receipt_fn; assert captured
/// relay/owner/teams/personas/global delivered to spawn, receipt constructed
/// and written, runtimes map contains new entry with context.scope.scope_id,
/// final captured disk record matches."
///
/// Cross-platform process helpers replace `sleep 10000` / `/usr/bin/true`:
/// - Seed runtime: `spawn_long_lived_child_for_test()` (survives sync eviction).
/// - Spawn closure: `spawn_noop_child_for_test()` (exits immediately; test only
///   checks in-memory state, not process liveness).
///
/// Non-empty personas, teams, and global are placed in both the context AND
/// the captured definitions_dir so the spawn_fn can assert they arrive.
#[tokio::test]
#[allow(clippy::await_holding_lock)] // SCOPE_GENERATION_TEST_LOCK serialises parallel tests
async fn test_full_tail_stop_spawn_receipt_register_save() {
    use crate::managed_agents::scope::SCOPE_GENERATION_TEST_LOCK;
    use crate::managed_agents::{
        storage::save_managed_agents_at, BackendKind, ManagedAgentPairRuntime, ManagedAgentRecord,
        ManagedAgentRuntimeKey,
    };
    use std::sync::{Arc, Mutex};
    use tauri::Manager;

    // Serialize generation-sensitive work across parallel tests.
    let _gen_guard = SCOPE_GENERATION_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    let tmp = tempfile::tempdir().unwrap();
    let pubkey = "bb".repeat(32);
    let relay_url = "wss://test.relay";
    let owner_hex = "cc".repeat(32);
    let scope_id = "scope-test-id";

    // Build a Ready record: provider+model in structured fields so the agent
    // passes the eligibility gate (old_ready = true). ANTHROPIC_API_KEY is
    // required by buzz_agent_requirements when provider=anthropic, so we set
    // it in env_vars to satisfy the readiness check. Two globals differ by one
    // env_var so env_changed = true → should_restart_on_config_change returns true.
    let mut record_env_vars = std::collections::BTreeMap::new();
    record_env_vars.insert(
        "ANTHROPIC_API_KEY".to_string(),
        "sk-test-key-for-readiness".to_string(),
    );
    let record = ManagedAgentRecord {
        pubkey: pubkey.clone(),
        name: "test-agent".to_string(),
        display_name: None,
        slug: None,
        persona_id: None,
        private_key_nsec: String::new(),
        auth_tag: None,
        relay_url: relay_url.to_string(),
        avatar_url: None,
        acp_command: crate::managed_agents::DEFAULT_ACP_COMMAND.to_string(),
        agent_command: String::new(),
        agent_command_override: None,
        agent_args: vec![],
        mcp_command: String::new(),
        turn_timeout_seconds: 0,
        idle_timeout_seconds: None,
        max_turn_duration_seconds: None,
        parallelism: 1,
        system_prompt: None,
        model: Some("claude-3-5-sonnet-20241022".to_string()),
        provider: Some("anthropic".to_string()),
        persona_source_version: None,
        env_vars: record_env_vars,
        start_on_app_launch: false,
        auto_restart_on_config_change: false,
        runtime_pid: None,
        backend: BackendKind::Local,
        backend_agent_id: None,
        provider_policy_pending: false,
        provider_binary_path: None,
        team_id: None,
        persona_team_dir: None,
        persona_name_in_team: None,
        created_at: crate::util::now_iso(),
        updated_at: crate::util::now_iso(),
        last_started_at: None,
        last_stopped_at: None,
        last_exit_code: None,
        last_error: None,
        last_error_code: None,
        respond_to: Default::default(),
        respond_to_allowlist: Default::default(),
        is_builtin: false,
        is_active: true,
        shared: false,
        source_team: None,
        source_team_persona_slug: None,
        catalog_source: None,
        library_ref: None,
        library_applied_revision: None,
        last_completed_deploy_attempt_id: None,
        definition_respond_to: None,
        definition_respond_to_allowlist: Default::default(),
        definition_parallelism: None,
        relay_mesh: None,
        effort_level: None,
        runtime: None,
        name_pool: vec![],
    };

    // Write initial store.
    save_managed_agents_at(tmp.path(), std::slice::from_ref(&record)).unwrap();
    std::fs::write(tmp.path().join("personas.json"), b"[]").unwrap();
    std::fs::write(tmp.path().join("global-agent-config.json"), b"{}").unwrap();

    let app = make_mock_app();
    let app_handle = app.handle().clone();

    // Seed a live runtime with a cross-platform long-lived child (avoids sync eviction).
    // `spawn_long_lived_child_for_test()` replaces `sleep 10000` / `ping -n 100000`.
    let rt_key = ManagedAgentRuntimeKey::new(&pubkey, relay_url).unwrap();
    let seeded_pid = {
        let state = app_handle.state::<crate::app_state::AppState>();
        let mut runtimes = state.managed_agent_processes.lock().unwrap();
        let child = spawn_long_lived_child_for_test();
        let pid = child.id();
        let process = crate::managed_agents::ManagedAgentProcess {
            child,
            log_path: std::path::PathBuf::new(),
            spawn_config: crate::managed_agents::spawn_snapshot::prospective_spawn_config_snapshot(
                &record,
                &[],
                &[],
                relay_url,
                &Default::default(),
                false,
            ),
            setup_mode: false,
            adapter_availability: None,
            start_nonce: "test-nonce".to_string(),
            #[cfg(windows)]
            job: None,
        };
        runtimes.insert(
            rt_key.clone(),
            ManagedAgentPairRuntime::starting(process, Some(scope_id.to_string())),
        );
        pid
    };

    let gen = crate::managed_agents::scope::current_scope_generation();
    let scope = crate::managed_agents::scope::WorkspaceAgentScope {
        scope_id: scope_id.to_string(),
        relay_url: relay_url.to_string(),
        owner_pubkey: owner_hex.clone(),
        definitions_dir: tmp.path().to_path_buf(),
        generation: gen,
    };

    // Build NON-EMPTY personas, teams, and global so the spawn_fn can assert
    // they are actually delivered to the captured context.
    let test_persona = crate::managed_agents::AgentDefinition {
        id: "test-persona-id".to_string(),
        display_name: "Test Persona".to_string(),
        avatar_url: None,
        system_prompt: "Test persona prompt.".to_string(),
        runtime: None,
        model: None,
        provider: None,
        name_pool: vec![],
        is_builtin: false,
        is_active: true,
        shared: false,
        source_team: None,
        source_team_persona_slug: None,
        catalog_source: None,
        env_vars: Default::default(),
        respond_to: None,
        respond_to_allowlist: Default::default(),
        parallelism: None,
        created_at: crate::util::now_iso(),
        updated_at: crate::util::now_iso(),
    };
    let test_team = crate::managed_agents::TeamRecord {
        id: "test-team-id".to_string(),
        name: "Test Team".to_string(),
        description: None,
        instructions: None,
        persona_ids: vec![],
        is_builtin: false,
        source_dir: None,
        is_symlink: false,
        symlink_target: None,
        version: None,
        created_at: crate::util::now_iso(),
        updated_at: crate::util::now_iso(),
    };
    let mut global_env_vars = std::collections::BTreeMap::new();
    global_env_vars.insert("CAPTURED_GLOBAL_VAR".to_string(), "test-value".to_string());
    let captured_global = crate::managed_agents::GlobalAgentConfig {
        env_vars: global_env_vars,
        ..Default::default()
    };

    let context = CapturedRestartContext {
        scope: scope.clone(),
        personas: vec![test_persona],
        teams: vec![test_team],
        global: captured_global.clone(),
        owner_hex: owner_hex.clone(),
        mesh_model_id: None,
    };

    // old_global (default) and new_global differ by one env_var so that
    // env_changed = true → should_restart_on_config_change returns true.
    let old_global = crate::managed_agents::GlobalAgentConfig::default();
    let mut new_global_env = std::collections::BTreeMap::new();
    new_global_env.insert("SOME_EXTRA_VAR".to_string(), "v2".to_string());
    let new_global = crate::managed_agents::GlobalAgentConfig {
        env_vars: new_global_env,
        ..Default::default()
    };

    let stop_called = Arc::new(Mutex::new(false));
    let spawn_relay = Arc::new(Mutex::new(None::<String>));
    let spawn_owner = Arc::new(Mutex::new(None::<String>));
    let spawn_got_nonempty_personas = Arc::new(Mutex::new(false));
    let spawn_got_nonempty_teams = Arc::new(Mutex::new(false));
    let spawn_got_nonempty_global = Arc::new(Mutex::new(false));
    let receipt_called = Arc::new(Mutex::new(false));
    // Capture all receipt fields for post-completion assertions.
    let receipt_pubkey = Arc::new(Mutex::new(None::<String>));
    let receipt_relay = Arc::new(Mutex::new(None::<String>));
    let receipt_pid = Arc::new(Mutex::new(0u32));
    let receipt_instance_id = Arc::new(Mutex::new(String::new()));
    let receipt_started_at = Arc::new(Mutex::new(String::new()));
    // Capture the PID of the spawned child so we can assert exact equality with
    // receipt.pid — proves the receipt carries the real child's PID, not an
    // arbitrary positive value.
    let spawned_child_pid = Arc::new(Mutex::new(0u32));

    let stop_called2 = stop_called.clone();
    let spawn_relay2 = spawn_relay.clone();
    let spawn_owner2 = spawn_owner.clone();
    let spawn_personas2 = spawn_got_nonempty_personas.clone();
    let spawn_teams2 = spawn_got_nonempty_teams.clone();
    let spawn_global2 = spawn_got_nonempty_global.clone();
    let receipt_called2 = receipt_called.clone();
    let receipt_pubkey2 = receipt_pubkey.clone();
    let receipt_relay2 = receipt_relay.clone();
    let receipt_pid2 = receipt_pid.clone();
    let receipt_iid2 = receipt_instance_id.clone();
    let receipt_sat2 = receipt_started_at.clone();
    let spawned_pid2 = spawned_child_pid.clone();
    let pubkey2 = pubkey.clone();

    let app_handle_for_assert = app_handle.clone();
    let pubkey_for_assert = pubkey.clone();
    let result = tokio::task::spawn_blocking(move || {
        restart_under_captured_epoch_for(
            &app_handle,
            &pubkey,
            &old_global,
            &new_global,
            &[],
            &context,
            // stop_fn: record the call, simulate success, remove runtime.
            move |_app, rec, runtimes| {
                *stop_called2.lock().unwrap() = true;
                runtimes.retain(|k, _| k.pubkey != rec.pubkey);
                Ok(())
            },
            // spawn_fn: record captured relay+owner+personas+teams+global, return a noop child.
            // `spawn_noop_child_for_test()` replaces `/usr/bin/true` — cross-platform.
            // Capture the child PID before wrapping so we can assert exact equality
            // with receipt.pid — fails if production uses any PID other than the child's.
            move |_app, rec, relay, owner, personas, global, teams| {
                *spawn_relay2.lock().unwrap() = Some(relay.to_string());
                *spawn_owner2.lock().unwrap() = owner.map(str::to_string);
                *spawn_personas2.lock().unwrap() = !personas.is_empty();
                *spawn_teams2.lock().unwrap() = !teams.is_empty();
                *spawn_global2.lock().unwrap() = !global.env_vars.is_empty();
                let child = spawn_noop_child_for_test();
                // Capture the real child PID before moving child into ManagedAgentProcess.
                *spawned_pid2.lock().unwrap() = child.id();
                Ok(crate::managed_agents::ManagedAgentProcess {
                    child,
                    log_path: std::path::PathBuf::new(),
                    spawn_config:
                        crate::managed_agents::spawn_snapshot::prospective_spawn_config_snapshot(
                            rec,
                            &[],
                            teams,
                            relay,
                            global,
                            false,
                        ),
                    setup_mode: false,
                    adapter_availability: None,
                    start_nonce: "test-nonce-spawn".to_string(),
                    #[cfg(windows)]
                    job: None,
                })
            },
            // write_receipt_fn: record all receipt fields for post-completion assertions.
            // Full key (pubkey + relay_url), pid, desktop_instance_id, started_at.
            move |_app, receipt| {
                *receipt_called2.lock().unwrap() = true;
                *receipt_pubkey2.lock().unwrap() = Some(receipt.key.pubkey.clone());
                *receipt_relay2.lock().unwrap() = Some(receipt.key.relay_url.clone());
                *receipt_pid2.lock().unwrap() = receipt.pid;
                *receipt_iid2.lock().unwrap() = receipt.desktop_instance_id.clone();
                *receipt_sat2.lock().unwrap() = receipt.started_at.clone();
                assert_eq!(
                    receipt.key.pubkey, pubkey2,
                    "receipt must carry the correct pubkey"
                );
                Ok(())
            },
        )
    })
    .await
    .expect("spawn_blocking must not panic");

    // Kill the seeded long-lived child now that the epoch has consumed it.
    let _ = crate::managed_agents::terminate_process(seeded_pid);

    assert!(
        matches!(result, Ok(())),
        "full tail must return Ok when stop+spawn+receipt all succeed: {result:?}"
    );
    assert!(*stop_called.lock().unwrap(), "stop_fn must be called");
    assert_eq!(
        spawn_relay.lock().unwrap().as_deref(),
        Some(relay_url),
        "spawn_fn must receive the captured relay URL"
    );
    assert_eq!(
        spawn_owner.lock().unwrap().as_deref(),
        Some(owner_hex.as_str()),
        "spawn_fn must receive the captured owner hex"
    );
    assert!(
        *spawn_got_nonempty_personas.lock().unwrap(),
        "spawn_fn must receive NON-EMPTY captured personas"
    );
    assert!(
        *spawn_got_nonempty_teams.lock().unwrap(),
        "spawn_fn must receive NON-EMPTY captured teams"
    );
    assert!(
        *spawn_got_nonempty_global.lock().unwrap(),
        "spawn_fn must receive a NON-EMPTY captured global env"
    );
    assert!(
        *receipt_called.lock().unwrap(),
        "write_receipt_fn must be called"
    );
    // ── Receipt field assertions ──────────────────────────────────────────────
    // A full relay-bearing key: pubkey + relay_url.
    assert_eq!(
        receipt_pubkey.lock().unwrap().as_deref(),
        Some(pubkey_for_assert.as_str()),
        "receipt.key.pubkey must match the restarted agent"
    );
    assert_eq!(
        receipt_relay.lock().unwrap().as_deref(),
        Some(relay_url),
        "receipt.key.relay_url must match the captured relay; fails if the spawn path \
         builds the receipt with an empty or wrong relay"
    );
    // Assert receipt.pid exactly matches the PID captured from the spawned child.
    // Fails if production writes any PID other than `child.id()` into the receipt.
    let expected_pid = *spawned_child_pid.lock().unwrap();
    assert!(expected_pid > 0, "spawned child PID must be non-zero");
    assert_eq!(
        *receipt_pid.lock().unwrap(),
        expected_pid,
        "receipt.pid must equal the spawned child's PID (child.id()); \
         fails if production assigns an arbitrary PID to the receipt"
    );
    // Assert receipt.desktop_instance_id equals the actual value produced by
    // current_instance_id(app) — proves the field is sourced from the app
    // handle, not left empty or set to a hardcoded value.
    let expected_instance_id = app_handle_for_assert.config().identifier.clone();
    assert_eq!(
        receipt_instance_id.lock().unwrap().as_str(),
        expected_instance_id.as_str(),
        "receipt.desktop_instance_id must equal current_instance_id(app)"
    );
    assert!(
        !receipt_started_at.lock().unwrap().is_empty(),
        "receipt.started_at must be a non-empty ISO timestamp"
    );
    // Verify the runtime is registered with the captured scope_id.
    {
        let state = app_handle_for_assert.state::<crate::app_state::AppState>();
        let runtimes = state.managed_agent_processes.lock().unwrap();
        let registered = runtimes
            .values()
            .any(|r| r.scope_id.as_deref() == Some(scope_id));
        assert!(
            registered,
            "runtime must be registered with the captured scope_id"
        );
    }
    // ── Final disk record assertions ──────────────────────────────────────────
    // The restart-produced state: last_started_at set, last_stopped_at cleared,
    // last_error cleared. Fails if the post-spawn save is removed or if the
    // record-update block is bypassed.
    let final_records =
        crate::managed_agents::storage::load_managed_agents_at(tmp.path()).unwrap_or_default();
    let final_rec = final_records
        .iter()
        .find(|r| r.pubkey == pubkey_for_assert)
        .expect("final disk record must contain the restarted agent");
    assert!(
        final_rec.last_started_at.is_some(),
        "last_started_at must be Some after a successful restart (set by post-spawn record update)"
    );
    assert!(
        final_rec.last_stopped_at.is_none(),
        "last_stopped_at must be None after restart (cleared by post-spawn record update)"
    );
    assert!(
        final_rec.last_error.is_none(),
        "last_error must be None after a successful restart (cleared by post-spawn record update)"
    );
}

/// Production driver proves preflight fires before stop — and with an eligible
/// runtime seeded both "mesh" and "stop" appear in the log in that order.
///
/// Thufir's test 6: "seed an eligible runtime through the cross-platform child
/// seam; event log from production driver proves preflight fn fires before stop
/// fn — unconditionally (both events must be present)."
///
/// Requirements for the agent to be an eligible restart candidate:
/// - `backend = Local` with a live pair runtime (seeded with long-lived child).
/// - `provider = anthropic`, `model = ...`, `ANTHROPIC_API_KEY` in env_vars
///   → `old_ready = true`.
/// - `old_global != new_global` (env_vars differ) → `env_changed = true`
///   → `should_restart_on_config_change = true`.
/// - `owner_pubkey` matches the mock app's signing key.
///
/// The `stop_fn` records "stop" and REMOVES the runtime from the map so the
/// epoch considers it properly stopped. The `spawn_fn` returns `Err` so the
/// epoch ends with `FailedAfterStop` — but both "mesh" and "stop" are in the
/// log before that.
///
/// Owner key must match the app's signing key — use the actual generated key
/// from the mock app's AppState.
#[tokio::test]
#[allow(clippy::await_holding_lock)] // SCOPE_GENERATION_TEST_LOCK serialises parallel tests
async fn test_relay_mesh_preflight_precedes_stop() {
    use super::super::restart_local_agent_on_config_change_for;
    use crate::commands::global_agent_config::RestartOutcome;
    use crate::managed_agents::scope::SCOPE_GENERATION_TEST_LOCK;
    use crate::managed_agents::storage::save_managed_agents_at;
    use crate::managed_agents::{
        BackendKind, ManagedAgentPairRuntime, ManagedAgentRecord, ManagedAgentRuntimeKey,
    };
    use tauri::Manager;

    // Serialize generation-sensitive work across parallel tests.
    let _gen_guard = SCOPE_GENERATION_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    let tmp = tempfile::tempdir().unwrap();

    let app = make_mock_app();
    let app_handle = app.handle().clone();

    // Get the actual owner pubkey from the mock app's signing keys.
    // The pre-stop phase checks hex == captured_scope.owner_pubkey; they must match.
    let actual_owner_hex = {
        let state = app_handle.state::<crate::app_state::AppState>();
        state
            .signing_keys()
            .expect("mock app must have signing keys")
            .public_key()
            .to_hex()
    };

    let agent_pubkey = "aa".repeat(32);

    // Build a Ready record: provider+model set and ANTHROPIC_API_KEY in env_vars.
    // old_global != new_global (env differ) → env_changed = true → eligible.
    let mut record_env_vars = std::collections::BTreeMap::new();
    record_env_vars.insert(
        "ANTHROPIC_API_KEY".to_string(),
        "sk-test-key-eligible".to_string(),
    );
    let agent_record = ManagedAgentRecord {
        pubkey: agent_pubkey.clone(),
        name: "test-agent-preflight".to_string(),
        display_name: None,
        slug: None,
        persona_id: None,
        private_key_nsec: String::new(),
        auth_tag: None,
        relay_url: "wss://relay.example".to_string(),
        avatar_url: None,
        acp_command: crate::managed_agents::DEFAULT_ACP_COMMAND.to_string(),
        agent_command: String::new(),
        agent_command_override: None,
        agent_args: vec![],
        mcp_command: String::new(),
        turn_timeout_seconds: 0,
        idle_timeout_seconds: None,
        max_turn_duration_seconds: None,
        parallelism: 1,
        system_prompt: None,
        model: Some("claude-3-5-sonnet-20241022".to_string()),
        provider: Some("anthropic".to_string()),
        persona_source_version: None,
        env_vars: record_env_vars,
        start_on_app_launch: false,
        auto_restart_on_config_change: false,
        runtime_pid: None,
        backend: BackendKind::Local,
        backend_agent_id: None,
        provider_policy_pending: false,
        provider_binary_path: None,
        team_id: None,
        persona_team_dir: None,
        persona_name_in_team: None,
        created_at: crate::util::now_iso(),
        updated_at: crate::util::now_iso(),
        last_started_at: None,
        last_stopped_at: None,
        last_exit_code: None,
        last_error: None,
        last_error_code: None,
        respond_to: Default::default(),
        respond_to_allowlist: Default::default(),
        is_builtin: false,
        is_active: true,
        shared: false,
        source_team: None,
        source_team_persona_slug: None,
        catalog_source: None,
        library_ref: None,
        library_applied_revision: None,
        last_completed_deploy_attempt_id: None,
        definition_respond_to: None,
        definition_respond_to_allowlist: Default::default(),
        definition_parallelism: None,
        relay_mesh: None,
        effort_level: None,
        runtime: None,
        name_pool: vec![],
    };
    save_managed_agents_at(tmp.path(), std::slice::from_ref(&agent_record)).unwrap();
    std::fs::write(tmp.path().join("personas.json"), b"[]").unwrap();
    std::fs::write(tmp.path().join("global-agent-config.json"), b"{}").unwrap();

    // Seed a live runtime for the agent (makes it eligible — avoids the
    // "no live pair runtime" Skipped path).
    let rt_key = ManagedAgentRuntimeKey::new(&agent_pubkey, "wss://relay.example").unwrap();
    let seeded_pid = {
        let state = app_handle.state::<crate::app_state::AppState>();
        let mut runtimes = state.managed_agent_processes.lock().unwrap();
        let child = spawn_long_lived_child_for_test();
        let pid = child.id();
        let process = crate::managed_agents::ManagedAgentProcess {
            child,
            log_path: std::path::PathBuf::new(),
            spawn_config: crate::managed_agents::spawn_snapshot::prospective_spawn_config_snapshot(
                &agent_record,
                &[],
                &[],
                "wss://relay.example",
                &Default::default(),
                false,
            ),
            setup_mode: false,
            adapter_availability: None,
            start_nonce: "test-nonce-preflight".to_string(),
            #[cfg(windows)]
            job: None,
        };
        runtimes.insert(
            rt_key,
            ManagedAgentPairRuntime::starting(process, Some("test-scope".to_string())),
        );
        pid
    };

    let gen = crate::managed_agents::scope::current_scope_generation();
    let scope = crate::managed_agents::scope::WorkspaceAgentScope {
        scope_id: "test-scope".to_string(),
        relay_url: "wss://relay.example".to_string(),
        // Use the actual app key so the owner-key check passes.
        owner_pubkey: actual_owner_hex.clone(),
        definitions_dir: tmp.path().to_path_buf(),
        generation: gen,
    };

    // old_global and new_global differ by one env_var so env_changed = true.
    let old_global = crate::managed_agents::GlobalAgentConfig::default();
    let mut new_global_env = std::collections::BTreeMap::new();
    new_global_env.insert("PREFLIGHT_TEST_VAR".to_string(), "v2".to_string());
    let new_global = crate::managed_agents::GlobalAgentConfig {
        env_vars: new_global_env,
        ..Default::default()
    };

    // Shared event log: "mesh" or "stop" entries in order.
    let event_log: std::sync::Arc<std::sync::Mutex<Vec<&'static str>>> =
        std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let log_mesh = event_log.clone();
    let log_stop = event_log.clone();

    let outcome = restart_local_agent_on_config_change_for(
        &app_handle,
        &agent_pubkey,
        &old_global,
        &new_global,
        &[],
        &scope,
        tmp.path(),
        // mesh_fn: records "mesh", then succeeds.
        move |_app, _model| {
            log_mesh.lock().unwrap().push("mesh");
            Box::pin(async { Ok(()) })
        },
        // stop_fn: records "stop", removes the runtime so spawn fails gracefully.
        move |_app, rec, runtimes| {
            log_stop.lock().unwrap().push("stop");
            runtimes.retain(|k, _| k.pubkey != rec.pubkey);
            Ok(())
        },
        // spawn_fn: returns Err so the epoch ends with FailedAfterStop.
        |_app, _rec, _relay, _owner, _personas, _global, _teams| {
            Err("spawn not available in test".to_string())
        },
        |_app, _receipt| Err("receipt not expected".to_string()),
    )
    .await;

    // Kill the seeded process now that the epoch has consumed it.
    let _ = crate::managed_agents::terminate_process(seeded_pid);

    // With an eligible runtime, the epoch progresses past preflight and stop.
    // stop_fn removed the runtime → spawn_fn fails → FailedAfterStop.
    assert!(
        matches!(outcome, RestartOutcome::FailedAfterStop),
        "with eligible runtime: stop fires and spawn fails → FailedAfterStop: {outcome:?}"
    );

    let log = event_log.lock().unwrap();
    // Both events must be present — mesh fires in the async pre-stop phase,
    // stop fires inside the epoch.
    let mesh_pos = log.iter().position(|&e| e == "mesh");
    let stop_pos = log.iter().position(|&e| e == "stop");
    assert!(
        mesh_pos.is_some(),
        "mesh_fn must be called (preflight runs in async pre-stop phase): {log:?}"
    );
    assert!(
        stop_pos.is_some(),
        "stop_fn must be called with an eligible live runtime: {log:?}"
    );
    let mesh_idx = mesh_pos.unwrap();
    let stop_idx = stop_pos.unwrap();
    assert!(
        mesh_idx < stop_idx,
        "preflight (mesh at {mesh_idx}) must precede stop (stop at {stop_idx}): {log:?}"
    );
}
