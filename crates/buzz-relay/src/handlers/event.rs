//! EVENT handler — WS dispatcher → ingest pipeline → fan-out.

use std::{collections::HashMap, sync::Arc};

use axum::body::Bytes;
use tracing::{debug, error, info, warn};

use buzz_core::event::StoredEvent;
use buzz_core::kind::{
    event_kind_u32, is_ephemeral, AUTHOR_ONLY_KINDS, KIND_AGENT_OBSERVER_FRAME, KIND_GIFT_WRAP,
    KIND_MESH_CONNECT_REQUEST, KIND_MESH_STATUS_REPORT, KIND_PRESENCE_UPDATE,
};
use buzz_core::observer::{
    content_looks_like_nip44, OBSERVER_AGENT_TAG, OBSERVER_FRAME_CONTROL, OBSERVER_FRAME_TAG,
    OBSERVER_FRAME_TELEMETRY,
};
use buzz_core::tenant::TenantContext;
use buzz_core::verification::verify_event;
use buzz_core::CommunityId;
use buzz_pubsub::EventTopic;
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
        30023 | 30078 | 30174 | 30175..=30177 | 30300 | 30315 | 31234 | 39000..=39003 => {
            kind.to_string()
        }
        40002..=40100 => kind.to_string(),
        41001 | 41010..=41012 => kind.to_string(),
        43001..=43006 => kind.to_string(),
        44100..=44101 | 44200 => kind.to_string(),
        45001..=45003 => kind.to_string(),
        46001..=46012 | 46020 | 46030..=46031 => kind.to_string(),
        48001 | 48100..=48103 | 48106 => kind.to_string(),
        49001 => kind.to_string(),
        _ => "other".to_string(),
    }
}

