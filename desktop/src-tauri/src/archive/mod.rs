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

pub mod store;

use nostr::{Event, JsonUtil};
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

// ── Archive internals ────────────────────────────────────────────────────────

/// A parsed, sig-verified candidate ready for further processing.
struct Parsed {
    event: Event,
    raw_json: String,
    matched_scope: MatchedScope,
}

/// One scope bucket: a set of candidates that share a scope type+value,
/// with the relay filter already built and the subscription kinds loaded.
struct Bucket {
    scope_type_str: String,
    scope_value: String,
    allowed_kinds: Vec<u64>,
    filter: serde_json::Value,
    group: Vec<Parsed>,
}

/// Output of the sync planning phase.
struct ArchivePlan {
    buckets: Vec<Bucket>,
    ephemeral: Vec<Parsed>,
    /// Events already accounted as dropped during planning (no subscription,
    /// unknown scope type, parse failure, bad sig).
    pre_dropped: u32,
}

/// Phase 1 (sync): parse all candidates, group persistent ones into per-scope
/// buckets, and load the subscription kinds for each bucket.
///
/// Returns an [`ArchivePlan`] with no `&Connection` remaining — safe to hold
/// across `.await`.
fn plan_archive(
    candidates: Vec<ArchiveCandidate>,
    identity_pk: &str,
    relay_url: &str,
    conn: &Connection,
) -> Result<ArchivePlan, String> {
    let mut persistent_raw: Vec<Parsed> = Vec::new();
    let mut ephemeral: Vec<Parsed> = Vec::new();
    let mut pre_dropped: u32 = 0;

    for cand in candidates {
        let event = match Event::from_json(&cand.raw_event_json) {
            Ok(e) => e,
            Err(_) => {
                pre_dropped += 1;
                continue;
            }
        };
        if !event.verify_id() || !event.verify_signature() {
            pre_dropped += 1;
            continue;
        }

        if cand.matched_scope.scope_type.is_ephemeral() {
            ephemeral.push(Parsed {
                event,
                raw_json: cand.raw_event_json,
                matched_scope: cand.matched_scope,
            });
        } else {
            persistent_raw.push(Parsed {
                event,
                raw_json: cand.raw_event_json,
                matched_scope: cand.matched_scope,
            });
        }
    }

    // Group persistent candidates by (scope_type, scope_value).
    use std::collections::HashMap;
    let mut scope_groups: HashMap<(String, String), Vec<Parsed>> = HashMap::new();
    for p in persistent_raw {
        let key = (
            p.matched_scope.scope_type.as_str().to_string(),
            p.matched_scope.scope_value.clone(),
        );
        scope_groups.entry(key).or_default().push(p);
    }

    let mut buckets: Vec<Bucket> = Vec::with_capacity(scope_groups.len());
    for ((scope_type_str, scope_value), mut group) in scope_groups {
        // No subscription → drop the whole group.
        let kinds_json = match store::get_subscription_kinds(
            conn,
            identity_pk,
            relay_url,
            &scope_type_str,
            &scope_value,
        )? {
            Some(k) => k,
            None => {
                pre_dropped += group.len() as u32;
                continue;
            }
        };

        let allowed_kinds: Vec<u64> =
            serde_json::from_str::<Vec<u64>>(&kinds_json).unwrap_or_default();

        // Deduplicate by event id within the bucket.
        let mut seen = std::collections::HashSet::new();
        group.retain(|p| seen.insert(p.event.id.to_hex()));

        let ids: Vec<String> = group.iter().map(|p| p.event.id.to_hex()).collect();

        // Build a *scoped* relay filter: ids + scope tag + kinds.
        let filter = match scope_type_str.as_str() {
            "channel_h" => serde_json::json!({
                "ids":   ids,
                "#h":    [&scope_value],
                "kinds": allowed_kinds,
            }),
            "referenced_e" => serde_json::json!({
                "ids":   ids,
                "#e":    [&scope_value],
                "kinds": allowed_kinds,
            }),
            _ => {
                pre_dropped += group.len() as u32;
                continue;
            }
        };

        buckets.push(Bucket {
            scope_type_str,
            scope_value,
            allowed_kinds,
            filter,
            group,
        });
    }

    Ok(ArchivePlan {
        buckets,
        ephemeral,
        pre_dropped,
    })
}

