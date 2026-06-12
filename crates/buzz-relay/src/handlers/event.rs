//! EVENT handler — WS dispatcher → ingest pipeline → fan-out.

use std::sync::Arc;

use tracing::{debug, error, info, warn};

use buzz_core::event::StoredEvent;
use buzz_core::kind::{
    event_kind_u32, is_ephemeral, KIND_AGENT_OBSERVER_FRAME, KIND_GIFT_WRAP,
    KIND_MESH_CONNECT_REQUEST, KIND_MESH_STATUS_REPORT, KIND_PRESENCE_UPDATE,
};
use buzz_core::observer::{
    content_looks_like_nip44, OBSERVER_AGENT_TAG, OBSERVER_FRAME_CONTROL, OBSERVER_FRAME_TAG,
    OBSERVER_FRAME_TELEMETRY,
};
use buzz_core::verification::verify_event;
use nostr::{Event, PublicKey};

use crate::connection::{AuthState, ConnectionState};
use crate::protocol::RelayMessage;
use crate::state::AppState;

use super::ingest::{IngestAuth, IngestError};

/// Increment the rejection counter with a bounded reason label.
fn reject(reason: &'static str) {
    metrics::counter!("buzz_events_rejected_total", "reason" => reason).increment(1);
}

/// Bound the `kind` label to prevent cardinality explosion from arbitrary Nostr kinds.
fn bounded_kind_label(kind: u32) -> String {
    match kind {
        0..=9 | 1059 | 1063 => kind.to_string(),
        8000..=8003 | 9000..=9022 | 9030..=9036 => kind.to_string(),
        13534..=13535 => kind.to_string(),
        20000..=29999 => kind.to_string(),
        30023 | 30315 | 39000..=39003 => kind.to_string(),
        40002..=40100 => kind.to_string(),
        41001 | 41010..=41012 => kind.to_string(),
        43001..=43006 => kind.to_string(),
        44100..=44101 => kind.to_string(),
        45001..=45003 => kind.to_string(),
        46001..=46012 | 46020 | 46030..=46031 => kind.to_string(),
        48001 | 48100..=48103 | 48106 => kind.to_string(),
        49001 => kind.to_string(),
        _ => "other".to_string(),
    }
}

/// Drop recipients without access before fan-out on a private channel.
///
/// Open and channel-less events skip membership filtering (open channel-scoped
/// events pay one visibility lookup; see `channel_visibility_cached`). For a
/// private channel, each recipient is kept only if its connection's
/// authenticated pubkey is a current member; unknown/unauthenticated recipients
/// fail closed. This is the cluster-wide backstop: even if a stale subscription
/// survives on another node after an open->private flip, its events are not
/// delivered here.
pub async fn filter_fanout_by_access(
    state: &Arc<AppState>,
    stored_event: &StoredEvent,
    matches: Vec<(crate::subscription::ConnId, crate::subscription::SubId)>,
) -> Vec<(crate::subscription::ConnId, crate::subscription::SubId)> {
    let Some(channel_id) = stored_event.channel_id else {
        return matches;
    };
    match state.channel_visibility_cached(channel_id).await {
        Ok(v) if v != "private" => return matches,
        Ok(_) => {}
        Err(e) => {
            // Fail closed: if we cannot determine visibility, do not leak a
            // possibly-private channel's events.
            warn!(%channel_id, "fan-out access filter: visibility lookup failed: {e}");
            return Vec::new();
        }
    }

    let mut allowed = Vec::with_capacity(matches.len());
    for (conn_id, sub_id) in matches {
        let Some(pubkey) = state.conn_manager.pubkey_for_conn(conn_id) else {
            continue;
        };
        match state.is_member_cached(channel_id, &pubkey).await {
            Ok(true) => allowed.push((conn_id, sub_id)),
            Ok(false) => {}
            Err(e) => {
                warn!(%channel_id, "fan-out access filter: membership lookup failed: {e}");
            }
        }
    }
    allowed
}

