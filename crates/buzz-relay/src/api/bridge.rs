//! Nostr HTTP bridge — POST /events, /query, /count with NIP-98 auth.
//!
//! These endpoints provide HTTP access to the relay's Nostr protocol,
//! authenticated via NIP-98 signed events.

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::Json,
};
use base64::Engine;
use serde_json::Value;

use crate::handlers::ingest::{IngestAuth, IngestError};
use crate::state::AppState;

use super::{api_error, internal_error, not_found};

// ── NIP-98 verification ──────────────────────────────────────────────────────

/// Verify bridge auth: NIP-98 (production) or X-Pubkey (dev mode).
///
/// Returns the authenticated public key and an event ID for replay detection.
/// For X-Pubkey dev mode, the event ID is a zero hash (no replay concern).
fn verify_bridge_auth(
    headers: &HeaderMap,
    method: &str,
    url: &str,
    body: Option<&[u8]>,
    require_auth_token: bool,
) -> Result<(nostr::PublicKey, [u8; 32]), (StatusCode, Json<Value>)> {
    // Try NIP-98 first (Authorization: Nostr <base64>)
    if let Some(auth_str) = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Nostr "))
    {
        let event_json = {
            use base64::engine::general_purpose::STANDARD as BASE64;
            let bytes = BASE64
                .decode(auth_str)
                .map_err(|_| api_error(StatusCode::UNAUTHORIZED, "invalid base64 in Nostr auth"))?;
            String::from_utf8(bytes)
                .map_err(|_| api_error(StatusCode::UNAUTHORIZED, "invalid UTF-8 in Nostr auth"))?
        };

        let event: nostr::Event = serde_json::from_str(&event_json)
            .map_err(|_| api_error(StatusCode::UNAUTHORIZED, "invalid NIP-98 event JSON"))?;
        let event_id_bytes = event.id.to_bytes();

        let pubkey = buzz_auth::verify_nip98_event(&event_json, url, method, body)
            .map_err(|e| api_error(StatusCode::UNAUTHORIZED, &format!("NIP-98: {e}")))?;

        return Ok((pubkey, event_id_bytes));
    }

    // Dev-mode fallback: X-Pubkey header (only when require_auth_token is false)
    if !require_auth_token {
        if let Some(hex_val) = headers.get("x-pubkey").and_then(|v| v.to_str().ok()) {
            let pubkey = nostr::PublicKey::from_hex(hex_val)
                .map_err(|_| api_error(StatusCode::UNAUTHORIZED, "invalid X-Pubkey hex"))?;
            // Zero event ID — no replay detection needed for dev mode
            return Ok((pubkey, [0u8; 32]));
        }
    }

    Err(api_error(StatusCode::UNAUTHORIZED, "missing Nostr auth"))
}

/// Check NIP-98 replay and record the event ID atomically.
///
/// Uses moka's `entry` API for atomic insert-if-absent — no race window
/// between "check if seen" and "mark as seen".
fn check_nip98_replay(
    state: &AppState,
    event_id_bytes: [u8; 32],
) -> Result<(), (StatusCode, Json<Value>)> {
    // Skip replay detection for dev-mode X-Pubkey auth (zero hash).
    if event_id_bytes == [0u8; 32] {
        return Ok(());
    }
    // Atomic: get_with inserts the value if absent and returns it.
    // If the entry already existed, this is a replay.
    let entry = state.nip98_seen.entry(event_id_bytes);
    let result = entry.or_insert(());
    if !result.is_fresh() {
        return Err(api_error(
            StatusCode::UNAUTHORIZED,
            "NIP-98: replay detected",
        ));
    }
    Ok(())
}

/// Reconstruct the canonical URL for NIP-98 verification from the relay config.
fn canonical_url(relay_url: &str, path: &str) -> String {
    let base = relay_url
        .trim()
        .trim_end_matches('/')
        .replace("wss://", "https://")
        .replace("ws://", "http://");
    format!("{base}{path}")
}

// ── Channel access helpers ───────────────────────────────────────────────────

/// Extract a channel UUID from a single filter's `#h` tag.
fn extract_channel_from_filter(filter: &nostr::Filter) -> Option<uuid::Uuid> {
    let h_tag = nostr::SingleLetterTag::lowercase(nostr::Alphabet::H);
    filter.generic_tags.get(&h_tag).and_then(|vs| {
        if vs.len() == 1 {
            vs.iter().next()?.parse::<uuid::Uuid>().ok()
        } else {
            None
        }
    })
}

// ── Custom filter field extractors ──────────────────────────────────────────
//
// The CLI injects extension fields (before_id, depth_limit, feed_types) into
// Nostr filter JSON. nostr::Filter silently drops unknown fields during
// deserialization, so we extract them from the raw JSON Value first.

const BRIDGE_FEED_MAX_LIMIT: i64 = 100;
const BRIDGE_THREAD_MAX_LIMIT: u32 = 500;

fn extract_before_id(raw: &Value) -> Option<Vec<u8>> {
    let hex_str = raw.get("before_id")?.as_str()?;
    if hex_str.len() == 64 {
        hex::decode(hex_str).ok()
    } else {
        None
    }
}

fn extract_depth_limit(raw: &Value) -> Option<u32> {
    raw.get("depth_limit")?
        .as_u64()
        .and_then(|n| u32::try_from(n).ok())
}

fn extract_feed_types(raw: &Value) -> Option<Vec<String>> {
    let arr = raw.get("feed_types")?.as_array()?;
    let types: Vec<String> = arr
        .iter()
        .filter_map(|v| v.as_str().map(String::from))
        .collect();
    if types.is_empty() {
        None
    } else {
        Some(types)
    }
}

fn event_in_accessible_channel(se: &buzz_core::StoredEvent, accessible: &[uuid::Uuid]) -> bool {
    match se.channel_id {
        Some(ch_id) => accessible.contains(&ch_id),
        None => true,
    }
}

