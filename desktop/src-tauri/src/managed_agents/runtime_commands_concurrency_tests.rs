//! Concurrency/determinism tests for `managed_agents/runtime_commands.rs`.
//!
//! Split from `runtime_commands_tests.rs` to keep each file under the
//! 1000-line size ratchet. Included via `#[path]` from there as `mod concurrency_tests;`.
//! `use super::*` gives access to all items in `runtime_commands_tests.rs`.

use super::*;

/// Writer-vs-compensation store-lock contention via the `compensate_drain_with_hook` seam.
///
/// Invariant under test: `compensate_drain_with_hook` owns the production store guard at and
/// through the post-restore persistence seam (`on_after_restore`). The callback receives a
/// borrow of the actual `MutexGuard<'_, ()>` returned by `managed_agents_store_lock.lock()`.
/// Dropping or moving that guard before the callback is a **compile error** — the borrow must
/// remain live at the call site.
///
/// Runtime integration: with no-op callbacks, the store guard is held continuously; with the
/// test callbacks, the writer completes one full transaction after the adapter releases the lock,
/// and both disk effects (`COMP_SENTINEL` from `on_after_restore`, `WRITER_EDIT` from the writer)
/// must be present in the final state.
///
/// What breaks it (compile-time): deleting `managed_agents_store_lock.lock()` in
/// `compensate_drain_with_hook` leaves no value for the `&_store` argument; dropping
/// or moving `_store` before `on_after_restore` makes the borrow-check invalid.
#[test]
fn test_compensate_drain_writer_vs_compensation_deterministic() {
    use crate::managed_agents::scope::{
        current_scope_generation, WorkspaceAgentScope, SCOPE_GENERATION_TEST_LOCK,
    };
    use std::thread;
    use tauri::Manager;

    let _gen_guard = SCOPE_GENERATION_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    let tmp = tempfile::tempdir().unwrap();
    let tmp_path = tmp.path().to_path_buf();

    // Seed the store with one agent record.
    let pubkey1 = "aa".repeat(32);
    let initial_record = crate::managed_agents::ManagedAgentRecord {
        pubkey: pubkey1.clone(),
        name: "test-agent".to_string(),
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
        model: None,
        provider: None,
        persona_source_version: None,
        env_vars: Default::default(),
        start_on_app_launch: true,
        auto_restart_on_config_change: false,
        runtime_pid: None,
        backend: crate::managed_agents::BackendKind::Local,
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
    crate::managed_agents::storage::save_managed_agents_at(
        &tmp_path,
        std::slice::from_ref(&initial_record),
    )
    .unwrap();

    let app = tauri::test::mock_builder()
        .manage(crate::app_state::build_app_state())
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("failed to build mock app");
    let app_handle = app.app_handle().clone();
    let state = app.state::<crate::app_state::AppState>();

    // Seed a live runtime so `start_pair_under_held_locks` returns AlreadyRunning.
    // This makes `compensate_drain_for` succeed (`comp_result = None`) without
    // needing a real agent binary — compensation finds the agent already started.
    let rt_key =
        crate::managed_agents::ManagedAgentRuntimeKey::new(pubkey1.clone(), "wss://relay.example")
            .unwrap();
    {
        let mut runtimes = state.managed_agent_processes.lock().unwrap();
        let child = spawn_long_lived_child_for_test();
        let process = crate::managed_agents::ManagedAgentProcess {
            child,
            log_path: std::path::PathBuf::new(),
            spawn_config: crate::managed_agents::spawn_snapshot::prospective_spawn_config_snapshot(
                &initial_record,
                &[],
                &[],
                "wss://relay.example",
                &Default::default(),
                false,
            ),
            setup_mode: false,
            adapter_availability: None,
            start_nonce: "test-nonce-writer".to_string(),
            #[cfg(windows)]
            job: None,
        };
        runtimes.insert(
            rt_key.clone(),
            crate::managed_agents::ManagedAgentPairRuntime::starting(
                process,
                Some("comp-drain-writer-test".to_string()),
            ),
        );
    }

    let gen = current_scope_generation();
    let scope = WorkspaceAgentScope {
        scope_id: "comp-drain-writer-test".to_string(),
        relay_url: "wss://relay.example".to_string(),
        owner_pubkey: "aa".repeat(32),
        definitions_dir: tmp_path.clone(),
        generation: gen,
    };
    state.commit_active_scope(scope.clone());

    let entry1 = make_drain_entry(&pubkey1, "wss://relay.example", true);
    let stopped = vec![entry1];

    // records_loaded_tx: compensation → writer (both locks are held; writer may proceed)
    // writer_committed_tx: writer → main   (writer's save is complete)
    let (records_loaded_tx, records_loaded_rx) = std::sync::mpsc::channel::<()>();
    let (writer_committed_tx, writer_committed_rx) = std::sync::mpsc::channel::<()>();

    // Spawn the writer BEFORE acquiring the transition guard.
    // Flow: wait for records_loaded → acquire managed_agents_store_lock (blocks until
    // on_after_restore releases it) → load → add WRITER_EDIT → save → send writer_committed.
    let tmp_wr = tmp_path.clone();
    let app_handle_wr = app_handle.clone();
    let wr_thread = thread::spawn(move || {
        // Wait for on_records_loaded to signal that compensation holds both locks.
        records_loaded_rx.recv().unwrap();

        // One complete production-shaped transaction: acquire the store lock (blocks
        // until on_after_restore saves COMP_SENTINEL and the adapter releases the lock),
        // load records, add WRITER_EDIT, save.
        let writer_state = app_handle_wr.state::<crate::app_state::AppState>();
        let _store = writer_state.managed_agents_store_lock.lock().unwrap();
        let mut records =
            crate::managed_agents::storage::load_managed_agents_at(&tmp_wr).unwrap_or_default();
        for r in &mut records {
            r.env_vars
                .insert("WRITER_EDIT".to_string(), "yes".to_string());
        }
        crate::managed_agents::storage::save_managed_agents_at(&tmp_wr, &records).unwrap();

        // Signal after the save succeeds, while this guarded transaction is complete.
        writer_committed_tx.send(()).unwrap();
    });

    let rt_guard = state.managed_agent_runtime_transition.lock().unwrap();

    let comp_result = compensate_drain_with_hook(
        &app_handle,
        &stopped,
        &scope,
        rt_guard,
        // on_records_loaded: release the writer so it can queue on the store lock.
        // Both locks are held here; the writer will block until on_after_restore
        // releases the store guard.
        |_records| {
            records_loaded_tx.send(()).unwrap();
        },
        // on_after_restore: borrows the actual store guard — a compile error if
        // that guard is dropped or removed before this call. Add COMP_SENTINEL
        // and save while the guard is live; the writer is still blocked.
        |records, _store_guard| {
            let _ = _store_guard; // borrow is live; guard may not have been dropped
            for r in records.iter_mut() {
                r.env_vars
                    .insert("COMP_SENTINEL".to_string(), "yes".to_string());
            }
            crate::managed_agents::storage::save_managed_agents_at(&tmp_path, records)
                .expect("on_after_restore: save must succeed — store lock held");
        },
    );

    // Block until the writer's transaction is complete, then join.
    writer_committed_rx.recv().unwrap();
    wr_thread.join().expect("writer thread panicked");

    // Kill the seeded long-lived child.
    let seeded_pid = {
        let runtimes = state.managed_agent_processes.lock().unwrap();
        runtimes.get(&rt_key).map(|r| r.child.id())
    };
    if let Some(pid) = seeded_pid {
        let _ = crate::managed_agents::terminate_process(pid);
    }

    // Compensation must have succeeded: agent was AlreadyRunning, no errors.
    assert!(
        comp_result.is_none(),
        "compensation must succeed (AlreadyRunning path): {comp_result:?}"
    );

    // ── Final disk state: BOTH effects must be present ───────────────────────
    // COMP_SENTINEL: saved by on_after_restore while the store guard was live.
    // WRITER_EDIT:   saved by the writer after the adapter released the lock.
    let final_records =
        crate::managed_agents::storage::load_managed_agents_at(&tmp_path).unwrap_or_default();
    let final_rec = final_records
        .iter()
        .find(|r| r.pubkey == pubkey1)
        .expect("agent record must be present on disk after both phases");

    assert_eq!(
        final_rec.env_vars.get("COMP_SENTINEL").map(String::as_str),
        Some("yes"),
        "COMP_SENTINEL must reach disk via on_after_restore while the store guard is live"
    );
    assert_eq!(
        final_rec.env_vars.get("WRITER_EDIT").map(String::as_str),
        Some("yes"),
        "WRITER_EDIT must be present — the writer runs after compensation releases the store lock"
    );
}

/// Production start-path contender is blocked on `managed_agent_runtime_transition`
/// while `compensate_drain_with_hook` holds it; the `on_transition_acquired` seam
/// fires after the guard is acquired, proving start cannot cross the
/// transition-acquired seam while compensation owns the mutex.
///
/// Invariant under test: `start_pair_for_with_hook` owns `managed_agent_runtime_transition`
/// at the `on_transition_acquired` boundary. The callback receives a borrow of the actual
/// `MutexGuard<'_, ()>`. Dropping that guard before the callback or removing the lock
/// call from the seam is a **compile error**.
///
/// Proof by mutex exclusion: compensation holds `managed_agent_runtime_transition` for the
/// duration of `compensate_drain_with_hook`. The contender's `on_transition_acquired` borrows
/// that same mutex's guard — it can only execute after compensation releases the mutex.
/// No timing assumption is required; the scheduler cannot place the callback inside the
/// compensation window because mutex exclusion prevents it.
///
/// `start_pair_lazy_for_with_hook` is the production-callable seam. Removing
/// `managed_agent_runtime_transition` from this path also removes it from production.
#[test]
fn test_compensate_drain_concurrent_start_is_blocked() {
    use crate::managed_agents::scope::{
        current_scope_generation, WorkspaceAgentScope, SCOPE_GENERATION_TEST_LOCK,
    };
    use std::thread;
    use tauri::Manager;

    let _gen_guard = SCOPE_GENERATION_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    let tmp = tempfile::tempdir().unwrap();
    let tmp_path = tmp.path().to_path_buf();

    let pubkey1 = "aa".repeat(32);
    let initial_record = crate::managed_agents::ManagedAgentRecord {
        pubkey: pubkey1.clone(),
        name: "contender-agent".to_string(),
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
        model: None,
        provider: None,
        persona_source_version: None,
        env_vars: Default::default(),
        start_on_app_launch: true,
        auto_restart_on_config_change: false,
        runtime_pid: None,
        backend: crate::managed_agents::BackendKind::Local,
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
    crate::managed_agents::storage::save_managed_agents_at(
        &tmp_path,
        std::slice::from_ref(&initial_record),
    )
    .unwrap();

    let app = tauri::test::mock_builder()
        .manage(crate::app_state::build_app_state())
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("failed to build mock app");
    let app_handle = app.app_handle().clone();
    let state = app.state::<crate::app_state::AppState>();

    let gen = current_scope_generation();
    let scope = WorkspaceAgentScope {
        scope_id: "comp-drain-contender-test".to_string(),
        relay_url: "wss://relay.example".to_string(),
        owner_pubkey: "aa".repeat(32),
        definitions_dir: tmp_path.clone(),
        generation: gen,
    };
    state.commit_active_scope(scope.clone());

    let entry1 = make_drain_entry(&pubkey1, "wss://relay.example", true);
    let stopped = vec![entry1];

    // contender_at_boundary_tx:    contender → test (inside seam, about to block on transition lock)
    // transition_acquired_tx:      contender → test (contender acquired transition guard)
    let (contender_at_boundary_tx, contender_at_boundary_rx) = std::sync::mpsc::channel::<()>();
    let (transition_acquired_tx, transition_acquired_rx) = std::sync::mpsc::channel::<()>();

    // Acquire the transition guard FIRST so the contender blocks on it.
    let rt_guard = state.managed_agent_runtime_transition.lock().unwrap();

    let app_contender = app_handle.clone();
    let pubkey_contender = pubkey1.clone();
    let contender = thread::spawn(move || {
        // `start_pair_lazy_for_with_hook` is the production-called seam.
        // on_before_transition fires just before the lock call — signals "entered the seam".
        // on_transition_acquired fires after the lock is acquired, receiving a borrow of the
        // actual transition guard — sends one positive receipt to the test.
        let _ = start_pair_lazy_for_with_hook(
            pubkey_contender,
            "wss://relay.example".to_string(),
            app_contender,
            // on_before_transition: contender is inside the seam and about to block.
            move || {
                contender_at_boundary_tx.send(()).unwrap();
            },
            // on_transition_acquired: borrows the actual transition guard at the seam boundary.
            // Removing or dropping the guard before this call is a compile error.
            // Sends one positive receipt — proves the guard was acquired and live.
            move |_transition_guard| {
                let _ = _transition_guard; // borrow is live; guard may not have been dropped
                transition_acquired_tx.send(()).unwrap();
            },
        );
    });

    // Wait for the contender to be inside the seam and blocked on the transition lock.
    contender_at_boundary_rx.recv().unwrap();

    // Run compensation while holding the transition guard. The contender cannot acquire it;
    // its on_transition_acquired callback cannot execute until compensation returns.
    let _comp_result = compensate_drain_with_hook(
        &app_handle,
        &stopped,
        &scope,
        rt_guard,
        |_records| {},
        |_records, _store_guard| {},
    );

    // After compensation returns, the contender can acquire the transition guard.
    // Wait for the positive receipt from on_transition_acquired.
    // Mutex exclusion guarantees this callback did not execute during compensation —
    // no timing assumption is required.
    transition_acquired_rx.recv().expect(
        "contender's on_transition_acquired must fire after compensate_drain_with_hook \
             releases the transition guard",
    );

    contender.join().expect("contender thread panicked");
}
