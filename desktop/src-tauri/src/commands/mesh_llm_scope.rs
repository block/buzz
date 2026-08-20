//! Scope-aware helpers for the Mesh LLM command layer.
//!
//! Extracted from `mesh_llm.rs` to stay within the file-size ratchet.
//! Most items here are `pub(super)` so they remain private to the module;
//! `mesh_stop_client` is `pub(crate)` and re-exported as `pub` from `mesh_llm`.

use std::future::Future;

use futures_util::future::BoxFuture;
use tauri::{AppHandle, Manager, State};

use crate::app_state::AppState;
use crate::mesh_llm;
type CmdResult<T> = Result<T, String>;

/// Check whether the currently live Mesh runtime's relay matches the active
/// workspace scope's relay.
///
/// Returns:
/// - `Ok(true)`  — relays match; the caller should proceed to a liveness probe.
/// - `Ok(false)` — stale client runtime from another scope; treat as absent
///                 and fall through to re-arm.
/// - `Err(msg)`  — serve-mode runtime pinned to another relay (fail closed), or
///                 no active workspace scope.
///
/// Assumes `state.mesh_llm_runtime.lock()` is NOT held by the caller.
pub(super) async fn check_mesh_runtime_relay_scope(state: &AppState) -> Result<bool, String> {
    let scope_relay = state
        .capture_active_scope()
        .map(|s| s.relay_url.clone())
        .ok_or("Buzz shared compute cannot start: no active workspace scope")?;
    let (runtime_relay, runtime_mode) = {
        let guard = state.mesh_llm_runtime.lock().await;
        let relay = guard
            .as_ref()
            .and_then(|r| r.start_request().relay_url.clone());
        let mode = guard.as_ref().map(|r| r.mode());
        (relay, mode)
    };

    let relay_matches = runtime_relay.as_deref().map_or(false, |bound| {
        crate::managed_agents::scope::normalize_relay_for_scope(bound)
            == crate::managed_agents::scope::normalize_relay_for_scope(&scope_relay)
    });

    if relay_matches {
        return Ok(true);
    }

    match runtime_mode {
        Some(mesh_llm::MeshNodeMode::Serve) => {
            // Fail closed: Share Compute is pinned to another relay.
            // The process has one runtime slot and one :9337 ingress.
            // No client can start while serve occupies it.
            let pinned_relay = runtime_relay.as_deref().unwrap_or("another relay");
            Err(format!(
                "Share Compute is currently pinned to {pinned_relay}. \
                 Stop sharing first, then switch workspaces to use \
                 Buzz shared compute on this workspace."
            ))
        }
        Some(mesh_llm::MeshNodeMode::Client) | None => {
            // Stale client from a prior workspace. Treat as absent —
            // fall through to re-arm a new client for the active scope.
            // (Option A forbids switching with a client active; this path
            // is only reached by a serve→client downgrade within the same
            // scope or after `mesh_stop_client` cleared the prior client.)
            Ok(false)
        }
    }
}

/// Check whether a client-mode Mesh runtime is currently active.
///
/// Returns `Err(msg)` with a user-facing message when a client runtime is
/// present — the caller should fail the workspace switch / identity import
/// with this message so the user knows to stop Mesh first.
///
/// Serve-mode runtimes and absent runtimes both return `Ok(())` — they are
/// machine-level (serve) or simply not running (absent) and do not block
/// a workspace switch.
///
/// Called from the Layer-1 async stage of `apply_workspace` and
/// `import_identity` before entering `spawn_blocking`.
pub(crate) async fn fail_if_client_mesh_active<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let guard = state.mesh_llm_runtime.lock().await;
    let is_client = guard
        .as_ref()
        .map_or(false, |r| r.mode() == mesh_llm::MeshNodeMode::Client);
    if is_client {
        return Err("A Buzz shared compute (client) session is active. \
             Stop it in the Shared Compute settings before switching workspaces."
            .to_string());
    }
    Ok(())
}

/// Acquire the `workspace_transition` lock, run the production
/// `fail_if_client_mesh_active` preflight, then invoke `transition_body` while
/// the guard remains held.
///
/// Delegates to [`with_workspace_transition_preflight_with_hook`] with a no-op
/// pre-acquisition hook. See that function for full documentation.
pub(crate) async fn with_workspace_transition_preflight<R, F, T>(
    app: &AppHandle<R>,
    transition_body: F,
) -> Result<T, String>
where
    R: tauri::Runtime,
    F: FnOnce() -> BoxFuture<'static, Result<T, String>>,
{
    with_workspace_transition_preflight_with_hook(app, || {}, transition_body).await
}

/// Inner implementation of [`with_workspace_transition_preflight`] with an
/// injectable `pre_acquisition_hook`.
///
/// `pre_acquisition_hook` fires once, synchronously, immediately before
/// `workspace_transition.lock().await`. In production this is `|| {}`; tests
/// inject a closure that signals "I'm about to acquire" so the test can
/// establish a deterministic ordering between holder and contender.
///
/// This is `pub(crate)` so tests in `mesh_llm_transition_tests` can drive the
/// exact lock-acquisition boundary while remaining invisible to external callers.
pub(crate) async fn with_workspace_transition_preflight_with_hook<R, H, F, T>(
    app: &AppHandle<R>,
    pre_acquisition_hook: H,
    transition_body: F,
) -> Result<T, String>
where
    R: tauri::Runtime,
    H: FnOnce(),
    F: FnOnce() -> BoxFuture<'static, Result<T, String>>,
{
    let state = app.state::<AppState>();
    pre_acquisition_hook();
    let _transition_guard = state.workspace_transition.lock().await;

    // Fail closed if a client-mode Mesh runtime is active.
    #[cfg(feature = "mesh-llm")]
    fail_if_client_mesh_active(app).await?;

    transition_body().await
}