// ── POST /events ─────────────────────────────────────────────────────────────

/// Submit a signed Nostr event via HTTP bridge (NIP-98 auth).
pub async fn submit_event(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let url = canonical_url(&state.config.relay_url, "/events");
    let (pubkey, event_id_bytes) = verify_bridge_auth(
        &headers,
        "POST",
        &url,
        Some(&body),
        state.config.require_auth_token,
    )?;
    check_nip98_replay(&state, event_id_bytes)?;
    let pubkey_bytes = pubkey.to_bytes().to_vec();

    // Enforce relay membership (with NIP-OA fallback via x-auth-tag header).
    let auth_tag = headers.get("x-auth-tag").and_then(|v| v.to_str().ok());
    super::relay_members::enforce_relay_membership(&state, &pubkey_bytes, auth_tag).await?;

    let event: nostr::Event = serde_json::from_slice(&body)
        .map_err(|e| api_error(StatusCode::BAD_REQUEST, &format!("invalid event JSON: {e}")))?;

    // Mesh signaling kinds (24620 status report, 24621 connect request) are
    // ephemeral and deliberately absent from ingest_event's per-kind allowlist.
    // The desktop's Rust coordinator publishes them via this bridge, so route
    // them to the mesh handlers — the HTTP twin of the WS door's special-casing
    // in handlers::event. Membership was enforced above; the handlers re-check
    // it fail-closed.
    let kind_u32 = buzz_core::kind::event_kind_u32(&event);
    if kind_u32 == buzz_core::kind::KIND_MESH_STATUS_REPORT
        || kind_u32 == buzz_core::kind::KIND_MESH_CONNECT_REQUEST
    {
        let event_id = event.id.to_hex();
        return match crate::handlers::mesh_signaling::handle_mesh_event_http(
            &state, &pubkey, &event,
        )
        .await
        {
            Ok(()) => Ok(Json(serde_json::json!({
                "event_id": event_id,
                "accepted": true,
                "message": "",
            }))),
            Err(msg) => Err(api_error(StatusCode::BAD_REQUEST, &msg)),
        };
    }

    let auth = IngestAuth::Http {
        pubkey,
        scopes: buzz_auth::Scope::all_known(), // Pure Nostr: full scopes, channel access via membership
        auth_method: crate::handlers::ingest::HttpAuthMethod::Nip98,
    };

    match crate::handlers::ingest::ingest_event(&state, event, auth).await {
        Ok(result) => Ok(Json(serde_json::json!({
            "event_id": result.event_id,
            "accepted": result.accepted,
            "message": result.message,
        }))),
        Err(e) => match e {
            IngestError::Rejected(msg) => Err(api_error(StatusCode::BAD_REQUEST, &msg)),
            IngestError::AuthFailed(msg) => Err(api_error(StatusCode::FORBIDDEN, &msg)),
            IngestError::Internal(msg) => Err(internal_error(&msg)),
        },
    }
}

// ── POST /query ──────────────────────────────────────────────────────────────