/// A bucket with the relay's response attached.
struct BucketWithResult {
    scope_type_str: String,
    scope_value: String,
    allowed_kinds: Vec<u64>,
    group: Vec<Parsed>,
    /// Event ids returned by the relay for the scoped filter.
    returned_ids: std::collections::HashSet<String>,
    /// True if the relay query failed (network error); entire group dropped.
    relay_failed: bool,
}

/// Phase 2 (async): fire one relay query per bucket and collect results.
///
/// `state` is `&AppState` — a `Copy` reference — so no `!Send` value is held
/// across `.await`.
async fn query_buckets(buckets: Vec<Bucket>, state: &AppState) -> Vec<BucketWithResult> {
    let mut results: Vec<BucketWithResult> = Vec::with_capacity(buckets.len());
    for bucket in buckets {
        let (returned_ids, relay_failed) = match query_relay(state, &[bucket.filter]).await {
            Ok(evs) => (evs.iter().map(|e| e.id.to_hex()).collect(), false),
            Err(_) => (std::collections::HashSet::new(), true),
        };
        results.push(BucketWithResult {
            scope_type_str: bucket.scope_type_str,
            scope_value: bucket.scope_value,
            allowed_kinds: bucket.allowed_kinds,
            group: bucket.group,
            returned_ids,
            relay_failed,
        });
    }
    results
}

/// Phase 3 (sync): apply relay results and write accepted events to the store.
fn commit_archive(
    bucket_results: Vec<BucketWithResult>,
    ephemeral: Vec<Parsed>,
    pre_dropped: u32,
    identity_pk: &str,
    relay_url: &str,
    now: i64,
    conn: &Connection,
) -> Result<ArchiveBatchResult, String> {
    let mut persisted: u32 = 0;
    let mut dropped: u32 = pre_dropped;

    // ── Persistent path ──────────────────────────────────────────────────────
    for result in bucket_results {
        if result.relay_failed {
            dropped += result.group.len() as u32;
            continue;
        }

        for p in result.group {
            let eid = p.event.id.to_hex();

            // Relay proof: event was returned for the scoped filter.
            if !result.returned_ids.contains(&eid) {
                dropped += 1;
                continue;
            }

            // Kind enforcement: event.kind must be in the subscription's list.
            if !result
                .allowed_kinds
                .contains(&(p.event.kind.as_u16() as u64))
            {
                dropped += 1;
                continue;
            }

            // The relay returning this event for {ids, #h/#e, kinds} IS the
            // proof of scope membership. Use scope_value directly; no local
            // tag re-derivation (which would incorrectly drop h-less events
            // matched via the relay's StoredEvent.channel_id fallback).
            store::upsert_archived_event(
                conn,
                identity_pk,
                relay_url,
                &eid,
                p.event.kind.as_u16() as i64,
                &p.event.pubkey.to_hex(),
                p.event.created_at.as_secs() as i64,
                &p.raw_json,
                now,
            )?;
            store::upsert_event_scope(
                conn,
                identity_pk,
                relay_url,
                &eid,
                &result.scope_type_str,
                &result.scope_value,
                now,
            )?;
            persisted += 1;
        }
    }

    // ── Ephemeral path (owner_p) ─────────────────────────────────────────────
    // Fully local validation — no relay query.
    for p in ephemeral {
        match validate_ephemeral_frame(
            &p.event,
            identity_pk,
            &p.matched_scope.scope_value,
            conn,
            identity_pk,
            relay_url,
        ) {
            Ok(()) => {}
            Err(_) => {
                dropped += 1;
                continue;
            }
        }

        let eid = p.event.id.to_hex();
        store::upsert_archived_event(
            conn,
            identity_pk,
            relay_url,
            &eid,
            p.event.kind.as_u16() as i64,
            &p.event.pubkey.to_hex(),
            p.event.created_at.as_secs() as i64,
            &p.raw_json,
            now,
        )?;
        store::upsert_event_scope(
            conn,
            identity_pk,
            relay_url,
            &eid,
            "owner_p",
            &p.matched_scope.scope_value,
            now,
        )?;
        persisted += 1;
    }

    Ok(ArchiveBatchResult { persisted, dropped })
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

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, JsonUtil, Keys, Kind, Tag};
    use rusqlite::Connection;

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
}