/// Publish a stored event to subscribers and kick off async side effects.
pub(crate) async fn dispatch_persistent_event(
    state: &Arc<AppState>,
    stored_event: &StoredEvent,
    kind_u32: u32,
    actor_pubkey_hex: &str,
) -> usize {
    let event_id_hex = stored_event.event.id.to_hex();

    let pubsub_channel = stored_event.channel_id.unwrap_or(uuid::Uuid::nil());
    state.mark_local_event(&stored_event.event.id);
    if let Err(e) = state
        .pubsub
        .publish_event(pubsub_channel, &stored_event.event)
        .await
    {
        state
            .local_event_ids
            .invalidate(&stored_event.event.id.to_bytes());
        warn!(event_id = %event_id_hex, "Redis publish failed: {e}");
    }

    let matches = state.sub_registry.fan_out(stored_event);
    let matches = filter_fanout_by_access(state, stored_event, matches).await;
    metrics::histogram!("buzz_fanout_recipients").record(matches.len() as f64);
    debug!(
        event_id = %event_id_hex,
        channel_id = ?stored_event.channel_id,
        match_count = matches.len(),
        "Fan-out"
    );

    let event_json = serde_json::to_string(&stored_event.event)
        .expect("nostr::Event serialization is infallible for well-formed events");
    // For viewer-private snapshots (kind:30622), live fan-out must reach only the
    // owner — a kindless `ids:[…]` subscription can otherwise match it. Pull paths
    // (HTTP /query, WS historical) are gated separately by reader_authorized_for_event.
    let dm_visibility_owner: Option<String> = (kind_u32 == buzz_core::kind::KIND_DM_VISIBILITY)
        .then(|| {
            let p = nostr::SingleLetterTag::lowercase(nostr::Alphabet::P);
            stored_event
                .event
                .tags
                .filter(nostr::TagKind::SingleLetter(p))
                .find_map(|t| t.content().map(|s| s.to_string()))
        })
        .flatten();
    let mut drop_count = 0u32;
    for (target_conn_id, sub_id) in &matches {
        if let Some(ref owner_hex) = dm_visibility_owner {
            let is_owner = state
                .conn_manager
                .pubkey_for(*target_conn_id)
                .is_some_and(|pk| hex::encode(pk) == *owner_hex);
            if !is_owner {
                continue;
            }
        }
        let msg = format!(r#"["EVENT","{}",{}]"#, sub_id, event_json);
        if !state.conn_manager.send_to(*target_conn_id, msg) {
            drop_count += 1;
        }
    }
    if drop_count > 0 {
        tracing::warn!(
            event_id = %event_id_hex,
            drop_count,
            "fan-out: {drop_count} connection(s) cancelled due to full/closed buffers"
        );
    }

    // Skip search indexing for NIP-17 gift wraps (ciphertext) and NIP-DV
    // visibility snapshots (per-viewer private hide state, owner-gated reads).
    if kind_u32 != KIND_GIFT_WRAP
        && kind_u32 != buzz_core::kind::KIND_DM_VISIBILITY
        && state
            .search_index_tx
            .try_send(stored_event.clone())
            .is_err()
    {
        metrics::counter!("buzz_search_index_errors_total").increment(1);
        warn!(event_id = %event_id_hex, "Search index channel full — dropping event");
    }

    // Audit via bounded channel (capacity 1000). Uses .send().await so entries
    // are never silently dropped — backpressure propagates to the event handler
    // if the queue is full. This is intentional: the audit advisory lock already
    // serializes writes (at most 1 in-flight), so a full queue means the audit
    // DB is genuinely overloaded and the relay should slow down rather than
    // accumulate unbounded in-memory state. DB write failures in the worker are
    // logged but not retried (same as the previous per-event tokio::spawn).
    let audit_entry = buzz_audit::NewAuditEntry {
        event_id: event_id_hex.clone(),
        event_kind: kind_u32,
        actor_pubkey: actor_pubkey_hex.to_string(),
        action: buzz_audit::AuditAction::EventCreated,
        channel_id: stored_event.channel_id,
        metadata: serde_json::Value::Null,
    };
    if let Err(e) = state.audit_tx.send(audit_entry).await {
        error!(event_id = %event_id_hex, "Audit channel closed — entry lost: {e}");
        metrics::counter!("buzz_audit_send_errors_total").increment(1);
    }

    // Skip workflow triggering for workflow-execution kinds and relay-signed workflow messages.
    let is_relay_workflow_msg = stored_event.event.pubkey == state.relay_keypair.public_key()
        && stored_event
            .event
            .tags
            .iter()
            .any(|t| t.as_slice().first().map(|s| s.as_str()) == Some("buzz:workflow"));

    if !buzz_core::kind::is_workflow_execution_kind(kind_u32)
        && !buzz_core::kind::is_command_kind(kind_u32)
        && !is_relay_workflow_msg
        && kind_u32 != KIND_GIFT_WRAP
    {
        let workflow_engine = Arc::clone(&state.workflow_engine);
        let workflow_event = stored_event.clone();
        let trigger_kind = kind_u32.to_string();
        tokio::spawn(async move {
            if let Err(e) = workflow_engine.on_event(&workflow_event).await {
                tracing::error!(event_id = ?workflow_event.event.id, "Workflow trigger failed: {e}");
            } else {
                metrics::counter!("buzz_workflow_runs_total", "trigger" => trigger_kind)
                    .increment(1);
            }
        });
    }

    matches.len()
}

/// Handle an EVENT message from a WebSocket connection.
///
/// Extracts auth from the WS connection, dispatches ephemeral events locally,
/// and delegates persistent events to [`super::ingest::ingest_event`].
pub async fn handle_event(event: Event, conn: Arc<ConnectionState>, state: Arc<AppState>) {
    let start = std::time::Instant::now();
    let event_id_hex = event.id.to_hex();
    let kind_u32 = event_kind_u32(&event);
    let kind_str = bounded_kind_label(kind_u32);
    debug!(event_id = %event_id_hex, kind = kind_u32, "EVENT");
    metrics::counter!("buzz_events_received_total", "kind" => kind_str.clone()).increment(1);

    // ── Extract auth from WS connection state ────────────────────────────
    let (conn_id, pubkey_bytes, auth_pubkey, scopes, channel_ids) = {
        let auth = conn.auth_state.read().await;
        match &*auth {
            AuthState::Authenticated(ctx) => (
                conn.conn_id,
                ctx.pubkey.to_bytes().to_vec(),
                ctx.pubkey,
                ctx.scopes.clone(),
                ctx.channel_ids.clone(),
            ),
            _ => {
                reject("auth");
                conn.send(RelayMessage::ok(
                    &event_id_hex,
                    false,
                    "auth-required: not authenticated",
                ));
                return;
            }
        }
    };

    // ── Pubkey / auth identity match (all events) ─────────────────────
    // Must run before both ephemeral and persistent branches. Persistent
    // events get a second check inside ingest_event() (step 3), but
    // ephemeral events bypass the pipeline entirely.
    let has_proxy_scope = scopes.contains(&buzz_auth::Scope::ProxySubmit);
    let is_gift_wrap = kind_u32 == KIND_GIFT_WRAP;
    if event.pubkey != auth_pubkey && !has_proxy_scope && !is_gift_wrap {
        reject("invalid");
        conn.send(RelayMessage::ok(
            &event_id_hex,
            false,
            "invalid: event pubkey does not match authenticated identity",
        ));
        return;
    }

    // ── Blocked kinds (both ephemeral and persistent) ─────────────────
    if kind_u32 == buzz_core::kind::KIND_AUTH {
        reject("invalid");
        conn.send(RelayMessage::ok(
            &event_id_hex,
            false,
            "invalid: AUTH events cannot be submitted via EVENT",
        ));
        return;
    }

    // ── Agent observer frames are owner-scoped, encrypted, and never stored ──
    if kind_u32 == KIND_AGENT_OBSERVER_FRAME {
        if !scopes.is_empty()
            && !scopes.contains(&buzz_auth::Scope::MessagesWrite)
            && !has_proxy_scope
        {
            reject("scope");
            conn.send(RelayMessage::ok(
                &event_id_hex,
                false,
                "restricted: insufficient scope for agent observer frames",
            ));
            return;
        }
        handle_agent_observer_event(event, conn_id, &event_id_hex, conn, state).await;
        return;
    }

    // ── Ephemeral events are WS-only (never stored) ──────────────────────
    // Scope enforcement for ephemeral kinds: require MessagesWrite or
    // ProxySubmit. Persistent events skip this gate and rely on
    // ingest_event()'s per-kind scope allowlist instead, so a token with
    // only ChannelsWrite can still submit kind:9002 via WS.
    if is_ephemeral(kind_u32) {
        if !scopes.is_empty()
            && !scopes.contains(&buzz_auth::Scope::MessagesWrite)
            && !has_proxy_scope
        {
            reject("scope");
            conn.send(RelayMessage::ok(
                &event_id_hex,
                false,
                "restricted: insufficient scope for ephemeral events",
            ));
            return;
        }
        // Mesh signaling kinds are direct desktop-user actions: the resulting
        // call-me-now routes to the *authenticated connection's* pubkey, so a
        // proxy-submitted mesh request would point the dial at the proxy, not the
        // user's desktop. Reject mesh kinds under proxy scope — they must come
        // from the member's own session.
        if has_proxy_scope
            && (kind_u32 == KIND_MESH_CONNECT_REQUEST || kind_u32 == KIND_MESH_STATUS_REPORT)
        {
            reject("scope");
            conn.send(RelayMessage::ok(
                &event_id_hex,
                false,
                "restricted: mesh signaling cannot be proxy-submitted",
            ));
            return;
        }
        handle_ephemeral_event(
            event,
            conn_id,
            &event_id_hex,
            pubkey_bytes,
            auth_pubkey,
            conn,
            state,
        )
        .await;
        return;
    }

    // ── Persistent events → ingest pipeline ──────────────────────────────
    let ingest_auth = IngestAuth::Nip42 {
        pubkey: auth_pubkey,
        scopes,
        channel_ids,
        conn_id,
    };

    match super::ingest::ingest_event(&state, event, ingest_auth).await {
        Ok(result) => {
            if result.accepted {
                metrics::counter!("buzz_events_stored_total", "kind" => kind_str).increment(1);
                info!(
                    event_id = %result.event_id,
                    kind = kind_u32,
                    conn_id = %conn_id,
                    "Event ingested"
                );
            }
            metrics::histogram!("buzz_event_processing_seconds")
                .record(start.elapsed().as_secs_f64());
            conn.send(RelayMessage::ok(
                &result.event_id,
                result.accepted,
                &result.message,
            ));
        }
        Err(e) => {
            // Sanitize internal errors — don't leak DB/system details over WS.
            let (msg, reason) = match &e {
                IngestError::Rejected(m) => (m.clone(), "invalid"),
                IngestError::AuthFailed(m) => (m.clone(), "auth"),
                IngestError::Internal(_) => ("error: internal server error".to_string(), "error"),
            };
            reject(reason);
            conn.send(RelayMessage::ok(&event_id_hex, false, &msg));
        }
    }
}

/// Handle ephemeral events (kind 20000–29999) — WS-only, never stored.
async fn handle_ephemeral_event(
    event: Event,
    conn_id: uuid::Uuid,
    event_id_hex: &str,
    pubkey_bytes: Vec<u8>,
    auth_pubkey: nostr::PublicKey,
    conn: Arc<ConnectionState>,
    state: Arc<AppState>,
) {
    let event_clone = event.clone();
    let verify_result = tokio::task::spawn_blocking(move || verify_event(&event_clone)).await;

    match verify_result {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            conn.send(RelayMessage::ok(
                event_id_hex,
                false,
                &format!("invalid: {e}"),
            ));
            return;
        }
        Err(_) => {
            conn.send(RelayMessage::ok(
                event_id_hex,
                false,
                "error: internal error",
            ));
            return;
        }
    }

    // Special handling for presence events (kind:20001).
    if event_kind_u32(&event) == KIND_PRESENCE_UPDATE {
        // Accept both bare strings ("online") and legacy JSON ({"status":"online"}).
        let raw = event.content.to_string();
        let status = if raw.starts_with('{') {
            serde_json::from_str::<serde_json::Value>(&raw)
                .ok()
                .and_then(|v| v.get("status")?.as_str().map(String::from))
                .unwrap_or(raw)
        } else if raw.len() > 128 {
            let mut end = 128;
            while !raw.is_char_boundary(end) {
                end -= 1;
            }
            raw[..end].to_string()
        } else {
            raw
        };

        if status == "offline" {
            let _ = state.pubsub.clear_presence(&auth_pubkey).await;
        } else {
            let _ = state.pubsub.set_presence(&auth_pubkey, &status).await;
        }

        let stored_event = StoredEvent::new(event.clone(), None);
        let matches = state.sub_registry.fan_out(&stored_event);
        metrics::histogram!("buzz_fanout_recipients").record(matches.len() as f64);
        let event_json = serde_json::to_string(&event)
            .expect("nostr::Event serialization is infallible for well-formed events");
        let mut drop_count = 0u32;
        for (target_conn_id, sub_id) in &matches {
            let msg = format!(r#"["EVENT","{}",{}]"#, sub_id, event_json);
            if !state.conn_manager.send_to(*target_conn_id, msg) {
                drop_count += 1;
            }
        }
        if drop_count > 0 {
            tracing::warn!(
                event_id = %event_id_hex,
                drop_count,
                "fan-out: {drop_count} connection(s) cancelled due to full/closed buffers"
            );
        }

        conn.send(RelayMessage::ok(event_id_hex, true, ""));
        return;
    }

    // Mesh status report (kind:24620). An authenticated relay member reports its
    // current mesh serve availability; the relay projects it into a relay-signed,
    // per-reporter kind:30621 discovery note. The report is ephemeral input; the
    // 30621 is the durable, relay-owned record.
    if event_kind_u32(&event) == KIND_MESH_STATUS_REPORT {
        let reporter_hex = auth_pubkey.to_hex();
        match super::mesh_signaling::handle_status_report(&state, &reporter_hex, &event).await {
            Ok(()) => {
                conn.send(RelayMessage::ok(event_id_hex, true, ""));
            }
            Err(reason) => {
                conn.send(RelayMessage::ok(event_id_hex, false, &reason));
            }
        }
        return;
    }

    // Mesh hole-punch signaling (kind:24621). An authenticated relay member
    // asks the relay to coordinate a direct iroh hole-punch to a peer it found
    // via kind:30621. The relay validates the target is also a member, then
    // emits the paired call-me-now (kind:24622). This is the relay's ONLY role
    // in the v1 direct-iroh mesh — validate membership + pair + fan out. It
    // never carries iroh traffic and stores no endpoint state.
    if event_kind_u32(&event) == KIND_MESH_CONNECT_REQUEST {
        // Per-requester rate limit shared with the HTTP door — see
        // `mesh_signaling::connect_request_rate_limited` for rationale.
        if super::mesh_signaling::connect_request_rate_limited(&state, &auth_pubkey) {
            conn.send(RelayMessage::ok(
                event_id_hex,
                false,
                "rate-limited: mesh connect request rate exceeded (20/sec)",
            ));
            return;
        }
        let requester_hex = auth_pubkey.to_hex();
        match super::mesh_signaling::handle_connect_request(&state, &requester_hex, &event).await {
            Ok(()) => {
                conn.send(RelayMessage::ok(event_id_hex, true, ""));
            }
            Err(reason) => {
                conn.send(RelayMessage::ok(event_id_hex, false, &reason));
            }
        }
        return;
    }

    // Check channel membership before publishing other ephemeral events.
    if let Some(ch_id) = super::ingest::extract_channel_id(&event) {
        if let Err(msg) =
            super::ingest::check_channel_membership(&state, ch_id, &pubkey_bytes).await
        {
            conn.send(RelayMessage::ok(event_id_hex, false, &msg));
            return;
        }

        // Mark as local before Redis publish to prevent double-delivery when
        // the event comes back through the Redis subscriber loop.
        state.mark_local_event(&event.id);

        if let Err(e) = state.pubsub.publish_event(ch_id, &event).await {
            state.local_event_ids.invalidate(&event.id.to_bytes());
            warn!(conn_id = %conn_id, event_id = %event_id_hex, "Ephemeral publish failed: {e}");
        }

        // Direct fan-out to local WS subscribers.
        // Pass the channel_id so fan_out() uses the channel-kind index.
        let stored_event = StoredEvent::new(event.clone(), Some(ch_id));
        let matches = state.sub_registry.fan_out(&stored_event);
        metrics::histogram!("buzz_fanout_recipients").record(matches.len() as f64);
        let event_json = serde_json::to_string(&event)
            .expect("nostr::Event serialization is infallible for well-formed events");
        let mut drop_count = 0u32;
        for (target_conn_id, sub_id) in &matches {
            let msg = format!(r#"["EVENT","{}",{}]"#, sub_id, event_json);
            if !state.conn_manager.send_to(*target_conn_id, msg) {
                drop_count += 1;
            }
        }
        if drop_count > 0 {
            tracing::warn!(
                event_id = %event_id_hex,
                drop_count,
                "fan-out: {drop_count} connection(s) cancelled due to full/closed buffers"
            );
        }
    } else {
        // Channel-less ephemeral events (e.g., NIP-AB pairing kind:24134).
        //
        // Sentinel pattern: we use `Uuid::nil()` (all-zeros UUID) as a
        // "global channel" routing key in Redis pub/sub. This lets other relay
        // nodes receive and fan out these events without any real channel_id.
        // The nil UUID is ONLY a Redis routing key — it never reaches the DB.
        // On the receiving end (main.rs subscriber loop), `is_nil()` is checked
        // and converted back to `None` so `fan_out()` uses the global index.
        state.mark_local_event(&event.id);

        if let Err(e) = state.pubsub.publish_event(uuid::Uuid::nil(), &event).await {
            state.local_event_ids.invalidate(&event.id.to_bytes());
            warn!(conn_id = %conn_id, event_id = %event_id_hex, "Ephemeral global publish failed: {e}");
        }

        // Direct fan-out to local WS subscribers.
        // Pass channel_id=None so fan_out() uses the global subscriber index.
        let stored_event = StoredEvent::new(event.clone(), None);
        let matches = state.sub_registry.fan_out(&stored_event);
        metrics::histogram!("buzz_fanout_recipients").record(matches.len() as f64);
        let event_json = serde_json::to_string(&event)
            .expect("nostr::Event serialization is infallible for well-formed events");
        let mut drop_count = 0u32;
        for (target_conn_id, sub_id) in &matches {
            let msg = format!(r#"["EVENT","{}",{}]"#, sub_id, event_json);
            if !state.conn_manager.send_to(*target_conn_id, msg) {
                drop_count += 1;
            }
        }
        if drop_count > 0 {
            tracing::warn!(
                event_id = %event_id_hex,
                drop_count,
                "fan-out: {drop_count} connection(s) cancelled due to full/closed buffers"
            );
        }
    }

    conn.send(RelayMessage::ok(event_id_hex, true, ""));
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AgentObserverDirection {
    Telemetry,
    Control,
}

#[derive(Debug, Clone, Copy)]
struct AgentObserverRoute {
    agent: PublicKey,
    owner: PublicKey,
    direction: AgentObserverDirection,
}

/// Handle encrypted agent observer frames (kind 24200).
///
/// These frames bypass storage and are routed as global ephemeral events. The
/// relay gates publication by the existing `agent_owner_pubkey` mapping and
/// gates subscription in the REQ handler via the cleartext `p` tag.
async fn handle_agent_observer_event(
    event: Event,
    conn_id: uuid::Uuid,
    event_id_hex: &str,
    conn: Arc<ConnectionState>,
    state: Arc<AppState>,
) {
    let event_clone = event.clone();
    let verify_result = tokio::task::spawn_blocking(move || verify_event(&event_clone)).await;
    match verify_result {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            conn.send(RelayMessage::ok(
                event_id_hex,
                false,
                &format!("invalid: {e}"),
            ));
            return;
        }
        Err(_) => {
            conn.send(RelayMessage::ok(
                event_id_hex,
                false,
                "error: internal error",
            ));
            return;
        }
    }

    // Freshness check: reject observer frames with stale/future timestamps
    let now = chrono::Utc::now().timestamp();
    let event_ts = event.created_at.as_secs() as i64;
    if (event_ts - now).unsigned_abs() > 300 {
        conn.send(RelayMessage::ok(
            event_id_hex,
            false,
            "invalid: observer frame timestamp outside ±5 minute freshness window",
        ));
        return;
    }

    let route = match agent_observer_route(&event) {
        Ok(Some(route)) => route,
        Ok(None) => {
            // Unknown frame value — silently drop, no error to publisher.
            conn.send(RelayMessage::ok(event_id_hex, true, ""));
            return;
        }
        Err(message) => {
            reject("invalid");
            conn.send(RelayMessage::ok(event_id_hex, false, &message));
            return;
        }
    };

    // Fast path: if this connection authenticated via NIP-OA and the verified
    // owner matches the observer frame's target owner, skip the DB lookup entirely.
    let session_owner_match = {
        let auth = conn.auth_state.read().await;
        if let crate::connection::AuthState::Authenticated(ctx) = &*auth {
            ctx.agent_owner_pubkey.as_ref() == Some(&route.owner)
        } else {
            false
        }
    };

    let agent_bytes = route.agent.to_bytes().to_vec();
    let owner_bytes = route.owner.to_bytes().to_vec();
    let cache_key = (agent_bytes.clone(), owner_bytes.clone());
    let is_owner = if session_owner_match {
        true
    } else {
        match state.observer_owner_cache.get(&cache_key) {
            Some(cached) => cached,
            None => {
                let result = state.db.is_agent_owner(&agent_bytes, &owner_bytes).await;
                match result {
                    Ok(v) => {
                        state.observer_owner_cache.insert(cache_key, v);
                        v
                    }
                    Err(e) => {
                        warn!(conn_id = %conn_id, event_id = %event_id_hex, "agent observer owner check failed: {e}");
                        conn.send(RelayMessage::ok(
                            event_id_hex,
                            false,
                            "error: internal server error",
                        ));
                        return;
                    }
                }
            }
        }
    };
    if !is_owner {
        reject("auth");
        conn.send(RelayMessage::ok(
            event_id_hex,
            false,
            "restricted: observer frame is not authorized for this agent owner",
        ));
        return;
    }

    // Rate limit telemetry frames only (100/sec per agent).
    // Control frames (owner → agent) bypass the limiter — they are rare and must not
    // be starved by bursty telemetry from the agent.
    if matches!(route.direction, AgentObserverDirection::Telemetry) {
        let agent_key: [u8; 32] = agent_bytes.as_slice().try_into().unwrap_or([0u8; 32]);
        let now = std::time::Instant::now();
        let mut entry = state
            .observer_rate_limiter
            .entry(agent_key)
            .or_insert((0, now));
        let (count, window_start) = entry.value_mut();
        if now.duration_since(*window_start).as_secs() >= 1 {
            *count = 1;
            *window_start = now;
        } else {
            *count += 1;
            if *count > 100 {
                conn.send(RelayMessage::ok(
                    event_id_hex,
                    false,
                    "rate-limited: observer frame rate exceeded (100/sec per agent)",
                ));
                return;
            }
        }
    }

    let event_json = match serde_json::to_string(&event) {
        Ok(json) => json,
        Err(e) => {
            error!(event_id = %event_id_hex, "Failed to serialize agent observer event: {e}");
            conn.send(RelayMessage::ok(
                event_id_hex,
                false,
                "error: internal server error",
            ));
            return;
        }
    };

    state.mark_local_event(&event.id);
    if let Err(e) = state.pubsub.publish_event(uuid::Uuid::nil(), &event).await {
        state.local_event_ids.invalidate(&event.id.to_bytes());
        warn!(conn_id = %conn_id, event_id = %event_id_hex, "Agent observer publish failed: {e}");
    }

    let stored_event = StoredEvent::new(event.clone(), None);
    let matches = state.sub_registry.fan_out(&stored_event);
    metrics::histogram!("buzz_fanout_recipients").record(matches.len() as f64);
    debug!(
        event_id = %event_id_hex,
        agent = %route.agent.to_hex(),
        owner = %route.owner.to_hex(),
        direction = ?route.direction,
        match_count = matches.len(),
        "Agent observer fan-out"
    );

    let mut drop_count = 0u32;
    for (target_conn_id, sub_id) in &matches {
        let msg = format!(r#"["EVENT","{}",{}]"#, sub_id, event_json);
        if !state.conn_manager.send_to(*target_conn_id, msg) {
            drop_count += 1;
        }
    }
    if drop_count > 0 {
        tracing::warn!(
            event_id = %event_id_hex,
            drop_count,
            "agent observer fan-out: {drop_count} connection(s) cancelled due to full/closed buffers"
        );
    }

    conn.send(RelayMessage::ok(event_id_hex, true, ""));
}

fn agent_observer_route(event: &Event) -> Result<Option<AgentObserverRoute>, String> {
    if !content_looks_like_nip44(&event.content) {
        return Err("invalid: observer content must be NIP-44 encrypted".into());
    }

    let recipient = parse_single_pubkey_tag(event, "p")?;
    let agent = parse_single_pubkey_tag(event, OBSERVER_AGENT_TAG)?;
    let frame = single_tag_content(event, OBSERVER_FRAME_TAG)?;

    let (owner, direction, expected_frame) = if event.pubkey == agent && recipient != agent {
        (
            recipient,
            AgentObserverDirection::Telemetry,
            OBSERVER_FRAME_TELEMETRY,
        )
    } else if recipient == agent && event.pubkey != agent {
        (
            event.pubkey,
            AgentObserverDirection::Control,
            OBSERVER_FRAME_CONTROL,
        )
    } else {
        return Err(
            "invalid: observer frame must be agent-to-owner telemetry or owner-to-agent control"
                .into(),
        );
    };

    if frame != expected_frame {
        // Unknown frame value — silently drop without notifying the publisher.
        return Ok(None);
    }

    Ok(Some(AgentObserverRoute {
        agent,
        owner,
        direction,
    }))
}

fn parse_single_pubkey_tag(event: &Event, tag_name: &str) -> Result<PublicKey, String> {
    let value = single_tag_content(event, tag_name)?;
    PublicKey::from_hex(value)
        .map_err(|_| format!("invalid: observer {tag_name} tag must be a hex pubkey"))
}

fn single_tag_content<'a>(event: &'a Event, tag_name: &str) -> Result<&'a str, String> {
    let mut values = event
        .tags
        .iter()
        .filter(|tag| tag.kind().to_string() == tag_name)
        .filter_map(|tag| tag.content());
    let Some(value) = values.next() else {
        return Err(format!("invalid: observer frame missing {tag_name} tag"));
    };
    if values.next().is_some() {
        return Err(format!(
            "invalid: observer frame has multiple {tag_name} tags"
        ));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use buzz_core::kind::{
        KIND_AGENT_OBSERVER_FRAME, KIND_CANVAS, KIND_FORUM_COMMENT, KIND_FORUM_POST,
        KIND_FORUM_VOTE, KIND_PRESENCE_UPDATE, KIND_STREAM_MESSAGE, KIND_STREAM_MESSAGE_DIFF,
    };
    use buzz_core::observer::{
        encrypt_observer_payload, OBSERVER_AGENT_TAG, OBSERVER_FRAME_CONTROL, OBSERVER_FRAME_TAG,
        OBSERVER_FRAME_TELEMETRY,
    };
    use nostr::{EventBuilder, Keys, Kind, Tag};

    #[test]
    fn channel_scoped_content_kinds_require_h_tags() {
        for kind in [
            KIND_STREAM_MESSAGE,
            KIND_STREAM_MESSAGE_DIFF,
            KIND_CANVAS,
            KIND_FORUM_POST,
            KIND_FORUM_VOTE,
            KIND_FORUM_COMMENT,
        ] {
            assert!(
                super::super::ingest::requires_h_channel_scope(kind),
                "kind {kind} should require h"
            );
        }
    }

    #[test]
    fn non_channel_kinds_do_not_require_h_tags() {
        assert!(
            !super::super::ingest::requires_h_channel_scope(nostr::Kind::Reaction.as_u16().into()),
            "reactions derive channel from the target event"
        );
        assert!(
            !super::super::ingest::requires_h_channel_scope(KIND_PRESENCE_UPDATE),
            "presence updates are global/ephemeral"
        );
    }

    #[test]
    fn agent_observer_route_accepts_agent_to_owner_telemetry() {
        let agent = Keys::generate();
        let owner = Keys::generate();
        let encrypted = encrypt_observer_payload(
            &agent,
            &owner.public_key(),
            &serde_json::json!({"type": "acp_read"}),
        )
        .expect("encrypt observer payload");
        let event = EventBuilder::new(Kind::Custom(KIND_AGENT_OBSERVER_FRAME as u16), encrypted)
            .tags([
                Tag::parse(["p", &owner.public_key().to_hex()]).expect("p tag"),
                Tag::parse([OBSERVER_AGENT_TAG, &agent.public_key().to_hex()]).expect("agent tag"),
                Tag::parse([OBSERVER_FRAME_TAG, OBSERVER_FRAME_TELEMETRY]).expect("frame tag"),
            ])
            .sign_with_keys(&agent)
            .expect("sign event");

        let route = super::agent_observer_route(&event)
            .expect("observer route")
            .expect("route should be Some");
        assert_eq!(route.agent, agent.public_key());
        assert_eq!(route.owner, owner.public_key());
        assert_eq!(route.direction, super::AgentObserverDirection::Telemetry);
    }

    #[test]
    fn agent_observer_route_accepts_owner_to_agent_control() {
        let agent = Keys::generate();
        let owner = Keys::generate();
        let encrypted = encrypt_observer_payload(
            &owner,
            &agent.public_key(),
            &serde_json::json!({"type": "cancel_turn"}),
        )
        .expect("encrypt observer payload");
        let event = EventBuilder::new(Kind::Custom(KIND_AGENT_OBSERVER_FRAME as u16), encrypted)
            .tags([
                Tag::parse(["p", &agent.public_key().to_hex()]).expect("p tag"),
                Tag::parse([OBSERVER_AGENT_TAG, &agent.public_key().to_hex()]).expect("agent tag"),
                Tag::parse([OBSERVER_FRAME_TAG, OBSERVER_FRAME_CONTROL]).expect("frame tag"),
            ])
            .sign_with_keys(&owner)
            .expect("sign event");

        let route = super::agent_observer_route(&event)
            .expect("observer route")
            .expect("route should be Some");
        assert_eq!(route.agent, agent.public_key());
        assert_eq!(route.owner, owner.public_key());
        assert_eq!(route.direction, super::AgentObserverDirection::Control);
    }

    #[test]
    fn agent_observer_route_rejects_plaintext_content() {
        let agent = Keys::generate();
        let owner = Keys::generate();
        let event = EventBuilder::new(
            Kind::Custom(KIND_AGENT_OBSERVER_FRAME as u16),
            "not encrypted",
        )
        .tags([
            Tag::parse(["p", &owner.public_key().to_hex()]).expect("p tag"),
            Tag::parse([OBSERVER_AGENT_TAG, &agent.public_key().to_hex()]).expect("agent tag"),
            Tag::parse([OBSERVER_FRAME_TAG, OBSERVER_FRAME_TELEMETRY]).expect("frame tag"),
        ])
        .sign_with_keys(&agent)
        .expect("sign event");

        let err = super::agent_observer_route(&event).expect_err("route should reject plaintext");
        assert!(err.contains("NIP-44"));
    }

    mod fanout_access {
        use std::collections::HashMap;
        use std::sync::atomic::AtomicU8;
        use std::sync::Arc;

        use buzz_core::StoredEvent;
        use nostr::{EventBuilder, Keys, Kind};
        use tokio::sync::{mpsc, Mutex};
        use tokio_util::sync::CancellationToken;
        use uuid::Uuid;

        use crate::handlers::event::filter_fanout_by_access;
        use crate::state::AppState;

        fn test_config() -> crate::config::Config {
            let mut config = crate::config::Config::from_env().expect("default config loads");
            config.require_relay_membership = false;
            config.redis_url = "redis://127.0.0.1:1".to_string();
            config
        }

        async fn test_state() -> Arc<AppState> {
            let config = test_config();
            let pool = sqlx::PgPool::connect_lazy(&config.database_url).expect("lazy pg pool");
            let db = buzz_db::Db::from_pool(pool.clone());
            let redis_pool = deadpool_redis::Config::from_url(&config.redis_url)
                .create_pool(Some(deadpool_redis::Runtime::Tokio1))
                .expect("redis pool");
            let pubsub = Arc::new(
                buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
                    .await
                    .expect("pubsub manager"),
            );
            let audit = buzz_audit::AuditService::new(pool);
            let auth = buzz_auth::AuthService::new(config.auth.clone());
            let search = buzz_search::SearchService::new(buzz_search::SearchConfig {
                url: config.typesense_url.clone(),
                api_key: config.typesense_key.clone(),
                collection: "events".to_string(),
            });
            let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
                db.clone(),
                buzz_workflow::WorkflowConfig::default(),
            ));
            let media_storage =
                buzz_media::MediaStorage::new(&config.media).expect("media storage");
            let (state, _audit_shutdown) = AppState::new(
                config,
                db,
                redis_pool,
                audit,
                pubsub,
                auth,
                search,
                workflow_engine,
                Keys::generate(),
                media_storage,
            );
            Arc::new(state)
        }

        fn register_conn(state: &AppState, pubkey: Option<Vec<u8>>) -> Uuid {
            let conn_id = Uuid::new_v4();
            let (tx, _rx) = mpsc::channel(1);
            state.conn_manager.register(
                conn_id,
                tx,
                CancellationToken::new(),
                Arc::new(AtomicU8::new(0)),
                Arc::new(Mutex::new(HashMap::new())),
            );
            if let Some(pk) = pubkey {
                state.conn_manager.set_authenticated_pubkey(conn_id, pk);
            }
            conn_id
        }

        fn channel_event(channel_id: Option<Uuid>) -> StoredEvent {
            let event = EventBuilder::new(Kind::Custom(9), "{}")
                .sign_with_keys(&Keys::generate())
                .expect("sign event");
            StoredEvent::new(event, channel_id)
        }

        #[tokio::test]
        async fn channel_less_event_passes_through() {
            let state = test_state().await;
            let conn = register_conn(&state, Some(vec![1u8; 32]));
            let matches = vec![(conn, "s".to_string())];
            let out = filter_fanout_by_access(&state, &channel_event(None), matches.clone()).await;
            assert_eq!(out, matches);
        }

        #[tokio::test]
        async fn open_channel_event_passes_through_unfiltered() {
            let state = test_state().await;
            let channel_id = Uuid::new_v4();
            state
                .channel_visibility_cache
                .insert(channel_id, "open".to_string());
            // A connection with no authenticated pubkey would be dropped on a
            // private channel; on open it must pass untouched.
            let conn = register_conn(&state, None);
            let matches = vec![(conn, "s".to_string())];
            let out =
                filter_fanout_by_access(&state, &channel_event(Some(channel_id)), matches.clone())
                    .await;
            assert_eq!(out, matches);
        }

        #[tokio::test]
        async fn private_channel_keeps_member_drops_non_member_and_unknown() {
            let state = test_state().await;
            let channel_id = Uuid::new_v4();
            state
                .channel_visibility_cache
                .insert(channel_id, "private".to_string());

            let member_pk = vec![1u8; 32];
            let non_member_pk = vec![2u8; 32];
            state
                .membership_cache
                .insert((channel_id, member_pk.clone()), true);
            state
                .membership_cache
                .insert((channel_id, non_member_pk.clone()), false);

            let member = register_conn(&state, Some(member_pk));
            let non_member = register_conn(&state, Some(non_member_pk));
            let unauthed = register_conn(&state, None);

            let matches = vec![
                (member, "m".to_string()),
                (non_member, "n".to_string()),
                (unauthed, "u".to_string()),
            ];
            let out =
                filter_fanout_by_access(&state, &channel_event(Some(channel_id)), matches).await;
            assert_eq!(out, vec![(member, "m".to_string())]);
        }
    }
}