/// Query events via HTTP bridge (NIP-98 auth). Returns JSON array of events.
///
/// Enforces channel access: results are filtered to channels the user can access.
pub async fn query_events(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let url = canonical_url(&state.config.relay_url, "/query");
    let (pubkey, event_id_bytes) = verify_bridge_auth(
        &headers,
        "POST",
        &url,
        Some(&body),
        state.config.require_auth_token,
    )?;
    check_nip98_replay(&state, event_id_bytes)?;
    let pubkey_bytes = pubkey.to_bytes().to_vec();

    let auth_tag = headers.get("x-auth-tag").and_then(|v| v.to_str().ok());
    super::relay_members::enforce_relay_membership(&state, &pubkey_bytes, auth_tag).await?;

    // Two-pass parse: preserve raw JSON for custom extension fields (before_id,
    // depth_limit, feed_types) that nostr::Filter silently drops.
    let raw_filters: Vec<Value> = serde_json::from_slice(&body)
        .map_err(|e| api_error(StatusCode::BAD_REQUEST, &format!("invalid filters: {e}")))?;
    let filters: Vec<nostr::Filter> = raw_filters
        .iter()
        .map(|v| serde_json::from_value(v.clone()))
        .collect::<Result<_, _>>()
        .map_err(|e| api_error(StatusCode::BAD_REQUEST, &format!("invalid filters: {e}")))?;

    // P-gated kinds (gift wraps, member notifications, observer frames) require
    // the caller's own pubkey in the #p tag — same enforcement as WS REQ handler.
    let authed_pubkey_hex = pubkey.to_hex();
    if !crate::handlers::req::p_gated_filters_authorized(&filters, &authed_pubkey_hex) {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "restricted: p-gated kinds require #p tag matching your pubkey",
        ));
    }
    if !crate::handlers::req::engram_filters_authorized(&filters, &authed_pubkey_hex) {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "restricted: agent-engram reads require authors=[self] or #p=[self]",
        ));
    }
    if !crate::handlers::req::author_only_filters_authorized(&filters, &authed_pubkey_hex) {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "restricted: author-only kinds require authors=[self]",
        ));
    }

    // Get channels this user can access — same enforcement as WS REQ handler.
    let accessible_channels = state
        .get_accessible_channel_ids_cached(&pubkey_bytes)
        .await
        .map_err(|e| internal_error(&format!("channel access lookup: {e}")))?;

    // ── NIP-50 search: route to Typesense if any filter has a `search` field ──
    if filters.iter().any(|f| f.search.is_some()) {
        return handle_bridge_search(
            &state,
            &filters,
            &accessible_channels,
            &authed_pubkey_hex,
            &pubkey_bytes,
        )
        .await;
    }

    // ── Presence: synthesize kind:20001 from Redis (ephemeral, never in DB) ──
    if let Some(presence_events) = synthesize_presence(&state, &filters).await {
        return Ok(Json(Value::Array(presence_events)));
    }

    let mut events: Vec<Value> = Vec::new();
    let mut handled: std::collections::HashSet<usize> = std::collections::HashSet::new();

    // ── feed_types: route to dedicated feed query functions ──
    for (idx, (raw, filter)) in raw_filters.iter().zip(filters.iter()).enumerate() {
        let feed_types = match extract_feed_types(raw) {
            Some(t) => t,
            None => continue,
        };

        let limit = filter
            .limit
            .map(|l| (l as i64).min(BRIDGE_FEED_MAX_LIMIT))
            .unwrap_or(20);
        let since = filter
            .since
            .and_then(|s| chrono::DateTime::from_timestamp(s.as_secs() as i64, 0));

        let mut seen_types = std::collections::HashSet::new();
        let mut seen = std::collections::HashSet::new();
        let mut feed_count = 0i64;
        for feed_type in &feed_types {
            let canonical = if feed_type == "agent_activity" {
                "activity"
            } else {
                feed_type.as_str()
            };
            if !seen_types.insert(canonical) {
                continue;
            }
            if feed_count >= limit {
                break;
            }
            let remaining = limit - feed_count;
            let type_events = match canonical {
                "mentions" => state
                    .db
                    .query_feed_mentions(&pubkey_bytes, &accessible_channels, since, remaining)
                    .await
                    .map_err(|e| internal_error(&format!("feed mentions error: {e}")))?,
                "needs_action" => state
                    .db
                    .query_feed_needs_action(&pubkey_bytes, &accessible_channels, since, remaining)
                    .await
                    .map_err(|e| internal_error(&format!("feed needs_action error: {e}")))?,
                "activity" => state
                    .db
                    .query_feed_activity(&accessible_channels, since, remaining)
                    .await
                    .map_err(|e| internal_error(&format!("feed activity error: {e}")))?,
                _ => continue,
            };
            for se in type_events {
                if !seen.insert(se.event.id) {
                    continue;
                }
                if !event_in_accessible_channel(&se, &accessible_channels) {
                    continue;
                }
                if let Ok(v) = serde_json::to_value(&se.event) {
                    events.push(v);
                    feed_count += 1;
                }
            }
        }
        handled.insert(idx);
    }

    // ── depth_limit: route thread queries to get_thread_replies ──
    let e_tag_key = nostr::SingleLetterTag::lowercase(nostr::Alphabet::E);
    for (idx, (raw, filter)) in raw_filters.iter().zip(filters.iter()).enumerate() {
        if handled.contains(&idx) {
            continue;
        }
        let depth = match extract_depth_limit(raw) {
            Some(d) => d,
            None => continue,
        };
        let e_values = match filter.generic_tags.get(&e_tag_key) {
            Some(vs) if vs.len() == 1 => vs,
            _ => continue,
        };
        let root_hex = match e_values.iter().next() {
            Some(h) => h,
            None => continue,
        };
        let root_bytes = match hex::decode(root_hex) {
            Ok(b) if b.len() == 32 => b,
            _ => continue,
        };

        if let Some(ch_id) = extract_channel_from_filter(filter) {
            if !accessible_channels.contains(&ch_id) {
                handled.insert(idx);
                continue;
            }
        }

        let limit = filter
            .limit
            .unwrap_or(100)
            .min(BRIDGE_THREAD_MAX_LIMIT as usize) as u32;
        let thread_replies = state
            .db
            .get_thread_replies(&root_bytes, Some(depth), limit, None)
            .await
            .map_err(|e| internal_error(&format!("thread query error: {e}")))?;

        if !thread_replies.is_empty() {
            let reply_ids: Vec<Vec<u8>> =
                thread_replies.iter().map(|r| r.event_id.clone()).collect();
            let id_refs: Vec<&[u8]> = reply_ids.iter().map(|b| b.as_slice()).collect();
            let stored = state
                .db
                .get_events_by_ids(&id_refs)
                .await
                .map_err(|e| internal_error(&format!("thread fetch error: {e}")))?;
            for se in stored {
                if !event_in_accessible_channel(&se, &accessible_channels) {
                    continue;
                }
                if let Ok(v) = serde_json::to_value(&se.event) {
                    events.push(v);
                }
            }
        }
        handled.insert(idx);
    }

    // ── Standard query path (with before_id injection) ──
    for (idx, (raw, filter)) in raw_filters.iter().zip(filters.iter()).enumerate() {
        if handled.contains(&idx) {
            continue;
        }

        if let Some(ch_id) = extract_channel_from_filter(filter) {
            if !accessible_channels.contains(&ch_id) {
                continue;
            }
        }

        let mut query =
            crate::handlers::req::build_event_query_from_filter(filter, &pubkey_bytes, &state)
                .await;

        if let Some(bid) = extract_before_id(raw) {
            if query.until.is_none() {
                return Err(api_error(
                    StatusCode::BAD_REQUEST,
                    "before_id requires until to be set",
                ));
            }
            query.before_id = Some(bid);
        }

        match state.db.query_events(&query).await {
            Ok(stored_events) => {
                for se in stored_events {
                    if !event_in_accessible_channel(&se, &accessible_channels) {
                        continue;
                    }
                    if !buzz_core::filter::filters_match(std::slice::from_ref(filter), &se) {
                        continue;
                    }
                    // Result-level read auth: never hand a viewer-private snapshot
                    // (kind:30622) to anyone but its owner, even via kindless `ids`.
                    if !buzz_core::filter::reader_authorized_for_event(
                        &se.event,
                        &authed_pubkey_hex,
                    ) {
                        continue;
                    }
                    if crate::handlers::req::is_author_only_event(&se.event, &pubkey_bytes) {
                        continue;
                    }
                    if let Ok(v) = serde_json::to_value(&se.event) {
                        events.push(v);
                    }
                }
            }
            Err(e) => {
                return Err(internal_error(&format!("query error: {e}")));
            }
        }
    }

    Ok(Json(Value::Array(events)))
}

