//! Observer-feed archive default — always enabled.
//!
//! `observer_archive_default_enabled()` returns `true` unconditionally.
//! The frontend calls this every startup to decide whether to reconcile the
//! `owner_p` subscription for kind 24200 (observer frames). Kind 24200 events
//! are ephemeral — not stored by the relay — so local archiving is the only
//! way to retain them.

/// Returns `true`: observer-feed archive defaults to enabled for all builds.
///
/// The frontend reconciles the `owner_p` subscription every startup when this
/// returns `true`. A user who has explicitly disabled the toggle keeps it off
/// because the Settings card's explicit-opt-out path deletes the subscription
/// and the seed hook skips identities that already have an explicit choice.
#[tauri::command]
pub fn observer_archive_default_enabled() -> bool {
    true
}
