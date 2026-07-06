//! Local-save archive — Tauri commands for archiving relay messages to a
//! per-identity SQLite database in the Buzz nest.
//!
//! # Architecture
//!
//! Two access proof paths, chosen by event kind:
//!
//! **Persistent scopes** (`channel_h`, `referenced_e`, and `owner_p`+44200):
//! the relay is the source of truth. Candidates are grouped and re-queried via
//! a batched authed `/query`; only events the relay returns are inserted.
//! For kind-44200 (agent turn metrics), content is decrypted at ingest and
//! stored as plaintext JSON — fail-closed (decrypt error → drop).
//!
//! **Ephemeral scope** (`owner_p`, kind 24200 observer frames): the relay
//! never stores these, so `/query` cannot verify them. The relay's REQ-time
//! `#p == authed reader` gate is the access control; local per-frame
//! validation (sig/id + kind + p-tag + agent tag + frame=telemetry + author
//! == agent) is applied fail-closed.

mod pipeline;
pub mod store;

use pipeline::{commit_archive, plan_archive, query_buckets};

use nostr::Event;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::app_state::AppState;
use crate::managed_agents::nest_dir;
use crate::relay::{query_relay, relay_ws_url_with_override};

// ── Constants ───────────────────────────────────────────────────────────────

const KIND_AGENT_OBSERVER_FRAME: u16 = 24200;
const KIND_AGENT_TURN_METRIC: u16 = 44200;
const OBSERVER_FRAME_TELEMETRY: &str = "telemetry";

// ── DB helpers ───────────────────────────────────────────────────────────────

fn open_db() -> Result<Connection, String> {
    let nest = nest_dir().ok_or("cannot resolve nest directory for archive")?;
    let db_path = nest.join("archive").join("archive.db");
    store::open_archive_db(&db_path)
}

fn identity_pubkey(state: &AppState) -> Result<String, String> {
    let keys = state.keys.lock().map_err(|e| e.to_string())?;
    Ok(keys.public_key().to_hex())
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

// ── Scope type ───────────────────────────────────────────────────────────────

/// The three supported archive scope discriminants.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScopeType {
    ChannelH,
    OwnerP,
    ReferencedE,
}

impl ScopeType {
    fn as_str(&self) -> &'static str {
        match self {
            ScopeType::ChannelH => "channel_h",
            ScopeType::OwnerP => "owner_p",
            ScopeType::ReferencedE => "referenced_e",
        }
    }

    fn is_ephemeral(&self) -> bool {
        matches!(self, ScopeType::OwnerP)
    }
}

// ── archive_events ───────────────────────────────────────────────────────────

/// One event candidate to archive.
#[derive(Debug, Deserialize)]
pub struct ArchiveCandidate {
    /// Raw Nostr event JSON.
    pub raw_event_json: String,
    /// Which save scope this candidate was matched against. The backend
    /// re-verifies this — it is never trusted blind.
    pub matched_scope: MatchedScope,
}

/// A scope match assertion from the frontend.
#[derive(Debug, Deserialize)]
pub struct MatchedScope {
    pub scope_type: ScopeType,
    pub scope_value: String,
}

/// Result of a batch archive call.
#[derive(Debug, Serialize)]
pub struct ArchiveBatchResult {
    /// Events successfully written to the store (event + scope rows).
    pub persisted: u32,
    /// Events dropped due to access denial or invalid payload (not an error).
    pub dropped: u32,
}

/// Archive a batch of event candidates.
///
/// - Persistent scopes (`channel_h`, `referenced_e`): grouped by scope, then
///   batch-queried against the relay; only returned events are inserted.
/// - Ephemeral scope (`owner_p`): local validation only (no `/query`).
///
/// # Send-safety
///
/// `rusqlite::Connection` is `!Send`. All DB work is bracketed in scoped
/// `{ let conn = open_db()?; ... }` blocks that drop the connection before any
/// `.await`, exactly matching the pattern in `managed_agents/persona_events.rs`.
#[tauri::command]
pub async fn archive_events(
    state: State<'_, AppState>,
    candidates: Vec<ArchiveCandidate>,
) -> Result<ArchiveBatchResult, String> {
    let identity_pk = identity_pubkey(&state)?;
    let relay_url = relay_ws_url_with_override(&state);
    let now = now_secs();

    // ── Phase 1: plan (sync) ─────────────────────────────────────────────────
    // Read subscriptions and build relay filters. Connection dropped before
    // any .await.
    let plan = {
        let conn = open_db()?;
        plan_archive(candidates, &identity_pk, &relay_url, &conn)?
        // conn drops here
    };

    // ── Phase 2: relay queries (async) ───────────────────────────────────────
    // No Connection in scope — future is Send.
    let state_ref: &AppState = &state;
    let bucket_results = query_buckets(plan.buckets, state_ref).await;

    // ── Phase 3: persist (sync) ──────────────────────────────────────────────
    let conn = open_db()?;
    let owner_keys = {
        let keys_guard = state.keys.lock().map_err(|e| e.to_string())?;
        keys_guard.clone()
        // guard drops here
    };
    commit_archive(
        bucket_results,
        plan.ephemeral,
        plan.pre_dropped,
        &identity_pk,
        &relay_url,
        &owner_keys,
        now,
        &conn,
    )
}

