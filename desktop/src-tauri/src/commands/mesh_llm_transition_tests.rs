//! Area-4 workspace-transition serialization tests for `commands/mesh_llm.rs`.
//!
//! Split from `mesh_llm_tests.rs` to keep each file under the 1000-line ratchet.
//! Included via `#[path]` from `mesh_llm_tests.rs` as `mod transition_tests;`.
//! `use super::*` gives access to all items in `mesh_llm_tests.rs`.

// ── Area 4 serialization direction tests ─────────────────────────────────────
//
// These three tests prove the lock-serialization contract between
// `with_workspace_transition_preflight` and `install_client_under_workspace_transition`.
// No port (`127.0.0.1:9337`) is touched — the install closure is always injected.

/// Active mock client → `mesh_stop_client` → runtime slot is `None` →
/// `with_workspace_transition_preflight` with a no-op body succeeds.
///
/// Proves the two-step Option A user flow:
/// 1. user calls "stop shared compute" (`mesh_stop_client`);
/// 2. workspace switch proceeds via the production transition-preflight helper.
///
/// The production `fail_if_client_mesh_active` is called inside
/// `with_workspace_transition_preflight` (under the guard). It must see an
/// absent runtime and return `Ok(())` — not the stale client that was there
/// before `mesh_stop_client` cleared it.
#[tokio::test]
async fn test_active_client_stop_then_transition_preflight_succeeds() {
    use tauri::Manager;

    let app = tauri::test::mock_builder()
        .manage(crate::app_state::build_app_state())
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("failed to build mock app");
    let app_handle = app.handle().clone();
    let state = app.state::<crate::app_state::AppState>();

    // Install a client-mode runtime — simulates "Mesh is running as a client".
    {
        let client_runtime = crate::mesh_llm::build_mock_client_runtime_for_test();
        *state.mesh_llm_runtime.lock().await = Some(client_runtime);
    }

    // Verify the client is present before the stop.
    {
        let guard = state.mesh_llm_runtime.lock().await;
        assert!(
            guard.is_some(),
            "pre-condition: a client runtime must be installed before stop"
        );
    }

    // Step 1 — production `mesh_stop_client` tears down the client.
    let stop_status = super::mesh_stop_client(app_handle.clone(), state.clone())
        .await
        .expect("mesh_stop_client must not error");
    assert_eq!(
        stop_status.state,
        crate::mesh_llm::MeshNodeState::Off,
        "mesh_stop_client must return Off status after tearing down the client"
    );

    // Step 2 — runtime slot must now be None.
    {
        let guard = state.mesh_llm_runtime.lock().await;
        assert!(
            guard.is_none(),
            "mesh_stop_client must clear the runtime slot; got {:?}",
            guard.as_ref().map(|r| r.mode())
        );
    }

    // Step 3 — production transition preflight must succeed (no client active).
    // The no-op body proves the lock was acquired and the preflight passed.
    let result = super::scope_impl::with_workspace_transition_preflight(&app_handle, || {
        Box::pin(async { Ok::<&str, String>("body ran") })
    })
    .await;

    assert!(
        result.is_ok(),
        "with_workspace_transition_preflight must succeed after mesh_stop_client cleared the slot: {result:?}"
    );
    assert_eq!(
        result.unwrap(),
        "body ran",
        "transition body must have been invoked and its return value propagated"
    );
}