// ── POST /count ──────────────────────────────────────────────────────────────

/// Count events via HTTP bridge (NIP-98 auth). Returns `{"count": N}`.
///
/// Enforces channel access: only counts events in channels the user can access.
/// For filters without a `#h` tag, falls back to per-event counting with access checks.
pub async fn count_events(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let url = canonical_url(&state.config.relay_url, "/count");
    let (pubkey, event_id_bytes) = verify_bridge_auth(
        &headers,
        "POST",
        &url,
        Some(&body),
        state.config.require_auth_token,
    )?;
    check_nip98_replay(&state, event_id_bytes)?;
    let pubkey_bytes = pubkey.to_bytes().to_vec();

    let auth_tag = headers.get("x-auth-tag").and_then(|v| v.to_str().ok());
    super::relay_members::enforce_relay_membership(&state, &pubkey_bytes, auth_tag).await?;

    let filters: Vec<nostr::Filter> = serde_json::from_slice(&body)
        .map_err(|e| api_error(StatusCode::BAD_REQUEST, &format!("invalid filters: {e}")))?;

    // P-gated kinds enforcement — same as WS REQ and /query.
    let authed_pubkey_hex = pubkey.to_hex();
    if !crate::handlers::req::p_gated_filters_authorized(&filters, &authed_pubkey_hex) {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "restricted: p-gated kinds require #p tag matching your pubkey",
        ));
    }
    if !crate::handlers::req::engram_filters_authorized(&filters, &authed_pubkey_hex) {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "restricted: agent-engram reads require authors=[self] or #p=[self]",
        ));
    }
    if !crate::handlers::req::author_only_filters_authorized(&filters, &authed_pubkey_hex) {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "restricted: author-only kinds require authors=[self]",
        ));
    }

    // Get channels this user can access.
    let accessible_channels = state
        .get_accessible_channel_ids_cached(&pubkey_bytes)
        .await
        .map_err(|e| internal_error(&format!("channel access lookup: {e}")))?;

    let mut total: u64 = 0;
    for filter in &filters {
        let needs_author_only_filtering =
            crate::handlers::req::filter_can_match_author_only_kinds(filter);

        // If filter targets a specific channel, verify access.
        if let Some(ch_id) = extract_channel_from_filter(filter) {
            if !accessible_channels.contains(&ch_id) {
                continue; // Skip filters targeting inaccessible channels.
            }
            // Channel is accessible — count with pushability check.
            let query =
                crate::handlers::req::build_event_query_from_filter(filter, &pubkey_bytes, &state)
                    .await;
            let author_is_self = filter.authors.as_ref().is_some_and(|authors| {
                !authors.is_empty()
                    && authors
                        .iter()
                        .all(|a| a.to_hex().eq_ignore_ascii_case(&authed_pubkey_hex))
            });
            if crate::handlers::req::filter_fully_pushable(filter)
                && (!needs_author_only_filtering || author_is_self)
            {
                match state.db.count_events(&query).await {
                    Ok(n) => total += n as u64,
                    Err(e) => {
                        return Err(internal_error(&format!("count error: {e}")));
                    }
                }
            } else {
                // Fallback: query + post-filter for non-pushable constraints.
                let mut q = query;
                q.limit = Some(100_000);
                q.max_limit = Some(100_000);
                match state.db.query_events(&q).await {
                    Ok(stored_events) => {
                        for se in stored_events {
                            if !buzz_core::filter::filters_match(std::slice::from_ref(filter), &se)
                            {
                                continue;
                            }
                            if crate::handlers::req::is_author_only_event(&se.event, &pubkey_bytes)
                            {
                                continue;
                            }
                            total += 1;
                        }
                    }
                    Err(e) => {
                        return Err(internal_error(&format!("count error: {e}")));
                    }
                }
            }
        } else {
            // No channel filter — use SQL-level channel_ids pushdown to count
            // only events in accessible channels (+ global events).
            let mut query =
                crate::handlers::req::build_event_query_from_filter(filter, &pubkey_bytes, &state)
                    .await;
            query.channel_ids = Some(accessible_channels.to_vec());

            let author_is_self = filter.authors.as_ref().is_some_and(|authors| {
                !authors.is_empty()
                    && authors
                        .iter()
                        .all(|a| a.to_hex().eq_ignore_ascii_case(&authed_pubkey_hex))
            });
            if crate::handlers::req::filter_fully_pushable(filter)
                && (!needs_author_only_filtering || author_is_self)
            {
                query.limit = None;
                match state.db.count_events(&query).await {
                    Ok(n) => total += n as u64,
                    Err(e) => {
                        return Err(internal_error(&format!("count error: {e}")));
                    }
                }
            } else {
                // Fallback: query with high limit + post-filter for correctness.
                query.limit = Some(100_000);
                query.max_limit = Some(100_000);
                match state.db.query_events(&query).await {
                    Ok(stored_events) => {
                        for se in stored_events {
                            if !buzz_core::filter::filters_match(std::slice::from_ref(filter), &se)
                            {
                                continue;
                            }
                            if crate::handlers::req::is_author_only_event(&se.event, &pubkey_bytes)
                            {
                                continue;
                            }
                            total += 1;
                        }
                    }
                    Err(e) => {
                        return Err(internal_error(&format!("count error: {e}")));
                    }
                }
            }
        }
    }

    Ok(Json(serde_json::json!({ "count": total })))
}

// ── NIP-50 search via HTTP bridge ────────────────────────────────────────────

