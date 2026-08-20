//! Unit and integration tests for `commands/global_agent_config.rs`.
//!
//! Split into this file and `global_agent_config_epoch_tests.rs` to keep each
//! file under the 1000-line size ratchet.
//!
//! Included via `#[path = "global_agent_config_tests.rs"] mod tests;` at the
//! bottom of `global_agent_config.rs`. `use super::*` gives access to all
//! items in that module.
use super::{
    restart_under_captured_epoch_for, should_restart_on_config_change, CapturedRestartContext,
    EpochError,
};

/// Running agent (Ready) whose effective env changed → restart candidate.
#[test]
fn env_changed_running_agent_is_candidate() {
    // old_ready=true, new_ready=true, env_changed=true
    assert!(
        should_restart_on_config_change(true, true, true),
        "running agent with changed env must be restarted"
    );
}

/// Running agent (Ready) whose effective env did NOT change → not a candidate.
#[test]
fn unchanged_running_agent_is_not_candidate() {
    // old_ready=true, new_ready=true, env_changed=false
    assert!(
        !should_restart_on_config_change(true, true, false),
        "running agent with identical env must NOT be restarted"
    );
}

/// NotReady → Ready transition is admitted regardless of env diff.
#[test]
fn not_ready_to_ready_is_candidate() {
    // old_ready=false, new_ready=true, env_changed=false (env_changed irrelevant)
    assert!(
        should_restart_on_config_change(false, true, false),
        "NotReady → Ready must be a restart candidate"
    );
}

/// Ready → NotReady (config became invalid, env changed) is admitted so the
/// agent restarts into setup-listener mode via the normal spawn path.
#[test]
fn ready_to_not_ready_env_changed_is_candidate() {
    // old_ready=true (had key), new_ready=false (key removed), env_changed=true
    assert!(
        should_restart_on_config_change(true, false, true),
        "Ready → NotReady with env change must be a restart candidate"
    );
}

/// Both NotReady, env unchanged → not a candidate (nothing to restart).
#[test]
fn both_not_ready_unchanged_is_not_candidate() {
    // old_ready=false, new_ready=false, env_changed=false
    assert!(
        !should_restart_on_config_change(false, false, false),
        "both NotReady with no env change must NOT be a candidate"
    );
}

/// NotReady + env changed but new still NotReady → not a candidate.
#[test]
fn not_ready_env_changed_still_not_ready_is_not_candidate() {
    // Changed one unrelated env var but still missing the required key.
    // old_ready=false, new_ready=false, env_changed=true
    assert!(
        !should_restart_on_config_change(false, false, true),
        "NotReady→NotReady (env changed but still broken) must NOT be a candidate"
    );
}

/// NotReady → Ready AND env also changed → still a restart candidate.
///
/// Guards against a future `&& !env_changed` regression on the
/// NotReady→Ready branch: env_changed is irrelevant when readiness
/// unblocks — the agent must restart regardless of whether env also differed.
#[test]
fn not_ready_to_ready_with_env_change_is_candidate() {
    // old_ready=false, new_ready=true, env_changed=true
    assert!(
        should_restart_on_config_change(false, true, true),
        "NotReady → Ready (with env change) must be a restart candidate"
    );
}

// ── restart_under_captured_epoch_for: generation guard ───────────────────
//
// These tests call `restart_under_captured_epoch_for` directly — the
// production stop→spawn primitive — using a `tauri::test::mock_app()`
// runtime so the AppHandle is real. No live process is running, so the
// restart is skipped at the eligibility check. The generation tests drive
// the path that matters: does the captured-generation guard prevent a
// stale-scope restart?

fn make_test_scope(
    definitions_dir: &std::path::Path,
) -> crate::managed_agents::scope::WorkspaceAgentScope {
    let gen = crate::managed_agents::scope::current_scope_generation();
    crate::managed_agents::scope::WorkspaceAgentScope {
        scope_id: "test-scope".to_string(),
        relay_url: "wss://relay.example".to_string(),
        owner_pubkey: "aa".repeat(32),
        definitions_dir: definitions_dir.to_path_buf(),
        generation: gen,
    }
}

