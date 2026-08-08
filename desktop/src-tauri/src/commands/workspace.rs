use nostr::Keys;
use serde::{Deserialize, Serialize};
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::app_state::AppState;
use crate::managed_agents::{
    effective_repos_dir, ensure_repos_symlink, nest_dir, restore_managed_agents_on_launch,
    try_regenerate_nest, write_persisted_repos_dir,
};
use crate::relay;

/// Adopt the pre-scoping global retention database's pending rows into `scope`.
///
/// Best-effort: a failure is logged and the boot proceeds. The migration's own
/// crash-safety guards make the next launch retry safely, and blocking the
/// workspace apply on it would be worse than a delayed publish.
fn migrate_legacy_retention_into(
    app: &AppHandle,
    scope: &crate::managed_agents::retention::RetentionScope,
) {
    let Ok(base_dir) = crate::managed_agents::managed_agents_base_dir(app) else {
        return;
    };
    match crate::managed_agents::retention::migrate_legacy_retention_db(
        &base_dir,
        &scope.db_path,
        &scope.owner_keys.public_key().to_hex(),
    ) {
        Ok(0) => {}
        Ok(copied) => {
            eprintln!("buzz-desktop: adopted {copied} legacy retained event(s) into this community")
        }
        Err(error) => eprintln!("buzz-desktop: legacy retention migration failed: {error}"),
    }
}

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
    agent_managed_profiles: Option<bool>,
    app: AppHandle,
) -> Result<(), String> {
    let restore_app = app.clone();
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
        // Serialize workspace mutation with launch-time restore. If restore
        // already crossed into spawning, this switch waits for its children to
        // be tracked. If the switch wins, restore's post-lock scope check stops
        // the stale launch.
        let restore_transition = state
            .managed_agent_runtime_transition
            .lock()
            .map_err(|e| e.to_string())?;
        // Serialize the scope transition with inbound private-config handling.
        // Inbound holds this lock from scope resolution through overlay insert,
        // so a patch decrypted for the old workspace cannot land after this clear.
        let _managed_agents_store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|e| e.to_string())?;
        state
            .private_managed_agent_overlay
            .lock()
            .map_err(|e| e.to_string())?
            .clear();
        {
            let mut override_guard = state.relay_url_override.lock().map_err(|e| e.to_string())?;
            *override_guard = Some(relay_url);
        }
        // Reset the Rust-side admission gate when switching workspace/community,
        // matching `resetRateLimitGate()` on the TS side (useCommunityInit.ts:38).
        crate::relay_admission::reset_gate_for_workspace_change();

        if let Some(keys) = parsed_keys {
            let mut keys_guard = state.keys.lock().map_err(|e| e.to_string())?;
            *keys_guard = keys;
        }
        drop(_managed_agents_store_guard);

        // Keep the backend-side reconcile guard aligned with the frontend
        // experiment before launch-time restore can spawn any agents. Missing
        // means the stable behavior: desktop remains authoritative.
        state
            .managed_agent_profile_reconcile_enabled
            .store(!agent_managed_profiles.unwrap_or(false), Ordering::Release);
        drop(restore_transition);

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

    let state = restore_app.state::<AppState>();
    super::agents::provider_access::reconcile_on_workspace_apply(&restore_app, &state).await?;

    // Backfill this exact relay+owner scope only after the workspace has been
    // applied. Running at process boot would target the fallback relay and
    // collapse every community into one pending-event store.
    let event_sync_task =
        match crate::managed_agents::retention::active_retention_scope(&restore_app, &state) {
            Ok(scope) => {
                // Adopt whatever the pre-scoping release left queued in the global
                // retention database BEFORE the scoped reconcile and flush run, so
                // stranded tombstones and archive requests publish on this boot
                // instead of being abandoned by the storage cutover.
                migrate_legacy_retention_into(&restore_app, &scope);
                let owner_pubkey = scope.owner_keys.public_key().to_hex();
                let db_path = scope.db_path.clone();
                let task = crate::event_sync::spawn_event_sync(
                    restore_app.clone(),
                    scope.owner_keys,
                    scope.db_path,
                );
                Some((owner_pubkey, db_path, task))
            }
            Err(error) => {
                eprintln!(
                    "buzz-desktop: scoped event-sync unavailable after workspace apply: {error}"
                );
                None
            }
        };
    crate::event_sync::replace_event_sync_task(event_sync_task)?;

    // Managed-agent restore waits for the frontend's authoritative relay
    // backfill. Starting it here would race both that backfill and the local
    // event-sync task above. Mesh restore remains independent and can proceed
    // while the agent configuration boundary is settling.
    #[cfg(feature = "mesh-llm")]
    {
        let restore_pending = state.managed_agent_restore_pending.load(Ordering::Acquire);
        let app = restore_app.clone();
        tauri::async_runtime::spawn(async move {
            let state = app.state::<AppState>();
            if restore_pending {
                if let Err(error) =
                    crate::commands::mesh_llm::restore_mesh_sharing(&app, &state).await
                {
                    eprintln!("buzz-desktop: failed to restore Share Compute: {error}");
                }
            }
            crate::mesh_llm::publish_current_status_once(&app, "workspace apply").await;
        });
    }

    Ok(())
}

/// Finish launch-time restore only after the selected community's private
/// agent backfill has been fully reconciled.
///
/// The relay and owner are supplied by the subscription that completed. A
/// stale completion from a previous workspace is ignored before it can consume
/// the one-shot restore request.
#[tauri::command]
pub async fn complete_managed_agent_bootstrap(
    owner_pubkey: String,
    arrival_relay_url: String,
    app: AppHandle,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let Some(scope) = crate::managed_agents::retention::arrival_retention_scope(
        &app,
        &state,
        &arrival_relay_url,
    )?
    else {
        return Ok(());
    };
    if !scope
        .owner_keys
        .public_key()
        .to_hex()
        .eq_ignore_ascii_case(owner_pubkey.trim())
    {
        return Ok(());
    }

    let event_sync_task = crate::event_sync::take_event_sync_task(
        &scope.owner_keys.public_key().to_hex(),
        &scope.db_path,
    )?;
    if let Some(task) = event_sync_task {
        task.await
            .map_err(|error| format!("managed-agent event sync failed: {error}"))?;
    }

    let sync_app = app.clone();
    let sync_owner_keys = scope.owner_keys.clone();
    let sync_db_path = scope.db_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        crate::event_sync::run_managed_agent_event_sync(&sync_app, &sync_owner_keys, &sync_db_path)
    })
    .await
    .map_err(|error| format!("managed-agent event sync failed: {error}"))??;

    // A community switch can complete while the blocking reconcile above is
    // running. Revalidate before consuming the process-wide one-shot restore
    // flag; an old community must never launch agents in the newly selected one.
    let Some(current_scope) = crate::managed_agents::retention::arrival_retention_scope(
        &app,
        &state,
        &arrival_relay_url,
    )?
    else {
        return Ok(());
    };
    if current_scope.db_path != scope.db_path
        || current_scope.owner_keys.public_key() != scope.owner_keys.public_key()
    {
        return Ok(());
    }

    if !state.managed_agent_restore_pending.load(Ordering::Acquire) {
        return Ok(());
    }
    let restore_scope = crate::managed_agents::ManagedAgentRestoreScope {
        owner_pubkey: scope.owner_keys.public_key().to_hex(),
        relay_url: scope.relay_url,
        db_path: scope.db_path,
    };
    restore_managed_agents_on_launch(&app, &state.shutdown_started, &restore_scope).await
}
