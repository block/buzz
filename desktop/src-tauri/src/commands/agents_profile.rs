//! Agent kind:0 profile reconciliation — split from `agents.rs` (file-size
//! guard). Owns the reconcile data carrier, the legacy-avatar backfill, and
//! the needs-sync predicate.

use tauri::{AppHandle, Manager};

use crate::app_state::AppState;
use crate::managed_agents::managed_agent_avatar_url;

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProfileReconcileOutcome {
    Reconciled,
    SkippedDisabled,
}

pub(crate) struct ProfileReconcileData {
    pub(crate) private_key_nsec: String,
    pub(crate) name: String,
    pub(crate) relay_url: String,
    /// Exact relay pinned by the caller for the deferred task — captured
    /// while the authorizing workspace/spawn was active (UI start, boot
    /// restore, migration queue). When set it wins unconditionally; a task
    /// left unpinned (no tenant boundary) resolves the current workspace at
    /// execution time. See `resolve_reconcile_relay`.
    pub(crate) target_relay_url: Option<String>,
    /// Expected avatar URL for the published profile. `None` for legacy records
    /// that predate the `avatar_url` field — these will be backfilled from the
    /// relay's existing kind:0 profile on first reconciliation.
    pub(crate) avatar_url: Option<String>,
    pub(crate) auth_tag: Option<String>,
    /// The agent's pubkey (hex). Needed to update the persisted record during
    /// avatar backfill migration.
    pub(crate) pubkey: String,
    /// The agent's command (e.g. "goose"). Used as fallback when no profile
    /// exists on the relay during avatar backfill.
    pub(crate) agent_command: String,
    /// Persona ID if this agent was created from a persona. Used during avatar
    /// backfill to recover the correct avatar from the persona record when the
    /// relay profile has been corrupted.
    pub(crate) persona_id: Option<String>,
    /// Expected kind:0 `about` — the agent's effective public description
    /// (owner-authored when present; see
    /// `managed_agents::record_effective_description`). `None` publishes an
    /// about-less profile.
    pub(crate) about: Option<String>,
}

/// Resolve the avatar to backfill for a legacy agent record (pre-PR-921, no
/// stored `avatar_url`).
///
/// Priority: the persona's avatar wins, because the old reconciliation code
/// could have overwritten the relay's kind:0 `picture` with the command default
/// — making the relay an unreliable source for persona-backed agents. Only fall
/// back to the relay's `picture`, then the command icon, for agents with no
/// persona avatar to recover from.
pub(super) fn resolve_legacy_avatar(
    persona_avatar: Option<String>,
    relay_picture: Option<String>,
    agent_command: &str,
) -> String {
    persona_avatar
        .or(relay_picture)
        .or_else(|| managed_agent_avatar_url(agent_command))
        .unwrap_or_default()
}

/// Resolve the relay a reconciliation task will query and publish on. The
/// pure core of the `reconcile_agent_profile` relay choice, extracted so the
/// pinning contract is unit-testable: a caller-pinned `target_relay_url`
/// (captured while the authorizing workspace was active) wins UNCONDITIONALLY
/// over the execution-time workspace read — otherwise a community switch
/// landing between spawn and execution would retarget the kind:0
/// query/publish to a tenant the caller never authorized. Only an unpinned
/// task (no tenant boundary) resolves the live workspace.
pub(super) fn resolve_reconcile_relay(
    target_relay_url: Option<&str>,
    record_relay_url: &str,
    workspace_relay_at_execution: &str,
) -> String {
    match target_relay_url {
        Some(pinned) => pinned.to_string(),
        None => {
            crate::relay::effective_agent_relay_url(record_relay_url, workspace_relay_at_execution)
        }
    }
}

pub(crate) fn profile_reconcile_data(
    record: &crate::managed_agents::ManagedAgentRecord,
    personas: &[crate::managed_agents::AgentDefinition],
) -> ProfileReconcileData {
    ProfileReconcileData {
        private_key_nsec: record.private_key_nsec.clone(),
        name: record.name.clone(),
        relay_url: record.relay_url.clone(),
        target_relay_url: None,
        avatar_url: record.avatar_url.clone(),
        auth_tag: record.auth_tag.clone(),
        pubkey: record.pubkey.clone(),
        agent_command: crate::managed_agents::record_agent_command(record, personas),
        persona_id: record.persona_id.clone(),
        about: crate::managed_agents::record_effective_description(record, personas),
    }
}

