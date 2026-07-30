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
/// Query and publish target the relay returned by `effective_agent_relay_url`
/// for every agent regardless of backend: an explicit per-agent `relay_url`
/// wins, and a blank one falls back to the active workspace relay. This keeps
/// reconciliation following the session's relay for never-pinned agents while
/// honoring a deliberate pin wherever it points.
pub(crate) async fn reconcile_agent_profile(
    state: &AppState,
    app: &AppHandle,
    agent_pubkey: &str,
    data: &ProfileReconcileData,
) -> Result<(), String> {
    use crate::relay::{query_agent_profile, sync_managed_agent_profile};

    // An explicit per-agent relay wins; an empty one falls back to the active
    // workspace relay. Resolved once and used for both the read and write-back.
    let relay_url = crate::relay::effective_agent_relay_url(
        &data.relay_url,
        &relay_ws_url_with_override(state),
    );

    // ── kind:10100 agent directory record ───────────────────────────────────
    // Published *before* both gates below, deliberately.
    //
    // The `managed_agent_profile_reconcile_enabled` guard tracks the
    // `agentManagedProfiles` experiment, which decides whether the desktop or
    // the agent itself owns the agent's kind:0 profile metadata. The directory
    // record is a different concern — discovery, not metadata — and nothing
    // else in the product publishes it. Gating it on that experiment would
    // leave remote agents undiscoverable for exactly the users who opted in.
    //
    // It also precedes the kind:0 staleness check further down: that check asks
    // whether profile metadata diverged, and for a steady-state agent the
    // answer is no, so anything behind it never runs for the agents that most
    // need announcing. The directory record must refresh on every start.
    //
    // Failures are logged, not propagated: a directory publish that cannot
    // reach the relay must not prevent kind:0 reconciliation or block startup.
    if let Err(e) = publish_agent_directory(state, app, &relay_url, agent_pubkey, "online").await {
        eprintln!("buzz-desktop: agent directory publish failed for {agent_pubkey}: {e}");
    }

    if !state
        .managed_agent_profile_reconcile_enabled
        .load(std::sync::atomic::Ordering::Acquire)
    {
        return Ok(());
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

    if !profile_needs_sync(existing.as_ref(), &data.name, expected_avatar.as_deref()) {
        return Ok(());
    }

    let agent_keys = Keys::parse(&data.private_key_nsec)
        .map_err(|e| format!("failed to parse agent keys: {e}"))?;

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

/// Publish the agent's kind:10100 directory record for the current state.
///
/// Reads the agent's channel memberships from the relay, projects the
/// publishable subset of its record, and publishes the result signed by the
/// agent's own key. `status` is the liveness the record should advertise —
/// `"online"` when the agent is starting, `"offline"` when it has stopped.
///
/// Both existing `reconcile_agent_profile` call sites (agent create/start and
/// boot restore) reach this through that function, so no new call site is
/// needed for an agent to appear in the directory.
pub(crate) async fn publish_agent_directory(
    state: &AppState,
    app: &AppHandle,
    relay_url: &str,
    agent_pubkey: &str,
    status: &str,
) -> Result<(), String> {
    let record = load_managed_agents(app)?
        .into_iter()
        .find(|r| r.pubkey == agent_pubkey)
        .ok_or_else(|| format!("no managed agent record for {agent_pubkey}"))?;

    // The record must be signed by the agent, not its owner — see
    // `relay::publish_agent_directory_record`. `load_managed_agents` rehydrates
    // the nsec from the keyring, so this is populated even though it is blanked
    // in the on-disk JSON.
    let agent_keys = Keys::parse(&record.private_key_nsec)
        .map_err(|e| format!("failed to parse agent keys: {e}"))?;

    let api_base = crate::relay::relay_http_base_url(relay_url);
    let auth_tag = record.auth_tag.as_deref();

    // Channels the agent belongs to: kind:39002 membership events tagging it,
    // whose `d` tag carries the channel id. Queried as the agent so the read is
    // scoped to what the agent itself can see.
    let member_events = crate::relay::query_relay_at_with_keys(
        state,
        &api_base,
        &[serde_json::json!({"kinds": [39002], "#p": [agent_pubkey]})],
        &agent_keys,
        auth_tag,
    )
    .await?;
    let channel_ids = channel_ids_from_member_events(&member_events);

    // kind:39000 is addressable — exactly one event per `d` tag — so a limit
    // equal to the id count is both necessary and sufficient. Mirrors
    // `get_channels`.
    let channel_names = if channel_ids.is_empty() {
        Vec::new()
    } else {
        let meta_events = crate::relay::query_relay_at_with_keys(
            state,
            &api_base,
            &[serde_json::json!({
                "kinds": [39000],
                "#d": channel_ids,
                "limit": channel_ids.len(),
            })],
            &agent_keys,
            auth_tag,
        )
        .await?;
        channel_names_from_meta_events(&meta_events, &channel_ids)
    };

    // `channel_add_policy` is required by the relay's side-effect handler for
    // this kind, and that handler writes the value straight into
    // `users.channel_add_policy`. So the agent's existing policy is read back
    // and re-sent verbatim: hardcoding a value here would reset a deliberate
    // `owner_only` or `nobody` to `anyone` on every single agent start.
    let prior_records = crate::relay::query_relay_at_with_keys(
        state,
        &api_base,
        &[serde_json::json!({
            "kinds": [buzz_sdk_pkg::kind::KIND_AGENT_PROFILE],
            "authors": [agent_pubkey],
            "limit": 1,
        })],
        &agent_keys,
        auth_tag,
    )
    .await
    .unwrap_or_default();
    let channel_add_policy = existing_channel_add_policy(&prior_records);

    let content = build_agent_directory_content(
        &record,
        &channel_ids,
        &channel_names,
        status,
        &channel_add_policy,
    );

    crate::relay::publish_agent_directory_record(state, relay_url, &agent_keys, &content, auth_tag)
        .await
}

/// Refresh a stopped agent's directory record so other clients stop offering
/// it in their mention autocomplete.
///
/// Fire-and-forget, mirroring the profile-reconcile pattern on the start path:
/// a directory refresh that cannot reach the relay must never fail the stop the
/// user asked for. kind:10100 is replaceable, so this is a plain re-publish
/// with the status flipped — no tombstone.
pub(crate) fn spawn_offline_directory_update(app: &AppHandle, agent_pubkey: &str) {
    let app = app.clone();
    let agent_pubkey = agent_pubkey.to_string();
    tauri::async_runtime::spawn(async move {
        use tauri::Manager;
        let state = app.state::<AppState>();
        let relay_url = relay_ws_url_with_override(&state);
        if let Err(e) =
            publish_agent_directory(&state, &app, &relay_url, &agent_pubkey, "offline").await
        {
            eprintln!(
                "buzz-desktop: offline directory update failed for agent {agent_pubkey}: {e}"
            );
        }
    });
}

/// The relay's default `channel_add_policy`, matching the schema default on
/// `users.channel_add_policy`. Sending this for an agent that has never
/// published a directory record leaves the stored policy exactly as it was.
const DEFAULT_CHANNEL_ADD_POLICY: &str = "anyone";

/// Recover the `channel_add_policy` an agent already published, if any.
///
/// This must be preserved rather than hardcoded. The relay's side effect for
/// kind:10100 writes whatever the record carries straight into
/// `users.channel_add_policy`, so publishing a fixed value would silently
/// rewrite an operator's deliberate `owner_only` or `nobody` choice every time
/// the agent starts. A missing or malformed prior record falls back to the
/// schema default, which is a no-op against a never-configured agent.
pub(super) fn existing_channel_add_policy(events: &[nostr::Event]) -> String {
    events
        .iter()
        .find_map(|ev| {
            serde_json::from_str::<serde_json::Value>(&ev.content)
                .ok()?
                .get("channel_add_policy")?
                .as_str()
                .map(str::to_string)
        })
        .unwrap_or_else(|| DEFAULT_CHANNEL_ADD_POLICY.to_string())
}

/// Extract channel ids from kind:39002 membership events.
///
/// The channel id lives in the event's `d` tag. An event without one is skipped
/// rather than failing the batch — one malformed membership record must not
/// stop an agent from announcing the channels it *is* in.
pub(super) fn channel_ids_from_member_events(events: &[nostr::Event]) -> Vec<String> {
    let mut ids: Vec<String> = events
        .iter()
        .filter_map(|ev| {
            ev.tags.iter().find_map(|t| {
                let slice = t.as_slice();
                if slice.len() >= 2 && slice[0] == "d" {
                    Some(slice[1].clone())
                } else {
                    None
                }
            })
        })
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

/// Resolve display names for `channel_ids` from kind:39000 metadata events,
/// falling back to the id when a channel has no readable metadata. Positional:
/// the returned vector lines up with `channel_ids`.
///
/// kind:39000 carries the channel id in its `d` tag and the human-readable name
/// in a `name` tag; `content` is empty on these events, so the name must be read
/// from the tags rather than parsed out of the body.
pub(super) fn channel_names_from_meta_events(
    events: &[nostr::Event],
    channel_ids: &[String],
) -> Vec<String> {
    let names: std::collections::HashMap<String, String> = events
        .iter()
        .filter_map(|ev| {
            let mut channel_id = None;
            let mut name = None;
            for tag in ev.tags.iter() {
                let slice = tag.as_slice();
                if slice.len() < 2 {
                    continue;
                }
                match slice[0].as_str() {
                    "d" => channel_id = Some(slice[1].clone()),
                    "name" => name = Some(slice[1].clone()),
                    _ => {}
                }
            }
            Some((channel_id?, name?))
        })
        .collect();

    channel_ids
        .iter()
        .map(|id| names.get(id).cloned().unwrap_or_else(|| id.clone()))
        .collect()
}

/// Build the world-readable content for an agent's kind:10100 directory record.
///
/// The relay serves kind:10100 to any authenticated member, so this is an
/// explicit allowlist of publishable fields rather than a projection of the
/// managed-agent record. Anything absent here is absent deliberately — in
/// particular the agent's nsec, its NIP-OA auth tag, its environment
/// variables, and its runtime/harness configuration must never leave the host.
///
/// `pubkey` is carried for readability only. `agents_from_events` overwrites it
/// with the event author, which is what makes a record's identity derive from
/// its signature rather than from a self-declared field — and is why only the
/// hosting machine, which holds the agent's key, can publish one.
///
/// `capabilities` is currently always empty: a managed agent record declares no
/// capability set, and the client already defaults the field when it is absent.
pub(super) fn build_agent_directory_content(
    record: &ManagedAgentRecord,
    channel_ids: &[String],
    channel_names: &[String],
    status: &str,
    channel_add_policy: &str,
) -> serde_json::Value {
    let name = record
        .display_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(&record.name);

    let agent_type = if record.agent_command.trim().is_empty() {
        "agent"
    } else {
        record.agent_command.trim()
    };

    serde_json::json!({
        "pubkey": record.pubkey,
        "name": name,
        "agent_type": agent_type,
        "channels": channel_names,
        "channel_ids": channel_ids,
        "capabilities": Vec::<String>::new(),
        "status": status,
        "respond_to": record.respond_to.as_str(),
        "respond_to_allowlist": record.respond_to_allowlist,
        "channel_add_policy": channel_add_policy,
    })
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