/// `with_workspace_transition_preflight` holds the lock (body blocks on a
/// oneshot channel); `install_client_under_workspace_transition` queues on the
/// same lock.  After the body completes (advancing the scope generation while
/// the lock is held), the install task acquires the lock and detects the stale
/// captured scope without invoking the install closure.
///
/// Handshake:
///   1. Transition task acquires the lock, signals "lock_held" BEFORE committing
///      scope B (so install's captured scope is still valid at its capture time).
///   2. Install task spawns, calls `install_client_under_workspace_transition`
///      with a pre-acquisition hook that signals "at_lock_boundary", then
///      blocks on `workspace_transition.lock().await`.
///   3. Test waits for "at_lock_boundary", then sends "release" to the
///      transition body. The body commits scope B and returns, releasing the lock.
///   4. Install task acquires the lock, re-validates scope, detects the stale
///      generation, returns Err without invoking the install closure.
///
/// Invariant: if the contender could bypass the lock, it would acquire before
/// the transition commits scope B, see a valid scope, and invoke the install
/// closure — `install_was_called` would be true and the assertion would fail.
#[tokio::test]
async fn test_transition_held_queued_install_detects_stale_scope() {
    use crate::managed_agents::scope::{
        next_scope_generation, WorkspaceAgentScope, SCOPE_GENERATION_TEST_LOCK,
    };
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use tauri::Manager;
    use tokio::sync::oneshot;

    // Serialize generation-sensitive work across parallel tests.
    let _gen_guard = SCOPE_GENERATION_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    let app = tauri::test::mock_builder()
        .manage(crate::app_state::build_app_state())
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("failed to build mock app");
    let app_handle = app.handle().clone();
    let state = app.state::<crate::app_state::AppState>();

    let base = std::path::PathBuf::from("/tmp/area4-test-dir");
    let relay_a = "wss://transition-test-a.example";
    let owner_a = "aa".repeat(32);

    // Capture a scope at the current generation — the install task will carry
    // this scope after "discovering a bootstrap target".
    let gen_a = next_scope_generation();
    let captured_scope =
        WorkspaceAgentScope::new(relay_a.to_string(), owner_a.clone(), &base, gen_a);
    state.commit_active_scope(captured_scope.clone());

    // Channels:
    //   lock_held:         transition body → test  (lock acquired, scope NOT yet committed)
    //   at_lock_boundary:  install hook → test     (install is about to call lock().await)
    //   body_release:      test → transition body  (ok to commit scope B and release)
    let (lock_held_tx, lock_held_rx) = oneshot::channel::<()>();
    let (at_lock_boundary_tx, at_lock_boundary_rx) = oneshot::channel::<()>();
    let (body_release_tx, body_release_rx) = oneshot::channel::<()>();

    // ── Side A: transition side — signals lock_held BEFORE committing scope B ─
    let app_a = app_handle.clone();
    let base_a = base.clone();
    let transition_task = tokio::task::spawn(async move {
        let app_a_ref = app_a.clone();
        super::scope_impl::with_workspace_transition_preflight(&app_a_ref, move || {
            Box::pin(async move {
                let state_a = app_a.state::<crate::app_state::AppState>();

                // Signal "lock held" BEFORE committing scope B.
                // The install task captures its scope before this point;
                // only after it queues at the lock does the holder commit B.
                let _ = lock_held_tx.send(());

                // Wait for the install task to queue at the lock boundary.
                let _ = body_release_rx.await;

                // Now commit scope B (advances generation, install sees stale).
                let gen_b = next_scope_generation();
                let new_scope = WorkspaceAgentScope::new(
                    "wss://transition-test-b.example".to_string(),
                    "bb".repeat(32),
                    &base_a,
                    gen_b,
                );
                state_a.commit_active_scope(new_scope);

                Ok::<(), String>(())
            })
        })
        .await
    });

    // Wait until the transition body holds the lock (before scope B is committed).
    lock_held_rx
        .await
        .expect("transition body must signal lock_held");

    // ── Side B: install side — pre-acquisition hook signals at_lock_boundary ──
    let install_was_called = Arc::new(AtomicBool::new(false));
    let install_called_clone = Arc::clone(&install_was_called);
    let app_b = app_handle.clone();
    let install_task = tokio::task::spawn(async move {
        super::scope_impl::install_client_under_workspace_transition_with_hook(
            &app_b,
            &captured_scope,
            // pre-acquisition hook: fires before lock().await — signals "queued".
            move || {
                let _ = at_lock_boundary_tx.send(());
            },
            || {
                let called = Arc::clone(&install_called_clone);
                async move {
                    called.store(true, Ordering::SeqCst);
                    Ok::<(), String>(())
                }
            },
        )
        .await
    });

    // Wait for install to reach the lock boundary, then unblock the transition.
    at_lock_boundary_rx
        .await
        .expect("install hook must signal at_lock_boundary");

    // Unblock the transition body → it commits scope B, releases the lock →
    // install acquires the lock, detects stale scope, returns Err.
    let _ = body_release_tx.send(());

    let install_result = install_task.await.expect("install task must not panic");
    transition_task
        .await
        .expect("transition task must not panic")
        .expect("transition body must succeed");

    // The install helper must have rejected the stale scope without invoking install.
    assert!(
        install_result.is_err(),
        "install_client_under_workspace_transition must return Err for a stale captured scope; \
         got Ok"
    );
    let err = install_result.unwrap_err();
    assert!(
        err.contains("stale") || err.contains("mismatch") || err.contains("scope"),
        "error must describe the stale/mismatched scope: {err}"
    );
    assert!(
        !install_was_called.load(Ordering::SeqCst),
        "install closure must NOT have been invoked when the scope was stale"
    );
}

