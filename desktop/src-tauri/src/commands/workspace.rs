use nostr::Keys;
use serde::{Deserialize, Serialize};
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::app_state::AppState;
use crate::managed_agents::{
    activate_workspace_agents, effective_repos_dir, ensure_repos_symlink, load_managed_agents,
    nest_dir, persist_workspace_repos_dir, rebind_agent_relay_urls, rebind_workspace_repos_dir,
    save_managed_agents, stamp_blank_agent_relay_urls, try_regenerate_nest,
    write_persisted_repos_dir,
};
use crate::relay;

#[derive(Deserialize)]
struct RelayInfoIcon {
    #[serde(default)]
    icon: Option<String>,
}

/// Fetch a relay's workspace icon from its NIP-11 relay information document.
///
/// Works for any workspace (active or not) with a plain unauthenticated HTTP
/// GET — no WebSocket session needed. Returns `None` when the relay has no
/// icon set, is unreachable, or serves a malformed document: the rail falls
/// back to initials in all three cases.
#[tauri::command]
pub async fn fetch_workspace_icon(
    relay_url: String,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    let http_url = relay::relay_http_base_url(&relay_url);
    let Ok(response) = state
        .http_client
        .get(&http_url)
        .header("Accept", "application/nostr+json")
        .send()
        .await
    else {
        return Ok(None);
    };
    if !response.status().is_success() {
        return Ok(None);
    }
    let doc = response
        .json::<RelayInfoIcon>()
        .await
        .unwrap_or(RelayInfoIcon { icon: None });
    Ok(doc.icon.filter(|icon| !icon.is_empty()))
}

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

/// Validate a candidate `repos_dir` without mutating the filesystem.
///
/// The Add/Edit workspace dialogs call this on submit to block Save on a bad
/// path, so a typo never reaches `apply_workspace`. Reuses the same
/// `validate_repos_dir` the boot/apply path uses — one source of truth for
/// "what's a valid repos dir". An empty/whitespace value clears the override
/// and is valid. `Err` carries the human-readable reason for inline display.
#[tauri::command]
pub async fn validate_repos_dir(dir: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let trimmed = dir.trim();
        if trimmed.is_empty() {
            return Ok(());
        }
        let nest = nest_dir().ok_or("cannot resolve home directory for nest")?;
        crate::managed_agents::validate_repos_dir(&nest, trimmed).map(|_| ())
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

/// Apply a workspace's configuration to the backend session.
///
/// Called by the frontend on app init (after reload) to configure the
/// Tauri backend with the selected workspace's relay URL, keys, and repos
/// directory.
///
/// A bad `repos_dir` is non-fatal: relay/keys always apply (the relay is the
/// active workspace's own choice — orthogonal to the filesystem repos dir),
/// the bad value is NOT persisted (so the next boot starts clean), the
/// `REPOS` symlink is skipped (REPOS stays a real dir), a `repos-dir-error`
/// event surfaces the reason, and the command returns `Ok`. The dialogs
/// already block a bad path at Save (`validate_repos_dir`); this fallback only
/// catches a value that went bad after save (deleted dir, unmounted volume).
#[tauri::command]
pub async fn apply_workspace(
    relay_url: String,
    nsec: Option<String>,
    repos_dir: Option<String>,
    app: AppHandle,
) -> Result<(), String> {
    let restore_app = app.clone();
    let activation_relay_url = relay_url.clone();
    tokio::task::spawn_blocking(move || {
        let state = app.state::<AppState>();

        // ── Validate before mutating ──────────────────────────────────────────
        let parsed_keys = match nsec.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(nsec_trimmed) => {
                Some(Keys::parse(nsec_trimmed).map_err(|e| format!("invalid nsec: {e}"))?)
            }
            None => None,
        };

        // Decide the effective repos_dir from the candidate. A bad path does NOT
        // reject — it is treated as if no override were set: relay/keys still
        // apply, the bad value is not persisted, and a `repos-dir-error` surfaces
        // the reason. Persisting a bad path would make every later boot read it,
        // fail to resolve the symlink, and silently skip agent restore. One
        // validate (inside `effective_repos_dir`) drives both the emit and the
        // persisted value. `nest` is resolved softly: when absent there is nothing
        // to persist or symlink, and relay/keys must still apply unconditionally.
        let nest = nest_dir();
        let effective_repos_dir = match nest.as_deref() {
            Some(nest) => match effective_repos_dir(nest, repos_dir.as_deref()) {
                Ok(value) => value,
                Err(error) => {
                    let _ = app.emit("repos-dir-error", error);
                    None
                }
            },
            None => None,
        };

        // ── Apply all state changes (nothing below can fail) ──────────────────
        {
            let mut override_guard = state.relay_url_override.lock().map_err(|e| e.to_string())?;
            *override_guard = Some(relay_url.clone());
        }

        if let Some(keys) = parsed_keys {
            let mut keys_guard = state.keys.lock().map_err(|e| e.to_string())?;
            *keys_guard = keys;
        }

        // ── One-shot legacy migration (non-fatal) ─────────────────────────────
        // Pin any blank-relay agent record to the first workspace applied
        // after boot — exactly what blank would have resolved to at boot
        // restore, so this is behavior-preserving while stopping the record
        // from floating to a later workspace switch. Runs before the restore
        // trigger below so restored agents spawn from stamped records. A
        // failure is logged and retried on the next boot.
        if state
            .agent_relay_stamp_pending
            .swap(false, Ordering::AcqRel)
        {
            if let Err(error) = pin_blank_agent_relays(&app, &state, &relay_url) {
                eprintln!("buzz-desktop: blank agent relay migration failed: {error}");
            }
        }

        // ── Filesystem side-effect (non-fatal) ────────────────────────────────
        // Persist the *effective* repos_dir (None when the candidate failed
        // validation) for the backend to read at boot, then re-point REPOS to
        // match. Persisting first makes the dotfile authoritative even if the
        // symlink apply fails here (e.g. a non-empty real REPOS): the next boot
        // reads the persisted value and resolves the symlink before any agent can
        // clone into REPOS. A bad candidate persists `None`, so the next boot is
        // clean and agent restore proceeds. Failure of either must NOT fail the
        // command — relay/keys are already applied. Surface symlink errors via
        // `repos-dir-error`.
        if let Some(nest) = nest.as_deref() {
            if let Err(error) = write_persisted_repos_dir(nest, effective_repos_dir.as_deref()) {
                eprintln!("buzz-desktop: persist repos dir failed: {error}");
            }
            // Per-relay map entry for THIS workspace (Phase 4 — REPOS
            // isolation): spawn resolves an agent's repos dir from this map
            // by the agent's pinned relay, so agents stay immune to the REPOS
            // symlink re-point below when a later switch moves it. Must land
            // before the activation task spawned after this closure starts
            // this workspace's agents. A bad candidate clears the entry, same
            // as the dotfile above. Serialized on the store lock: a community
            // relay edit fires rebind_agent_relay (which moves this map's
            // entries under that lock) nearly simultaneously with this apply,
            // and both are read-modify-writes of the same file — unserialized,
            // one side's update would be lost.
            match state.managed_agents_store_lock.lock() {
                Ok(_store_guard) => {
                    if let Err(error) = persist_workspace_repos_dir(
                        nest,
                        &relay_url,
                        effective_repos_dir.as_deref(),
                    ) {
                        eprintln!("buzz-desktop: persist per-relay repos dir failed: {error}");
                    }
                }
                Err(error) => {
                    eprintln!("buzz-desktop: persist per-relay repos dir skipped: {error}");
                }
            }
            if let Err(error) = ensure_repos_symlink(nest, effective_repos_dir.as_deref()) {
                eprintln!("buzz-desktop: repos dir setup failed: {error}");
                let _ = app.emit("repos-dir-error", error);
            }
        }

        try_regenerate_nest(&app);

        Ok::<(), String>(())
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))??;

    // Lazy per-workspace activation: runs on EVERY apply, and self-gates so
    // each workspace's agents start at most once per app session (launch
    // restore is simply the first activation). Nothing is stopped on switch —
    // the previous workspace's agents keep running against their own relay.
    // The one-shot restore flag is consumed only for Share Compute (mesh-llm),
    // which must be up before its dependent agents spawn.
    #[cfg(feature = "mesh-llm")]
    let mesh_restore_pending = restore_app
        .state::<AppState>()
        .managed_agent_restore_pending
        .swap(false, Ordering::AcqRel);
    let app = restore_app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        #[cfg(feature = "mesh-llm")]
        if mesh_restore_pending {
            if let Err(error) = crate::commands::mesh_llm::restore_mesh_sharing(&app, &state).await
            {
                eprintln!("buzz-desktop: failed to restore Share Compute: {error}");
            }
        }
        #[cfg(feature = "mesh-llm")]
        crate::mesh_llm::publish_current_status_once(&app, "workspace apply").await;
        if let Err(error) =
            activate_workspace_agents(&app, &state.shutdown_started, &activation_relay_url).await
        {
            eprintln!("buzz-desktop: failed to activate workspace agents: {error}");
        }
    });

    Ok(())
}

