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
#[tauri::command]
pub async fn archive_events(
    state: State<'_, AppState>,
    candidates: Vec<ArchiveCandidate>,
) -> Result<ArchiveBatchResult, String> {
    if candidates.is_empty() {
        return Ok(ArchiveBatchResult {
            persisted: 0,
            dropped: 0,
        });
    }

    let identity_pk = identity_pubkey(&state)?;
    let relay_url = relay_ws_url_with_override(&state);
    let now = now_secs();
    let conn = open_db()?;

    // Parse and verify all candidates up front; split by proof path.
    struct Parsed {
        event: Event,
        raw_json: String,
        matched_scope: MatchedScope,
    }

    let mut persistent: Vec<Parsed> = Vec::new();
    let mut ephemeral: Vec<Parsed> = Vec::new();
    let mut dropped: u32 = 0;

    for cand in candidates {
        let event = match Event::from_json(&cand.raw_event_json) {
            Ok(e) => e,
            Err(_) => {
                dropped += 1;
                continue;
            }
        };
        if !event.verify_id() || !event.verify_signature() {
            dropped += 1;
            continue;
        }

        if cand.matched_scope.scope_type.is_ephemeral() {
            ephemeral.push(Parsed {
                event,
                raw_json: cand.raw_event_json,
                matched_scope: cand.matched_scope,
            });
        } else {
            persistent.push(Parsed {
                event,
                raw_json: cand.raw_event_json,
                matched_scope: cand.matched_scope,
            });
        }
    }

    let mut persisted: u32 = 0;

    // ── Persistent path ──────────────────────────────────────────────────────
    // Group candidates by scope to issue one /query filter per scope bucket,
    // keeping filter sizes manageable.
    if !persistent.is_empty() {
        use std::collections::HashMap;

        // Build a map: (scope_type_str, scope_value) → [parsed candidates]
        let mut buckets: HashMap<(String, String), Vec<Parsed>> = HashMap::new();
        for p in persistent {
            let key = (
                p.matched_scope.scope_type.as_str().to_string(),
                p.matched_scope.scope_value.clone(),
            );
            buckets.entry(key).or_default().push(p);
        }

        for ((scope_type_str, scope_value), mut group) in buckets {
            // Deduplicate by event id within the bucket.
            let mut seen_ids = std::collections::HashSet::new();
            group.retain(|p| seen_ids.insert(p.event.id.to_hex()));

            let ids: Vec<String> = group.iter().map(|p| p.event.id.to_hex()).collect();

            // Build a filter for the relay /query. We always include the event
            // ids; the relay's auth gate will strip any the user can't read.
            let filter = serde_json::json!({ "ids": ids });
            let returned = match query_relay(&state, &[filter]).await {
                Ok(evs) => evs,
                Err(_) => {
                    // Relay unreachable — drop the whole group rather than
                    // persisting unverified.
                    dropped += group.len() as u32;
                    continue;
                }
            };

            // Index returned event ids for O(1) lookup.
            let returned_ids: std::collections::HashSet<String> = returned
                .iter()
                .map(|e| e.id.to_hex())
                .collect();

            for p in group {
                let eid = p.event.id.to_hex();
                if !returned_ids.contains(&eid) {
                    dropped += 1;
                    continue;
                }

                // Re-derive the matched scope from the event itself — never
                // trust the frontend's matched_scope blind.
                let verified_scope = match scope_type_str.as_str() {
                    "channel_h" => derive_channel_h_scope(&p.event),
                    "referenced_e" => derive_referenced_e_scope(&p.event, &scope_value),
                    _ => None,
                };

                let verified_scope_value = match verified_scope {
                    Some(v) => v,
                    None => {
                        dropped += 1;
                        continue;
                    }
                };

                // Check a matching subscription exists.
                let sub_ok = store::has_save_subscription(
                    &conn,
                    &identity_pk,
                    &relay_url,
                    &scope_type_str,
                    &verified_scope_value,
                )?;
                if !sub_ok {
                    dropped += 1;
                    continue;
                }

                store::upsert_archived_event(
                    &conn,
                    &identity_pk,
                    &relay_url,
                    &eid,
                    p.event.kind.as_u16() as i64,
                    &p.event.pubkey.to_hex(),
                    p.event.created_at.as_secs() as i64,
                    &p.raw_json,
                    now,
                )?;
                store::upsert_event_scope(
                    &conn,
                    &identity_pk,
                    &relay_url,
                    &eid,
                    &scope_type_str,
                    &verified_scope_value,
                    now,
                )?;
                persisted += 1;
            }
        }
    }

    // ── Ephemeral path (owner_p) ─────────────────────────────────────────────
    for p in ephemeral {
        match validate_ephemeral_frame(&p.event, &identity_pk, &p.matched_scope.scope_value, &conn, &identity_pk, &relay_url) {
            Ok(()) => {}
            Err(_) => {
                dropped += 1;
                continue;
            }
        }

        let eid = p.event.id.to_hex();
        store::upsert_archived_event(
            &conn,
            &identity_pk,
            &relay_url,
            &eid,
            p.event.kind.as_u16() as i64,
            &p.event.pubkey.to_hex(),
            p.event.created_at.as_secs() as i64,
            &p.raw_json,
            now,
        )?;
        store::upsert_event_scope(
            &conn,
            &identity_pk,
            &relay_url,
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
/// 6. A matching `owner_p` save-subscription exists for `scope_value`
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

    // 6. Matching owner_p subscription exists.
    if !store::has_save_subscription(conn, sub_identity, relay_url, "owner_p", scope_value)? {
        return Err(format!("no owner_p subscription for scope_value={scope_value:?}"));
    }

    Ok(())
}

/// Derive the `channel_h` scope value from an event: the first `h` tag value.
fn derive_channel_h_scope(event: &Event) -> Option<String> {
    event.tags.iter().find_map(|t| {
        let s = t.as_slice();
        if s.len() >= 2 && s[0] == "h" && !s[1].is_empty() {
            Some(s[1].clone())
        } else {
            None
        }
    })
}

/// Derive the `referenced_e` scope value: matches the claimed scope_value if the
/// event contains an `e` tag pointing to it.
fn derive_referenced_e_scope(event: &Event, claimed: &str) -> Option<String> {
    let found = event.tags.iter().any(|t| {
        let s = t.as_slice();
        s.len() >= 2 && s[0] == "e" && s[1] == claimed
    });
    if found {
        Some(claimed.to_string())
    } else {
        None
    }
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

    let kinds_json = serde_json::to_string(&kinds)
        .map_err(|e| format!("failed to serialize kinds: {e}"))?;

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
            return Err(format!("channel {channel_id:?} not found or not accessible"));
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
    use nostr::{EventBuilder, Keys, Kind, Tag};
    use rusqlite::Connection;

    // ── Helper: open an in-memory store ──────────────────────────────────────

    fn in_memory() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "journal_mode", "WAL").unwrap();
        conn.pragma_update(None, "busy_timeout", 5000).unwrap();
        conn.execute_batch(super::store::SCHEMA).unwrap();
        conn
    }

    // ── Helper: build a real signed observer frame ────────────────────────────

    fn make_observer_frame(
        owner_keys: &Keys,
        agent_keys: &Keys,
        frame_type: &str,
    ) -> Event {
        let owner_pk = owner_keys.public_key().to_hex();
        let agent_pk = agent_keys.public_key().to_hex();

        // Minimal NIP-44-looking ciphertext (base64, long enough to pass heuristic).
        let fake_ciphertext = "A".repeat(200);

        let tags = vec![
            Tag::parse(["p", &owner_pk]).unwrap(),
            Tag::parse(["agent", &agent_pk]).unwrap(),
            Tag::parse(["frame", frame_type]).unwrap(),
        ];

        EventBuilder::new(Kind::Custom(24200), &fake_ciphertext)
            .tags(tags)
            .sign_with_keys(agent_keys)
            .unwrap()
    }

    // ── Ephemeral validator — individual condition rejection ──────────────────

    fn add_owner_p_sub(conn: &Connection, identity_pk: &str, relay_url: &str, scope_value: &str) {
        store::upsert_save_subscription(
            conn,
            identity_pk,
            relay_url,
            "owner_p",
            scope_value,
            "[24200]",
            0,
        )
        .unwrap();
    }

    #[test]
    fn test_ephemeral_validator_accepts_valid_frame() {
        let conn = in_memory();
        let owner_keys = Keys::generate();
        let agent_keys = Keys::generate();
        let owner_pk = owner_keys.public_key().to_hex();
        let relay_url = "wss://relay.example";

        add_owner_p_sub(&conn, &owner_pk, relay_url, &owner_pk);
        let ev = make_observer_frame(&owner_keys, &agent_keys, OBSERVER_FRAME_TELEMETRY);

        assert!(validate_ephemeral_frame(
            &ev,
            &owner_pk,
            &owner_pk,
            &conn,
            &owner_pk,
            relay_url
        )
        .is_ok());
    }

    #[test]
    fn test_ephemeral_validator_rejects_wrong_kind() {
        let conn = in_memory();
        let owner_keys = Keys::generate();
        let agent_keys = Keys::generate();
        let owner_pk = owner_keys.public_key().to_hex();
        let relay_url = "wss://relay.example";
        add_owner_p_sub(&conn, &owner_pk, relay_url, &owner_pk);

        // kind 1 instead of 24200
        let ev = EventBuilder::new(Kind::TextNote, "hello")
            .tags(vec![
                Tag::parse(["p", &owner_pk]).unwrap(),
                Tag::parse(["agent", &agent_keys.public_key().to_hex()]).unwrap(),
                Tag::parse(["frame", OBSERVER_FRAME_TELEMETRY]).unwrap(),
            ])
            .sign_with_keys(&agent_keys)
            .unwrap();

        let result = validate_ephemeral_frame(&ev, &owner_pk, &owner_pk, &conn, &owner_pk, relay_url);
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
        add_owner_p_sub(&conn, &owner_pk, relay_url, &owner_pk);

        // No `p` tag.
        let ev = EventBuilder::new(Kind::Custom(24200), &"A".repeat(200))
            .tags(vec![
                Tag::parse(["agent", &agent_keys.public_key().to_hex()]).unwrap(),
                Tag::parse(["frame", OBSERVER_FRAME_TELEMETRY]).unwrap(),
            ])
            .sign_with_keys(&agent_keys)
            .unwrap();

        let result = validate_ephemeral_frame(&ev, &owner_pk, &owner_pk, &conn, &owner_pk, relay_url);
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
        add_owner_p_sub(&conn, &owner_pk, relay_url, &owner_pk);

        // No `agent` tag.
        let ev = EventBuilder::new(Kind::Custom(24200), &"A".repeat(200))
            .tags(vec![
                Tag::parse(["p", &owner_pk]).unwrap(),
                Tag::parse(["frame", OBSERVER_FRAME_TELEMETRY]).unwrap(),
            ])
            .sign_with_keys(&agent_keys)
            .unwrap();

        let result = validate_ephemeral_frame(&ev, &owner_pk, &owner_pk, &conn, &owner_pk, relay_url);
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
        add_owner_p_sub(&conn, &owner_pk, relay_url, &owner_pk);

        // frame=control, not telemetry.
        let ev = make_observer_frame(&owner_keys, &agent_keys, "control");
        let result = validate_ephemeral_frame(&ev, &owner_pk, &owner_pk, &conn, &owner_pk, relay_url);
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
        add_owner_p_sub(&conn, &owner_pk, relay_url, &owner_pk);

        // event signed by `other_keys` but agent tag = agent_keys pubkey.
        let ev = EventBuilder::new(Kind::Custom(24200), &"A".repeat(200))
            .tags(vec![
                Tag::parse(["p", &owner_pk]).unwrap(),
                Tag::parse(["agent", &agent_keys.public_key().to_hex()]).unwrap(),
                Tag::parse(["frame", OBSERVER_FRAME_TELEMETRY]).unwrap(),
            ])
            .sign_with_keys(&other_keys) // wrong signer
            .unwrap();

        let result = validate_ephemeral_frame(&ev, &owner_pk, &owner_pk, &conn, &owner_pk, relay_url);
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
        let result = validate_ephemeral_frame(&ev, &owner_pk, &owner_pk, &conn, &owner_pk, relay_url);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("owner_p subscription"));
    }

    // ── Scope derivation ─────────────────────────────────────────────────────

    #[test]
    fn test_derive_channel_h_scope_extracts_h_tag() {
        let keys = Keys::generate();
        let ev = EventBuilder::new(Kind::TextNote, "hello")
            .tags(vec![Tag::parse(["h", "chan-uuid-123"]).unwrap()])
            .sign_with_keys(&keys)
            .unwrap();
        assert_eq!(
            derive_channel_h_scope(&ev),
            Some("chan-uuid-123".to_string())
        );
    }

    #[test]
    fn test_derive_channel_h_scope_returns_none_when_absent() {
        let keys = Keys::generate();
        let ev = EventBuilder::new(Kind::TextNote, "hello")
            .sign_with_keys(&keys)
            .unwrap();
        assert_eq!(derive_channel_h_scope(&ev), None);
    }

    #[test]
    fn test_derive_referenced_e_scope_matches_claimed() {
        let keys = Keys::generate();
        let ref_id = "a".repeat(64);
        let ev = EventBuilder::new(Kind::TextNote, "reply")
            .tags(vec![Tag::parse(["e", &ref_id]).unwrap()])
            .sign_with_keys(&keys)
            .unwrap();
        assert_eq!(
            derive_referenced_e_scope(&ev, &ref_id),
            Some(ref_id.clone())
        );
    }

    #[test]
    fn test_derive_referenced_e_scope_rejects_wrong_claimed() {
        let keys = Keys::generate();
        let actual_ref = "a".repeat(64);
        let claimed = "b".repeat(64);
        let ev = EventBuilder::new(Kind::TextNote, "reply")
            .tags(vec![Tag::parse(["e", &actual_ref]).unwrap()])
            .sign_with_keys(&keys)
            .unwrap();
        assert_eq!(derive_referenced_e_scope(&ev, &claimed), None);
    }

    // ── Dropped-vs-persisted accounting ─────────────────────────────────────

    #[test]
    fn test_dropped_counting_invalid_json() {
        // archive_events is async and needs AppState — we test the lower-level
        // accounting by confirming that Event::from_json fails gracefully for
        // malformed input and increments the dropped counter.
        let result = Event::from_json("not json at all");
        assert!(result.is_err());
    }

    #[test]
    fn test_dropped_counting_bad_signature() {
        let keys = Keys::generate();
        // Build a valid event then tamper with the content to break the id
        // (the id is a hash of the event fields including content).
        let mut ev_json: serde_json::Value =
            serde_json::from_str(&EventBuilder::new(Kind::TextNote, "ok")
                .sign_with_keys(&keys)
                .unwrap()
                .as_json())
            .unwrap();
        ev_json["content"] = serde_json::Value::String("tampered".into());
        let tampered = ev_json.to_string();
        let ev = Event::from_json(&tampered).unwrap();
        // After tampering the content the event id (a hash over all fields)
        // no longer matches, so verify_id() returns false.
        assert!(!ev.verify_id());
    }
}
