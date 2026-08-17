use crate::app_state::AppState;

/// Acquire the lifecycle serialization guard using the repository-wide error
/// shape. Callers must take this before the managed-agent store and runtime
/// maps so restore, start, stop, delete, and security-authority edits cannot
/// interleave an unregistered process generation.
pub(crate) fn lock(state: &AppState) -> Result<std::sync::MutexGuard<'_, ()>, String> {
    state
        .managed_agent_runtime_transition
        .lock()
        .map_err(|error| error.to_string())
}