/// Decide whether a search hit should be returned to the caller.
///
/// Mirrors the WS NIP-50 path's post-filter step in `handlers/req.rs`:
/// Typesense receives only the kind/authors/time pushdown, so any other filter
/// constraint (`#p`, `#h`, `#e`, `#d`, `ids`, …) must be enforced here against
/// the full stored event. Without this, an authorized engram search such as
/// `{"kinds":[30174],"#p":[self]}` would leak text-matching envelopes whose
/// `#p` belongs to a different owner — the NIP-AE read gate at the filter
/// layer would be bypassed for `/query`.
///
/// `accessible_channels` is the caller's channel scope; channel-scoped hits
/// outside that set are rejected regardless of NIP-01 match.
fn search_hit_accepted(
    filter: &nostr::Filter,
    stored: &buzz_core::StoredEvent,
    accessible_channels: &[uuid::Uuid],
    reader_pubkey_hex: &str,
) -> bool {
    if !buzz_core::filter::filters_match(std::slice::from_ref(filter), stored) {
        return false;
    }
    if let Some(ch_id) = stored.channel_id {
        if !accessible_channels.contains(&ch_id) {
            return false;
        }
    }
    if !buzz_core::filter::reader_authorized_for_event(&stored.event, reader_pubkey_hex) {
        return false;
    }
    true
}

/// Handle search filters by routing to Typesense, then fetching full events from DB.
/// Returns first page of results (no pagination for bridge MVP).
async fn handle_bridge_search(
    state: &AppState,
    filters: &[nostr::Filter],
    accessible_channels: &[uuid::Uuid],
    reader_pubkey_hex: &str,
    pubkey_bytes: &[u8],
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    // Bridge always includes global (non-channel) events — same as WS with full scopes.
    let channel_scope = match crate::handlers::req::build_search_channel_scope_filter(
        accessible_channels,
        true, // include_global
    ) {
        Some(f) => f,
        None => return Ok(Json(Value::Array(Vec::new()))),
    };

    let mut events: Vec<Value> = Vec::new();
    let mut seen_ids: std::collections::HashSet<[u8; 32]> = std::collections::HashSet::new();

    for filter in filters {
        let search_text = match &filter.search {
            Some(s) if !s.is_empty() => s.clone(),
            _ => continue,
        };

        let limit = filter.limit.unwrap_or(100).min(500) as u32;
        if limit == 0 {
            continue;
        }

        // Build Typesense filter — push channel scope + NIP-01 constraints.
        let h_tag = nostr::SingleLetterTag::lowercase(nostr::Alphabet::H);
        let filter_channel_scope =
            if let Some(vs) = filter.generic_tags.get(&h_tag).filter(|vs| !vs.is_empty()) {
                let valid: Vec<String> = vs
                    .iter()
                    .filter_map(|v| v.parse::<uuid::Uuid>().ok())
                    .filter(|id| accessible_channels.contains(id))
                    .map(|id| id.to_string())
                    .collect();
                if valid.is_empty() {
                    continue; // All #h values inaccessible — skip filter.
                }
                format!("channel_id:=[{}]", valid.join(","))
            } else {
                channel_scope.clone()
            };

        let mut filter_parts = vec![filter_channel_scope];
        if let Some(ref kinds) = filter.kinds {
            if !kinds.is_empty() {
                let kind_vals: Vec<String> = kinds.iter().map(|k| k.as_u16().to_string()).collect();
                filter_parts.push(format!("kind:=[{}]", kind_vals.join(",")));
            }
        }
        if let Some(ref authors) = filter.authors {
            if !authors.is_empty() {
                let author_vals: Vec<String> = authors.iter().map(|a| a.to_hex()).collect();
                filter_parts.push(format!("pubkey:=[{}]", author_vals.join(",")));
            }
        }
        if let Some(since) = filter.since {
            filter_parts.push(format!("created_at:>={}", since.as_secs()));
        }
        if let Some(until) = filter.until {
            filter_parts.push(format!("created_at:<={}", until.as_secs()));
        }

        let filter_by = filter_parts.join(" && ");

        let search_query = buzz_search::SearchQuery {
            q: search_text,
            filter_by: Some(filter_by),
            sort_by: None, // Typesense default = relevance
            page: 1,
            per_page: limit,
        };

        let search_result = state
            .search
            .search(&search_query)
            .await
            .map_err(|e| internal_error(&format!("search error: {e}")))?;

        // Fetch full events from DB by ID.
        let hit_ids: Vec<Vec<u8>> = search_result
            .hits
            .into_iter()
            .filter_map(|h| hex::decode(&h.event_id).ok())
            .filter(|bytes| bytes.len() == 32)
            .collect();

        if hit_ids.is_empty() {
            continue;
        }

        let id_refs: Vec<&[u8]> = hit_ids.iter().map(|b| b.as_slice()).collect();
        let stored_events = state
            .db
            .get_events_by_ids(&id_refs)
            .await
            .map_err(|e| internal_error(&format!("search fetch error: {e}")))?;

        // Build lookup map to preserve Typesense relevance ordering.
        let event_map: std::collections::HashMap<[u8; 32], &buzz_core::StoredEvent> = stored_events
            .iter()
            .map(|ev| (ev.event.id.to_bytes(), ev))
            .collect();

        for hit_id in &hit_ids {
            let id_array: [u8; 32] = match hit_id.as_slice().try_into() {
                Ok(a) => a,
                Err(_) => continue,
            };
            let stored = match event_map.get(&id_array) {
                Some(ev) => ev,
                None => continue,
            };
            if !search_hit_accepted(filter, stored, accessible_channels, reader_pubkey_hex) {
                continue;
            }
            if crate::handlers::req::is_author_only_event(&stored.event, pubkey_bytes) {
                continue;
            }
            // Dedup across filters.
            if !seen_ids.insert(id_array) {
                continue;
            }
            if let Ok(v) = serde_json::to_value(&stored.event) {
                events.push(v);
            }
        }
    }

    Ok(Json(Value::Array(events)))
}

// ── POST /hooks/{id} — Webhook trigger ───────────────────────────────────────

/// Query parameters for the webhook trigger endpoint.
#[derive(serde::Deserialize)]
pub struct WebhookQuery {
    /// Webhook secret for authentication. Prefer the `X-Webhook-Secret` header instead.
    pub secret: Option<String>,
}