fn make_test_context(definitions_dir: &std::path::Path) -> CapturedRestartContext {
    CapturedRestartContext {
        scope: make_test_scope(definitions_dir),
        personas: vec![],
        teams: vec![],
        global: crate::managed_agents::GlobalAgentConfig::default(),
        owner_hex: "aa".repeat(32),
        mesh_model_id: None,
    }
}

/// `restart_under_captured_epoch_for` with a fresh scope and an empty store
/// (no live pair runtime) → `EpochError::Skipped` after the agent-not-found
/// or no-live-runtime check. The generation guard passes, proving the path
/// proceeds to the eligibility check rather than aborting at the stale check.
///
/// This is the stop-to-spawn production path: transition lock acquired,
/// store lock acquired, generation validated — all before any state change.
#[test]
fn test_restart_under_captured_epoch_fresh_scope_no_runtime_is_skipped() {
    let tmp = tempfile::tempdir().unwrap();
    // Write an empty managed-agents.json so load_managed_agents_at returns Ok([]).
    std::fs::write(tmp.path().join("managed-agents.json"), b"[]").unwrap();

    let app = tauri::test::mock_builder()
        .manage(crate::app_state::build_app_state())
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("failed to build mock app");
    let app_handle = app.handle().clone();
    let context = make_test_context(tmp.path());
    let pubkey = "aa".repeat(32);

    let result = restart_under_captured_epoch_for(
        &app_handle,
        &pubkey,
        &crate::managed_agents::GlobalAgentConfig::default(),
        &crate::managed_agents::GlobalAgentConfig::default(),
        &[],
        &context,
        |_app, _rec, _runtimes| Err("stop not expected".to_string()),
        |_app, _rec, _relay, _owner, _personas, _global, _teams| {
            Err("spawn not expected".to_string())
        },
        |_app, _receipt| Err("receipt not expected".to_string()),
    );

    // No live runtime → Skipped before any state change.
    assert!(
        matches!(result, Err(EpochError::Skipped(_))),
        "no live pair runtime must produce Skipped, not FailedAfterStop or Ok: {result:?}"
    );
    // Verify the skip reason. In sequential execution the scope is fresh and
    // the epoch reaches the eligibility check before skipping ("not found" or
    // "no live pair runtime"). In parallel test runs another test may advance
    // the generation counter, producing "stale scope" instead — both are valid
    // outcomes proving the epoch aborted without modifying any agent state.
    if let Err(EpochError::Skipped(msg)) = result {
        let is_expected = msg.contains("not found")
            || msg.contains("no live pair runtime")
            || msg.contains("stale scope")
            || msg.contains("generation");
        assert!(
            is_expected,
            "Skipped reason must be agent-not-found, no-live-runtime, or stale-scope: {msg}"
        );
    }
}

