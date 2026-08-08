//! Injectable seams for `managed_agents/runtime_commands.rs`.
//!
//! Extracted here to keep `runtime_commands.rs` under the 1000-line size ratchet.
//! Included via `#[path]` from `runtime_commands.rs`.

use super::*;

/// Lazy start seam with injectable before/after-transition hooks.
///
/// Delegates to [`start_pair_for_with_hook`] with `lazy = true`. Tests that
/// need a mock-runtime contender call this directly — it is the exact function
/// that production `start_managed_agent_runtime_pair_lazy` calls through
/// `start_pair_lazy_for`, so removing the transition guard from this path would
/// also remove it from production.
pub(crate) fn start_pair_lazy_for_with_hook<R: tauri::Runtime>(
    pubkey: String,
    relay_url: String,
    app: tauri::AppHandle<R>,
    on_before_transition: impl FnOnce(),
    on_transition_acquired: impl FnOnce(&std::sync::MutexGuard<'_, ()>),
) -> Result<ManagedAgentRuntimeStatus, String> {
    start_pair_for_with_hook(
        pubkey,
        relay_url,
        true,
        None,
        app,
        on_before_transition,
        on_transition_acquired,
    )
}

/// Generic start-pair seam with injectable hooks.
///
/// - `on_before_transition`: fires BEFORE `managed_agent_runtime_transition` is
///   locked. Tests signal "entered the seam" from here.
///
/// - `on_transition_acquired`: fires AFTER `managed_agent_runtime_transition` is
///   acquired (borrowing the actual guard) but BEFORE `managed_agents_store_lock`
///   is attempted. The argument is a borrow of the actual transition guard; a
///   drop or removal of that guard before this call is a compile error.
///
/// Production delegates with `|| {}` and `|_| {}` for the two hooks —
/// release-invisible no-ops.
pub(crate) fn start_pair_for_with_hook<R: tauri::Runtime>(
    pubkey: String,
    relay_url: String,
    lazy: bool,
    expected_updated_at: Option<&str>,
    app: tauri::AppHandle<R>,
    on_before_transition: impl FnOnce(),
    on_transition_acquired: impl FnOnce(&std::sync::MutexGuard<'_, ()>),
) -> Result<ManagedAgentRuntimeStatus, String> {
    let state = app.state::<AppState>();
    on_before_transition();
    let transition_guard = state
        .managed_agent_runtime_transition
        .lock()
        .map_err(|e| e.to_string())?;
    if state
        .shutdown_started
        .load(std::sync::atomic::Ordering::Acquire)
    {
        return Err("desktop shutdown has started".into());
    }
    on_transition_acquired(&transition_guard);
    let _store = state
        .managed_agents_store_lock
        .lock()
        .map_err(|e| e.to_string())?;
    let mut records = load_managed_agents(&app)?;
    start_pair_under_held_locks(
        &app,
        &state,
        pubkey,
        relay_url,
        lazy,
        expected_updated_at,
        &mut records,
    )
}