/// Run only the Mesh-preflight portion of the workspace transition check.
///
/// Called by production commands (`apply_workspace`, `import_identity`) that
/// already hold `workspace_transition` and need to avoid duplicating the
/// `fail_if_client_mesh_active` call inline. The guard must already be acquired
/// and remain alive for the duration of the transition.
///
/// No-op when the `mesh-llm` feature is disabled.
pub(crate) async fn run_mesh_transition_preflight<R>(app: &AppHandle<R>) -> Result<(), String>
where
    R: tauri::Runtime,
{
    #[cfg(feature = "mesh-llm")]
    fail_if_client_mesh_active(app).await?;
    #[cfg(not(feature = "mesh-llm"))]
    let _ = app;
    Ok(())
}

/// Acquire the `workspace_transition` lock, validate the full captured scope
/// identity under the guard — `(scope_id, normalized relay, owner_pubkey,
/// generation)` — then call the injected `install` closure.
///
/// Delegates to [`install_client_under_workspace_transition_with_hook`] with a
/// no-op pre-acquisition hook. See that function for full documentation.
pub(crate) async fn install_client_under_workspace_transition<R, I, Fut>(
    app: &AppHandle<R>,
    captured_scope: &crate::managed_agents::scope::WorkspaceAgentScope,
    install: I,
) -> Result<(), String>
where
    R: tauri::Runtime,
    I: FnOnce() -> Fut,
    Fut: Future<Output = Result<(), String>>,
{
    install_client_under_workspace_transition_with_hook(app, captured_scope, || {}, install).await
}

/// Inner implementation of [`install_client_under_workspace_transition`] with an
/// injectable `pre_acquisition_hook`.
///
/// `pre_acquisition_hook` fires once, synchronously, immediately before
/// `workspace_transition.lock().await`. In production this is `|| {}`; tests
/// inject a closure that signals "I'm about to acquire" so the test can
/// establish a deterministic ordering between holder and contender.
///
/// Fails if:
/// - no active scope exists at the time of validation;
/// - any identity field of the captured scope differs from the current active scope;
/// - the generation counter has advanced (a workspace switch occurred).
///
/// This is `pub(crate)` so tests in `mesh_llm_transition_tests` can drive the
/// exact lock-acquisition boundary while remaining invisible to external callers.
pub(crate) async fn install_client_under_workspace_transition_with_hook<R, H, I, Fut>(
    app: &AppHandle<R>,
    captured_scope: &crate::managed_agents::scope::WorkspaceAgentScope,
    pre_acquisition_hook: H,
    install: I,
) -> Result<(), String>
where
    R: tauri::Runtime,
    H: FnOnce(),
    I: FnOnce() -> Fut,
    Fut: Future<Output = Result<(), String>>,
{
    let state = app.state::<AppState>();
    pre_acquisition_hook();
    let _transition_guard = state.workspace_transition.lock().await;

    // Validate the captured scope's generation and full identity under the guard.
    // If the workspace switched after discovery but before we acquired the lock,
    // abort without invoking install.
    crate::managed_agents::scope::validate_scope_generation(captured_scope)
        .map_err(|e| format!("mesh client install: captured scope stale: {e}"))?;

    let active = state.capture_active_scope().ok_or(
        "mesh client install: no active workspace scope after lock acquisition".to_string(),
    )?;

    // Validate the full scope identity — not just the generation counter.
    let normalize = crate::managed_agents::scope::normalize_relay_for_scope;
    if active.scope_id != captured_scope.scope_id
        || normalize(&active.relay_url) != normalize(&captured_scope.relay_url)
        || active.owner_pubkey != captured_scope.owner_pubkey
    {
        return Err(format!(
            "mesh client install: captured scope identity mismatch \
             (captured scope_id={}, relay={}, owner={}; \
              active scope_id={}, relay={}, owner={})",
            captured_scope.scope_id,
            captured_scope.relay_url,
            captured_scope.owner_pubkey,
            active.scope_id,
            active.relay_url,
            active.owner_pubkey,
        ));
    }

    install().await
}

/// Stop the local Mesh **client** (consuming) runtime.
///
/// Only tears down a client-mode runtime. Serve-mode and absent runtimes are
/// left unchanged — this command has no effect on sharing nodes.
///
/// Required by Option A: a workspace switch fails while a client is active;
/// the user calls this command to stop the client before the switch proceeds.
#[tauri::command]
pub(crate) async fn mesh_stop_client<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: State<'_, AppState>,
) -> CmdResult<mesh_llm::MeshNodeStatus> {
    let (taken, bound_relay_url) = {
        let mut guard = state.mesh_llm_runtime.lock().await;
        if let Some(runtime) = guard.as_ref() {
            if runtime.mode() != mesh_llm::MeshNodeMode::Client {
                return runtime.status().await.map_err(|e| e.to_string());
            }
        } else {
            return Ok(mesh_llm::stopped_status());
        }
        let bound_relay_url = guard
            .as_ref()
            .and_then(|r| r.start_request().relay_url.clone());
        (guard.take(), bound_relay_url)
    };
    if let Some(runtime) = taken {
        runtime.stop().await.map_err(|e| e.to_string())?;
    }
    mesh_llm::publish_stopped_status_once_at(&app, bound_relay_url.as_deref(), "stop").await;
    Ok(mesh_llm::stopped_status())
}