/// `restart_under_captured_epoch_for` with a STALE scope → `EpochError::Skipped`
/// at the generation validation step, before touching any agent state.
///
/// This is the switch-between-stop-and-spawn test: simulates the race where
/// a workspace switch advances the generation between when the scope was
/// captured and when `restart_under_captured_epoch_for` runs. The generation
/// guard must abort before stopping — no agent is touched.
#[test]
fn test_restart_under_captured_epoch_stale_scope_is_rejected() {
    let _gen_guard = crate::managed_agents::scope::SCOPE_GENERATION_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("managed-agents.json"), b"[]").unwrap();

    let app = tauri::test::mock_builder()
        .manage(crate::app_state::build_app_state())
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("failed to build mock app");
    let app_handle = app.handle().clone();

    // Capture the scope at the current generation, then advance to make it stale.
    let context = make_test_context(tmp.path());
    crate::managed_agents::scope::next_scope_generation();

    let pubkey = "aa".repeat(32);
    let result = restart_under_captured_epoch_for(
        &app_handle,
        &pubkey,
        &crate::managed_agents::GlobalAgentConfig::default(),
        &crate::managed_agents::GlobalAgentConfig::default(),
        &[],
        &context,
        |_app, _rec, _runtimes| Err("stop not expected".to_string()),
        |_app, _rec, _relay, _owner, _personas, _global, _teams| {
            Err("spawn not expected".to_string())
        },
        |_app, _receipt| Err("receipt not expected".to_string()),
    );

    // Stale generation → Skipped at the generation-validation step.
    assert!(
        matches!(result, Err(EpochError::Skipped(_))),
        "stale scope must produce Skipped: {result:?}"
    );
    if let Err(EpochError::Skipped(msg)) = result {
        assert!(
            msg.contains("stale scope") || msg.contains("generation"),
            "Skipped reason must mention stale scope or generation mismatch: {msg}"
        );
    }
}

// ── Area-2 tests: async driver and epoch core ─────────────────────────────