/// Validate an ephemeral observer frame (kind 24200) against ALL local rules.
///
/// Rules (verbatim from spec):
/// 1. kind == 24200
/// 2. `#p` contains `identity_pubkey`
/// 3. `agent` tag is present
/// 4. `frame == "telemetry"` (control frames are not archived)
/// 5. event author (pubkey) == agent tag value
/// 6. A matching `owner_p` save-subscription exists AND its `kinds` list
///    includes `24200` (kinds enforcement mirrors the persistent path).
fn validate_ephemeral_frame(
    event: &Event,
    identity_pk: &str,
    scope_value: &str,
    conn: &Connection,
    sub_identity: &str,
    relay_url: &str,
) -> Result<(), String> {
    // 1. Kind guard.
    if event.kind.as_u16() != KIND_AGENT_OBSERVER_FRAME {
        return Err(format!(
            "expected kind {KIND_AGENT_OBSERVER_FRAME}, got {}",
            event.kind.as_u16()
        ));
    }

    // 2. `#p` contains current identity.
    let p_matches = event.tags.iter().any(|t| {
        let s = t.as_slice();
        s.len() >= 2 && s[0] == "p" && s[1] == identity_pk
    });
    if !p_matches {
        return Err("observer frame #p does not match current identity".into());
    }

    // 3. `agent` tag present.
    let agent_value = event
        .tags
        .iter()
        .find_map(|t| {
            let s = t.as_slice();
            if s.len() >= 2 && s[0] == "agent" {
                Some(s[1].clone())
            } else {
                None
            }
        })
        .ok_or_else(|| "observer frame missing `agent` tag".to_string())?;

    // 4. `frame == "telemetry"`.
    let frame_value = event
        .tags
        .iter()
        .find_map(|t| {
            let s = t.as_slice();
            if s.len() >= 2 && s[0] == "frame" {
                Some(s[1].clone())
            } else {
                None
            }
        })
        .ok_or_else(|| "observer frame missing `frame` tag".to_string())?;
    if frame_value != OBSERVER_FRAME_TELEMETRY {
        return Err(format!("expected frame=telemetry, got {frame_value:?}"));
    }

    // 5. Event author == agent tag value.
    if event.pubkey.to_hex() != agent_value {
        return Err("observer frame author does not match agent tag".into());
    }

    // 6. Matching owner_p subscription exists AND kind 24200 is in its kinds list.
    let kinds_json =
        store::get_subscription_kinds(conn, sub_identity, relay_url, "owner_p", scope_value)?
            .ok_or_else(|| format!("no owner_p subscription for scope_value={scope_value:?}"))?;
    let allowed_kinds: Vec<u64> = serde_json::from_str::<Vec<u64>>(&kinds_json).unwrap_or_default();
    if !allowed_kinds.contains(&(KIND_AGENT_OBSERVER_FRAME as u64)) {
        return Err(format!(
            "owner_p subscription kinds {kinds_json:?} does not include {KIND_AGENT_OBSERVER_FRAME}"
        ));
    }

    Ok(())
}

// ── create_save_subscription ─────────────────────────────────────────────────