pub(crate) fn load_pending_profile_reconciliations(
    app: &AppHandle,
    workspace_relay: &str,
) -> Result<Vec<(String, ProfileReconcileData)>, String> {
    let state = app.state::<AppState>();
    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;
    let store_path = crate::managed_agents::managed_agents_store_path(app)?;
    let queue_path = crate::migration::profile_reconcile_queue_path(&store_path);
    if !queue_path.exists() {
        return Ok(Vec::new());
    }

    let relay_key = crate::migration::profile_reconcile_relay_key(workspace_relay)?;
    let pending = crate::migration::read_profile_reconcile_queue(&queue_path)?;
    let records = crate::managed_agents::load_managed_agents(app)?;
    let personas = crate::managed_agents::load_personas(app).unwrap_or_default();
    Ok(records
        .iter()
        // A queue write deliberately precedes the migrated agent-store write.
        // If the process dies between them, retain (but do not execute) the
        // stale item until the next boot finishes renaming the record.
        .filter(|record| {
            pending.iter().any(|entry| {
                entry.pubkey == record.pubkey
                    && entry.expected_name == record.name
                    && !entry
                        .reconciled_relays
                        .iter()
                        .any(|relay| relay == &relay_key)
            })
        })
        .map(|record| {
            let mut data = profile_reconcile_data(record, &personas);
            // Pin the relay captured by the caller. Otherwise a fast community
            // switch could make a queued task for A run on B.
            data.target_relay_url = Some(workspace_relay.to_string());
            (record.pubkey.clone(), data)
        })
        .collect())
}

pub(crate) fn mark_profile_reconciled(
    app: &AppHandle,
    pubkey: &str,
    relay_url: &str,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;
    let store_path = crate::managed_agents::managed_agents_store_path(app)?;
    let queue_path = crate::migration::profile_reconcile_queue_path(&store_path);
    if !queue_path.exists() {
        return Ok(());
    }
    let relay_key = crate::migration::profile_reconcile_relay_key(relay_url)?;
    let mut pending = crate::migration::read_profile_reconcile_queue(&queue_path)?;
    crate::migration::record_profile_reconciled(&mut pending, pubkey, relay_key);
    crate::migration::write_profile_reconcile_queue(&queue_path, &pending)
}

/// Reconcile an agent's kind:0 profile on the relay.
///
/// Queries the relay for the agent's existing profile and re-publishes if missing
/// or stale (display_name or picture mismatch). This is fire-and-forget — errors
/// are returned to the caller for logging but never block agent startup.
///
/// For legacy records (pre-PR-921) where `avatar_url` is `None`, this function
/// backfills via `resolve_legacy_avatar` — preferring the persona record's avatar
/// over the relay's `picture`, since the old code may have corrupted the relay
/// profile — and persists the updated record. After backfill, normal
/// reconciliation proceeds.
///
/// Query and publish target the caller-pinned `target_relay_url` when set
/// (UI start, boot restore, migration queue — captured while the authorizing
/// workspace was active); an unpinned task falls back to
/// `effective_agent_relay_url` against the workspace at execution time. This
/// keeps deferred reconciliation from following a community switch it was
/// never authorized for while honoring a deliberate per-agent pin wherever
/// it points.
pub(crate) async fn reconcile_agent_profile(
    state: &AppState,
    app: &AppHandle,
    agent_pubkey: &str,
    data: &ProfileReconcileData,
) -> Result<ProfileReconcileOutcome, String> {
    use crate::relay::{query_agent_profile, sync_managed_agent_profile};

    // Resolved ONCE and used for both the read and the write-back. A pinned
    // `target_relay_url` wins unconditionally — see `resolve_reconcile_relay`.
    let relay_url = resolve_reconcile_relay(
        data.target_relay_url.as_deref(),
        &data.relay_url,
        &relay_ws_url_with_override(state),
    );

    if !state
        .managed_agent_profile_reconcile_enabled()
        .load(std::sync::atomic::Ordering::Acquire)
    {
        return Ok(ProfileReconcileOutcome::SkippedDisabled);
    }

    // Query the relay for the agent's existing kind:0 profile.
    let existing = query_agent_profile(state, &relay_url, agent_pubkey).await?;

    // Resolve the expected avatar — backfilling for legacy records that have no
    // stored avatar_url yet.
    let expected_avatar = match data.avatar_url.as_deref() {
        Some(url) => url.to_string(),
        None => {
            // Legacy record: the relay profile may have been corrupted by the
            // old reconciliation code (it overwrote the persona avatar with the
            // command default), so the persona record is the authoritative source.
            let persona_avatar = data.persona_id.as_ref().and_then(|pid| {
                load_personas(app)
                    .ok()?
                    .into_iter()
                    .find(|p| p.id == *pid)?
                    .avatar_url
            });

            let backfilled = resolve_legacy_avatar(
                persona_avatar,
                existing.as_ref().and_then(|info| info.picture.clone()),
                &data.agent_command,
            );

            // Persist the backfilled avatar so this migration only runs once.
            if !backfilled.is_empty() {
                let _store_guard = state
                    .managed_agents_store_lock
                    .lock()
                    .map_err(|e| e.to_string())?;
                let mut records = load_managed_agents(app)?;
                if let Some(record) = records.iter_mut().find(|r| r.pubkey == data.pubkey) {
                    record.avatar_url = Some(backfilled.clone());
                    save_managed_agents(app, &records)?;
                }
            }

            backfilled
        }
    };

    let expected_avatar = if expected_avatar.is_empty() {
        None
    } else {
        Some(expected_avatar)
    };

    if !profile_needs_sync(
        existing.as_ref(),
        &data.name,
        expected_avatar.as_deref(),
        data.about.as_deref(),
    ) {
        return Ok(ProfileReconcileOutcome::Reconciled);
    }

    let agent_keys = Keys::parse(&data.private_key_nsec)
        .map_err(|e| format!("failed to parse agent keys: {e}"))?;

    if !state
        .managed_agent_profile_reconcile_enabled()
        .load(std::sync::atomic::Ordering::Acquire)
    {
        return Ok(ProfileReconcileOutcome::SkippedDisabled);
    }

    sync_managed_agent_profile(
        state,
        &relay_url,
        &agent_keys,
        &data.name,
        expected_avatar.as_deref(),
        data.about.as_deref(),
        data.auth_tag.as_deref(),
    )
    .await?;
    Ok(ProfileReconcileOutcome::Reconciled)
}