/// `install_client_under_workspace_transition` holds the lock (install closure
/// blocks on a oneshot channel after installing a mock client);
/// `with_workspace_transition_preflight` queues on the same lock.  After the
/// install body completes, the transition task acquires the lock, runs
/// `fail_if_client_mesh_active`, and observes the installed client.
///
/// Handshake:
///   1. Install task acquires the lock, signals "lock_held" BEFORE installing the
///      client runtime (so the client is not yet visible to the contender).
///   2. Transition task calls `with_workspace_transition_preflight` with a
///      pre-acquisition hook that signals "at_lock_boundary", then blocks on
///      `workspace_transition.lock().await`.
///   3. Test waits for "at_lock_boundary", then sends "release" to the
///      install closure. The closure installs the client and returns, releasing
///      the lock.
///   4. Transition task acquires the lock, runs `fail_if_client_mesh_active`,
///      observes the installed client, returns Err.
///
/// Invariant: if the contender could bypass the lock, it would run
/// `fail_if_client_mesh_active` before the client is installed — seeing no
/// client — and return Ok. The `transition_result.is_err()` assertion would
/// then fail, proving the lock is not enforced.
#[tokio::test]
async fn test_install_held_transition_preflight_observes_client() {
    use crate::managed_agents::scope::{
        next_scope_generation, WorkspaceAgentScope, SCOPE_GENERATION_TEST_LOCK,
    };
    use tauri::Manager;
    use tokio::sync::oneshot;

    // Serialize generation-sensitive work across parallel tests.
    let _gen_guard = SCOPE_GENERATION_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    let app = tauri::test::mock_builder()
        .manage(crate::app_state::build_app_state())
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("failed to build mock app");
    let app_handle = app.handle().clone();
    let state = app.state::<crate::app_state::AppState>();

    let base = std::path::PathBuf::from("/tmp/area4-test-dir-inv");
    let relay = "wss://install-first-test.example";
    let owner = "cc".repeat(32);

    // Set up an active scope so install_client_under_workspace_transition can
    // validate full scope identity under the guard.
    let gen = next_scope_generation();
    let scope = WorkspaceAgentScope::new(relay.to_string(), owner.clone(), &base, gen);
    state.commit_active_scope(scope.clone());

    // Channels:
    //   lock_held:        install body → test  (lock acquired, client NOT yet installed)
    //   at_lock_boundary: transition hook → test (transition is about to call lock().await)
    //   install_release:  test → install body   (ok to install client and release lock)
    let (lock_held_tx, lock_held_rx) = oneshot::channel::<()>();
    let (at_lock_boundary_tx, at_lock_boundary_rx) = oneshot::channel::<()>();
    let (install_release_tx, install_release_rx) = oneshot::channel::<()>();

    // ── Side A: install side — signals lock_held BEFORE installing the client ─
    let app_a = app_handle.clone();
    let scope_a = scope.clone();
    let install_task = tokio::task::spawn(async move {
        let app_a_for_closure = app_a.clone();
        super::scope_impl::install_client_under_workspace_transition(
            &app_a,
            &scope_a,
            move || async move {
                // Signal "lock held" BEFORE installing the client.
                // The transition task captures its pre-lock state before this
                // point; only after it queues does the install closure install.
                let _ = lock_held_tx.send(());

                // Wait for the transition task to queue at the lock boundary.
                let _ = install_release_rx.await;

                // Now install the mock client runtime under the held lock.
                let state_a = app_a_for_closure.state::<crate::app_state::AppState>();
                let client_runtime = crate::mesh_llm::build_mock_client_runtime_for_test();
                *state_a.mesh_llm_runtime.lock().await = Some(client_runtime);

                Ok::<(), String>(())
            },
        )
        .await
    });

    // Wait until the install body holds the lock (before client is installed).
    lock_held_rx
        .await
        .expect("install body must signal lock_held");

    // ── Side B: transition side — pre-acquisition hook signals at_lock_boundary
    let app_b = app_handle.clone();
    let transition_task = tokio::task::spawn(async move {
        super::scope_impl::with_workspace_transition_preflight_with_hook(
            &app_b,
            // pre-acquisition hook: fires before lock().await — signals "queued".
            move || {
                let _ = at_lock_boundary_tx.send(());
            },
            || Box::pin(async { Ok::<&str, String>("body ran") }),
        )
        .await
    });

    // Wait for transition to reach the lock boundary, then unblock the install.
    at_lock_boundary_rx
        .await
        .expect("transition hook must signal at_lock_boundary");

    // Unblock the install closure → it installs the client, releases the lock →
    // transition acquires the lock, observes the client, returns Err.
    let _ = install_release_tx.send(());

    install_task
        .await
        .expect("install task must not panic")
        .expect("install closure must succeed");

    let transition_result = transition_task
        .await
        .expect("transition task must not panic");

    // `fail_if_client_mesh_active` must observe the installed client and reject.
    assert!(
        transition_result.is_err(),
        "with_workspace_transition_preflight must return Err when a client was installed \
         while holding the lock; got Ok"
    );
    let err = transition_result.unwrap_err();
    assert!(
        err.contains("client") || err.contains("Stop") || err.contains("shared compute"),
        "error must describe the active client and how to stop it: {err}"
    );
}