/// Create a save subscription after running a per-scope access probe.
///
/// Probes:
/// - `channel_h`: verify the current user is a member of the channel (kind 39002
///   `#p` contains our pubkey, or we are the event author / open channel).
/// - `referenced_e`: the referenced event id is currently readable via `/query`.
/// - `owner_p`: restricted to the current identity's own pubkey (v1).
#[tauri::command]
pub async fn create_save_subscription(
    state: State<'_, AppState>,
    scope_type: ScopeType,
    scope_value: String,
    kinds: Vec<u32>,
) -> Result<(), String> {
    let identity_pk = identity_pubkey(&state)?;
    let relay_url = relay_ws_url_with_override(&state);
    let now = now_secs();

    // Reject kinds outside the valid NIP-01 range 0..=65535 — the nostr crate
    // silently truncates larger values via `v as u16`, which would create
    // unmatchable filters.
    for &k in &kinds {
        if k > u32::from(u16::MAX) {
            return Err(format!("kind {k} is out of the valid range 0..=65535"));
        }
    }

    // Per-scope access probe.
    match &scope_type {
        ScopeType::ChannelH => {
            probe_channel_access(&state, &identity_pk, &scope_value).await?;
        }
        ScopeType::ReferencedE => {
            probe_event_readable(&state, &scope_value).await?;
        }
        ScopeType::OwnerP => {
            // v1: only the current identity's own pubkey is allowed.
            if scope_value != identity_pk {
                return Err(format!(
                    "owner_p scope_value must equal current identity pubkey in v1 (got {scope_value:?})"
                ));
            }
        }
    }

    let kinds_json =
        serde_json::to_string(&kinds).map_err(|e| format!("failed to serialize kinds: {e}"))?;

    let conn = open_db()?;
    store::upsert_save_subscription(
        &conn,
        &identity_pk,
        &relay_url,
        scope_type.as_str(),
        &scope_value,
        &kinds_json,
        now,
    )
}

/// Probe: the current user has access to `channel_id` (kind 39002 lists them).
async fn probe_channel_access(
    state: &AppState,
    identity_pk: &str,
    channel_id: &str,
) -> Result<(), String> {
    // Fetch the channel's members event (kind 39002, #d = channel_id).
    let events = query_relay(
        state,
        &[serde_json::json!({
            "kinds": [39002],
            "#d": [channel_id],
            "limit": 1
        })],
    )
    .await?;

    // If no members event exists this could be an open channel — try to read
    // its metadata (kind 39000) as a fallback proof of readability.
    if events.is_empty() {
        let meta = query_relay(
            state,
            &[serde_json::json!({
                "kinds": [39000],
                "#d": [channel_id],
                "limit": 1
            })],
        )
        .await?;
        if meta.is_empty() {
            return Err(format!(
                "channel {channel_id:?} not found or not accessible"
            ));
        }
        // Open channel — readable, access granted.
        return Ok(());
    }

    // Check that the current identity is listed as a member.
    let ev = &events[0];
    let is_member = ev.tags.iter().any(|t| {
        let s = t.as_slice();
        s.len() >= 2 && s[0] == "p" && s[1] == identity_pk
    });
    // Also allow if we are the event author (e.g. the workspace owner who
    // published the members event may not be in the `#p` list themselves).
    let is_author = ev.pubkey.to_hex() == identity_pk;

    if is_member || is_author {
        Ok(())
    } else {
        Err(format!(
            "current identity is not a member of channel {channel_id:?}"
        ))
    }
}

/// Probe: the given event id is currently readable by the current user.
async fn probe_event_readable(state: &AppState, event_id: &str) -> Result<(), String> {
    let events = query_relay(
        state,
        &[serde_json::json!({
            "ids": [event_id],
            "limit": 1
        })],
    )
    .await?;

    if events.is_empty() {
        return Err(format!("event {event_id:?} not found or not accessible"));
    }
    Ok(())
}

// ── merge_save_subscription_kinds ────────────────────────────────────────────

/// Atomically merge `kind` into the `owner_p` save subscription for the
/// current identity + relay.
///
/// Reads the existing `kinds` array, unions in `kind`, and writes back — all
/// inside a single SQLite transaction. This prevents the TOCTOU race where two
/// concurrent seed hooks (observer seed + metric seed) each read an empty row
/// and the last writer clobbers the first.
///
/// Called by both `useObserverArchiveSeed` (kind 24200) and
/// `useAgentMetricArchiveSeed` (kind 44200) instead of the former
/// list → merge-in-TS → create pattern.
#[tauri::command]
pub fn merge_save_subscription_kinds(state: State<'_, AppState>, kind: u32) -> Result<(), String> {
    if kind > u32::from(u16::MAX) {
        return Err(format!("kind {kind} is out of the valid range 0..=65535"));
    }

    let identity_pk = identity_pubkey(&state)?;
    let relay_url = relay_ws_url_with_override(&state);
    let now = now_secs();
    let conn = open_db()?;
    store::merge_owner_p_kinds(&conn, &identity_pk, &relay_url, &identity_pk, kind, now)
}