/// Webhook trigger endpoint. No user auth — the webhook secret authenticates the caller.
///
/// Prefers `X-Webhook-Secret` header over `?secret=` query param (headers aren't logged
/// by most proxies). Returns 202 Accepted; execution is async.
pub async fn workflow_webhook(
    State(state): State<Arc<AppState>>,
    Path(id_str): Path<String>,
    Query(query): Query<WebhookQuery>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    let id = uuid::Uuid::parse_str(&id_str)
        .map_err(|_| api_error(StatusCode::BAD_REQUEST, "invalid workflow UUID"))?;

    let workflow = state
        .db
        .get_workflow(id)
        .await
        .map_err(|_| not_found("workflow not found"))?;

    let def: buzz_workflow::WorkflowDef = serde_json::from_value(workflow.definition.clone())
        .map_err(|e| super::internal_error(&format!("corrupt workflow definition: {e}")))?;

    if !matches!(def.trigger, buzz_workflow::TriggerDef::Webhook) {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "workflow does not have a webhook trigger",
        ));
    }

    // Verify webhook secret. Prefer header (not logged by proxies); fall back to query param.
    let stored_secret = crate::webhook_secret::extract_secret(&workflow.definition);
    let provided_secret = headers
        .get("x-webhook-secret")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .or_else(|| query.secret.clone())
        .unwrap_or_default();

    match &stored_secret {
        Some(secret) => {
            if !crate::webhook_secret::verify_secret(&provided_secret, secret) {
                tracing::warn!("webhook: invalid secret for workflow {id}");
                return Err(api_error(StatusCode::UNAUTHORIZED, "authentication failed"));
            }
        }
        None => {
            return Err(api_error(
                StatusCode::UNAUTHORIZED,
                "webhook secret required but not configured — re-save the workflow to generate one",
            ));
        }
    }

    // Parse optional JSON body as trigger context.
    let body_json: Option<Value> =
        if body.is_empty() {
            None
        } else {
            Some(serde_json::from_slice(&body).map_err(|e| {
                api_error(StatusCode::BAD_REQUEST, &format!("invalid JSON body: {e}"))
            })?)
        };

    // Build trigger context from webhook body fields.
    let mut trigger_ctx = buzz_workflow::executor::TriggerContext {
        channel_id: workflow
            .channel_id
            .map(|ch| ch.to_string())
            .unwrap_or_default(),
        ..Default::default()
    };
    if let Some(Value::Object(ref map)) = body_json {
        for (k, v) in map {
            let val_str = match v {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            trigger_ctx.webhook_fields.insert(k.clone(), val_str);
        }
    }
    let trigger_ctx_json = serde_json::to_value(&trigger_ctx).ok();

    let run_id = state
        .db
        .create_workflow_run(id, None, trigger_ctx_json.as_ref())
        .await
        .map_err(|e| super::internal_error(&format!("db error: {e}")))?;

    // Spawn workflow execution asynchronously.
    let engine = Arc::clone(&state.workflow_engine);
    let db = state.db.clone();
    let def_value = workflow.definition.clone();
    let trigger_ctx_clone = trigger_ctx.clone();
    tokio::spawn(async move {
        let def: buzz_workflow::WorkflowDef = match serde_json::from_value(def_value) {
            Ok(d) => d,
            Err(e) => {
                tracing::error!("webhook: failed to parse definition: {e}");
                if let Err(db_err) = db
                    .update_workflow_run(
                        run_id,
                        buzz_db::workflow::RunStatus::Failed,
                        0,
                        &serde_json::json!([]),
                        Some(&format!("definition parse error: {e}")),
                    )
                    .await
                {
                    tracing::error!("webhook: failed to mark run as failed: {db_err}");
                }
                return;
            }
        };

        let result = buzz_workflow::executor::execute_from_step(
            &engine,
            run_id,
            &def,
            &trigger_ctx_clone,
            0,
            None,
        )
        .await;
        engine.finalize_run(run_id, result, None).await;
    });

    Ok((
        StatusCode::ACCEPTED,
        Json(serde_json::json!({
            "run_id": run_id.to_string(),
            "workflow_id": id.to_string(),
            "status": "pending",
        })),
    ))
}

// ── Presence synthesis from Redis ────────────────────────────────────────────