/// Stamp legacy blank-relay agent records with the applied workspace relay.
///
/// Store-lock + load/save wrapper around [`stamp_blank_agent_relay_urls`];
/// returns the number of records stamped.
fn pin_blank_agent_relays(
    app: &AppHandle,
    state: &AppState,
    relay_url: &str,
) -> Result<usize, String> {
    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|e| e.to_string())?;
    let mut records = load_managed_agents(app)?;
    let changed = stamp_blank_agent_relay_urls(&mut records, relay_url);
    if changed > 0 {
        save_managed_agents(app, &records)?;
    }
    Ok(changed)
}

/// Re-pin managed-agent records from `old_relay_url` onto `new_relay_url`.
///
/// Called by the frontend when a community's relay URL is edited. Every agent
/// record is pinned to its home relay, so without this rebind the edited
/// community's agents would stay pinned to — and orphan on — the old URL.
/// Matching is normalized (trailing slash, scheme/host case). Returns the
/// number of records rebound.
#[tauri::command]
pub async fn rebind_agent_relay(
    old_relay_url: String,
    new_relay_url: String,
    app: AppHandle,
) -> Result<usize, String> {
    tokio::task::spawn_blocking(move || {
        if new_relay_url.trim().is_empty() {
            return Err("new relay URL is required".to_string());
        }
        let state = app.state::<AppState>();
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|e| e.to_string())?;
        let mut records = load_managed_agents(&app)?;
        let changed = rebind_agent_relay_urls(&mut records, &old_relay_url, &new_relay_url);
        if changed > 0 {
            save_managed_agents(&app, &records)?;
        }
        // Carry the workspace's per-relay repos_dir along with its agents —
        // spawn resolves BUZZ_REPOS_DIR from the map by pinned relay, so
        // leaving the entry on the old URL would silently drop repos-dir
        // isolation for every rebound agent. Non-fatal: the next
        // apply_workspace of the edited community re-stamps the entry.
        if let Some(nest) = nest_dir() {
            if let Err(error) = rebind_workspace_repos_dir(&nest, &old_relay_url, &new_relay_url) {
                eprintln!("buzz-desktop: rebind per-relay repos dir failed: {error}");
            }
        }
        Ok(changed)
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}