fn event_frame_for_sub(sub_id: &str, event_json: &str) -> String {
    format!(r#"["EVENT","{}",{}]"#, sub_id, event_json)
}

fn event_frame_bytes_for_sub(sub_id: &str, event_json: &str) -> Arc<Bytes> {
    Arc::new(Bytes::from(event_frame_for_sub(sub_id, event_json)))
}

fn fanout_frame_cache<'a, I>(sub_ids: I, event_json: &str) -> HashMap<&'a str, Arc<Bytes>>
where
    I: IntoIterator<Item = &'a str>,
{
    let mut frames = HashMap::new();
    for sub_id in sub_ids {
        frames
            .entry(sub_id)
            .or_insert_with(|| event_frame_bytes_for_sub(sub_id, event_json));
    }
    frames
}

fn send_fanout_frames<'a, I>(
    state: &AppState,
    recipients: I,
    frames: &HashMap<&'a str, Arc<Bytes>>,
) -> u32
where
    I: IntoIterator<Item = (crate::subscription::ConnId, &'a str)>,
{
    let mut drop_count = 0u32;
    for (conn_id, sub_id) in recipients {
        let frame = frames
            .get(sub_id)
            .expect("fan-out frame cache covers every recipient subscription id");
        if !state
            .conn_manager
            .send_to_text_bytes(conn_id, Arc::clone(frame))
        {
            drop_count += 1;
        }
    }
    drop_count
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
///
/// `threaded` is an optional visibility read resolved earlier in the same
/// request (E1 phase-2, §4.8 phase-2 addendum). It is consulted only when its
/// `(community_id, channel_id)` exactly match this fan-out's — a mismatched or
/// absent bundle falls back to the fresh fail-closed lookup below, never to
/// "assume open". Membership checks stay fresh either way; the threaded value
/// only replaces the visibility SELECT.
pub async fn filter_fanout_by_access(
    state: &AppState,
    community_id: CommunityId,
    stored_event: &StoredEvent,
    matches: Vec<(crate::subscription::ConnId, crate::subscription::SubId)>,
    threaded: Option<&crate::state::ThreadedChannelVisibility>,
) -> Vec<(crate::subscription::ConnId, crate::subscription::SubId)> {
    // First enforce the receiver-side tenant label. Subscription indexes are
    // community-scoped, but stale/injected matches and future fan-out helpers
    // must still fail closed at the send chokepoint: a connection bound to
    // community A may never receive an event labelled community B.
    let matches: Vec<_> = matches
        .into_iter()
        .filter(|(conn_id, _)| {
            state.conn_manager.community_for_conn(*conn_id) == Some(community_id)
        })
        .collect();

    // Author-only kinds (NIP-ER reminders, NIP-37 draft wraps) may only ever
    // be delivered to the event's own author. This gate lives here — the
    // chokepoint shared by the ingest fan-out path and the Redis cross-node
    // `subscribe_local` path, the only paths that route author-only kinds —
    // so no such delivery can bypass it. It runs before (and independent of)
    // the channel-membership filter below.
    let matches = if AUTHOR_ONLY_KINDS.contains(&event_kind_u32(&stored_event.event)) {
        let author = stored_event.event.pubkey.to_bytes();
        matches
            .into_iter()
            .filter(|(conn_id, _)| {
                state
                    .conn_manager
                    .pubkey_for_conn(*conn_id)
                    .is_some_and(|pk| pk == author)
            })
            .collect()
    } else {
        matches
    };

    let Some(channel_id) = stored_event.channel_id else {
        return matches;
    };
    // Fence 3 (§4.8 phase-2): the threaded value is used only when it was
    // resolved under exactly this (community_id, channel_id); anything else
    // falls through to the fresh lookup. Fence 1: absence of a usable threaded
    // value is never "open" — it is the same fail-closed path as before.
    let visibility = match threaded {
        Some(t) if t.community_id == community_id && t.channel_id == channel_id => {
            Ok(t.visibility.clone())
        }
        _ => {
            state
                .channel_visibility_cached(community_id, channel_id, None)
                .await
        }
    };
    match visibility {
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
        match state
            .is_member_cached(community_id, channel_id, &pubkey)
            .await
        {
            Ok(true) => allowed.push((conn_id, sub_id)),
            Ok(false) => {}
            Err(e) => {
                warn!(%channel_id, "fan-out access filter: membership lookup failed: {e}");
            }
        }
    }
    allowed
}

/// Deliver one event to this relay's local subscribers through the access gate.
///
/// This is the single guarded send path for relay-local EVENT delivery. It runs
/// `fan_out()` to find matching subscriptions, then `filter_fanout_by_access()`
/// to drop recipients without access (private-channel non-members, author-only
/// kinds delivered to non-authors), then writes the EVENT frames. The invariant
/// it enforces: a registered subscription is never sufficient for delivery —
/// delivery always revalidates access on the sending pod, so a stale
/// subscription surviving a membership/visibility change (e.g. after an
/// open→private flip or a cross-pod cache lag) cannot leak events.
///
/// All relay-local live fan-out routes through here. The two exceptions are
/// `dispatch_persistent_event` (persistent ingest) and `fan_out_pubsub_event`
/// (Redis cross-node), which call `filter_fanout_by_access` inline: the former
/// layers an additional per-recipient DM-visibility-owner gate on top, the
/// latter skips local echoes — both are equivalent to this helper plus their
/// own extra step.
pub(crate) async fn fan_out_event_to_local_subscribers(
    state: &AppState,
    community_id: CommunityId,
    stored: &StoredEvent,
) {
    let matches = state.sub_registry.fan_out_scoped(community_id, stored);
    let matches = filter_fanout_by_access(state, community_id, stored, matches, None).await;
    metrics::histogram!("buzz_fanout_recipients").record(matches.len() as f64);
    if matches.is_empty() {
        return;
    }

    let event_json = match serde_json::to_string(&stored.event) {
        Ok(json) => json,
        Err(e) => {
            error!(event_id = %stored.event.id.to_hex(), "Failed to serialize event for fan-out: {e}");
            return;
        }
    };
    let frames = fanout_frame_cache(
        matches.iter().map(|(_, sub_id)| sub_id.as_str()),
        &event_json,
    );
    let drop_count = send_fanout_frames(
        state,
        matches
            .iter()
            .map(|(conn_id, sub_id)| (*conn_id, sub_id.as_str())),
        &frames,
    );
    if drop_count > 0 {
        tracing::warn!(
            event_id = %stored.event.id.to_hex(),
            drop_count,
            "fan-out: {drop_count} connection(s) cancelled due to full/closed buffers"
        );
    }
}

/// Fan out one event received from Redis pub/sub to this relay's local subscribers.
#[tracing::instrument(skip_all)]
pub async fn fan_out_pubsub_event(state: &Arc<AppState>, channel_event: buzz_pubsub::ChannelEvent) {
    // The Redis topic carries the tenant-local routing scope explicitly:
    // `Channel(id)` for a per-channel event, `Global` for a channel-less one.
    // Convert back to the `Option<Uuid>` channel id `fan_out()` indexes on —
    // `Global` selects the global subscriber index.
    let channel_id = match channel_event.topic {
        buzz_pubsub::EventTopic::Channel(id) => Some(id),
        buzz_pubsub::EventTopic::Global => None,
    };
    let community_id = channel_event.community_id;
    let stored = StoredEvent::new(channel_event.event, channel_id);

    // Skip events that were already fanned out in-process (local echo). The
    // dedup key is `(community_id, event_id)` — a same-id event arriving for a
    // *different* community is a distinct delivery and must not be suppressed.
    // The cache has TTL-based eviction (60s) so entries are bounded regardless
    // of subscriber health.
    let event_id_bytes = stored.event.id.to_bytes();
    let echo_key = (community_id, event_id_bytes);
    if state.local_event_ids.get(&echo_key).is_some() {
        state.local_event_ids.invalidate(&echo_key);
        return;
    }

    let matches = state.sub_registry.fan_out_scoped(community_id, &stored);
    let matches = filter_fanout_by_access(state, community_id, &stored, matches, None).await;
    metrics::counter!("buzz_multinode_fanout_total").increment(1);
    if matches.is_empty() {
        return;
    }

    let event_json = match serde_json::to_string(&stored.event) {
        Ok(json) => json,
        Err(e) => {
            tracing::error!("Failed to serialize event for multi-node fan-out: {e}");
            return;
        }
    };
    let frames = fanout_frame_cache(
        matches.iter().map(|(_, sub_id)| sub_id.as_str()),
        &event_json,
    );
    let drop_count = send_fanout_frames(
        state,
        matches
            .iter()
            .map(|(conn_id, sub_id)| (*conn_id, sub_id.as_str())),
        &frames,
    );
    if drop_count > 0 {
        tracing::warn!(
            event_id = %stored.event.id.to_hex(),
            drop_count,
            "multi-node fan-out: {drop_count} connection(s) dropped"
        );
    }
}

/// Schedule post-commit delivery/side effects for a stored event.
///
/// This intentionally returns after only the bounded audit enqueue has completed:
/// NIP-01 `OK` means the event was durably accepted, not that Redis publish,
/// local fan-out, or workflow triggering have completed. Keeping audit enqueue on
/// the awaited path preserves the bounded-channel backpressure posture when the
/// audit DB is overloaded; the spawned task still runs the same guarded fan-out
/// path, Redis publish, `mark_local_event` echo dedupe, and delivery metrics as
/// the former inline path.
pub(crate) async fn dispatch_persistent_event(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    stored_event: &StoredEvent,
    kind_u32: u32,
    actor_pubkey_hex: &str,
    threaded_visibility: Option<crate::state::ThreadedChannelVisibility>,
) -> usize {
    let event_id_hex = stored_event.event.id.to_hex();
    enqueue_event_created_audit(
        tenant,
        state,
        stored_event,
        kind_u32,
        actor_pubkey_hex,
        &event_id_hex,
    )
    .await;

    let tenant = tenant.clone();
    let state = Arc::clone(state);
    let stored_event = stored_event.clone();
    let actor_pubkey_hex = actor_pubkey_hex.to_owned();

    metrics::counter!("buzz_post_commit_dispatch_scheduled_total").increment(1);
    tokio::spawn(async move {
        let recipients = dispatch_persistent_event_inner(
            &tenant,
            &state,
            &stored_event,
            kind_u32,
            &actor_pubkey_hex,
            false,
            threaded_visibility,
        )
        .await;
        debug!(
            event_id = %event_id_hex,
            recipients,
            "post-commit dispatch complete"
        );
    });

    0
}

/// Returns `true` when an event of `kind_u32` should trigger workspace-level
/// workflow dispatch.
///
/// This is the single canonical gate that `dispatch_persistent_event_inner`
/// uses.  Extracting it as a named function lets tests call the real predicate
/// rather than maintaining a parallel closure that can silently diverge.
///
/// Excluded from dispatch:
/// - Workflow-execution kinds (avoid re-triggering the engine on its own output)
/// - Command kinds (internal relay commands)
/// - Relay-signed workflow messages (tagged `buzz:workflow`)
/// - `KIND_GIFT_WRAP` (encrypted payloads — content is opaque)
/// - `AUTHOR_ONLY_KINDS` (NIP-37 draft wraps, NIP-ER reminders — private
///   per-user state that must never reach workspace-level automations)
pub(crate) fn should_dispatch_workflow(kind_u32: u32, is_relay_workflow_msg: bool) -> bool {
    !buzz_core::kind::is_workflow_execution_kind(kind_u32)
        && !buzz_core::kind::is_command_kind(kind_u32)
        && !is_relay_workflow_msg
        && kind_u32 != KIND_GIFT_WRAP
        // Author-only kinds (NIP-ER reminders, NIP-37 draft wraps) are private
        // per-user state that must not trigger workspace-level workflows.
        // AUTHOR_ONLY_KINDS.contains is the permanent guard at this seam.
        && !AUTHOR_ONLY_KINDS.contains(&kind_u32)
}

/// Run post-commit delivery/side effects for a stored event.
async fn dispatch_persistent_event_inner(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    stored_event: &StoredEvent,
    kind_u32: u32,
    actor_pubkey_hex: &str,
    enqueue_audit: bool,
    threaded_visibility: Option<crate::state::ThreadedChannelVisibility>,
) -> usize {
    // No `crate::conformance` emit here — the spec doesn't have a
    // separate fan-out action. Acceptance was already recorded at the
    // ingest seam (`crates/buzz-relay/src/handlers/ingest.rs`'s
    // WriteInsert/WriteInsertGlobal/WriteDuplicate emit). The fan-out
    // surfaces as `ReadMessageRows` observations on the subscriber side
    // (read seam in req.rs, emitted by the held-back read-seam diff).
    let event_id_hex = stored_event.event.id.to_hex();

    let topic = match stored_event.channel_id {
        Some(channel_id) => EventTopic::Channel(channel_id),
        None => EventTopic::Global,
    };
    state.mark_local_event(tenant.community(), &stored_event.event.id);
    if let Err(e) = state
        .pubsub
        .publish_event(tenant, topic, &stored_event.event)
        .await
    {
        state
            .local_event_ids
            .invalidate(&(tenant.community(), stored_event.event.id.to_bytes()));
        warn!(event_id = %event_id_hex, "Redis publish failed: {e}");
    }

    let matches = state
        .sub_registry
        .fan_out_scoped(tenant.community(), stored_event);
    let matches = filter_fanout_by_access(
        state,
        tenant.community(),
        stored_event,
        matches,
        threaded_visibility.as_ref(),
    )
    .await;
    metrics::histogram!("buzz_fanout_recipients").record(matches.len() as f64);
    debug!(
        event_id = %event_id_hex,
        channel_id = ?stored_event.channel_id,
        match_count = matches.len(),
        "Fan-out"
    );

    let event_json = match serde_json::to_string(&stored_event.event) {
        Ok(json) => json,
        Err(e) => {
            error!(event_id = %event_id_hex, "Failed to serialize event for fan-out: {e}");
            metrics::counter!("buzz_post_commit_dispatch_errors_total", "stage" => "serialize")
                .increment(1);
            return 0;
        }
    };
    // For viewer-private events (kind:30622 DM visibility, kind:44200 agent turn
    // metrics), live fan-out must reach only the owner — a kindless `ids:[…]`
    // subscription can otherwise match it. Pull paths (HTTP /query, WS historical)
    // are gated separately by reader_authorized_for_event.
    let owner_only_kind = kind_u32 == buzz_core::kind::KIND_DM_VISIBILITY
        || kind_u32 == buzz_core::kind::KIND_AGENT_TURN_METRIC;
    let private_event_owner: Option<String> = owner_only_kind
        .then(|| {
            let p = nostr::SingleLetterTag::lowercase(nostr::Alphabet::P);
            stored_event
                .event
                .tags
                .filter(nostr::TagKind::SingleLetter(p))
                .find_map(|t| t.content().map(|s| s.to_string()))
        })
        .flatten();
    // Author-only delivery gating (NIP-ER reminders) is enforced centrally in
    // filter_fanout_by_access, applied to `matches` above before this loop. The
    // DM visibility owner gate is an additional delivery fence, so build shared
    // frames only after applying it to the already access-filtered recipient set.
    let recipients: Vec<_> = matches
        .iter()
        .filter_map(|(target_conn_id, sub_id)| {
            if let Some(ref owner_hex) = private_event_owner {
                let is_owner = state
                    .conn_manager
                    .pubkey_for(*target_conn_id)
                    .is_some_and(|pk| hex::encode(pk) == *owner_hex);
                if !is_owner {
                    return None;
                }
            }
            Some((*target_conn_id, sub_id.as_str()))
        })
        .collect();
    let frames = fanout_frame_cache(recipients.iter().map(|(_, sub_id)| *sub_id), &event_json);
    let drop_count = send_fanout_frames(state, recipients, &frames);
    if drop_count > 0 {
        tracing::warn!(
            event_id = %event_id_hex,
            drop_count,
            "fan-out: {drop_count} connection(s) cancelled due to full/closed buffers"
        );
    }

    // Search indexing is no longer a separate worker step: under Postgres FTS
    // the searchable row IS the persisted event row (the `insert_event` write
    // populates the FTS column via a generated `tsvector`), so there is no
    // out-of-band index to feed. The old Typesense `index_event` worker and its
    // `search_index_tx` mpsc are gone with the Typesense backend.

    if enqueue_audit {
        enqueue_event_created_audit(
            tenant,
            state,
            stored_event,
            kind_u32,
            actor_pubkey_hex,
            &event_id_hex,
        )
        .await;
    }

    // Skip workflow triggering for workflow-execution kinds and relay-signed workflow messages.
    let is_relay_workflow_msg = stored_event.event.pubkey == state.relay_keypair.public_key()
        && stored_event
            .event
            .tags
            .iter()
            .any(|t| t.as_slice().first().map(|s| s.as_str()) == Some("buzz:workflow"));

    if should_dispatch_workflow(kind_u32, is_relay_workflow_msg) {
        let workflow_engine = Arc::clone(&state.workflow_engine);
        let workflow_event = stored_event.clone();
        let trigger_kind = kind_u32.to_string();
        let workflow_community_host = tenant.host().to_owned();
        // The event was stored under `tenant.community()`; `StoredEvent` does
        // not carry the community, so pass it explicitly. The same channel UUID
        // can exist in another community — scoping the workflow lookup to this
        // community keeps a colliding channel id in B from triggering A's
        // workflows.
        let workflow_community = tenant.community();
        tokio::spawn(async move {
            if let Err(e) = workflow_engine
                .on_event(workflow_community, &workflow_event)
                .await
            {
                tracing::error!(event_id = ?workflow_event.event.id, "Workflow trigger failed: {e}");
            } else {
                metrics::counter!(
                    "buzz_workflow_runs_total",
                    "trigger" => trigger_kind,
                    "community" => workflow_community_host
                )
                .increment(1);
            }
        });
    }

    matches.len()
}

async fn enqueue_event_created_audit(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    stored_event: &StoredEvent,
    kind_u32: u32,
    actor_pubkey_hex: &str,
    event_id_hex: &str,
) {
    // Audit via bounded channel (capacity 1000). Uses .send().await so entries
    // are never silently dropped — backpressure propagates to the event handler
    // if the queue is full. This is intentional: the audit advisory lock already
    // serializes writes (at most 1 in-flight), so a full queue means the audit
    // DB is genuinely overloaded and the relay should slow down rather than
    // accumulate unbounded in-memory state. DB write failures in the worker are
    // logged but not retried (same as the previous per-event tokio::spawn).
    let audit_entry = buzz_audit::NewAuditEntry {
        community_id: tenant.community(),
        action: buzz_audit::AuditAction::EventCreated,
        // Record the *actor* the caller resolved (authenticated principal for
        // ingest, triggering user for workflow posts), not `stored_event.event
        // .pubkey`. For relay-signed events (workflow sink, side-effect emits)
        // the claimed author is the relay key, so deriving from the event would
        // erase the human behind the action from the audit trail. This mirrors
        // the pre-rewrite semantics, ported to the raw-bytes column.
        actor_pubkey: hex::decode(actor_pubkey_hex).ok(),
        object_id: Some(event_id_hex.to_owned()),
        detail: serde_json::json!({
            "event_kind": kind_u32,
            "channel_id": stored_event.channel_id,
        }),
    };
    if let Err(e) = state.audit_tx.send(audit_entry).await {
        error!(event_id = %event_id_hex, "Audit channel closed — entry lost: {e}");
        metrics::counter!("buzz_audit_send_errors_total").increment(1);
    }
}

/// Handle an EVENT message from a WebSocket connection.
///
/// Extracts auth from the WS connection, dispatches ephemeral events locally,
/// and delegates persistent events to [`super::ingest::ingest_event`].
#[tracing::instrument(skip_all, fields(event_id, kind))]
pub async fn handle_event(event: Event, conn: Arc<ConnectionState>, state: Arc<AppState>) {
    let start = std::time::Instant::now();
    let event_id_hex = event.id.to_hex();
    let kind_u32 = event_kind_u32(&event);
    let kind_str = bounded_kind_label(kind_u32);

    // Record the declared span fields now that we have the values.
    tracing::Span::current()
        .record("event_id", event_id_hex.as_str())
        .record("kind", kind_u32);

    debug!(event_id = %event_id_hex, kind = kind_u32, "EVENT");
    // Fleet-wide received counter: kind-only, no community tag.
    // Rationale: bounded_kind_label passes through all 10k values in
    // 20000..=29999 (client-controlled ephemeral range). Crossing kind ×
    // community would produce up to millions of series. Keep kind fleet-wide.
    metrics::counter!("buzz_events_received_total", "kind" => kind_str.clone()).increment(1);
    // Per-community volume counter: community-only, no kind tag.
    // Use this for per-community throughput graphs; the fleet counter above
    // for per-kind breakdowns.
    metrics::counter!(
        "buzz_community_events_received_total",
        "community" => conn.tenant.host().to_owned()
    )
    .increment(1);

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

    // Must run before both ephemeral and persistent branches. Persistent
    // events get a second check inside ingest_event() (step 3), but
    // ephemeral events bypass the pipeline entirely.
    let is_gift_wrap = kind_u32 == KIND_GIFT_WRAP;
    if event.pubkey != auth_pubkey && !is_gift_wrap {
        reject("invalid");
        conn.send(RelayMessage::ok(
            &event_id_hex,
            false,
            "invalid: event pubkey does not match authenticated identity",
        ));
        return;
    }

    if kind_u32 == buzz_core::kind::KIND_AUTH {
        reject("invalid");
        conn.send(RelayMessage::ok(
            &event_id_hex,
            false,
            "invalid: AUTH events cannot be submitted via EVENT",
        ));
        return;
    }

    if kind_u32 == KIND_AGENT_OBSERVER_FRAME {
        if !scopes.is_empty() && !scopes.contains(&buzz_auth::Scope::MessagesWrite) {
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

    // Scope enforcement for ephemeral kinds: require MessagesWrite.
    // Persistent events skip this gate and rely on
    // ingest_event()'s per-kind scope allowlist instead, so a token with
    // only ChannelsWrite can still submit kind:9002 via WS.
    if is_ephemeral(kind_u32) {
        if !scopes.is_empty() && !scopes.contains(&buzz_auth::Scope::MessagesWrite) {
            reject("scope");
            conn.send(RelayMessage::ok(
                &event_id_hex,
                false,
                "restricted: insufficient scope for ephemeral events",
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

    let ingest_auth = IngestAuth::Nip42 {
        pubkey: auth_pubkey,
        scopes,
        channel_ids,
        conn_id,
    };

    match super::ingest::ingest_event(&state, &conn.tenant, event, ingest_auth).await {
        Ok(result) => {
            if result.accepted {
                // Fleet-wide stored counter: kind-only, no community tag.
                // Same cardinality rationale as buzz_events_received_total above.
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
            let _ = state
                .pubsub
                .clear_presence(&conn.tenant, &auth_pubkey)
                .await;
        } else {
            let _ = state
                .pubsub
                .set_presence(&conn.tenant, &auth_pubkey, &status)
                .await;
        }

        // Presence is a channel-less ephemeral event. After updating Redis
        // presence state, let it fall through to the shared global ephemeral
        // publish/fan-out path below so other relay nodes receive the live delta.
    }

    // Mesh status report (kind:24620). An authenticated relay member reports its
    // current mesh serve availability; the relay projects it into a relay-signed,
    // per-reporter kind:30621 discovery note. The report is ephemeral input; the
    // 30621 is the durable, relay-owned record.
    if event_kind_u32(&event) == KIND_MESH_STATUS_REPORT {
        let reporter_hex = auth_pubkey.to_hex();
        match super::mesh_signaling::handle_status_report(
            &state,
            &conn.tenant,
            &reporter_hex,
            &event,
        )
        .await
        {
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
        if super::mesh_signaling::connect_request_rate_limited(
            &state,
            conn.tenant.community(),
            &auth_pubkey,
        ) {
            conn.send(RelayMessage::ok(
                event_id_hex,
                false,
                "rate-limited: mesh connect request rate exceeded (20/sec)",
            ));
            return;
        }
        let requester_hex = auth_pubkey.to_hex();
        match super::mesh_signaling::handle_connect_request(
            &state,
            &conn.tenant,
            &requester_hex,
            &event,
        )
        .await
        {
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
        if let Err(msg) = super::ingest::check_channel_membership(
            &conn.tenant,
            &state,
            ch_id,
            &pubkey_bytes,
            None,
        )
        .await
        {
            conn.send(RelayMessage::ok(event_id_hex, false, &msg));
            return;
        }

        // Mark as local before Redis publish to prevent double-delivery when
        // the event comes back through the Redis subscriber loop.
        state.mark_local_event(conn.tenant.community(), &event.id);

        if let Err(e) = state
            .pubsub
            .publish_event(&conn.tenant, EventTopic::Channel(ch_id), &event)
            .await
        {
            state
                .local_event_ids
                .invalidate(&(conn.tenant.community(), event.id.to_bytes()));
            warn!(conn_id = %conn_id, event_id = %event_id_hex, "Ephemeral publish failed: {e}");
        }

        // Direct fan-out to local WS subscribers, through the guarded send path
        // so a stale subscription on a removed/non-member connection cannot
        // receive this private-channel ephemeral event.
        // Pass the channel_id so fan_out() uses the channel-kind index.
        let stored_event = StoredEvent::new(event.clone(), Some(ch_id));
        fan_out_event_to_local_subscribers(&state, conn.tenant.community(), &stored_event).await;
    } else {
        // Channel-less ephemeral events (e.g., NIP-AB pairing kind:24134).
        //
        // Sentinel pattern: we use `Uuid::nil()` (all-zeros UUID) as a
        // "global channel" routing key in Redis pub/sub. This lets other relay
        // nodes receive and fan out these events without any real channel_id.
        // The nil UUID is ONLY a Redis routing key — it never reaches the DB.
        // On the receiving end (main.rs subscriber loop), `is_nil()` is checked
        // and converted back to `None` so `fan_out()` uses the global index.
        state.mark_local_event(conn.tenant.community(), &event.id);

        if let Err(e) = state
            .pubsub
            .publish_event(&conn.tenant, EventTopic::Global, &event)
            .await
        {
            state
                .local_event_ids
                .invalidate(&(conn.tenant.community(), event.id.to_bytes()));
            warn!(conn_id = %conn_id, event_id = %event_id_hex, "Ephemeral global publish failed: {e}");
        }

        // Direct fan-out to local WS subscribers through the guarded send path.
        // Pass channel_id=None so fan_out() uses the global subscriber index;
        // filter_fanout_by_access no-ops for channel-less events except the
        // author-only-kind gate.
        let stored_event = StoredEvent::new(event.clone(), None);
        fan_out_event_to_local_subscribers(&state, conn.tenant.community(), &stored_event).await;
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

/// Check + bump the per-agent observer telemetry limit (100/sec window).
///
/// Observer frames are ephemeral, but the rejection is visible to the sender.
/// Scope the counter by community so an agent key active in one tenant does not
/// consume another tenant's logical rate budget.
fn observer_frame_rate_limited(
    state: &AppState,
    community_id: CommunityId,
    agent_key: [u8; 32],
) -> bool {
    let now = std::time::Instant::now();
    let mut entry = state
        .observer_rate_limiter
        .entry((community_id, agent_key))
        .or_insert((0, now));
    let (count, window_start) = entry.value_mut();
    if now.duration_since(*window_start).as_secs() >= 1 {
        *count = 1;
        *window_start = now;
        false
    } else {
        *count += 1;
        *count > 100
    }
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
                let result = state
                    .db
                    .is_agent_owner(conn.tenant.community(), &agent_bytes, &owner_bytes)
                    .await;
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
        if observer_frame_rate_limited(&state, conn.tenant.community(), agent_key) {
            conn.send(RelayMessage::ok(
                event_id_hex,
                false,
                "rate-limited: observer frame rate exceeded (100/sec per agent)",
            ));
            return;
        }
    }

    state.mark_local_event(conn.tenant.community(), &event.id);
    if let Err(e) = state
        .pubsub
        .publish_event(&conn.tenant, EventTopic::Global, &event)
        .await
    {
        state
            .local_event_ids
            .invalidate(&(conn.tenant.community(), event.id.to_bytes()));
        warn!(conn_id = %conn_id, event_id = %event_id_hex, "Agent observer publish failed: {e}");
    }

    let stored_event = StoredEvent::new(event.clone(), None);
    debug!(
        event_id = %event_id_hex,
        agent = %route.agent.to_hex(),
        owner = %route.owner.to_hex(),
        direction = ?route.direction,
        "Agent observer fan-out"
    );
    fan_out_event_to_local_subscribers(&state, conn.tenant.community(), &stored_event).await;

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
    use std::sync::Arc;

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
    fn fanout_event_frame_matches_legacy_format_byte_for_byte() {
        let sub_id = "sub-id";
        let event_json = r#"{"id":"abc","tags":[["p","target"]],"content":"hello"}"#;
        let expected = format!(r#"["EVENT","{}",{}]"#, sub_id, event_json);

        assert_eq!(super::event_frame_for_sub(sub_id, event_json), expected);
        assert_eq!(
            super::event_frame_bytes_for_sub(sub_id, event_json)
                .as_ref()
                .as_ref(),
            expected.as_bytes()
        );
    }

    #[test]
    fn fanout_frame_cache_reuses_frames_within_one_cycle_only() {
        let event_json = r#"{"id":"abc"}"#;
        let frames = super::fanout_frame_cache(["same", "other", "same"], event_json);

        assert_eq!(frames.len(), 2, "duplicate sub ids share one cached frame");
        assert_eq!(
            frames.get("same").expect("same frame").as_ref().as_ref(),
            format!(r#"["EVENT","same",{}]"#, event_json).as_bytes()
        );

        let next_cycle = super::fanout_frame_cache(["same"], event_json);
        assert!(
            !Arc::ptr_eq(
                frames.get("same").expect("same frame"),
                next_cycle.get("same").expect("same frame in next cycle")
            ),
            "fan-out frame sharing must not escape a single cycle"
        );
    }

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

    #[tokio::test]
    async fn observer_frame_rate_limiter_is_scoped_by_community() {
        let state = fanout_access::test_state().await;
        let agent_key = Keys::generate().public_key().to_bytes();
        let community_a = buzz_core::tenant::CommunityId::from_uuid(uuid::Uuid::from_u128(0xAAAA));
        let community_b = buzz_core::tenant::CommunityId::from_uuid(uuid::Uuid::from_u128(0xBBBB));

        for _ in 0..100 {
            assert!(!super::observer_frame_rate_limited(
                &state,
                community_a,
                agent_key
            ));
        }
        assert!(super::observer_frame_rate_limited(
            &state,
            community_a,
            agent_key
        ));
        assert!(
            !super::observer_frame_rate_limited(&state, community_b, agent_key),
            "A's exhausted budget must not rate-limit the same agent key in B"
        );
    }

    mod pubsub_fanout {
        use std::collections::HashMap;
        use std::sync::atomic::AtomicU8;
        use std::sync::Arc;

        use axum::extract::ws::Message;
        use buzz_core::kind::{KIND_MEMBER_ADDED_NOTIFICATION, KIND_PRESENCE_UPDATE};
        use buzz_pubsub::{ChannelEvent, EventTopic};
        use nostr::{EventBuilder, Filter, Keys, Kind};
        use tokio::sync::{mpsc, Mutex};
        use tokio_util::sync::CancellationToken;
        use uuid::Uuid;

        use crate::handlers::event::fan_out_pubsub_event;
        use crate::state::AppState;

        async fn test_state() -> Arc<AppState> {
            super::fanout_access::test_state().await
        }

        fn register_global_sub(
            state: &AppState,
            sub_id: &str,
            filter: Filter,
            pubkey: Option<Vec<u8>>,
        ) -> (Uuid, mpsc::Receiver<Message>) {
            let conn_id = Uuid::new_v4();
            let (tx, rx) = mpsc::channel(10);
            let (ctrl_tx, _ctrl_rx) = mpsc::channel(10);
            state.conn_manager.register(
                conn_id,
                tx,
                ctrl_tx,
                CancellationToken::new(),
                buzz_core::tenant::CommunityId::from_uuid(Uuid::nil()),
                Arc::new(AtomicU8::new(0)),
                Arc::new(Mutex::new(HashMap::new())),
                3,
            );
            if let Some(pubkey) = pubkey {
                state.conn_manager.set_authenticated_pubkey(conn_id, pubkey);
            }
            state
                .sub_registry
                .register(conn_id, sub_id.to_string(), vec![filter], None);
            (conn_id, rx)
        }

        fn register_presence_sub(
            state: &AppState,
            sub_id: &str,
        ) -> (Uuid, mpsc::Receiver<Message>) {
            register_global_sub(
                state,
                sub_id,
                Filter::new().kind(Kind::Custom(KIND_PRESENCE_UPDATE as u16)),
                None,
            )
        }

        fn register_membership_sub(
            state: &AppState,
            sub_id: &str,
            target: &Keys,
        ) -> (Uuid, mpsc::Receiver<Message>) {
            register_global_sub(
                state,
                sub_id,
                Filter::new()
                    .kind(Kind::Custom(KIND_MEMBER_ADDED_NOTIFICATION as u16))
                    .pubkey(target.public_key()),
                Some(target.public_key().to_bytes().to_vec()),
            )
        }

        fn presence_event(status: &str) -> nostr::Event {
            EventBuilder::new(Kind::Custom(KIND_PRESENCE_UPDATE as u16), status)
                .sign_with_keys(&Keys::generate())
                .expect("sign presence")
        }

        fn membership_event(target: &Keys, channel_id: Uuid) -> nostr::Event {
            EventBuilder::new(Kind::Custom(KIND_MEMBER_ADDED_NOTIFICATION as u16), "{}")
                .tags([
                    nostr::Tag::parse(["p", &target.public_key().to_hex()]).expect("p tag"),
                    nostr::Tag::parse(["h", &channel_id.to_string()]).expect("h tag"),
                ])
                .sign_with_keys(&Keys::generate())
                .expect("sign membership notification")
        }

        fn event_from_ws_message(msg: Message) -> nostr::Event {
            let Message::Text(text) = msg else {
                panic!("expected text ws message");
            };
            let v: serde_json::Value = serde_json::from_str(&text).expect("EVENT frame JSON");
            assert_eq!(v[0], "EVENT");
            serde_json::from_value(v[2].clone()).expect("nostr event")
        }

        #[tokio::test]
        async fn global_presence_pubsub_event_fans_out_to_local_subscribers() {
            let state = test_state().await;
            let (_conn_id, mut rx) = register_presence_sub(&state, "presence");
            let event = presence_event("online");
            let event_id = event.id;

            fan_out_pubsub_event(
                &state,
                ChannelEvent {
                    community_id: buzz_core::tenant::CommunityId::from_uuid(Uuid::nil()),
                    topic: EventTopic::Global,
                    event,
                },
            )
            .await;

            let delivered = event_from_ws_message(rx.try_recv().expect("presence delivered"));
            assert_eq!(delivered.id, event_id);
            assert!(rx.try_recv().is_err(), "presence is delivered once");
        }

        #[tokio::test]
        async fn local_echo_presence_pubsub_event_is_not_delivered_twice() {
            let state = test_state().await;
            let (_conn_id, mut rx) = register_presence_sub(&state, "presence");
            let event = presence_event("online");

            let community = buzz_core::tenant::CommunityId::from_uuid(Uuid::nil());
            state.mark_local_event(community, &event.id);
            fan_out_pubsub_event(
                &state,
                ChannelEvent {
                    community_id: community,
                    topic: EventTopic::Global,
                    event,
                },
            )
            .await;

            assert!(
                rx.try_recv().is_err(),
                "Redis echo of locally fanned-out presence must be suppressed"
            );
        }

        #[tokio::test]
        async fn local_echo_suppression_is_scoped_to_its_community() {
            // Non-interference: a local publish of event X in community A must
            // NOT suppress delivery of a *distinct* event sharing X's id arriving
            // via Redis for community B. The echo-dedup key is
            // `(community_id, event_id)`, so marking A/X leaves B/X deliverable.
            // Before this fix the cache was keyed on the bare event id, so an
            // action in A would silently drop B's same-id delivery for the TTL.
            let state = test_state().await;
            let (_conn_id, mut rx) = register_presence_sub(&state, "presence");

            // The subscriber registers under the nil community (see
            // `register_global_sub`), so B — the community whose delivery must
            // survive — is nil. A is a *foreign* community: a local mark there
            // must be irrelevant to B's fan-out.
            let community_a = buzz_core::tenant::CommunityId::from_uuid(Uuid::new_v4());
            let community_b = buzz_core::tenant::CommunityId::from_uuid(Uuid::nil());

            // Same Nostr event id, delivered for community B.
            let event = presence_event("online");
            let event_id = event.id;

            // A locally published this id; mark it for A only.
            state.mark_local_event(community_a, &event_id);

            // B's same-id event arrives via the Redis subscriber path.
            fan_out_pubsub_event(
                &state,
                ChannelEvent {
                    community_id: community_b,
                    topic: EventTopic::Global,
                    event,
                },
            )
            .await;

            let delivered = event_from_ws_message(
                rx.try_recv()
                    .expect("B's same-id event must be delivered — A's local mark is B-irrelevant"),
            );
            assert_eq!(delivered.id, event_id);
        }

        #[tokio::test]
        async fn global_membership_pubsub_event_fans_out_by_p_tag() {
            let state = test_state().await;
            let target = Keys::generate();
            let other = Keys::generate();
            let (_target_conn, mut target_rx) =
                register_membership_sub(&state, "membership-target", &target);
            let (_other_conn, mut other_rx) =
                register_membership_sub(&state, "membership-other", &other);
            let event = membership_event(&target, Uuid::new_v4());
            let event_id = event.id;

            fan_out_pubsub_event(
                &state,
                ChannelEvent {
                    community_id: buzz_core::tenant::CommunityId::from_uuid(Uuid::nil()),
                    topic: EventTopic::Global,
                    event,
                },
            )
            .await;

            let delivered = event_from_ws_message(
                target_rx
                    .try_recv()
                    .expect("target receives membership notification"),
            );
            assert_eq!(delivered.id, event_id);
            assert!(
                other_rx.try_recv().is_err(),
                "membership notification should only reach matching #p subscribers"
            );
        }

        async fn redis_url_if_available() -> Option<String> {
            let redis_url =
                std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
            let pool = deadpool_redis::Config::from_url(&redis_url)
                .create_pool(Some(deadpool_redis::Runtime::Tokio1))
                .ok()?;
            let mut conn = pool.get().await.ok()?;
            redis::cmd("PING")
                .query_async::<String>(&mut conn)
                .await
                .ok()?;
            Some(redis_url)
        }

        fn spawn_pubsub_fanout_loop(state: Arc<AppState>) -> tokio::task::JoinHandle<()> {
            let mut rx = state.pubsub.subscribe_local();
            tokio::spawn(async move {
                while let Ok(channel_event) = rx.recv().await {
                    fan_out_pubsub_event(&state, channel_event).await;
                }
            })
        }

        #[tokio::test]
        async fn redis_presence_publish_reaches_second_relay_and_suppresses_origin_echo() {
            let Some(redis_url) = redis_url_if_available().await else {
                eprintln!("skipping Redis round-trip presence fan-out test: Redis unavailable");
                return;
            };

            let origin = super::fanout_access::test_state_with_redis_url(&redis_url).await;
            let receiver = super::fanout_access::test_state_with_redis_url(&redis_url).await;

            let origin_subscriber = tokio::spawn(origin.pubsub.clone().run_subscriber());
            let receiver_subscriber = tokio::spawn(receiver.pubsub.clone().run_subscriber());
            let origin_fanout = spawn_pubsub_fanout_loop(origin.clone());
            let receiver_fanout = spawn_pubsub_fanout_loop(receiver.clone());

            let (_origin_conn, mut origin_rx) = register_presence_sub(&origin, "origin-presence");
            let (_receiver_conn, mut receiver_rx) =
                register_presence_sub(&receiver, "receiver-presence");

            // Under the community-scoped bus, Redis delivery is demand-driven:
            // a relay only PSUBSCRIBEs `buzz:{community}:global` after it retains
            // interest in that topic. Both relays share one explicit tenant and
            // retain Global before publishing — origin too, so the echo-
            // suppression assertion still exercises `mark_local_event` against a
            // relay that *is* subscribed (the case that matters).
            let tenant = buzz_core::tenant::TenantContext::resolved(
                buzz_core::tenant::CommunityId::from_uuid(Uuid::nil()),
                "test",
            );
            origin
                .pubsub
                .retain_topic(&tenant, EventTopic::Global)
                .await;
            receiver
                .pubsub
                .retain_topic(&tenant, EventTopic::Global)
                .await;

            // Match buzz-pubsub's own Redis round-trip test: give PSUBSCRIBE a
            // bounded moment to attach before publishing the single test event.
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;

            let event = presence_event("online");
            let event_id = event.id;
            origin.mark_local_event(tenant.community(), &event.id);
            origin
                .pubsub
                .publish_event(&tenant, EventTopic::Global, &event)
                .await
                .expect("publish presence through Redis");

            let delivered =
                tokio::time::timeout(std::time::Duration::from_secs(2), receiver_rx.recv())
                    .await
                    .expect("presence reached second relay")
                    .expect("receiver connection still open");
            let delivered = event_from_ws_message(delivered);
            assert_eq!(delivered.id, event_id);
            assert!(
                tokio::time::timeout(std::time::Duration::from_millis(100), receiver_rx.recv())
                    .await
                    .is_err(),
                "second relay receives the presence event exactly once"
            );
            assert!(
                tokio::time::timeout(std::time::Duration::from_millis(250), origin_rx.recv())
                    .await
                    .is_err(),
                "origin relay suppresses the Redis echo after local fan-out"
            );

            origin_subscriber.abort();
            receiver_subscriber.abort();
            origin_fanout.abort();
            receiver_fanout.abort();
        }

        /// Regression guard: the `EventCreated` audit entry must record the
        /// caller-resolved *actor*, not the stored event's claimed author. For
        /// relay-signed events (workflow posts, side-effect emits) the event
        /// author is the relay key, while the actor is the human who triggered
        /// the action — deriving the audit actor from `event.pubkey` would erase
        /// that human from the trail. This test signs the event with one key and
        /// passes a *different* actor hex, then asserts `audit_log.actor_pubkey`
        /// is the actor, not the signer.
        #[tokio::test]
        async fn audit_records_caller_actor_not_relay_signer_for_relay_signed_event() {
            use buzz_core::event::StoredEvent;
            use buzz_core::tenant::{CommunityId, TenantContext};

            let Some((state, audit_shutdown, pool)) = super::fanout_access::audit_state().await
            else {
                eprintln!("skipping audit-actor provenance test: Postgres/Redis unavailable");
                return;
            };

            // Seed a community so the audit_log FK is satisfiable.
            let community_uuid = Uuid::new_v4();
            let host = format!("audit-actor-test-{}.example", community_uuid.simple());
            sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
                .bind(community_uuid)
                .bind(&host)
                .execute(&pool)
                .await
                .expect("seed community");
            let tenant = TenantContext::resolved(CommunityId::from_uuid(community_uuid), host);

            // Relay-signed event tagged buzz:workflow so workflow triggering is
            // skipped; the signer is the RELAY key, distinct from the actor.
            let signer = &state.relay_keypair;
            let actor = Keys::generate();
            let actor_hex = actor.public_key().to_hex();
            assert_ne!(
                signer.public_key().to_hex(),
                actor_hex,
                "test precondition: relay signer must differ from actor"
            );
            let event = EventBuilder::new(Kind::from(KIND_PRESENCE_UPDATE as u16), "online")
                .tags([nostr::Tag::parse(["buzz:workflow", "true"]).expect("workflow tag")])
                .sign_with_keys(signer)
                .expect("sign relay event");
            let event_id_hex = event.id.to_hex();
            let stored = StoredEvent::new(event, None);

            super::super::dispatch_persistent_event_inner(
                &tenant,
                &state,
                &stored,
                KIND_PRESENCE_UPDATE,
                &actor_hex,
                true,
                None,
            )
            .await;

            // Flush the audit worker so the row is committed before we read it.
            audit_shutdown
                .drain(std::time::Duration::from_secs(5))
                .await;

            let actor_bytes: Vec<u8> = sqlx::query_scalar(
                "SELECT actor_pubkey FROM audit_log \
                 WHERE community_id = $1 AND object_id = $2",
            )
            .bind(community_uuid)
            .bind(&event_id_hex)
            .fetch_one(&pool)
            .await
            .expect("audit row written");

            assert_eq!(
                actor_bytes,
                actor.public_key().to_bytes().to_vec(),
                "audit must record the caller-supplied actor"
            );
            assert_ne!(
                actor_bytes,
                signer.public_key().to_bytes().to_vec(),
                "audit must NOT record the relay signer as the actor"
            );
        }

        /// Integrated isolation: a community resolved from the request's
        /// `TenantContext` at relay ingest lands in *that* community's audit
        /// chain and nothing else. This is the conformance `audit_log` row's
        /// "one chain per community" obligation proven through the *relay* path
        /// (`dispatch_persistent_event`), not just the direct `AuditService::log`
        /// call that `buzz_audit::service::tests::chains_are_independent_per_community`
        /// covers — it pins that the host→`TenantContext`→chain wiring keeps
        /// tenants isolated end-to-end. No WS-AUTH in the loop, so it is not
        /// blocked on the NIP-42 work: it drives the dispatch fn directly with
        /// two explicit tenants.
        #[tokio::test]
        async fn audit_chain_is_isolated_per_tenant_through_relay_ingest() {
            use buzz_audit::AuditService;
            use buzz_core::event::StoredEvent;
            use buzz_core::tenant::{CommunityId, TenantContext};

            let Some((state, audit_shutdown, pool)) = super::fanout_access::audit_state().await
            else {
                eprintln!("skipping audit isolation test: Postgres/Redis unavailable");
                return;
            };

            // Two communities on the same relay process / same Postgres.
            let mut tenants = Vec::new();
            for label in ["a", "b"] {
                let id = Uuid::new_v4();
                let host = format!("audit-iso-{label}-{}.example", id.simple());
                sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
                    .bind(id)
                    .bind(&host)
                    .execute(&pool)
                    .await
                    .expect("seed community");
                tenants.push((
                    id,
                    TenantContext::resolved(CommunityId::from_uuid(id), host),
                ));
            }
            let (a_id, tenant_a) = &tenants[0];
            let (b_id, tenant_b) = &tenants[1];

            // Ingest one event under each tenant. Each event is signed by an
            // arbitrary actor; the audit community comes from the *tenant*, not
            // the event — that is the property under test. The two events carry
            // distinct content so they get distinct ids: that is what makes the
            // cross-leak assertions below non-trivial (each id must appear only
            // in its own community's chain).
            let actor = Keys::generate();
            let actor_hex = actor.public_key().to_hex();
            let ingest = |tenant: &TenantContext, content: &str| {
                let event = EventBuilder::new(Kind::from(KIND_PRESENCE_UPDATE as u16), content)
                    .sign_with_keys(&actor)
                    .expect("sign event");
                let object_id = event.id.to_hex();
                let stored = StoredEvent::new(event, None);
                (object_id, stored, tenant.clone())
            };
            let (a_object, a_stored, ta) = ingest(tenant_a, "online-a");
            let (b_object, b_stored, tb) = ingest(tenant_b, "online-b");
            assert_ne!(
                a_object, b_object,
                "test precondition: the two events must have distinct ids"
            );

            super::super::dispatch_persistent_event_inner(
                &ta,
                &state,
                &a_stored,
                KIND_PRESENCE_UPDATE,
                &actor_hex,
                true,
                None,
            )
            .await;
            super::super::dispatch_persistent_event_inner(
                &tb,
                &state,
                &b_stored,
                KIND_PRESENCE_UPDATE,
                &actor_hex,
                true,
                None,
            )
            .await;

            audit_shutdown
                .drain(std::time::Duration::from_secs(5))
                .await;

            // Read each chain back through the operator-internal API.
            let svc = AuditService::new(pool.clone());
            let a_rows = svc
                .get_entries(CommunityId::from_uuid(*a_id), 1, 1000)
                .await
                .expect("read A chain");
            let b_rows = svc
                .get_entries(CommunityId::from_uuid(*b_id), 1, 1000)
                .await
                .expect("read B chain");

            // A's chain contains A's event and never B's; reverse holds too.
            assert!(
                a_rows.iter().all(|e| e.community_id == *a_id),
                "A read leaked another community's rows"
            );
            assert!(
                a_rows
                    .iter()
                    .any(|e| e.object_id.as_deref() == Some(a_object.as_str())),
                "A's ingested event is missing from A's chain"
            );
            assert!(
                !a_rows
                    .iter()
                    .any(|e| e.object_id.as_deref() == Some(b_object.as_str())),
                "B's event id appeared in A's audit chain — tenant isolation broken"
            );
            assert!(
                b_rows.iter().all(|e| e.community_id == *b_id),
                "B read leaked another community's rows"
            );
            assert!(
                !b_rows
                    .iter()
                    .any(|e| e.object_id.as_deref() == Some(a_object.as_str())),
                "A's event id appeared in B's audit chain — tenant isolation broken"
            );

            // Each chain verifies independently over its own range.
            let a_max = a_rows.iter().map(|e| e.seq).max().expect("A has entries");
            let b_max = b_rows.iter().map(|e| e.seq).max().expect("B has entries");
            assert!(
                svc.verify_chain(CommunityId::from_uuid(*a_id), 1, a_max)
                    .await
                    .expect("verify A"),
                "A's chain must verify independently"
            );
            assert!(
                svc.verify_chain(CommunityId::from_uuid(*b_id), 1, b_max)
                    .await
                    .expect("verify B"),
                "B's chain must verify independently"
            );
        }
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

        pub(super) fn test_config() -> crate::config::Config {
            let mut config = crate::config::Config::from_env().expect("default config loads");
            config.require_relay_membership = false;
            config.redis_url = "redis://127.0.0.1:1".to_string();
            config
        }

        pub(super) async fn test_state_with_redis_url(redis_url: &str) -> Arc<AppState> {
            let mut config = test_config();
            config.redis_url = redis_url.to_string();
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
            let audit = buzz_audit::AuditService::new(pool.clone());
            let auth = buzz_auth::AuthService::new(config.auth.clone());
            let search = buzz_search::SearchService::new(pool.clone());
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

        pub(super) async fn test_state() -> Arc<AppState> {
            test_state_with_redis_url("redis://127.0.0.1:1").await
        }

        /// Real-PG, real-Redis state that hands back the audit shutdown handle so
        /// a test can drain queued audit entries before asserting on `audit_log`.
        /// `None` when Postgres or Redis is unavailable (test skips).
        pub(super) async fn audit_state() -> Option<(
            Arc<AppState>,
            crate::state::AuditShutdownHandle,
            sqlx::PgPool,
        )> {
            let mut config = test_config();
            config.redis_url = "redis://127.0.0.1:6379".to_string();
            let pool = sqlx::PgPool::connect(&config.database_url).await.ok()?;
            // Require a real Redis so dispatch's publish_event doesn't error-log.
            let redis_pool = deadpool_redis::Config::from_url(&config.redis_url)
                .create_pool(Some(deadpool_redis::Runtime::Tokio1))
                .ok()?;
            redis::cmd("PING")
                .query_async::<String>(&mut redis_pool.get().await.ok()?)
                .await
                .ok()?;
            let db = buzz_db::Db::from_pool(pool.clone());
            let pubsub = Arc::new(
                buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
                    .await
                    .ok()?,
            );
            let audit = buzz_audit::AuditService::new(pool.clone());
            let auth = buzz_auth::AuthService::new(config.auth.clone());
            let search = buzz_search::SearchService::new(pool.clone());
            let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
                db.clone(),
                buzz_workflow::WorkflowConfig::default(),
            ));
            let media_storage = buzz_media::MediaStorage::new(&config.media).ok()?;
            let (state, audit_shutdown) = AppState::new(
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
            Some((Arc::new(state), audit_shutdown, pool))
        }

        fn register_conn(state: &AppState, pubkey: Option<Vec<u8>>) -> Uuid {
            let conn_id = Uuid::new_v4();
            let (tx, _rx) = mpsc::channel(1);
            let (ctrl_tx, _ctrl_rx) = mpsc::channel(1);
            state.conn_manager.register(
                conn_id,
                tx,
                ctrl_tx,
                CancellationToken::new(),
                buzz_core::tenant::CommunityId::from_uuid(Uuid::nil()),
                Arc::new(AtomicU8::new(0)),
                Arc::new(Mutex::new(HashMap::new())),
                3,
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
            let out = filter_fanout_by_access(
                &state,
                buzz_core::tenant::CommunityId::from_uuid(Uuid::nil()),
                &channel_event(None),
                matches.clone(),
                None,
            )
            .await;
            assert_eq!(out, matches);
        }

        #[tokio::test]
        async fn open_channel_event_passes_through_unfiltered() {
            let state = test_state().await;
            let channel_id = Uuid::new_v4();
            let community_id = buzz_core::tenant::CommunityId::from_uuid(Uuid::nil());
            state
                .channel_visibility_cache
                .insert((community_id, channel_id), "open".to_string());
            // A connection with no authenticated pubkey would be dropped on a
            // private channel; on open it must pass untouched.
            let conn = register_conn(&state, None);
            let matches = vec![(conn, "s".to_string())];
            let out = filter_fanout_by_access(
                &state,
                community_id,
                &channel_event(Some(channel_id)),
                matches.clone(),
                None,
            )
            .await;
            assert_eq!(out, matches);
        }

        #[tokio::test]
        async fn private_channel_keeps_member_drops_non_member_and_unknown() {
            let state = test_state().await;
            let channel_id = Uuid::new_v4();
            let community_id = buzz_core::tenant::CommunityId::from_uuid(Uuid::nil());
            state
                .channel_visibility_cache
                .insert((community_id, channel_id), "private".to_string());

            let member_pk = vec![1u8; 32];
            let non_member_pk = vec![2u8; 32];
            state
                .membership_cache
                .insert((community_id, channel_id, member_pk.clone()), true);
            state
                .membership_cache
                .insert((community_id, channel_id, non_member_pk.clone()), false);

            let member = register_conn(&state, Some(member_pk));
            let non_member = register_conn(&state, Some(non_member_pk));
            let unauthed = register_conn(&state, None);

            let matches = vec![
                (member, "m".to_string()),
                (non_member, "n".to_string()),
                (unauthed, "u".to_string()),
            ];
            let out = filter_fanout_by_access(
                &state,
                community_id,
                &channel_event(Some(channel_id)),
                matches,
                None,
            )
            .await;
            assert_eq!(out, vec![(member, "m".to_string())]);
        }

        #[tokio::test]
        async fn author_only_reminder_delivers_to_author_only() {
            let state = test_state().await;

            let author_keys = Keys::generate();
            let author_pk = author_keys.public_key().to_bytes().to_vec();
            let other_pk = vec![9u8; 32];

            // KIND_EVENT_REMINDER (30300) is in AUTHOR_ONLY_KINDS and is stored
            // globally (channel_id = None), so the gate must apply independent
            // of any channel-membership check.
            let reminder = EventBuilder::new(
                Kind::Custom(buzz_core::kind::KIND_EVENT_REMINDER as u16),
                "{}",
            )
            .sign_with_keys(&author_keys)
            .expect("sign reminder");
            let stored = StoredEvent::new(reminder, None);

            let author_conn = register_conn(&state, Some(author_pk));
            let other_conn = register_conn(&state, Some(other_pk));
            let unauthed_conn = register_conn(&state, None);

            let matches = vec![
                (author_conn, "a".to_string()),
                (other_conn, "o".to_string()),
                (unauthed_conn, "u".to_string()),
            ];
            let out = filter_fanout_by_access(
                &state,
                buzz_core::tenant::CommunityId::from_uuid(Uuid::nil()),
                &stored,
                matches,
                None,
            )
            .await;

            // Only the author's subscription survives; the non-author and the
            // unauthenticated connection are both dropped.
            assert_eq!(out, vec![(author_conn, "a".to_string())]);
        }

        // -- E1 phase-2: threaded-visibility fences (§4.8 phase-2 addendum) --

        /// Fence 3: a threaded visibility resolved under a different
        /// (community, channel) must be ignored — the filter falls back to
        /// the fresh fail-closed lookup, not the threaded value.
        #[tokio::test]
        async fn threaded_visibility_mismatch_falls_back_to_fresh_lookup() {
            let state = test_state().await;
            let channel_id = Uuid::new_v4();
            let community_id = buzz_core::tenant::CommunityId::from_uuid(Uuid::nil());
            // Fresh lookup path resolves private via the fail-safe cache.
            state
                .channel_visibility_cache
                .insert((community_id, channel_id), "private".to_string());

            // Threaded value says "open" but for a DIFFERENT channel.
            let threaded = crate::state::ThreadedChannelVisibility {
                community_id,
                channel_id: Uuid::new_v4(),
                visibility: "open".to_string(),
            };

            // Unauthenticated conn: kept on open, dropped on private.
            let conn = register_conn(&state, None);
            let matches = vec![(conn, "s".to_string())];
            let out = filter_fanout_by_access(
                &state,
                community_id,
                &channel_event(Some(channel_id)),
                matches,
                Some(&threaded),
            )
            .await;
            assert!(
                out.is_empty(),
                "mismatched threaded visibility must not be consulted; \
                 fresh lookup says private → unauthenticated conn dropped"
            );
        }

        /// Matching threaded `private` gates recipients without a DB read,
        /// identically to the fresh-lookup private path.
        #[tokio::test]
        async fn threaded_visibility_private_filters_members_only() {
            let state = test_state().await;
            let channel_id = Uuid::new_v4();
            let community_id = buzz_core::tenant::CommunityId::from_uuid(Uuid::nil());

            let member_pk = vec![1u8; 32];
            let non_member_pk = vec![2u8; 32];
            state
                .membership_cache
                .insert((community_id, channel_id, member_pk.clone()), true);
            state
                .membership_cache
                .insert((community_id, channel_id, non_member_pk.clone()), false);

            let threaded = crate::state::ThreadedChannelVisibility {
                community_id,
                channel_id,
                visibility: "private".to_string(),
            };

            let member = register_conn(&state, Some(member_pk));
            let non_member = register_conn(&state, Some(non_member_pk));
            let matches = vec![(member, "m".to_string()), (non_member, "n".to_string())];
            let out = filter_fanout_by_access(
                &state,
                community_id,
                &channel_event(Some(channel_id)),
                matches,
                Some(&threaded),
            )
            .await;
            assert_eq!(out, vec![(member, "m".to_string())]);
        }

        /// Matching threaded `open` passes recipients through with no
        /// visibility SELECT (no visibility cache entry exists and the lazy
        /// PG pool in `test_state` would error a fresh lookup → fail closed;
        /// passing through proves the threaded value was used).
        #[tokio::test]
        async fn threaded_visibility_open_passes_through() {
            let state = test_state().await;
            let channel_id = Uuid::new_v4();
            let community_id = buzz_core::tenant::CommunityId::from_uuid(Uuid::nil());

            let threaded = crate::state::ThreadedChannelVisibility {
                community_id,
                channel_id,
                visibility: "open".to_string(),
            };

            let conn = register_conn(&state, None);
            let matches = vec![(conn, "s".to_string())];
            let out = filter_fanout_by_access(
                &state,
                community_id,
                &channel_event(Some(channel_id)),
                matches.clone(),
                Some(&threaded),
            )
            .await;
            assert_eq!(out, matches);
        }
    }

    // ---------------------------------------------------------------------
    // Red-team — Attack 4 (cross-community read)
    // ---------------------------------------------------------------------
    //
    // Spec property pinned: `Inv_NonInterference` / `Inv_ReadConfinement` /
    // `Inv_LabelPropagation` from `docs/spec/MultiTenantRelay.tla` (lines
    // 985+). Seam action this exercises: `ReadMessageRows` for the receiving
    // connection — the relay must never deliver to a community-A connection an
    // event labelled `{B}` for any `B != A`.
    //
    // Mutation class (per the TLA+ header lines 43-91): M1/M3/M12 family —
    // unscoped read/fan-out paths where the receiver's tenant is not consulted
    // against the event's tenant.
    //
    // What this module proves about the code at `fb0d6a4ea`:
    //
    //  * `ConnEntry` (`crates/buzz-relay/src/state.rs:30-44`) records
    //    `authenticated_pubkey` but NO community/tenant binding. The fan-out
    //    path has no way to ask "what community is this socket bound to."
    //  * `SubscriptionRegistry` (`crates/buzz-relay/src/subscription.rs:57+`)
    //    indexes subscriptions by `(channel_id, kind)` / `(kind, #p)` / `kind`
    //    / wildcard — never by community.
    //  * `filter_fanout_by_access` (this file, line 62) accepts the *event*'s
    //    `community_id` (from the publishing tenant / Redis topic) but never
    //    compares it to the *receiving* connection's tenant. For channel-less
    //    (global) events the function short-circuits to `return matches`
    //    (line 89-91) — pass-through with no isolation check.
    //
    // Consequence: when a single pod hosts connections from multiple
    // communities (the rewrite's explicit design — stateless workers, any pod
    // serves any community), a same-pod ingest of a community-B global event
    // matches and delivers to a community-A connection whose subscription's
    // event-content predicates happen to match (e.g. a presence sub keyed on a
    // pubkey that exists in both communities, an `#p`-tagged membership
    // notification, or any wildcard global sub). That is the literal negation
    // of `Inv_NonInterference`.
    //
    // The two tests below pin the contract. The first documents the current
    // (broken) behavior so the gap is named in code; it MUST be deleted in the
    // same change that fixes the leak. The second is the regression guard —
    // it goes red on this revision (the relay delivers the cross-community
    // event) and turns green when the structural fix lands: connection-level
    // tenant binding (`ConnEntry { community: CommunityId, .. }`) plus a
    // tenant cross-check in `filter_fanout_by_access` such that a match where
    // `conn.community != event.community` is dropped.
    //
    // Routing: per Eva (lane partition, fb0d6a4ea handoff thread), the patch
    // is owned by Max — the same structural fix his reminder-fanout lane
    // already needs. This module is the spec for "closed."
    mod redteam {
        use std::collections::HashMap;
        use std::sync::atomic::AtomicU8;
        use std::sync::Arc;

        use buzz_core::kind::KIND_PRESENCE_UPDATE;
        use buzz_core::StoredEvent;
        use nostr::{EventBuilder, Keys, Kind};
        use tokio::sync::{mpsc, Mutex};
        use tokio_util::sync::CancellationToken;
        use uuid::Uuid;

        use crate::handlers::event::filter_fanout_by_access;
        use crate::state::AppState;

        async fn test_state() -> Arc<AppState> {
            super::fanout_access::test_state().await
        }

        fn register_conn(
            state: &AppState,
            community_id: buzz_core::tenant::CommunityId,
            pubkey: Option<Vec<u8>>,
        ) -> Uuid {
            let conn_id = Uuid::new_v4();
            let (tx, _rx) = mpsc::channel(1);
            let (ctrl_tx, _ctrl_rx) = mpsc::channel(1);
            state.conn_manager.register(
                conn_id,
                tx,
                ctrl_tx,
                CancellationToken::new(),
                community_id,
                Arc::new(AtomicU8::new(0)),
                Arc::new(Mutex::new(HashMap::new())),
                3,
            );
            if let Some(pk) = pubkey {
                state.conn_manager.set_authenticated_pubkey(conn_id, pk);
            }
            conn_id
        }

        /// Regression gate for the Inv_NonInterference fix.
        ///
        /// This test asserts the CORRECT shape: when the receiving
        /// connection is bound to community A and the event is labelled
        /// community B, `filter_fanout_by_access` must drop the recipient.
        ///
        /// This regression failed on `fb0d6a4ea` (the red-team artifact;
        /// the leak was the failure). The structural fix it pins:
        ///
        ///   1. `ConnEntry` carries a `community: CommunityId` set when the
        ///      socket's host resolves at handshake.
        ///   2. `ConnectionManager` exposes
        ///      `community_for_conn(conn_id) -> Option<CommunityId>`.
        ///   3. `filter_fanout_by_access` (or a wrapper at the call sites)
        ///      drops any `(conn_id, sub_id)` where
        ///      `community_for_conn(conn_id) != Some(event_community)`.
        ///
        /// Turning this test green is the definition of "Attack 4 is
        /// closed" for the global-event seam.
        #[tokio::test]
        async fn channel_less_event_must_drop_recipient_in_different_community() {
            let state = test_state().await;

            // Same pubkey on two different community-bound sockets — the
            // multi-tenant pod case the rewrite must serve safely.
            let shared_pk = vec![7u8; 32];
            let community_a = buzz_core::tenant::CommunityId::from_uuid(Uuid::from_u128(0xAAAA));
            let community_b = buzz_core::tenant::CommunityId::from_uuid(Uuid::from_u128(0xBBBB));
            let a_socket = register_conn(&state, community_a, Some(shared_pk.clone()));

            let presence = EventBuilder::new(Kind::Custom(KIND_PRESENCE_UPDATE as u16), "online")
                .sign_with_keys(&Keys::generate())
                .expect("sign presence");
            let stored = StoredEvent::new(presence, None);

            let matches = vec![(a_socket, "presence".to_string())];
            let out = filter_fanout_by_access(&state, community_b, &stored, matches, None).await;

            // Correct behavior: A-socket dropped because its tenant != B.
            assert!(
                out.is_empty(),
                "Inv_NonInterference: a connection bound to community A \
                 must not receive a community-B event. Got: {out:?}"
            );
        }

        /// Dispatch-predicate tripwire: kind:31234 (NIP-37 draft wrap) and
        /// kind:30300 (NIP-ER reminder) are in `AUTHOR_ONLY_KINDS` so
        /// `should_dispatch_workflow` returns `false` for them, permanently
        /// suppressing workflow triggers for author-only events.
        ///
        /// The test calls `should_dispatch_workflow` — the same function
        /// that `dispatch_persistent_event_inner` calls — so deleting or
        /// changing the guard in the production path reds this test.
        /// A kind:9 positive control proves the predicate is not trivially
        /// always-false.
        #[test]
        fn draft_kind_is_excluded_from_workflow_dispatch_by_author_only_guard() {
            // kind:31234 must be excluded from workflow dispatch.
            // is_relay_workflow_msg=false for any user-submitted event.
            assert!(
                !super::super::should_dispatch_workflow(buzz_core::kind::KIND_DRAFT, false),
                "should_dispatch_workflow must return false for kind:31234 — \
                 a draft wrap must NEVER reach the workflow engine"
            );

            // kind:30300 (NIP-ER reminder) is also AUTHOR_ONLY and must be excluded.
            assert!(
                !super::super::should_dispatch_workflow(
                    buzz_core::kind::KIND_EVENT_REMINDER,
                    false
                ),
                "should_dispatch_workflow must return false for kind:30300 — \
                 reminders are author-only and must not trigger workspace workflows"
            );

            // Positive control: a plain channel message (kind:9) must return true
            // so we know the predicate is not trivially always-false.
            let kind_channel_message: u32 = 9;
            assert!(
                super::super::should_dispatch_workflow(kind_channel_message, false),
                "should_dispatch_workflow must return true for kind:9 channel messages \
                 (positive control) — the gate must be live, not trivially always-false"
            );

            // is_relay_workflow_msg=true must suppress dispatch for any kind.
            assert!(
                !super::super::should_dispatch_workflow(kind_channel_message, true),
                "should_dispatch_workflow must return false when is_relay_workflow_msg=true \
                 (relay-signed workflow messages must not re-trigger the engine)"
            );
        }
    }
}
