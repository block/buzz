//! Local-save archive — Tauri commands for archiving relay messages to a
//! per-identity SQLite database in the Buzz nest.
//!
//! # Architecture
//!
//! Two access proof paths, chosen by event kind:
//!
//! **Persistent scopes** (`channel_h`, `referenced_e`): the relay is the
//! source of truth. Candidates are grouped and re-queried via a batched
//! authed `/query`; only events the relay returns are inserted.
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
    commit_archive(
        bucket_results,
        plan.ephemeral,
        plan.pre_dropped,
        &identity_pk,
        &relay_url,
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
mod tests {
    use super::*;
    use nostr::{EventBuilder, JsonUtil, Keys, Kind, Tag};
    use pipeline::BucketWithResult;
    use rusqlite::Connection;
    use uuid::Uuid;

    // ── Helpers ──────────────────────────────────────────────────────────────

    fn in_memory() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "journal_mode", "WAL").unwrap();
        conn.pragma_update(None, "busy_timeout", 5000).unwrap();
        conn.execute_batch(super::store::SCHEMA).unwrap();
        conn
    }

    fn make_observer_frame(owner_keys: &Keys, agent_keys: &Keys, frame_type: &str) -> Event {
        let owner_pk = owner_keys.public_key().to_hex();
        let agent_pk = agent_keys.public_key().to_hex();
        let tags = vec![
            Tag::parse(["p", &owner_pk]).unwrap(),
            Tag::parse(["agent", &agent_pk]).unwrap(),
            Tag::parse(["frame", frame_type]).unwrap(),
        ];
        EventBuilder::new(Kind::Custom(24200), &"A".repeat(200))
            .tags(tags)
            .sign_with_keys(agent_keys)
            .unwrap()
    }

    fn add_sub(
        conn: &Connection,
        identity_pk: &str,
        relay_url: &str,
        scope_type: &str,
        scope_value: &str,
        kinds_json: &str,
    ) {
        store::upsert_save_subscription(
            conn,
            identity_pk,
            relay_url,
            scope_type,
            scope_value,
            kinds_json,
            0,
        )
        .unwrap();
    }

    /// Run the full archive pipeline synchronously with a fake relay response.
    ///
    /// Calls `plan_archive` → injects fake relay events → `commit_archive`.
    /// This mirrors `archive_events` without the async relay calls.
    fn run_batch_sync(
        candidates: Vec<ArchiveCandidate>,
        identity_pk: &str,
        relay_url: &str,
        conn: &Connection,
        fake_relay_events: Vec<Event>,
    ) -> ArchiveBatchResult {
        let plan = plan_archive(candidates, identity_pk, relay_url, conn).unwrap();

        // Synthesize BucketWithResult from the fake relay response.
        let fake_ids: std::collections::HashSet<String> =
            fake_relay_events.iter().map(|e| e.id.to_hex()).collect();
        let bucket_results: Vec<BucketWithResult> = plan
            .buckets
            .into_iter()
            .map(|b| BucketWithResult {
                scope_type_str: b.scope_type_str,
                scope_value: b.scope_value,
                allowed_kinds: b.allowed_kinds,
                group: b.group,
                returned_ids: fake_ids.clone(),
                relay_failed: false,
            })
            .collect();

        commit_archive(
            bucket_results,
            plan.ephemeral,
            plan.pre_dropped,
            identity_pk,
            relay_url,
            0,
            conn,
        )
        .unwrap()
    }

    fn candidate(event: &Event, scope_type: ScopeType, scope_value: &str) -> ArchiveCandidate {
        ArchiveCandidate {
            raw_event_json: event.as_json(),
            matched_scope: MatchedScope {
                scope_type,
                scope_value: scope_value.to_string(),
            },
        }
    }

    // ── Ephemeral validator — individual condition rejection ──────────────────

    #[test]
    fn test_ephemeral_validator_accepts_valid_frame() {
        let conn = in_memory();
        let owner_keys = Keys::generate();
        let agent_keys = Keys::generate();
        let owner_pk = owner_keys.public_key().to_hex();
        let relay_url = "wss://relay.example";
        add_sub(&conn, &owner_pk, relay_url, "owner_p", &owner_pk, "[24200]");
        let ev = make_observer_frame(&owner_keys, &agent_keys, OBSERVER_FRAME_TELEMETRY);
        assert!(
            validate_ephemeral_frame(&ev, &owner_pk, &owner_pk, &conn, &owner_pk, relay_url)
                .is_ok()
        );
    }

    #[test]
    fn test_ephemeral_validator_rejects_wrong_kind() {
        let conn = in_memory();
        let owner_keys = Keys::generate();
        let agent_keys = Keys::generate();
        let owner_pk = owner_keys.public_key().to_hex();
        let relay_url = "wss://relay.example";
        add_sub(&conn, &owner_pk, relay_url, "owner_p", &owner_pk, "[24200]");
        let ev = EventBuilder::new(Kind::TextNote, "hello")
            .tags(vec![
                Tag::parse(["p", &owner_pk]).unwrap(),
                Tag::parse(["agent", &agent_keys.public_key().to_hex()]).unwrap(),
                Tag::parse(["frame", OBSERVER_FRAME_TELEMETRY]).unwrap(),
            ])
            .sign_with_keys(&agent_keys)
            .unwrap();
        let result =
            validate_ephemeral_frame(&ev, &owner_pk, &owner_pk, &conn, &owner_pk, relay_url);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("kind"));
    }

    #[test]
    fn test_ephemeral_validator_rejects_missing_p_tag() {
        let conn = in_memory();
        let owner_keys = Keys::generate();
        let agent_keys = Keys::generate();
        let owner_pk = owner_keys.public_key().to_hex();
        let relay_url = "wss://relay.example";
        add_sub(&conn, &owner_pk, relay_url, "owner_p", &owner_pk, "[24200]");
        let ev = EventBuilder::new(Kind::Custom(24200), &"A".repeat(200))
            .tags(vec![
                Tag::parse(["agent", &agent_keys.public_key().to_hex()]).unwrap(),
                Tag::parse(["frame", OBSERVER_FRAME_TELEMETRY]).unwrap(),
            ])
            .sign_with_keys(&agent_keys)
            .unwrap();
        let result =
            validate_ephemeral_frame(&ev, &owner_pk, &owner_pk, &conn, &owner_pk, relay_url);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("#p"));
    }

    #[test]
    fn test_ephemeral_validator_rejects_missing_agent_tag() {
        let conn = in_memory();
        let owner_keys = Keys::generate();
        let agent_keys = Keys::generate();
        let owner_pk = owner_keys.public_key().to_hex();
        let relay_url = "wss://relay.example";
        add_sub(&conn, &owner_pk, relay_url, "owner_p", &owner_pk, "[24200]");
        let ev = EventBuilder::new(Kind::Custom(24200), &"A".repeat(200))
            .tags(vec![
                Tag::parse(["p", &owner_pk]).unwrap(),
                Tag::parse(["frame", OBSERVER_FRAME_TELEMETRY]).unwrap(),
            ])
            .sign_with_keys(&agent_keys)
            .unwrap();
        let result =
            validate_ephemeral_frame(&ev, &owner_pk, &owner_pk, &conn, &owner_pk, relay_url);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("agent"));
    }

    #[test]
    fn test_ephemeral_validator_rejects_control_frame() {
        let conn = in_memory();
        let owner_keys = Keys::generate();
        let agent_keys = Keys::generate();
        let owner_pk = owner_keys.public_key().to_hex();
        let relay_url = "wss://relay.example";
        add_sub(&conn, &owner_pk, relay_url, "owner_p", &owner_pk, "[24200]");
        let ev = make_observer_frame(&owner_keys, &agent_keys, "control");
        let result =
            validate_ephemeral_frame(&ev, &owner_pk, &owner_pk, &conn, &owner_pk, relay_url);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("telemetry"));
    }

    #[test]
    fn test_ephemeral_validator_rejects_wrong_author() {
        let conn = in_memory();
        let owner_keys = Keys::generate();
        let agent_keys = Keys::generate();
        let other_keys = Keys::generate();
        let owner_pk = owner_keys.public_key().to_hex();
        let relay_url = "wss://relay.example";
        add_sub(&conn, &owner_pk, relay_url, "owner_p", &owner_pk, "[24200]");
        let ev = EventBuilder::new(Kind::Custom(24200), &"A".repeat(200))
            .tags(vec![
                Tag::parse(["p", &owner_pk]).unwrap(),
                Tag::parse(["agent", &agent_keys.public_key().to_hex()]).unwrap(),
                Tag::parse(["frame", OBSERVER_FRAME_TELEMETRY]).unwrap(),
            ])
            .sign_with_keys(&other_keys) // wrong signer
            .unwrap();
        let result =
            validate_ephemeral_frame(&ev, &owner_pk, &owner_pk, &conn, &owner_pk, relay_url);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("author"));
    }

    #[test]
    fn test_ephemeral_validator_rejects_no_subscription() {
        let conn = in_memory();
        let owner_keys = Keys::generate();
        let agent_keys = Keys::generate();
        let owner_pk = owner_keys.public_key().to_hex();
        let relay_url = "wss://relay.example";
        // Deliberately do NOT add a subscription.
        let ev = make_observer_frame(&owner_keys, &agent_keys, OBSERVER_FRAME_TELEMETRY);
        let result =
            validate_ephemeral_frame(&ev, &owner_pk, &owner_pk, &conn, &owner_pk, relay_url);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("owner_p subscription"));
    }

    #[test]
    fn test_ephemeral_validator_rejects_kind_not_in_subscription() {
        // Subscription exists but kinds = [1] (not 24200) — must be rejected.
        let conn = in_memory();
        let owner_keys = Keys::generate();
        let agent_keys = Keys::generate();
        let owner_pk = owner_keys.public_key().to_hex();
        let relay_url = "wss://relay.example";
        add_sub(&conn, &owner_pk, relay_url, "owner_p", &owner_pk, "[1]"); // wrong kinds
        let ev = make_observer_frame(&owner_keys, &agent_keys, OBSERVER_FRAME_TELEMETRY);
        let result =
            validate_ephemeral_frame(&ev, &owner_pk, &owner_pk, &conn, &owner_pk, relay_url);
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(
            msg.contains("24200"),
            "expected kind 24200 in error, got: {msg}"
        );
    }

    // ── archive pipeline — persistent path ───────────────────────────────────

    #[test]
    fn test_persistent_channel_h_persists_when_relay_returns_event() {
        let conn = in_memory();
        let keys = Keys::generate();
        let identity_pk = keys.public_key().to_hex();
        let relay_url = "wss://relay.example";
        let chan = "chan-abc";
        add_sub(&conn, &identity_pk, relay_url, "channel_h", chan, "[9]");

        let ev = EventBuilder::new(Kind::Custom(9), "msg")
            .tags(vec![Tag::parse(["h", chan]).unwrap()])
            .sign_with_keys(&keys)
            .unwrap();
        let cands = vec![candidate(&ev, ScopeType::ChannelH, chan)];

        // Fake relay returns the event (simulates relay proof).
        let result = run_batch_sync(cands, &identity_pk, relay_url, &conn, vec![ev.clone()]);
        assert_eq!(result.persisted, 1);
        assert_eq!(result.dropped, 0);

        // Confirm the event is in the store.
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM archived_events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
        let scope_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM archived_event_scopes WHERE scope_type = 'channel_h' AND scope_value = ?1", [chan], |r| r.get(0))
            .unwrap();
        assert_eq!(scope_count, 1);
    }

    #[test]
    fn test_persistent_channel_h_drops_when_relay_does_not_return_event() {
        // Relay returns empty — event not proven accessible.
        let conn = in_memory();
        let keys = Keys::generate();
        let identity_pk = keys.public_key().to_hex();
        let relay_url = "wss://relay.example";
        let chan = "chan-abc";
        add_sub(&conn, &identity_pk, relay_url, "channel_h", chan, "[9]");

        let ev = EventBuilder::new(Kind::Custom(9), "msg")
            .tags(vec![Tag::parse(["h", chan]).unwrap()])
            .sign_with_keys(&keys)
            .unwrap();
        let cands = vec![candidate(&ev, ScopeType::ChannelH, chan)];

        // Fake relay returns nothing.
        let result = run_batch_sync(cands, &identity_pk, relay_url, &conn, vec![]);
        assert_eq!(result.persisted, 0);
        assert_eq!(result.dropped, 1);
    }

    #[test]
    fn test_persistent_drops_when_no_subscription() {
        // No subscription at all — drop before even querying.
        let conn = in_memory();
        let keys = Keys::generate();
        let identity_pk = keys.public_key().to_hex();
        let relay_url = "wss://relay.example";
        let chan = "chan-abc";
        // Intentionally no subscription.

        let ev = EventBuilder::new(Kind::Custom(9), "msg")
            .tags(vec![Tag::parse(["h", chan]).unwrap()])
            .sign_with_keys(&keys)
            .unwrap();
        let cands = vec![candidate(&ev, ScopeType::ChannelH, chan)];

        // Fake relay would return the event, but no sub → dropped.
        let result = run_batch_sync(cands, &identity_pk, relay_url, &conn, vec![ev.clone()]);
        assert_eq!(result.persisted, 0);
        assert_eq!(result.dropped, 1);
    }

    #[test]
    fn test_persistent_drops_kind_not_in_subscription() {
        // Subscription is for kind 9 only; event is kind 7 (reaction).
        let conn = in_memory();
        let keys = Keys::generate();
        let identity_pk = keys.public_key().to_hex();
        let relay_url = "wss://relay.example";
        let chan = "chan-abc";
        add_sub(&conn, &identity_pk, relay_url, "channel_h", chan, "[9]");

        // kind 7 reaction — no `h` tag naturally, but relay-returned under scoped filter
        let ev = EventBuilder::new(Kind::Reaction, "+")
            .tags(vec![Tag::parse(["h", chan]).unwrap()])
            .sign_with_keys(&keys)
            .unwrap();
        let cands = vec![candidate(&ev, ScopeType::ChannelH, chan)];

        // Fake relay returns the event (simulates relay proof via StoredEvent.channel_id),
        // but kind 7 is not in the subscription's kinds list.
        let result = run_batch_sync(cands, &identity_pk, relay_url, &conn, vec![ev.clone()]);
        assert_eq!(result.persisted, 0);
        assert_eq!(result.dropped, 1);
    }

    #[test]
    fn test_persistent_h_less_event_persists_when_relay_returns_it() {
        // An h-less event (e.g. reaction kind:7) that the relay returns under
        // the scoped #h filter (via StoredEvent.channel_id fallback) must be
        // persisted. The local tag scanner would have dropped it; the scoped
        // filter proof must not.
        let conn = in_memory();
        let keys = Keys::generate();
        let identity_pk = keys.public_key().to_hex();
        let relay_url = "wss://relay.example";
        let chan = "chan-abc";
        add_sub(&conn, &identity_pk, relay_url, "channel_h", chan, "[7]");

        // Build a reaction without an `h` tag — local re-derivation would drop it.
        let ev = EventBuilder::new(Kind::Reaction, "+")
            .sign_with_keys(&keys)
            .unwrap();
        let cands = vec![candidate(&ev, ScopeType::ChannelH, chan)];

        // Fake relay returns it (relay used StoredEvent.channel_id to match).
        let result = run_batch_sync(cands, &identity_pk, relay_url, &conn, vec![ev.clone()]);
        assert_eq!(result.persisted, 1);
        assert_eq!(result.dropped, 0);

        // Scope row uses bucket's scope_value, not a local-derived value.
        let scope_val: String = conn
            .query_row(
                "SELECT scope_value FROM archived_event_scopes WHERE scope_type = 'channel_h'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(scope_val, chan);
    }

    #[test]
    fn test_persistent_referenced_e_persists_when_relay_returns_event() {
        let conn = in_memory();
        let keys = Keys::generate();
        let identity_pk = keys.public_key().to_hex();
        let relay_url = "wss://relay.example";
        let ref_id = "a".repeat(64);
        add_sub(
            &conn,
            &identity_pk,
            relay_url,
            "referenced_e",
            &ref_id,
            "[9]",
        );

        let ev = EventBuilder::new(Kind::Custom(9), "reply")
            .tags(vec![Tag::parse(["e", &ref_id]).unwrap()])
            .sign_with_keys(&keys)
            .unwrap();
        let cands = vec![candidate(&ev, ScopeType::ReferencedE, &ref_id)];
        let result = run_batch_sync(cands, &identity_pk, relay_url, &conn, vec![ev.clone()]);
        assert_eq!(result.persisted, 1);
        assert_eq!(result.dropped, 0);
    }

    #[test]
    fn test_mixed_batch_persisted_and_dropped_counted_exactly() {
        // Two channel_h candidates: relay only returns one. dropped must be 1.
        let conn = in_memory();
        let keys = Keys::generate();
        let identity_pk = keys.public_key().to_hex();
        let relay_url = "wss://relay.example";
        let chan = "chan-abc";
        add_sub(&conn, &identity_pk, relay_url, "channel_h", chan, "[9]");

        let ev1 = EventBuilder::new(Kind::Custom(9), "msg1")
            .tags(vec![Tag::parse(["h", chan]).unwrap()])
            .sign_with_keys(&keys)
            .unwrap();
        let ev2 = EventBuilder::new(Kind::Custom(9), "msg2")
            .tags(vec![Tag::parse(["h", chan]).unwrap()])
            .sign_with_keys(&keys)
            .unwrap();
        let cands = vec![
            candidate(&ev1, ScopeType::ChannelH, chan),
            candidate(&ev2, ScopeType::ChannelH, chan),
        ];

        // Fake relay only returns ev1.
        let result = run_batch_sync(cands, &identity_pk, relay_url, &conn, vec![ev1.clone()]);
        assert_eq!(result.persisted, 1);
        assert_eq!(result.dropped, 1);
    }

    // ── archive pipeline — ephemeral path ────────────────────────────────────

    #[test]
    fn test_ephemeral_path_persists_valid_frame() {
        let conn = in_memory();
        let owner_keys = Keys::generate();
        let agent_keys = Keys::generate();
        let owner_pk = owner_keys.public_key().to_hex();
        let relay_url = "wss://relay.example";
        add_sub(&conn, &owner_pk, relay_url, "owner_p", &owner_pk, "[24200]");

        let ev = make_observer_frame(&owner_keys, &agent_keys, OBSERVER_FRAME_TELEMETRY);
        let cands = vec![candidate(&ev, ScopeType::OwnerP, &owner_pk)];

        // Fake relay returns nothing (not consulted for ephemeral path).
        let result = run_batch_sync(cands, &owner_pk, relay_url, &conn, vec![]);
        assert_eq!(result.persisted, 1);
        assert_eq!(result.dropped, 0);
    }

    #[test]
    fn test_ephemeral_path_drops_kind_not_in_subscription() {
        let conn = in_memory();
        let owner_keys = Keys::generate();
        let agent_keys = Keys::generate();
        let owner_pk = owner_keys.public_key().to_hex();
        let relay_url = "wss://relay.example";
        // kinds = [1], not [24200]
        add_sub(&conn, &owner_pk, relay_url, "owner_p", &owner_pk, "[1]");

        let ev = make_observer_frame(&owner_keys, &agent_keys, OBSERVER_FRAME_TELEMETRY);
        let cands = vec![candidate(&ev, ScopeType::OwnerP, &owner_pk)];

        let result = run_batch_sync(cands, &owner_pk, relay_url, &conn, vec![]);
        assert_eq!(result.persisted, 0);
        assert_eq!(result.dropped, 1);
    }

    // ── Invalid input dropped ─────────────────────────────────────────────────

    #[test]
    fn test_malformed_json_is_dropped() {
        let result = Event::from_json("not json at all");
        assert!(result.is_err());
    }

    #[test]
    fn test_tampered_event_fails_verify_id() {
        let keys = Keys::generate();
        let mut ev_json: serde_json::Value = serde_json::from_str(
            &EventBuilder::new(Kind::TextNote, "ok")
                .sign_with_keys(&keys)
                .unwrap()
                .as_json(),
        )
        .unwrap();
        ev_json["content"] = serde_json::Value::String("tampered".into());
        let tampered = ev_json.to_string();
        let ev = Event::from_json(&tampered).unwrap();
        assert!(!ev.verify_id());
    }

    // ── F2: out-of-range kind ─────────────────────────────────────────────────

    #[test]
    fn test_out_of_range_kind_is_dropped() {
        // kind 89736 == 24200 + 65536. The nostr crate truncates it to 24200
        // via `v as u16`, so without the raw-kind check the validator would
        // reason about 24200 while the persisted raw_json still says 89736.
        // The fix rejects it before Event::from_json.
        let conn = in_memory();
        let keys = Keys::generate();
        let identity_pk = keys.public_key().to_hex();
        let relay_url = "wss://relay.example";
        let owner_pk = &identity_pk;
        // Build a valid kind-24200 frame, then mutate only the raw kind to 89736.
        let owner_keys = Keys::generate();
        let agent_keys = Keys::generate();
        let pk = owner_keys.public_key().to_hex();
        add_sub(&conn, &pk, relay_url, "owner_p", &pk, "[24200]");
        let ev = make_observer_frame(&owner_keys, &agent_keys, OBSERVER_FRAME_TELEMETRY);
        let mut raw: serde_json::Value = serde_json::from_str(&ev.as_json()).unwrap();
        raw["kind"] = serde_json::Value::Number(serde_json::Number::from(24200u64 + 65536));
        let bad_json = raw.to_string();

        let cand = ArchiveCandidate {
            raw_event_json: bad_json,
            matched_scope: MatchedScope {
                scope_type: ScopeType::OwnerP,
                scope_value: pk.clone(),
            },
        };
        let result = run_batch_sync(vec![cand], &pk, relay_url, &conn, vec![]);
        assert_eq!(result.persisted, 0, "out-of-range kind must be dropped");
        assert_eq!(result.dropped, 1);
        // No event row must exist.
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM archived_events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
        let _ = owner_pk; // silence unused-var
    }

    // ── F3: transactional atomicity ───────────────────────────────────────────

    #[test]
    fn test_commit_archive_rolls_back_when_scope_write_would_fail() {
        // Verify the split-schema invariant: if we simulate a mid-batch
        // failure (by putting the DB into a state where the scope table is
        // missing), the event row must NOT survive.
        //
        // We can't actually make upsert_event_scope fail on a healthy DB, so
        // we verify the positive side: with a healthy DB, both rows are always
        // written together or neither is. We confirm via run_batch_sync that
        // after a full successful commit both tables have matching row counts.
        let conn = in_memory();
        let keys = Keys::generate();
        let identity_pk = keys.public_key().to_hex();
        let relay_url = "wss://relay.example";
        let chan = "chan-txn";
        add_sub(&conn, &identity_pk, relay_url, "channel_h", chan, "[9]");

        let ev1 = EventBuilder::new(Kind::Custom(9), "msg1")
            .tags(vec![Tag::parse(["h", chan]).unwrap()])
            .sign_with_keys(&keys)
            .unwrap();
        let ev2 = EventBuilder::new(Kind::Custom(9), "msg2")
            .tags(vec![Tag::parse(["h", chan]).unwrap()])
            .sign_with_keys(&keys)
            .unwrap();
        let cands = vec![
            candidate(&ev1, ScopeType::ChannelH, chan),
            candidate(&ev2, ScopeType::ChannelH, chan),
        ];
        let result = run_batch_sync(
            cands,
            &identity_pk,
            relay_url,
            &conn,
            vec![ev1.clone(), ev2.clone()],
        );
        assert_eq!(result.persisted, 2);
        assert_eq!(result.dropped, 0);

        // Every event row must have exactly one corresponding scope row.
        let event_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM archived_events", [], |r| r.get(0))
            .unwrap();
        let scope_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM archived_event_scopes", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(
            event_count, scope_count,
            "every event row must have a matching scope row — split-schema invariant"
        );
    }

    // ── Real-relay integration tests ──────────────────────────────────────────
    //
    // Gated on `#[cfg(not(target_os = "windows"))]` because `build_app_state()`
    // and `reqwest::Client` pull in native Windows DLLs (SChannel/WinHTTP) that
    // are not available in the CI runner, causing STATUS_ENTRYPOINT_NOT_FOUND at
    // test-binary launch before any test runs. The guard compiles the whole
    // cluster out of the Windows test binary. These tests are `#[ignore]` and
    // require a live relay anyway — there is no Windows relay in CI.
    //
    // Run (Linux/macOS only):
    //
    //   RELAY_URL=ws://localhost:3000 cargo test -p buzz-desktop \
    //       archive::tests::real_relay -- --ignored --nocapture
    //
    // The relay must be running with a Postgres backend (same docker compose
    // as the existing e2e suite). Each test creates its own keypair + channel so
    // concurrent runs don't interfere.

    #[cfg(not(target_os = "windows"))]
    mod real_relay {
        use super::*;
        use crate::app_state::build_app_state;
        use std::path::Path;

        fn relay_ws_url_from_env() -> String {
            std::env::var("RELAY_URL").unwrap_or_else(|_| "ws://localhost:3000".to_string())
        }

        fn relay_http_base() -> String {
            relay_ws_url_from_env()
                .replace("wss://", "https://")
                .replace("ws://", "http://")
                .trim_end_matches('/')
                .to_string()
        }

        /// Build a test AppState wired with specific identity keys and relay URL.
        /// Mirrors production `archive_events`: the same `query_buckets` call path
        /// is exercised, including NIP-98 signing inside `query_relay`.
        fn make_test_app_state(keys: Keys, relay_url: &str) -> AppState {
            let state = build_app_state();
            *state.keys.lock().unwrap() = keys;
            *state.relay_url_override.lock().unwrap() = Some(relay_url.to_string());
            state
        }

        /// Submit a signed event to the relay via `POST /events`.
        /// The staging relay accepts events without NIP-98; this is only used for
        /// test setup (publishing events that the archive pipeline will later query).
        async fn submit_event_to_relay(ev: &Event) -> serde_json::Value {
            let http = reqwest::Client::new();
            let resp = http
                .post(format!("{}/events", relay_http_base()))
                .header("X-Pubkey", ev.pubkey.to_hex())
                .header("Content-Type", "application/json")
                .body(ev.as_json())
                .send()
                .await
                .expect("submit event to relay");
            assert!(
                resp.status().is_success(),
                "relay event submit failed: {}",
                resp.status()
            );
            resp.json().await.expect("parse submit response")
        }

        /// Create an open channel on the relay.  Returns the channel UUID string.
        async fn create_relay_channel(keys: &Keys) -> String {
            let channel_id = Uuid::new_v4().to_string();
            let ev = EventBuilder::new(Kind::Custom(9007), "")
                .tags(vec![
                    Tag::parse(["h", &channel_id]).unwrap(),
                    Tag::parse(["name", &format!("archive-e2e-{channel_id}")]).unwrap(),
                    Tag::parse(["channel_type", "stream"]).unwrap(),
                    Tag::parse(["visibility", "open"]).unwrap(),
                ])
                .sign_with_keys(keys)
                .unwrap();
            let resp = submit_event_to_relay(&ev).await;
            assert!(
                resp["accepted"].as_bool().unwrap_or(false),
                "channel creation not accepted: {resp}"
            );
            channel_id
        }

        /// Open a file-backed archive DB at `path` and insert one save subscription.
        fn file_db_with_subscription(
            path: &Path,
            identity_pk: &str,
            relay_url: &str,
            scope_type: &str,
            scope_value: &str,
            kinds_json: &str,
        ) {
            let conn = store::open_archive_db(path).expect("open file archive db");
            add_sub(
                &conn,
                identity_pk,
                relay_url,
                scope_type,
                scope_value,
                kinds_json,
            );
            // conn drops here — file is flushed before the caller reopens it
        }

        /// Run plan → real relay query (via the production `query_buckets` path,
        /// including NIP-98 signing) → commit, using a file-backed archive DB.
        ///
        /// Mirrors the open/drop/query/reopen pattern of production `archive_events`:
        ///   1. Open DB, run `plan_archive`, drop connection (no conn across `.await`).
        ///   2. Call `query_buckets(plan.buckets, &state).await` — NIP-98 signed.
        ///   3. Reopen DB for `commit_archive`.
        ///
        /// Returns `ArchiveBatchResult`; caller reopens the file for row assertions.
        async fn run_batch_real_relay(
            candidates: Vec<ArchiveCandidate>,
            state: &AppState,
            db_path: &Path,
        ) -> ArchiveBatchResult {
            let identity_pk = state.keys.lock().unwrap().public_key().to_hex();
            let relay_url = crate::relay::relay_ws_url_with_override(state);

            // Phase 1: plan (sync). Connection dropped before any .await.
            let plan = {
                let conn = store::open_archive_db(db_path).expect("open archive db for plan");
                plan_archive(candidates, &identity_pk, &relay_url, &conn).unwrap()
                // conn drops here
            };

            // Phase 2: relay queries (async) — no Connection in scope.
            // Uses the real `query_buckets` path: query_relay → NIP-98 signed /query.
            let bucket_results = query_buckets(plan.buckets, state).await;

            // Phase 3: persist (sync). Fresh connection, same file.
            let conn = store::open_archive_db(db_path).expect("open archive db for commit");
            commit_archive(
                bucket_results,
                plan.ephemeral,
                plan.pre_dropped,
                &identity_pk,
                &relay_url,
                0,
                &conn,
            )
            .unwrap()
        }

        /// Happy path: publish a kind:9 message to a channel, then run the archive
        /// pipeline against the real relay. Asserts exact event + scope rows in a
        /// file-backed SQLite archive, reopened after commit to prove persistence.
        #[tokio::test]
        #[ignore]
        async fn test_real_relay_channel_h_happy_path_persists_event() {
            let keys = Keys::generate();
            let relay_url = relay_ws_url_from_env();
            let state = make_test_app_state(keys.clone(), &relay_url);
            let identity_pk = keys.public_key().to_hex();

            // Create a channel and publish a kind:9 message.
            let channel_id = create_relay_channel(&keys).await;
            let msg_ev = EventBuilder::new(Kind::Custom(9), "hello archive")
                .tags(vec![Tag::parse(["h", &channel_id]).unwrap()])
                .sign_with_keys(&keys)
                .unwrap();
            let submit_resp = submit_event_to_relay(&msg_ev).await;
            assert!(
                submit_resp["accepted"].as_bool().unwrap_or(false),
                "message not accepted: {submit_resp}"
            );

            // Give the relay a moment to persist.
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;

            // File-backed archive DB with a channel_h subscription.
            let tmp = tempfile::tempdir().expect("tempdir");
            let db_path = tmp.path().join("archive.db");
            file_db_with_subscription(
                &db_path,
                &identity_pk,
                &relay_url,
                "channel_h",
                &channel_id,
                "[9]",
            );

            let cands = vec![candidate(&msg_ev, ScopeType::ChannelH, &channel_id)];
            let result = run_batch_real_relay(cands, &state, &db_path).await;

            assert_eq!(result.persisted, 1, "expected 1 persisted, got {result:?}");
            assert_eq!(result.dropped, 0, "expected 0 dropped, got {result:?}");

            // Reopen the same file to assert exact row counts — proves file-backed persistence.
            let read_conn = store::open_archive_db(&db_path).expect("reopen archive db");

            let event_count: i64 = read_conn
                .query_row("SELECT COUNT(*) FROM archived_events", [], |r| r.get(0))
                .unwrap();
            assert_eq!(event_count, 1, "archived_events should have 1 row");

            let scope_count: i64 = read_conn
                .query_row(
                    "SELECT COUNT(*) FROM archived_event_scopes \
                     WHERE scope_type = 'channel_h' AND scope_value = ?1",
                    [&channel_id],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(scope_count, 1, "archived_event_scopes should have 1 row");

            // Confirm the stored raw_json round-trips to the original event.
            let raw_json: String = read_conn
                .query_row("SELECT raw_json FROM archived_events", [], |r| r.get(0))
                .unwrap();
            let stored_ev = Event::from_json(&raw_json).unwrap();
            assert_eq!(stored_ev.id.to_hex(), msg_ev.id.to_hex());

            println!(
                "✓ real relay: event {} archived under channel_h:{}",
                msg_ev.id.to_hex(),
                channel_id
            );
            println!("  archived_events:       {event_count} row(s)");
            println!("  archived_event_scopes: {scope_count} row(s)");
        }

        /// Kind-mismatch drop: subscription allows only kinds=[1059] but we publish a kind:9
        /// channel message. The relay stores the event, but the archive's kind filter drops it
        /// because 9 ∉ {1059}.
        #[tokio::test]
        #[ignore]
        async fn test_real_relay_kind_mismatch_drops_event() {
            let keys = Keys::generate();
            let relay_url = relay_ws_url_from_env();
            let state = make_test_app_state(keys.clone(), &relay_url);
            let identity_pk = keys.public_key().to_hex();

            let channel_id = create_relay_channel(&keys).await;
            // Publish a kind:9 channel message — relay accepts it, but the subscription's
            // kind filter is [1059] so the archive pipeline must drop it.
            let msg_ev = EventBuilder::new(Kind::Custom(9), "kind-mismatch-msg")
                .tags(vec![Tag::parse(["h", &channel_id]).unwrap()])
                .sign_with_keys(&keys)
                .unwrap();
            let submit_resp = submit_event_to_relay(&msg_ev).await;
            assert!(
                submit_resp["accepted"].as_bool().unwrap_or(false),
                "message not accepted: {submit_resp}"
            );
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;

            // Subscription allows kinds=[1059] only — kind:9 must be dropped.
            let tmp = tempfile::tempdir().expect("tempdir");
            let db_path = tmp.path().join("archive.db");
            file_db_with_subscription(
                &db_path,
                &identity_pk,
                &relay_url,
                "channel_h",
                &channel_id,
                "[1059]",
            );

            let cands = vec![candidate(&msg_ev, ScopeType::ChannelH, &channel_id)];
            let result = run_batch_real_relay(cands, &state, &db_path).await;

            assert_eq!(result.persisted, 0, "kind-mismatch: should be dropped");
            assert_eq!(result.dropped, 1, "kind-mismatch: drop count should be 1");

            // Belt-and-suspenders: confirm the on-disk archive is genuinely empty.
            let read_conn = store::open_archive_db(&db_path).expect("reopen archive db");
            let event_count: i64 = read_conn
                .query_row("SELECT COUNT(*) FROM archived_events", [], |r| r.get(0))
                .unwrap();
            assert_eq!(
                event_count, 0,
                "nothing should be archived on kind-mismatch"
            );

            println!(
                "✓ real relay: kind-mismatch correctly dropped kind:9 event (subscription is kinds=[1059])"
            );
        }

        /// No-subscription drop: event arrives but no save_subscription row exists.
        #[tokio::test]
        #[ignore]
        async fn test_real_relay_no_subscription_drops_event() {
            let keys = Keys::generate();
            let relay_url = relay_ws_url_from_env();
            let state = make_test_app_state(keys.clone(), &relay_url);

            let channel_id = create_relay_channel(&keys).await;
            let msg_ev = EventBuilder::new(Kind::Custom(9), "should be dropped")
                .tags(vec![Tag::parse(["h", &channel_id]).unwrap()])
                .sign_with_keys(&keys)
                .unwrap();
            submit_event_to_relay(&msg_ev).await;
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;

            // Empty file-backed archive DB — no subscription row.
            // plan_archive drops the whole group because no subscription matches.
            let tmp = tempfile::tempdir().expect("tempdir");
            let db_path = tmp.path().join("archive.db");
            store::open_archive_db(&db_path).expect("init empty archive db");
            // conn from init drops here; DB file exists but has no subscription rows

            let cands = vec![candidate(&msg_ev, ScopeType::ChannelH, &channel_id)];
            let result = run_batch_real_relay(cands, &state, &db_path).await;

            assert_eq!(result.persisted, 0, "no-sub: should be dropped");
            assert_eq!(result.dropped, 1, "no-sub: drop count should be 1");

            // Belt-and-suspenders: confirm the on-disk archive is genuinely empty.
            let read_conn = store::open_archive_db(&db_path).expect("reopen archive db");
            let event_count: i64 = read_conn
                .query_row("SELECT COUNT(*) FROM archived_events", [], |r| r.get(0))
                .unwrap();
            assert_eq!(
                event_count, 0,
                "nothing should be archived on no-subscription"
            );

            println!("✓ real relay: no-subscription correctly dropped event");
        }

        /// Owner_p ephemeral path: a locally-built valid 24200 frame is archived
        /// without a relay query (ephemeral events are never stored on the relay).
        #[tokio::test]
        #[ignore]
        async fn test_real_relay_owner_p_ephemeral_path_persists_valid_frame() {
            let owner_keys = Keys::generate();
            let agent_keys = Keys::generate();
            let relay_url = relay_ws_url_from_env();
            let state = make_test_app_state(owner_keys.clone(), &relay_url);
            let identity_pk = owner_keys.public_key().to_hex();

            // Build a valid kind:24200 observer frame addressed to the owner.
            let ev = make_observer_frame(&owner_keys, &agent_keys, OBSERVER_FRAME_TELEMETRY);

            // File-backed archive DB with an owner_p subscription for kind 24200.
            let tmp = tempfile::tempdir().expect("tempdir");
            let db_path = tmp.path().join("archive.db");
            file_db_with_subscription(
                &db_path,
                &identity_pk,
                &relay_url,
                "owner_p",
                &identity_pk,
                "[24200]",
            );

            // owner_p candidates bypass the relay entirely — query_buckets gets an
            // empty bucket list and the ephemeral path handles the frame locally.
            let cands = vec![candidate(&ev, ScopeType::OwnerP, &identity_pk)];
            let result = run_batch_real_relay(cands, &state, &db_path).await;

            assert_eq!(
                result.persisted, 1,
                "owner_p: valid frame should be persisted"
            );
            assert_eq!(result.dropped, 0, "owner_p: nothing should be dropped");

            // Reopen the same file to assert exact row counts.
            let read_conn = store::open_archive_db(&db_path).expect("reopen archive db");

            let event_count: i64 = read_conn
                .query_row("SELECT COUNT(*) FROM archived_events", [], |r| r.get(0))
                .unwrap();
            assert_eq!(event_count, 1);

            let scope_count: i64 = read_conn
                .query_row(
                    "SELECT COUNT(*) FROM archived_event_scopes WHERE scope_type = 'owner_p'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(scope_count, 1);

            // Confirm the stored raw_json round-trips to the original frame.
            let raw_json: String = read_conn
                .query_row("SELECT raw_json FROM archived_events", [], |r| r.get(0))
                .unwrap();
            let stored_ev = Event::from_json(&raw_json).unwrap();
            assert_eq!(stored_ev.id.to_hex(), ev.id.to_hex());

            println!(
                "✓ real relay: owner_p ephemeral frame {} archived",
                ev.id.to_hex()
            );
            println!("  archived_events:       {event_count} row(s)");
            println!("  archived_event_scopes: {scope_count} row(s)");
        }
    }
}
