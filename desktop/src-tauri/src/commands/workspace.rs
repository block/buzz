use nostr::Keys;
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::app_state::AppState;
use crate::managed_agents::{ensure_repos_symlink, nest_dir, try_regenerate_nest};
use crate::relay;

#[derive(Serialize)]
pub struct ActiveWorkspaceInfo {
    relay_url: String,
    pubkey: String,
}

/// Returns the current active workspace info (relay URL + pubkey).
#[tauri::command]
pub fn get_active_workspace(state: State<'_, AppState>) -> Result<ActiveWorkspaceInfo, String> {
    let keys = state.keys.lock().map_err(|e| e.to_string())?;
    let relay_url = relay::relay_ws_url_with_override(&state);
    Ok(ActiveWorkspaceInfo {
        relay_url,
        pubkey: keys.public_key().to_hex(),
    })
}

/// Apply a workspace's configuration to the backend session.
///
/// Called by the frontend on app init (after reload) to configure the
/// Tauri backend with the selected workspace's relay URL, keys, and repos
/// directory.
///
/// Validation runs before any state mutation: an invalid `repos_dir` (bad
/// path) rejects cleanly with nothing applied. The `REPOS` symlink itself is
/// a filesystem *side-effect* — its failure (e.g. a non-empty real `REPOS`
/// refusing a downgrade, or a renamed external target on a later launch) is
/// non-fatal: relay/keys still apply, the command returns `Ok`, and a
/// `repos-dir-error` event surfaces the failure to the frontend.
#[tauri::command]
pub fn apply_workspace(
    relay_url: String,
    nsec: Option<String>,
    repos_dir: Option<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    // ── Validate before mutating ──────────────────────────────────────────
    let parsed_keys = match nsec.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(nsec_trimmed) => {
            Some(Keys::parse(nsec_trimmed).map_err(|e| format!("invalid nsec: {e}"))?)
        }
        None => None,
    };

    // Normalize repos_dir to a trimmed non-empty value. `None`/empty clears
    // the override (REPOS falls back to a real dir). A bad path is rejected
    // here — before any mutation — so the dialog sees a clean Err.
    let repos_dir = repos_dir
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    if let Some(dir) = repos_dir.as_deref() {
        let nest = nest_dir().ok_or("cannot resolve home directory for nest")?;
        // Validate without mutating the filesystem. Keeps the command's
        // "validate-first, nothing below can fail" contract honest. Also emit
        // the error so it surfaces even at the init call site (which swallows
        // the returned Err to console for the relay/keys path).
        if let Err(error) = crate::managed_agents::validate_repos_dir(&nest, dir) {
            let _ = app.emit("repos-dir-error", error.clone());
            return Err(error);
        }
    }

    // ── Apply all state changes (nothing below can fail) ──────────────────
    {
        let mut override_guard = state.relay_url_override.lock().map_err(|e| e.to_string())?;
        *override_guard = Some(relay_url);
    }

    if let Some(keys) = parsed_keys {
        let mut keys_guard = state.keys.lock().map_err(|e| e.to_string())?;
        *keys_guard = keys;
    }

    // ── Filesystem side-effect (non-fatal) ────────────────────────────────
    // Re-point REPOS to match repos_dir. Failure here (downgrade refused,
    // external target gone) must NOT fail the command — relay/keys are already
    // applied. Surface it via a `repos-dir-error` event the frontend toasts.
    if let Some(nest) = nest_dir() {
        if let Err(error) = ensure_repos_symlink(&nest, repos_dir.as_deref()) {
            eprintln!("buzz-desktop: repos dir setup failed: {error}");
            let _ = app.emit("repos-dir-error", error);
        }
    }

    try_regenerate_nest(&app);

    Ok(())
}
