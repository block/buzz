//! Agent-turn-metric archive default — always enabled.
//!
//! `agent_metric_archive_default_enabled()` returns `true` unconditionally.
//! The frontend calls this once at startup to decide whether to seed the
//! `owner_p` [44200] save subscription for the current identity on first run.
//! The `hasExplicitChoice` guard in the TS seed hook ensures a user who has
//! explicitly opted out remains opted out.

/// Returns `true`: agent-turn-metric archive defaults to enabled for all builds.
///
/// The frontend uses this to decide whether to auto-seed an `owner_p` [44200]
/// save subscription on first run. Existing explicit choices (stored in
/// localStorage per identity) are preserved by the seed hook's `hasExplicitChoice`
/// guard — this default only applies to identities that have never made a choice.
#[tauri::command]
pub fn agent_metric_archive_default_enabled() -> bool {
    true
}