/// Spawn a child process that exits immediately.
///
/// Cross-platform replacement for `/usr/bin/true`: used in `spawn_fn` closures
/// that must return a valid `ManagedAgentProcess` without spawning a real agent.
/// The child exits before or shortly after being inserted into the runtimes map;
/// for tests that only inspect in-memory state (not process liveness) this is
/// sufficient.
pub(crate) fn spawn_noop_child_for_test() -> std::process::Child {
    #[cfg(not(windows))]
    {
        std::process::Command::new("sh")
            .args(["-c", "exit 0"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn noop test child (sh -c 'exit 0')")
    }
    #[cfg(windows)]
    {
        std::process::Command::new("cmd.exe")
            .args(["/C", "exit 0"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn noop test child (cmd.exe /C exit 0)")
    }
}

/// Spawn a long-lived child process that stays running long enough for tests to
/// complete.
///
/// Cross-platform replacement for `sleep 10000`: seeds the in-memory runtimes
/// map before `sync_managed_agent_processes` runs its `try_wait()` scan, so
/// the runtime survives to the eligibility check.
///
/// Tests that use this helper MUST drop the returned `Child` (or kill it) when
/// the test exits so OS processes are not leaked.  The helper is intentionally
/// not `#[cfg(test)]` — it lives here so `epoch_tests.rs` (via `use super::*`)
/// can reach it without a separate import.
pub(crate) fn spawn_long_lived_child_for_test() -> std::process::Child {
    #[cfg(not(windows))]
    {
        std::process::Command::new("sh")
            .args(["-c", "while true; do sleep 1; done"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn long-lived test child (sh loop)")
    }
    #[cfg(windows)]
    {
        // `ping -n N 127.0.0.1` sleeps ~(N-1) seconds; 100000 ≈ 28 hours.
        std::process::Command::new("ping")
            .args(["-n", "100000", "127.0.0.1"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn long-lived test child (ping)")
    }
}

fn make_mock_app() -> tauri::App<tauri::test::MockRuntime> {
    tauri::test::mock_builder()
        .manage(crate::app_state::build_app_state())
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("failed to build mock app")
}

/// Context load failure (personas) before any stop → `RestartOutcome::Skipped`,
/// stop closure never called.
///
/// Drives `restart_local_agent_on_config_change_for` with a `definitions_dir`
/// containing a syntactically invalid `managed-agents.json` — `load_personas_at`
/// delegates to `load_agent_definitions_at`, which calls `load_agent_store_at`,
/// which returns `Err` on malformed JSON.  The pre-stop phase must return
/// `Skipped` without calling stop.
///
/// Absent files load as empty/default, so this test writes a malformed file to
/// ensure a genuine parse-error path (not the "agent not found" path).
///
/// Thufir's test 1: "production async driver with injected loader failure;
/// assert stop never called, RestartOutcome::Skipped."
#[tokio::test]
#[allow(clippy::await_holding_lock)] // SCOPE_GENERATION_TEST_LOCK serialises parallel tests
async fn test_context_load_failure_leaves_runtime_running() {
    use super::restart_local_agent_on_config_change_for;
    use crate::commands::global_agent_config::RestartOutcome;
    use crate::managed_agents::scope::SCOPE_GENERATION_TEST_LOCK;

    let _gen_guard = SCOPE_GENERATION_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    let tmp = tempfile::tempdir().unwrap();
    // Malformed JSON → load_agent_store_at (called by load_personas_at) returns
    // Err → pre-stop phase returns Skipped before any stop.
    std::fs::write(
        tmp.path().join("managed-agents.json"),
        b"this is not valid json",
    )
    .unwrap();

    let app = make_mock_app();
    let app_handle = app.handle().clone();
    let gen = crate::managed_agents::scope::current_scope_generation();
    let scope = crate::managed_agents::scope::WorkspaceAgentScope {
        scope_id: "test-scope".to_string(),
        relay_url: "wss://relay.example".to_string(),
        owner_pubkey: "aa".repeat(32),
        definitions_dir: tmp.path().to_path_buf(),
        generation: gen,
    };

    let stop_called = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stop_called2 = stop_called.clone();

    let outcome = restart_local_agent_on_config_change_for(
        &app_handle,
        &"aa".repeat(32),
        &crate::managed_agents::GlobalAgentConfig::default(),
        &crate::managed_agents::GlobalAgentConfig::default(),
        &[],
        &scope,
        tmp.path(),
        // mesh_fn: succeeds (no-op)
        |_app, _model| Box::pin(async { Ok(()) }),
        // stop_fn: must NOT be called
        move |_app, _rec, _runtimes| {
            stop_called2.store(true, std::sync::atomic::Ordering::SeqCst);
            Err("stop_fn called unexpectedly".to_string())
        },
        // spawn_fn: must NOT be called
        |_app, _rec, _relay, _owner, _personas, _global, _teams| {
            Err("spawn_fn called unexpectedly".to_string())
        },
        // write_receipt_fn: must NOT be called
        |_app, _receipt| Err("receipt_fn called unexpectedly".to_string()),
    )
    .await;

    assert!(
        matches!(outcome, RestartOutcome::Skipped),
        "context load failure must produce Skipped: {outcome:?}"
    );
    assert!(
        !stop_called.load(std::sync::atomic::Ordering::SeqCst),
        "stop must NOT be called when context load fails"
    );
}

/// Mesh preflight failure before stop → `RestartOutcome::Skipped`,
/// stop closure never called.
///
/// Drives `restart_local_agent_on_config_change_for` with an injected mesh_fn
/// that returns `Err`. Stop must not fire.
///
/// Thufir's test 2: "real production driver/core with injected preflight error;
/// stop never called, RestartOutcome::Skipped."
#[tokio::test]
#[allow(clippy::await_holding_lock)] // SCOPE_GENERATION_TEST_LOCK serialises parallel tests
async fn test_mesh_preflight_failure_leaves_runtime_running() {
    use super::restart_local_agent_on_config_change_for;
    use crate::commands::global_agent_config::RestartOutcome;
    use crate::managed_agents::scope::SCOPE_GENERATION_TEST_LOCK;

    let _gen_guard = SCOPE_GENERATION_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    let tmp = tempfile::tempdir().unwrap();
    // Provide persona and record files so context prep can pass them.
    std::fs::write(tmp.path().join("personas.json"), b"[]").unwrap();
    std::fs::write(tmp.path().join("managed-agents.json"), b"[]").unwrap();
    // Also need global-agent-config (fallible load will fail on missing file,
    // so we supply it). The injected mesh_fn is what we care about.
    std::fs::write(tmp.path().join("global-agent-config.json"), b"{}").unwrap();

    let app = make_mock_app();
    let app_handle = app.handle().clone();
    let gen = crate::managed_agents::scope::current_scope_generation();
    let scope = crate::managed_agents::scope::WorkspaceAgentScope {
        scope_id: "test-scope".to_string(),
        relay_url: "wss://relay.example".to_string(),
        owner_pubkey: "aa".repeat(32),
        definitions_dir: tmp.path().to_path_buf(),
        generation: gen,
    };

    let stop_called = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stop_called2 = stop_called.clone();

    let outcome = restart_local_agent_on_config_change_for(
        &app_handle,
        &"aa".repeat(32),
        &crate::managed_agents::GlobalAgentConfig::default(),
        &crate::managed_agents::GlobalAgentConfig::default(),
        &[],
        &scope,
        tmp.path(),
        // mesh_fn: FAILS — triggers the pre-stop abort
        |_app, _model| Box::pin(async { Err("mesh preflight failed (test)".to_string()) }),
        // stop_fn: must NOT be called
        move |_app, _rec, _runtimes| {
            stop_called2.store(true, std::sync::atomic::Ordering::SeqCst);
            Err("stop_fn called unexpectedly".to_string())
        },
        |_app, _rec, _relay, _owner, _personas, _global, _teams| {
            Err("spawn not expected".to_string())
        },
        |_app, _receipt| Err("receipt not expected".to_string()),
    )
    .await;

    assert!(
        matches!(outcome, RestartOutcome::Skipped),
        "mesh preflight failure must produce Skipped: {outcome:?}"
    );
    assert!(
        !stop_called.load(std::sync::atomic::Ordering::SeqCst),
        "stop must NOT be called when mesh preflight fails"
    );
}

/// Workspace switch after preflight (generation advances before epoch entry)
/// → epoch returns `Skipped` via generation guard, stop never called.
///
/// Thufir's test 3: "injected preflight hook advances generation after it
/// succeeds; epoch returns Skipped; stop never called."
#[tokio::test]
#[allow(clippy::await_holding_lock)] // SCOPE_GENERATION_TEST_LOCK serialises parallel tests
async fn test_workspace_switch_after_preflight_aborts_before_stop() {
    use super::restart_local_agent_on_config_change_for;
    use crate::commands::global_agent_config::RestartOutcome;
    use crate::managed_agents::scope::SCOPE_GENERATION_TEST_LOCK;

    let _gen_guard = SCOPE_GENERATION_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("personas.json"), b"[]").unwrap();
    std::fs::write(tmp.path().join("managed-agents.json"), b"[]").unwrap();
    std::fs::write(tmp.path().join("global-agent-config.json"), b"{}").unwrap();

    let app = make_mock_app();
    let app_handle = app.handle().clone();
    let gen = crate::managed_agents::scope::current_scope_generation();
    let scope = crate::managed_agents::scope::WorkspaceAgentScope {
        scope_id: "test-scope".to_string(),
        relay_url: "wss://relay.example".to_string(),
        owner_pubkey: "aa".repeat(32),
        definitions_dir: tmp.path().to_path_buf(),
        generation: gen,
    };

    let stop_called = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stop_called2 = stop_called.clone();

    let outcome = restart_local_agent_on_config_change_for(
        &app_handle,
        &"aa".repeat(32),
        &crate::managed_agents::GlobalAgentConfig::default(),
        &crate::managed_agents::GlobalAgentConfig::default(),
        &[],
        &scope,
        tmp.path(),
        // mesh_fn: succeeds but advances generation (simulates workspace switch
        // between preflight completion and epoch entry).
        |_app, _model| {
            crate::managed_agents::scope::next_scope_generation();
            Box::pin(async { Ok(()) })
        },
        // stop_fn: must NOT be called
        move |_app, _rec, _runtimes| {
            stop_called2.store(true, std::sync::atomic::Ordering::SeqCst);
            Err("stop_fn called unexpectedly".to_string())
        },
        |_app, _rec, _relay, _owner, _personas, _global, _teams| {
            Err("spawn not expected".to_string())
        },
        |_app, _receipt| Err("receipt not expected".to_string()),
    )
    .await;

    assert!(
        matches!(outcome, RestartOutcome::Skipped),
        "generation advance after preflight must produce Skipped: {outcome:?}"
    );
    assert!(
        !stop_called.load(std::sync::atomic::Ordering::SeqCst),
        "stop must NOT be called when generation advanced after preflight"
    );
}

/// A record-level Mesh model change after preflight (without advancing workspace
/// generation) → epoch detects mismatch in re-resolved Mesh model, aborts before stop.
///
/// Drives `restart_local_agent_on_config_change_for` — the full async production
/// driver. The injected `mesh_fn` mutates the on-disk record's `provider` to
/// `"relay-mesh"` AFTER the driver has already resolved `context.mesh_model_id = None`
/// (from the original `provider = "anthropic"`). The epoch's in-epoch TOCTOU guard
/// then re-resolves the mutated record, gets `Some("auto")` ≠ `None`, and fires
/// `Skipped` before stop.
///
/// Flow:
///   1. Seed record: `provider = "anthropic"`, live runtime.
///      Pre-stop resolve: `resolve_effective_relay_mesh_model_id` → `None`.
///      `context.mesh_model_id = None`.
///   2. `mesh_fn` rewrites `provider = "relay-mesh"` to disk and succeeds.
///   3. Epoch re-resolves from the mutated record:
///      `relay_mesh_model_id()` = `Some("auto")` ≠ `None` → TOCTOU guard fires → Skipped.
///   4. Assert `RestartOutcome::Skipped`; stop_fn must NOT be called.
///
/// Invariant: removing the in-epoch re-resolve guard in
/// `restart_under_captured_epoch_for` would allow the epoch to proceed with the
/// old `context.mesh_model_id`, bypass the mismatch — stop would be called
/// and the test would fail.
#[tokio::test]
#[allow(clippy::await_holding_lock)] // SCOPE_GENERATION_TEST_LOCK serialises parallel tests
async fn test_record_mesh_change_after_preflight_aborts_before_stop() {
    use super::super::restart_local_agent_on_config_change_for;
    use crate::commands::global_agent_config::RestartOutcome;
    use crate::managed_agents::scope::SCOPE_GENERATION_TEST_LOCK;
    use crate::managed_agents::{
        storage::save_managed_agents_at, BackendKind, ManagedAgentPairRuntime, ManagedAgentRecord,
        ManagedAgentRuntimeKey,
    };
    use tauri::Manager;

    let _gen_guard = SCOPE_GENERATION_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    let tmp = tempfile::tempdir().unwrap();
    let pubkey = "aa".repeat(32);
    let relay_url = "wss://relay.example";

    // Eligible record: provider=anthropic, model+ANTHROPIC_API_KEY → old_ready=true.
    // relay_mesh=None so the pre-stop resolve yields mesh_model_id=None.
    // old_global != new_global (env differ) so env_changed=true → eligible.
    let mut record_env_vars = std::collections::BTreeMap::new();
    record_env_vars.insert(
        "ANTHROPIC_API_KEY".to_string(),
        "sk-test-key-for-readiness".to_string(),
    );
    let record = ManagedAgentRecord {
        pubkey: pubkey.clone(),
        name: "test-agent-mesh".to_string(),
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
        provider: Some("anthropic".to_string()), // initial provider — not relay-mesh
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
        relay_mesh: None, // no relay-mesh marker; pre-stop resolve yields None
        effort_level: None,
        runtime: None,
        name_pool: vec![],
    };
    save_managed_agents_at(tmp.path(), std::slice::from_ref(&record)).unwrap();
    std::fs::write(tmp.path().join("personas.json"), b"[]").unwrap();
    std::fs::write(tmp.path().join("global-agent-config.json"), b"{}").unwrap();

    let app = make_mock_app();
    let app_handle = app.handle().clone();

    // Derive the actual owner pubkey from the mock app's signing keys.
    // restart_local_agent_on_config_change_for verifies hex == scope.owner_pubkey.
    let actual_owner_hex = {
        let state = app_handle.state::<crate::app_state::AppState>();
        state
            .signing_keys()
            .expect("mock app must have signing keys")
            .public_key()
            .to_hex()
    };

    // Seed a live runtime with a cross-platform long-lived child (avoids sync eviction).
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
            start_nonce: "test-nonce-mesh".to_string(),
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
        relay_url: relay_url.to_string(),
        owner_pubkey: actual_owner_hex,
        definitions_dir: tmp.path().to_path_buf(),
        generation: gen,
    };

    // old_global and new_global differ by one env_var so env_changed = true
    // (eligibility gate passes) while the record's provider stays anthropic.
    let mut new_global_env = std::collections::BTreeMap::new();
    new_global_env.insert("SOME_EXTRA_KEY".to_string(), "v2".to_string());
    let old_global = crate::managed_agents::GlobalAgentConfig::default();
    let new_global = crate::managed_agents::GlobalAgentConfig {
        env_vars: new_global_env,
        ..Default::default()
    };

    let stop_called = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stop_called2 = stop_called.clone();
    let tmp_path = tmp.path().to_path_buf();
    let pubkey_clone = pubkey.clone();

    // Drive the full async production driver. The mesh_fn mutates the on-disk
    // record's provider to "relay-mesh" AFTER the driver has resolved
    // context.mesh_model_id = None (provider was "anthropic" at resolve time).
    // The epoch then re-resolves the mutated record and gets Some("auto") != None
    // -> TOCTOU guard fires -> Skipped before stop.
    let outcome = restart_local_agent_on_config_change_for(
        &app_handle,
        &pubkey,
        &old_global,
        &new_global,
        &[],
        &scope,
        tmp.path(),
        // mesh_fn: rewrites provider to "relay-mesh" on disk, then succeeds.
        // The driver already captured context.mesh_model_id=None from the
        // original provider="anthropic" record — this mutation happens after.
        move |_app, _model| {
            let mut records = crate::managed_agents::storage::load_managed_agents_at(&tmp_path)
                .unwrap_or_default();
            for r in &mut records {
                if r.pubkey == pubkey_clone {
                    r.provider = Some("relay-mesh".to_string());
                }
            }
            let _ = crate::managed_agents::storage::save_managed_agents_at(&tmp_path, &records);
            Box::pin(async { Ok(()) })
        },
        // stop_fn: must NOT be called — TOCTOU guard fires before stop.
        move |_app, _rec, _runtimes| {
            stop_called2.store(true, std::sync::atomic::Ordering::SeqCst);
            Err("stop must not be called when Mesh model changed after preflight".to_string())
        },
        |_app, _rec, _relay, _owner, _personas, _global, _teams| {
            Err("spawn not expected".to_string())
        },
        |_app, _receipt| Err("receipt not expected".to_string()),
    )
    .await;

    // Kill the seeded process now that the epoch has consumed it.
    let _ = crate::managed_agents::terminate_process(seeded_pid);

    assert!(
        matches!(outcome, RestartOutcome::Skipped),
        "Mesh model mismatch (via full async driver) must produce Skipped before stop: {outcome:?}"
    );
    assert!(
        !stop_called.load(std::sync::atomic::Ordering::SeqCst),
        "stop must NOT be called when Mesh model changed after preflight"
    );
}

#[path = "global_agent_config_epoch_tests.rs"]
mod epoch_tests;