// ── remove_save_subscription_kind ────────────────────────────────────────────

/// Atomically remove `kind` from the `owner_p` save subscription for the
/// current identity + relay.
///
/// Mirrors `merge_save_subscription_kinds`: reads existing kinds, removes
/// `kind`, then deletes the row if the list becomes empty or updates it
/// otherwise.  Uses `BEGIN IMMEDIATE` to prevent the same snapshot race as the
/// merge path.
///
/// Called by both `useObserverArchiveSeed` / `handleObserverToggle` (kind
/// 24200) and the metric equivalents (kind 44200) on toggle-OFF, replacing the
/// former TS-side read-modify-overwrite + whole-row `deleteSaveSubscription`
/// which would drop the *other* kind if `subs` state was stale.
#[tauri::command]
pub fn remove_save_subscription_kind(state: State<'_, AppState>, kind: u32) -> Result<(), String> {
    if kind > u32::from(u16::MAX) {
        return Err(format!("kind {kind} is out of the valid range 0..=65535"));
    }

    let identity_pk = identity_pubkey(&state)?;
    let relay_url = relay_ws_url_with_override(&state);
    let conn = open_db()?;
    store::remove_owner_p_kind(&conn, &identity_pk, &relay_url, &identity_pk, kind)
}

// ── list_save_subscriptions ──────────────────────────────────────────────────

/// List all save subscriptions for the current identity + relay.
#[tauri::command]
pub fn list_save_subscriptions(
    state: State<'_, AppState>,
) -> Result<Vec<store::SaveSubscription>, String> {
    let identity_pk = identity_pubkey(&state)?;
    let relay_url = relay_ws_url_with_override(&state);
    let conn = open_db()?;
    store::list_save_subscriptions(&conn, &identity_pk, &relay_url)
}

// ── delete_save_subscription ─────────────────────────────────────────────────

/// Delete a save subscription.
///
/// Does NOT purge already-archived event data — retention is decoupled in v1.
/// GC of orphaned event rows happens in P4 purge commands, not here.
#[tauri::command]
pub fn delete_save_subscription(
    state: State<'_, AppState>,
    scope_type: ScopeType,
    scope_value: String,
) -> Result<bool, String> {
    let identity_pk = identity_pubkey(&state)?;
    let relay_url = relay_ws_url_with_override(&state);
    let conn = open_db()?;
    store::delete_save_subscription(
        &conn,
        &identity_pk,
        &relay_url,
        scope_type.as_str(),
        &scope_value,
    )
}

// ── read_archived_events ─────────────────────────────────────────────────────

/// Default page size for `read_archived_events`.
const DEFAULT_READ_LIMIT: i64 = 50;

/// Read a paginated page of archived events for a scope.
///
/// Returns at most `limit` events (default `DEFAULT_READ_LIMIT`) in
/// newest-first order. Pass the compound cursor `(before_created_at,
/// before_id)` — both `Some` — from the last row of the previous page to
/// walk backwards. The predicate mirrors `ORDER BY created_at DESC, id DESC`
/// so same-second siblings are never skipped at a page boundary. Pass
/// `None`/`None` to start at the newest end.
/// A returned page shorter than `limit` signals that the archive is exhausted.
///
/// `kinds` is an optional filter; an empty array means "no kinds matched"
/// (not "all kinds") — callers should pass `null`/`None` when they want all.
///
/// Note: stored row payloads are not uniform — kind 44200 rows store the raw
/// metric payload JSON, while all other kinds store full Nostr Event JSON. A
/// caller doing `Event::from_json` on an unfiltered read must filter by kind
/// first (today's only reader filters `kinds: [24200]`).
#[tauri::command]
pub fn read_archived_events(
    state: State<'_, AppState>,
    scope_type: ScopeType,
    scope_value: String,
    kinds: Option<Vec<i64>>,
    before_created_at: Option<i64>,
    before_id: Option<String>,
    limit: Option<i64>,
) -> Result<Vec<String>, String> {
    let identity_pk = identity_pubkey(&state)?;
    let relay_url = relay_ws_url_with_override(&state);
    let conn = open_db()?;
    store::read_archived_events(
        &conn,
        &identity_pk,
        &relay_url,
        scope_type.as_str(),
        &scope_value,
        kinds.as_deref(),
        before_created_at,
        before_id.as_deref(),
        limit.unwrap_or(DEFAULT_READ_LIMIT),
    )
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "mod_tests.rs"]
mod mod_tests;