/// Decide whether a published profile is missing or stale relative to the
/// expected name, avatar, and about. A missing profile always needs sync; a
/// present one is stale when the display name, picture, or about diverges.
/// For about, `None` and the empty string are treated as equal so an
/// about-less profile never triggers a pointless republish loop.
pub(super) fn profile_needs_sync(
    existing: Option<&crate::relay::AgentProfileInfo>,
    expected_name: &str,
    expected_avatar: Option<&str>,
    expected_about: Option<&str>,
) -> bool {
    match existing {
        None => true,
        Some(info) => {
            let name_matches = info.display_name.as_deref() == Some(expected_name);
            let picture_matches = info.picture.as_deref() == expected_avatar;
            let about_matches = info.about.as_deref().unwrap_or("") == expected_about.unwrap_or("");
            !name_matches || !picture_matches || !about_matches
        }
    }
}

/// Publish a managed agent's kind:0 profile with the authored public
/// description as `about`, resolving the effective
/// relay URL from the record's stored value. Returns the sync error (if any)
/// rather than failing the caller — profile publish is best-effort in the
/// create and snapshot-import flows that share this helper.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn publish_agent_profile_with_about(
    state: &AppState,
    record_relay_url: &str,
    agent_keys: &nostr::Keys,
    display_name: &str,
    avatar_url: Option<&str>,
    about: Option<&str>,
    auth_tag: Option<&str>,
) -> Option<String> {
    let relay_url = crate::relay::effective_agent_relay_url(
        record_relay_url,
        &relay_ws_url_with_override(state),
    );
    crate::relay::sync_managed_agent_profile(
        state,
        &relay_url,
        agent_keys,
        display_name,
        avatar_url,
        about,
        auth_tag,
    )
    .await
    .err()
}

/// Publish a fresh persona-backed agent's kind:0 profile, computing the
/// effective public `about` from the persona itself.
/// Shared by flows in files at the size ratchet (snapshot import).
pub(crate) async fn publish_persona_profile(
    state: &AppState,
    record_relay_url: &str,
    agent_keys: &nostr::Keys,
    display_name: &str,
    avatar_url: Option<&str>,
    persona: &crate::managed_agents::AgentDefinition,
    auth_tag: Option<&str>,
) -> Option<String> {
    let about = crate::managed_agents::effective_agent_description(persona.description.as_deref());
    publish_agent_profile_with_about(
        state,
        record_relay_url,
        agent_keys,
        display_name,
        avatar_url,
        about.as_deref(),
        auth_tag,
    )
    .await
}

// Async so the blocking body (disk reads/writes + process termination) runs off
// the main UI thread via spawn_blocking. State is re-derived from the owned
// AppHandle inside the closure (`State<'_, _>` is borrowed, MutexGuard is !Send).