/// If all filters target kind:20001 or kind:40902 with authors, synthesize
/// presence from Redis instead of querying the DB (ephemeral events are never
/// stored, and kind:40902 snapshots are relay-generated on demand).
///
/// Returns `Some(events)` if handled, `None` to fall through to normal query.
async fn synthesize_presence(state: &AppState, filters: &[nostr::Filter]) -> Option<Vec<Value>> {
    use buzz_core::kind::{KIND_PRESENCE_SNAPSHOT, KIND_PRESENCE_UPDATE};

    // Only intercept if every filter targets kind:20001 or 40902 with authors.
    let mut all_pubkeys: Vec<nostr::PublicKey> = Vec::new();
    for filter in filters {
        let kinds = filter.kinds.as_ref()?;
        let only_kind = kinds.iter().next()?;
        let k = only_kind.as_u16() as u32;
        if kinds.len() != 1 || (k != KIND_PRESENCE_UPDATE && k != KIND_PRESENCE_SNAPSHOT) {
            return None;
        }
        let authors = filter.authors.as_ref()?;
        if authors.is_empty() {
            return None;
        }
        all_pubkeys.extend(authors.iter().copied());
    }

    if all_pubkeys.is_empty() {
        return Some(Vec::new());
    }

    // Dedup pubkeys.
    all_pubkeys.sort_by_key(|pk| pk.to_hex());
    all_pubkeys.dedup();

    // Look up Redis.
    let presence_map = state
        .pubsub
        .get_presence_bulk(&all_pubkeys)
        .await
        .unwrap_or_default();

    if presence_map.is_empty() {
        return Some(Vec::new());
    }

    // Synthesize kind:20001 events signed by the relay.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let mut events = Vec::with_capacity(presence_map.len());
    for (pubkey_hex, status) in &presence_map {
        // Build a synthetic event: relay-signed, content = status, p-tag = subject.
        let tags = vec![nostr::Tag::parse(["p", pubkey_hex]).ok()?];
        let event =
            nostr::EventBuilder::new(nostr::Kind::Custom(KIND_PRESENCE_UPDATE as u16), status)
                .tags(tags)
                .custom_created_at(nostr::Timestamp::from(now))
                .sign_with_keys(&state.relay_keypair)
                .ok()?;

        if let Ok(v) = serde_json::to_value(&event) {
            events.push(v);
        }
    }

    Some(events)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{Alphabet, EventBuilder, Keys, Kind, SingleLetterTag, Tag};

    /// Build a kind:30174 engram envelope authored by `agent`, tagged with `owner`.
    fn engram_envelope(agent: &Keys, owner_hex: &str) -> buzz_core::StoredEvent {
        let d_tag = Tag::custom(
            nostr::TagKind::SingleLetter(SingleLetterTag::lowercase(Alphabet::D)),
            ["abcd1234"],
        );
        let p_tag = Tag::custom(
            nostr::TagKind::SingleLetter(SingleLetterTag::lowercase(Alphabet::P)),
            [owner_hex],
        );
        let ev = EventBuilder::new(Kind::Custom(30174), "engram body")
            .tags([d_tag, p_tag])
            .sign_with_keys(agent)
            .expect("sign engram");
        buzz_core::StoredEvent::new(ev, None)
    }

    /// Regression test for the NIP-AE `/query` search leak (PR #593 review).
    ///
    /// Setup: two engram envelopes by different agents for different owners.
    /// An authorized search for `{kinds:[30174], #p:[owner_a]}` would be
    /// approved by the engram gate (owner_a is querying engrams addressed to
    /// them). Typesense's pushdown only carries `kind:=[30174]`, so the
    /// envelope for owner_b can come back as a text-match hit. The post-filter
    /// in `search_hit_accepted` must reject it.
    #[test]
    fn search_hit_rejects_envelope_with_mismatched_p_tag() {
        let agent_a = Keys::generate();
        let agent_b = Keys::generate();
        let owner_a = Keys::generate().public_key().to_hex();
        let owner_b = Keys::generate().public_key().to_hex();

        let env_for_a = engram_envelope(&agent_a, &owner_a);
        let env_for_b = engram_envelope(&agent_b, &owner_b);

        let p_tag = SingleLetterTag::lowercase(Alphabet::P);
        let filter = nostr::Filter::new()
            .kind(Kind::Custom(30174))
            .custom_tags(p_tag, [&owner_a]);

        // 30174 is not owner-gated, so any reader hex is fine here.
        let reader = Keys::generate().public_key().to_hex();
        assert!(
            search_hit_accepted(&filter, &env_for_a, &[], &reader),
            "envelope addressed to owner_a must be returned"
        );
        assert!(
            !search_hit_accepted(&filter, &env_for_b, &[], &reader),
            "envelope addressed to owner_b must NOT be returned for a #p=[owner_a] search"
        );
    }

    /// `authors=[agent_a]` search must not return an envelope authored by agent_b,
    /// even if Typesense's text match would otherwise surface it. (Typesense does
    /// carry an `authors` pushdown today, so this is defence-in-depth; mirroring
    /// the WS contract.)
    #[test]
    fn search_hit_rejects_event_with_mismatched_author() {
        let agent_a = Keys::generate();
        let agent_b = Keys::generate();
        let owner = Keys::generate().public_key().to_hex();

        let env_a = engram_envelope(&agent_a, &owner);
        let env_b = engram_envelope(&agent_b, &owner);

        let filter = nostr::Filter::new()
            .kind(Kind::Custom(30174))
            .author(agent_a.public_key());

        let reader = Keys::generate().public_key().to_hex();
        assert!(search_hit_accepted(&filter, &env_a, &[], &reader));
        assert!(
            !search_hit_accepted(&filter, &env_b, &[], &reader),
            "authors=[agent_a] search must not return events authored by agent_b"
        );
    }

    /// Channel-scoped events outside the caller's accessible-channel set are
    /// rejected by the post-filter regardless of NIP-01 match.
    #[test]
    fn search_hit_rejects_inaccessible_channel() {
        let agent = Keys::generate();
        let owner = Keys::generate().public_key().to_hex();
        let mut stored = engram_envelope(&agent, &owner);
        let scoped_channel = uuid::Uuid::new_v4();
        stored.channel_id = Some(scoped_channel);

        let p_tag = SingleLetterTag::lowercase(Alphabet::P);
        let filter = nostr::Filter::new()
            .kind(Kind::Custom(30174))
            .custom_tags(p_tag, [&owner]);

        let reader = Keys::generate().public_key().to_hex();
        assert!(
            !search_hit_accepted(&filter, &stored, &[], &reader),
            "channel-scoped hit must be rejected when caller has no channel access"
        );
        assert!(
            search_hit_accepted(&filter, &stored, &[scoped_channel], &reader),
            "channel-scoped hit must be accepted when caller has access to that channel"
        );
    }

    // ── Custom filter field extractor tests ──

    #[test]
    fn extract_before_id_valid_hex() {
        let hex = "a".repeat(64);
        let raw = serde_json::json!({ "before_id": hex });
        let result = extract_before_id(&raw);
        assert!(result.is_some());
        assert_eq!(result.unwrap().len(), 32);
    }

    #[test]
    fn extract_before_id_short_hex() {
        let raw = serde_json::json!({ "before_id": "a".repeat(63) });
        assert!(extract_before_id(&raw).is_none());
    }

    #[test]
    fn extract_before_id_long_hex() {
        let raw = serde_json::json!({ "before_id": "a".repeat(65) });
        assert!(extract_before_id(&raw).is_none());
    }

    #[test]
    fn extract_before_id_invalid_hex_chars() {
        let raw = serde_json::json!({ "before_id": "z".repeat(64) });
        assert!(extract_before_id(&raw).is_none());
    }

    #[test]
    fn extract_before_id_absent() {
        let raw = serde_json::json!({});
        assert!(extract_before_id(&raw).is_none());
    }

    #[test]
    fn extract_before_id_non_string() {
        let raw = serde_json::json!({ "before_id": 12345 });
        assert!(extract_before_id(&raw).is_none());
    }

    #[test]
    fn extract_depth_limit_valid() {
        let raw = serde_json::json!({ "depth_limit": 3 });
        assert_eq!(extract_depth_limit(&raw), Some(3));
    }

    #[test]
    fn extract_depth_limit_zero() {
        let raw = serde_json::json!({ "depth_limit": 0 });
        assert_eq!(extract_depth_limit(&raw), Some(0));
    }

    #[test]
    fn extract_depth_limit_u32_max() {
        let raw = serde_json::json!({ "depth_limit": u32::MAX });
        assert_eq!(extract_depth_limit(&raw), Some(u32::MAX));
    }

    #[test]
    fn extract_depth_limit_overflow() {
        let raw = serde_json::json!({ "depth_limit": (u32::MAX as u64) + 1 });
        assert!(extract_depth_limit(&raw).is_none());
    }

    #[test]
    fn extract_depth_limit_negative() {
        let raw = serde_json::json!({ "depth_limit": -1 });
        assert!(extract_depth_limit(&raw).is_none());
    }

    #[test]
    fn extract_depth_limit_absent() {
        let raw = serde_json::json!({});
        assert!(extract_depth_limit(&raw).is_none());
    }

    #[test]
    fn extract_depth_limit_float() {
        let raw = serde_json::json!({ "depth_limit": 3.5 });
        assert!(extract_depth_limit(&raw).is_none());
    }

    #[test]
    fn extract_feed_types_valid() {
        let raw = serde_json::json!({ "feed_types": ["mentions", "activity"] });
        assert_eq!(
            extract_feed_types(&raw),
            Some(vec!["mentions".to_string(), "activity".to_string()])
        );
    }

    #[test]
    fn extract_feed_types_empty_array() {
        let raw = serde_json::json!({ "feed_types": [] });
        assert!(extract_feed_types(&raw).is_none());
    }

    #[test]
    fn extract_feed_types_mixed_types() {
        let raw = serde_json::json!({ "feed_types": ["mentions", 42, "activity"] });
        assert_eq!(
            extract_feed_types(&raw),
            Some(vec!["mentions".to_string(), "activity".to_string()])
        );
    }

    #[test]
    fn extract_feed_types_absent() {
        let raw = serde_json::json!({});
        assert!(extract_feed_types(&raw).is_none());
    }

    #[test]
    fn extract_feed_types_non_array() {
        let raw = serde_json::json!({ "feed_types": "mentions" });
        assert!(extract_feed_types(&raw).is_none());
    }

    #[test]
    fn event_accessible_no_channel() {
        let keys = Keys::generate();
        let ev = EventBuilder::new(Kind::Custom(1), "test")
            .sign_with_keys(&keys)
            .unwrap();
        let se = buzz_core::StoredEvent::new(ev, None);
        assert!(event_in_accessible_channel(&se, &[]));
    }

    #[test]
    fn event_accessible_matching_channel() {
        let keys = Keys::generate();
        let ev = EventBuilder::new(Kind::Custom(1), "test")
            .sign_with_keys(&keys)
            .unwrap();
        let ch = uuid::Uuid::new_v4();
        let mut se = buzz_core::StoredEvent::new(ev, None);
        se.channel_id = Some(ch);
        assert!(event_in_accessible_channel(&se, &[ch]));
    }

    #[test]
    fn event_inaccessible_channel() {
        let keys = Keys::generate();
        let ev = EventBuilder::new(Kind::Custom(1), "test")
            .sign_with_keys(&keys)
            .unwrap();
        let ch = uuid::Uuid::new_v4();
        let other = uuid::Uuid::new_v4();
        let mut se = buzz_core::StoredEvent::new(ev, None);
        se.channel_id = Some(ch);
        assert!(!event_in_accessible_channel(&se, &[other]));
    }

    /// NIP-DV regression: a relay-signed kind:30622 snapshot must not leak via
    /// search through a kindless `ids:[snapshot_id]` filter that carries no #p.
    /// `filters_match` passes (id matches), channel check passes (channel_id =
    /// None), so only the result-level `reader_authorized_for_event` check
    /// stands between a third party and the owner's private hide set.
    #[test]
    fn search_hit_rejects_dm_visibility_for_kindless_ids_third_party() {
        let relay = Keys::generate();
        let viewer = Keys::generate().public_key().to_hex();
        let third_party = Keys::generate().public_key().to_hex();

        let d_tag = Tag::custom(
            nostr::TagKind::SingleLetter(SingleLetterTag::lowercase(Alphabet::D)),
            [&viewer],
        );
        let p_tag = Tag::custom(
            nostr::TagKind::SingleLetter(SingleLetterTag::lowercase(Alphabet::P)),
            [&viewer],
        );
        let ev = EventBuilder::new(Kind::Custom(buzz_core::kind::KIND_DM_VISIBILITY as u16), "")
            .tags([d_tag, p_tag])
            .sign_with_keys(&relay)
            .expect("sign snapshot");
        let stored = buzz_core::StoredEvent::new(ev.clone(), None);

        // Kindless filter — the exact bypass shape: no #p, just the id.
        let filter = nostr::Filter::new().id(ev.id);

        assert!(
            !search_hit_accepted(&filter, &stored, &[], &third_party),
            "third party must not receive a DM-visibility snapshot via kindless ids search"
        );
        assert!(
            search_hit_accepted(&filter, &stored, &[], &viewer),
            "owner must still receive their own snapshot"
        );
    }
}
