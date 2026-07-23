//! Agent kind:0 profile reconciliation — split from `agents.rs` (file-size
//! guard). Owns the reconcile data carrier, the legacy-avatar backfill, and
//! the needs-sync predicate.

use tauri::AppHandle;

use crate::app_state::AppState;
use crate::managed_agents::managed_agent_avatar_url;

use super::*;

pub(crate) struct ProfileReconcileData {
    pub(crate) private_key_nsec: String,
    pub(crate) name: String,
    pub(crate) relay_url: String,
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
}

impl ProfileReconcileData {
    pub(crate) fn from_record(
        record: &crate::managed_agents::ManagedAgentRecord,
        personas: &[crate::managed_agents::AgentDefinition],
    ) -> Self {
        Self {
            private_key_nsec: record.private_key_nsec.clone(),
            name: record.name.clone(),
            relay_url: record.relay_url.clone(),
            avatar_url: record.avatar_url.clone(),
            auth_tag: record.auth_tag.clone(),
            pubkey: record.pubkey.clone(),
            agent_command: crate::managed_agents::record_agent_command(record, personas),
            persona_id: record.persona_id.clone(),
        }
    }
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

/// Reconcile an agent's kind:0 profile on its legacy effective relay.
///
/// UI-start and legacy restore callers still use the record/workspace relay
/// selection. Pair-scoped runtime reconciliation must instead call
/// [`reconcile_agent_profile_at_relay`] with the community relay explicitly.
pub(crate) async fn reconcile_agent_profile(
    state: &AppState,
    app: &AppHandle,
    agent_pubkey: &str,
    data: &ProfileReconcileData,
) -> Result<(), String> {
    let relay_url = crate::relay::effective_agent_relay_url(
        &data.relay_url,
        &relay_ws_url_with_override(state),
    );
    reconcile_agent_profile_at_relay(state, app, agent_pubkey, &relay_url, data, true).await
}

pub(super) fn profile_query_uses_agent_keys(auth_tag: Option<&str>) -> bool {
    auth_tag.is_some()
}

pub(super) fn validate_profile_relay(relay_url: &str) -> Result<String, String> {
    let relay_url = relay_url.trim().trim_end_matches('/');
    let parsed = reqwest::Url::parse(relay_url).map_err(|e| format!("invalid relay URL: {e}"))?;
    if parsed.scheme() != "ws" && parsed.scheme() != "wss" {
        return Err("relay URL must use ws or wss".into());
    }
    if parsed.host().is_none() || !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("invalid relay URL authority".into());
    }
    let scheme_len = relay_url
        .find("://")
        .ok_or_else(|| "relay URL must include a scheme".to_string())?;
    let mut normalized = relay_url.to_string();
    normalized.replace_range(..scheme_len, parsed.scheme());
    Ok(normalized)
}

/// Reconcile an agent's kind:0 profile on one explicit relay.
///
/// Queries that relay for the agent's existing profile and re-publishes if
/// missing or stale (display_name or picture mismatch). Errors are returned to
/// the caller so a multi-relay caller can isolate failures without blocking the
/// remaining relays or agent startup.
///
/// For legacy records (pre-PR-921) where `avatar_url` is `None`, this function
/// resolves the expected avatar via `resolve_legacy_avatar`. Callers on the
/// agent's legacy effective relay may opt into persisting that backfill. Relay
/// fan-out callers must not persist it: concurrent destination-relay reads can
/// disagree, and a relay-local result is not authoritative for the single
/// stored agent record.
pub(crate) async fn reconcile_agent_profile_at_relay(
    state: &AppState,
    app: &AppHandle,
    agent_pubkey: &str,
    relay_url: &str,
    data: &ProfileReconcileData,
    persist_legacy_avatar: bool,
) -> Result<(), String> {
    use crate::relay::{
        query_agent_profile, query_agent_profile_with_keys, sync_managed_agent_profile,
    };

    let relay_url = validate_profile_relay(relay_url)?;

    if !state
        .managed_agent_profile_reconcile_enabled
        .load(std::sync::atomic::Ordering::Acquire)
    {
        return Ok(());
    }

    let agent_keys = if profile_query_uses_agent_keys(data.auth_tag.as_deref()) {
        Some(
            Keys::parse(&data.private_key_nsec)
                .map_err(|e| format!("failed to parse agent keys: {e}"))?,
        )
    } else {
        None
    };

    // NIP-OA agents query as themselves with owner delegation. Pre-NIP-OA
    // records have no auth tag and retain the legacy owner-authenticated read:
    // on closed relays the owner may be admitted while the old agent is not.
    let existing = if let Some(agent_keys) = agent_keys.as_ref() {
        query_agent_profile_with_keys(
            state,
            &relay_url,
            agent_pubkey,
            agent_keys,
            data.auth_tag.as_deref(),
        )
        .await?
    } else {
        query_agent_profile(state, &relay_url, agent_pubkey).await?
    };

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

            // Only the legacy single-relay path may persist this migration.
            // Fan-out tasks can observe different relay-local pictures and must
            // not race to update the one global agent record.
            if persist_legacy_avatar && !backfilled.is_empty() {
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

    if !profile_needs_sync(existing.as_ref(), &data.name, expected_avatar.as_deref()) {
        return Ok(());
    }

    let agent_keys = match agent_keys {
        Some(keys) => keys,
        None => Keys::parse(&data.private_key_nsec)
            .map_err(|e| format!("failed to parse agent keys: {e}"))?,
    };

    if !state
        .managed_agent_profile_reconcile_enabled
        .load(std::sync::atomic::Ordering::Acquire)
    {
        return Ok(());
    }

    sync_managed_agent_profile(
        state,
        &relay_url,
        &agent_keys,
        &data.name,
        expected_avatar.as_deref(),
        data.auth_tag.as_deref(),
    )
    .await
}

/// Decide whether a published profile is missing or stale relative to the
/// expected name and avatar. A missing profile always needs sync; a present
/// one is stale when either the display name or picture diverges.
pub(super) fn profile_needs_sync(
    existing: Option<&crate::relay::AgentProfileInfo>,
    expected_name: &str,
    expected_avatar: Option<&str>,
) -> bool {
    match existing {
        None => true,
        Some(info) => {
            let name_matches = info.display_name.as_deref() == Some(expected_name);
            let picture_matches = info.picture.as_deref() == expected_avatar;
            !name_matches || !picture_matches
        }
    }
}

// Async so the blocking body (disk reads/writes + process termination) runs off
// the main UI thread via spawn_blocking. State is re-derived from the owned
// AppHandle inside the closure (`State<'_, _>` is borrowed, MutexGuard is !Send).
