#![deny(unsafe_code)]

mod acp;
mod config;
mod durable_outbox;
mod engram_fetch;
mod filter;
mod observer;
mod pool;
mod pool_lifecycle;
mod queue;
mod relay;
mod setup_mode;
mod usage;

pub use usage::TurnUsage;

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use acp::{AcpClient, BuzzSessionEvent, ConversationResetCommit, EnvVar, McpServer};
use anyhow::Result;
use buzz_core::kind::{
    KIND_MEMBER_ADDED_NOTIFICATION, KIND_MEMBER_REMOVED_NOTIFICATION, KIND_STREAM_MESSAGE,
    KIND_STREAM_REMINDER, KIND_WORKFLOW_APPROVAL_REQUESTED,
};
use buzz_core::observer::{
    decrypt_observer_payload, encrypt_observer_payload, OBSERVER_FRAME_TELEMETRY,
    OBSERVER_MAX_PLAINTEXT_LEN,
};
use clap::Parser;
use config::{
    AuthAgentArgs, AuthMethodsArgs, AuthenticateArgs, Config, DedupMode, ModelsArgs,
    MultipleEventHandling, RespondTo, SubscribeMode,
};
use durable_outbox::{
    DurableLifecycleEnvelope, DurableOutbox, DurableResetPhase, DurableResetRecord,
};
use filter::SubscriptionRule;
use futures_util::FutureExt;
use nostr::{PublicKey, ToBech32};
use pool::{
    AgentPool, ControlSignal, OwnedAgent, PromptContext, PromptOutcome, PromptResult, PromptSource,
    SessionState, TimeoutKind,
};
use pool_lifecycle::PoolLifecycle;
use queue::{CancelReason, ConversationKey, EventQueue, FlushBatch, QueuedEvent, ThreadTags};
use relay::{HarnessRelay, RelayEventPublisher};
use tokio::sync::{mpsc, watch, Semaphore};
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

/// Check if argv[1] matches a subcommand name, before any clap parsing.
///
/// This avoids clap rejecting harness flags (like `--private-key`) that aren't
/// declared on the subcommand's `Parser`. The `models` path has its own
/// dedicated parser; the default path uses the existing `CliArgs`.
///
/// **Constraint**: subcommand must be argv[1] — flags before the subcommand
/// name (e.g., `buzz-acp --verbose models`) are not supported.
fn is_subcommand(name: &str) -> bool {
    std::env::args().nth(1).map(|a| a == name).unwrap_or(false)
}

/// Timeout for lightweight helper subcommands (spawn + initialize + model/method probes).
const MODELS_TIMEOUT: Duration = Duration::from_secs(10);

/// Timeout for `buzz-acp authenticate`. Browser-based vendor auth can require
/// human interaction, so it must not share the short probe timeout.
const AUTHENTICATE_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// Publish a kind:20001 presence update event via the WebSocket connection.
///
/// Ephemeral kinds (20000-29999) are rejected by the HTTP bridge, so presence
/// updates must be routed through the WS path.
///
/// Content is a bare status string (`"online"`, `"away"`, `"offline"`) matching
/// the desktop client's format. The relay stores this in Redis and synthesizes
/// it back on presence queries.
async fn publish_presence(
    publisher: &relay::RelayEventPublisher,
    keys: &nostr::Keys,
    status: &str,
) -> Result<(), relay::RelayError> {
    use buzz_core::kind::KIND_PRESENCE_UPDATE;
    use nostr::{EventBuilder, Kind};

    let event = EventBuilder::new(Kind::Custom(KIND_PRESENCE_UPDATE as u16), status)
        .tags([])
        .sign_with_keys(keys)
        .map_err(|e| relay::RelayError::Http(format!("presence sign error: {e}")))?;
    publisher.publish_event(event).await?;
    Ok(())
}

fn emit_runtime_lifecycle(
    observer: Option<&observer::ObserverHandle>,
    start_nonce: &str,
    pubkey: &str,
    relay_url: &str,
    lifecycle: &str,
    error: Option<&str>,
) {
    if let Some(observer) = observer {
        observer.emit(
            "managed_agent_runtime_lifecycle",
            None,
            &observer::ObserverContext::default(),
            serde_json::json!({
                "pubkey": pubkey,
                "relayUrl": relay_url,
                "startNonce": start_nonce,
                "lifecycle": lifecycle,
                "error": error,
            }),
        );
    }
}

/// Resolve the agent's owner pubkey at startup.
///
/// Priority:
/// 1. `BUZZ_AUTH_TAG` env var — NIP-OA attestation signed by the owner.
///    Verified against the agent's own pubkey to extract the owner pubkey.
/// 2. `--agent-owner` CLI flag / `BUZZ_ACP_AGENT_OWNER` env var.
fn resolve_agent_owner(config: &Config) -> Option<String> {
    // Try BUZZ_AUTH_TAG first (NIP-OA attestation).
    if let Ok(auth_tag) = std::env::var("BUZZ_AUTH_TAG") {
        if !auth_tag.is_empty() {
            let agent_pk = config.keys.public_key();
            match buzz_sdk::nip_oa::verify_auth_tag(&auth_tag, &agent_pk) {
                Ok(owner_pk) => {
                    let owner_hex = owner_pk.to_hex().to_ascii_lowercase();
                    tracing::info!("owner resolved from BUZZ_AUTH_TAG: {owner_hex}");
                    return Some(owner_hex);
                }
                Err(e) => {
                    tracing::warn!("BUZZ_AUTH_TAG verification failed: {e} — falling back");
                }
            }
        }
    }

    // Fall back to --agent-owner config.
    config.agent_owner.clone()
}

/// Cache for the agent's owner pubkey.
///
/// Owner is now provided via `--agent-owner` config flag (no REST lookup).
/// Cache for the agent's owner pubkey + sibling lookups.
///
/// Siblings are other agents whose NIP-OA auth tag proves the same owner.
/// Lookup results are cached for the process lifetime (attestations are immutable).
struct OwnerCache {
    pubkey: Option<String>,
    /// author_hex → is_sibling (true = same owner, false = not)
    siblings: std::sync::Mutex<HashMap<String, bool>>,
}

impl OwnerCache {
    fn new(initial: Option<String>) -> Self {
        Self {
            pubkey: initial,
            siblings: std::sync::Mutex::new(HashMap::new()),
        }
    }

    /// Return the cached owner pubkey.
    fn get(&self) -> Option<&str> {
        self.pubkey.as_deref()
    }

    /// Check if author is a known sibling (cached result).
    fn is_known_sibling(&self, author: &str) -> Option<bool> {
        self.siblings.lock().ok()?.get(author).copied()
    }

    /// Cache a sibling lookup result.
    fn cache_sibling(&self, author: String, is_sibling: bool) {
        if let Ok(mut map) = self.siblings.lock() {
            // Cap at 256 entries to prevent unbounded growth.
            if map.len() >= 256 {
                map.clear();
            }
            map.insert(author, is_sibling);
        }
    }
}

/// Check if `author` is the owner OR a sibling (same owner via NIP-OA).
///
/// For unknown authors, queries their kind:0 profile to extract the NIP-OA
/// auth tag and verify the owner matches. Result is cached.
async fn is_owner_or_sibling(
    author: &str,
    owner_cache: &OwnerCache,
    rest_client: &relay::RestClient,
) -> bool {
    let my_owner = match owner_cache.get() {
        Some(o) => o,
        None => return false, // no owner configured — fail closed
    };

    // Direct owner check.
    if author == my_owner {
        return true;
    }

    // Check sibling cache.
    if let Some(cached) = owner_cache.is_known_sibling(author) {
        return cached;
    }

    // Query the author's kind:0 profile to check for NIP-OA auth tag.
    let is_sibling = check_sibling_via_profile(author, my_owner, rest_client).await;
    owner_cache.cache_sibling(author.to_string(), is_sibling);
    is_sibling
}

/// Inbound author gate decision: does this author's event fire a turn?
///
/// Coarse security policy applied before subscription rules. Both `OwnerOnly`
/// and `Allowlist` accept the owner and same-owner siblings; `Allowlist`
/// additionally accepts the explicit external pubkey list.
///
/// # DM hardening (`is_dm`)
///
/// Clients auto-p-tag every DM participant, so in a DM *any* participant's
/// message looks like a mention and would fire a turn. Combined with
/// agent-initiated DMs (the agent can be asked to DM a third party), that
/// turns `anyone`/`allowlist` modes into transitive access grants: whoever
/// lands in a DM with the agent can prompt it. To close that hole, when
/// `is_dm` is true only the owner and cryptographically verified same-owner
/// siblings may fire a turn — the explicit allowlist and `anyone` mode do
/// NOT apply inside DMs. `Nobody` still drops everything. Callers must
/// resolve `is_dm` fail-closed: unknown channel type ⇒ treat as DM.
async fn author_allowed(
    respond_to: &RespondTo,
    allowlist: &HashSet<String>,
    author: &str,
    is_dm: bool,
    owner_cache: &OwnerCache,
    rest_client: &relay::RestClient,
) -> bool {
    if is_dm {
        return match respond_to {
            RespondTo::Nobody => false,
            _ => is_owner_or_sibling(author, owner_cache, rest_client).await,
        };
    }
    match respond_to {
        RespondTo::Anyone => true,
        RespondTo::Nobody => false,
        RespondTo::OwnerOnly => is_owner_or_sibling(author, owner_cache, rest_client).await,
        RespondTo::Allowlist => {
            allowlist.contains(author)
                || is_owner_or_sibling(author, owner_cache, rest_client).await
        }
    }
}

/// Resolve whether `channel_id` is a DM, for the inbound author gate.
///
/// Resolution order:
/// 1. Startup discovery metadata (`startup_info`) — covers channels known at
///    process start.
/// 2. Per-loop resolution cache (`cache`) — covers channels resolved since.
/// 3. Lazy REST fetch of the channel's kind:39000 metadata — covers channels
///    the agent was added to *after* startup (the exploit path: an
///    agent-initiated DM is exactly such a channel).
///
/// Fail-closed: if the fetch fails or times out, the channel is treated as a
/// DM for this event and the result is NOT cached, so a later event retries
/// the fetch instead of pinning a mis-classification.
pub(crate) async fn is_dm_channel(
    channel_id: Uuid,
    channel_info: &pool::ChannelInfoResolver,
) -> bool {
    resolve_dm_channel_scope(channel_id, channel_info).await.0
}

/// Return `(author_gate_is_dm, conversation_scope_is_dm)`.
///
/// Unknown metadata must be restrictive for authorization but isolated for
/// context: treating an unknown public channel as a stable DM conversation
/// would collapse unrelated top-level messages into one agent session.
async fn resolve_dm_channel_scope(
    channel_id: Uuid,
    channel_info: &pool::ChannelInfoResolver,
) -> (bool, bool) {
    match channel_info.resolve(channel_id).await {
        Some(info) => {
            let is_dm = info.channel_type == "dm";
            (is_dm, is_dm)
        }
        None => {
            tracing::warn!(
                channel_id = %channel_id,
                "channel type unresolved — treating as DM for author gate while isolating conversation scope"
            );
            (true, false)
        }
    }
}

/// Query an author's kind:0 profile and check if their NIP-OA auth tag
/// proves the same owner as us.
async fn check_sibling_via_profile(
    author: &str,
    expected_owner: &str,
    rest_client: &relay::RestClient,
) -> bool {
    let filter = nostr::Filter::new()
        .kind(nostr::Kind::Metadata)
        .author(match nostr::PublicKey::from_hex(author) {
            Ok(pk) => pk,
            Err(_) => return false,
        })
        .limit(1);

    let resp = match tokio::time::timeout(Duration::from_millis(2000), rest_client.query(&[filter]))
        .await
    {
        Ok(Ok(v)) => v,
        _ => return false, // timeout or error — fail closed
    };

    // Look for an "auth" tag in the profile event.
    let events = match resp.as_array() {
        Some(arr) => arr,
        None => return false,
    };
    let event = match events.first() {
        Some(e) => e,
        None => return false,
    };
    let tags = match event.get("tags").and_then(|t| t.as_array()) {
        Some(t) => t,
        None => return false,
    };

    // Find ["auth", owner_pk, conditions, sig] and verify the Schnorr signature.
    // Don't trust the relay — verify ourselves.
    let agent_pk = match nostr::PublicKey::from_hex(author) {
        Ok(pk) => pk,
        Err(_) => return false,
    };

    for tag in tags {
        let parts = match tag.as_array() {
            Some(p) if p.len() >= 4 => p,
            _ => continue,
        };
        if parts[0].as_str() != Some("auth") {
            continue;
        }
        let tag_owner = match parts[1].as_str() {
            Some(o) => o,
            None => continue,
        };
        // Only verify if the owner field matches ours.
        if !tag_owner.eq_ignore_ascii_case(expected_owner) {
            continue;
        }
        // Cryptographically verify the NIP-OA attestation signature.
        let tag_json = serde_json::to_string(tag).unwrap_or_default();
        match buzz_sdk::nip_oa::verify_auth_tag(&tag_json, &agent_pk) {
            Ok(_) => {
                tracing::debug!(author, expected_owner, "sibling verified via NIP-OA");
                return true;
            }
            Err(e) => {
                tracing::debug!(author, "NIP-OA auth tag verification failed: {e}");
            }
        }
    }

    false
}

const OBSERVER_PUBLISH_INTERVAL: Duration = Duration::from_millis(167);
const OBSERVER_PUBLISH_LIMIT_PER_MINUTE: usize = 90;

struct ObserverPublishPacer {
    next_publish: tokio::time::Instant,
    published: VecDeque<tokio::time::Instant>,
}

impl ObserverPublishPacer {
    fn new() -> Self {
        Self {
            // No initial burst: even the first snapshot frame waits for its slot.
            next_publish: tokio::time::Instant::now() + OBSERVER_PUBLISH_INTERVAL,
            published: VecDeque::with_capacity(OBSERVER_PUBLISH_LIMIT_PER_MINUTE),
        }
    }

    async fn wait(&mut self) {
        loop {
            let now = tokio::time::Instant::now();
            while self
                .published
                .front()
                .is_some_and(|sent| now.duration_since(*sent) >= Duration::from_secs(60))
            {
                self.published.pop_front();
            }

            let minute_slot = self.published.front().and_then(|sent| {
                (self.published.len() >= OBSERVER_PUBLISH_LIMIT_PER_MINUTE)
                    .then_some(*sent + Duration::from_secs(60))
            });
            let publish_at =
                minute_slot.map_or(self.next_publish, |slot| slot.max(self.next_publish));
            if publish_at > now {
                tokio::time::sleep_until(publish_at).await;
                continue;
            }

            let published_at = tokio::time::Instant::now();
            self.published.push_back(published_at);
            self.next_publish = published_at + OBSERVER_PUBLISH_INTERVAL;
            return;
        }
    }
}

fn spawn_relay_observer_publisher(
    observer: observer::ObserverHandle,
    publisher: RelayEventPublisher,
    keys: nostr::Keys,
    agent_pubkey_hex: String,
    owner_pubkey_hex: String,
    owner_pubkey: PublicKey,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        // Subscribe BEFORE snapshotting so an event emitted between the two
        // calls is never lost: it lands in the snapshot, the live receiver, or
        // both. The overlap is deduped in the run loop via the snapshot's
        // high-water `seq` (monotonic, assigned at emit).
        let rx = observer.subscribe();
        let snapshot = observer.snapshot();
        run_relay_observer_publisher(
            snapshot,
            rx,
            publisher,
            keys,
            agent_pubkey_hex,
            owner_pubkey_hex,
            owner_pubkey,
        )
        .await;
    })
}

async fn run_relay_observer_publisher(
    snapshot: Vec<observer::ObserverEvent>,
    mut rx: tokio::sync::broadcast::Receiver<observer::ObserverEvent>,
    publisher: RelayEventPublisher,
    keys: nostr::Keys,
    agent_pubkey_hex: String,
    owner_pubkey_hex: String,
    owner_pubkey: PublicKey,
) {
    let mut coalescer = ObserverChunkCoalescer::default();
    let mut pacer = ObserverPublishPacer::new();
    let max_snapshot_seq = snapshot.iter().map(|event| event.seq).max().unwrap_or(0);
    for event in snapshot {
        for event in coalescer.ingest(event) {
            publish_relay_observer_event(
                &publisher,
                &keys,
                &agent_pubkey_hex,
                &owner_pubkey_hex,
                &owner_pubkey,
                &mut pacer,
                event,
            )
            .await;
        }
    }

    let mut flush_interval = tokio::time::interval(std::time::Duration::from_millis(500));
    flush_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            result = rx.recv() => {
                match result {
                    Ok(event) => {
                        // Skip live events already delivered via the snapshot
                        // (the subscribe-before-snapshot overlap).
                        if event.seq <= max_snapshot_seq {
                            continue;
                        }
                        for event in coalescer.ingest(event) {
                            publish_relay_observer_event(
                                &publisher, &keys, &agent_pubkey_hex,
                                &owner_pubkey_hex, &owner_pubkey, &mut pacer, event,
                            ).await;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(count)) => {
                        for event in coalescer.flush() {
                            publish_relay_observer_event(
                                &publisher, &keys, &agent_pubkey_hex,
                                &owner_pubkey_hex, &owner_pubkey, &mut pacer, event,
                            ).await;
                        }
                        tracing::warn!(dropped = count, "relay observer publisher lagged");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        for event in coalescer.flush() {
                            publish_relay_observer_event(
                                &publisher, &keys, &agent_pubkey_hex,
                                &owner_pubkey_hex, &owner_pubkey, &mut pacer, event,
                            ).await;
                        }
                        break;
                    }
                }
            }
            _ = flush_interval.tick() => {
                // Periodic flush ensures live streaming even during continuous chunk delivery.
                for event in coalescer.flush() {
                    publish_relay_observer_event(
                        &publisher, &keys, &agent_pubkey_hex,
                        &owner_pubkey_hex, &owner_pubkey, &mut pacer, event,
                    ).await;
                }
            }
        }
    }
}

#[derive(Default)]
struct ObserverChunkCoalescer {
    pending: Vec<PendingObserverChunk>,
}

struct PendingObserverChunk {
    key: ObserverChunkKey,
    event: observer::ObserverEvent,
    text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ObserverChunkKey {
    update_type: String,
    message_id: Option<String>,
    channel_id: Option<String>,
    session_id: Option<String>,
    turn_id: Option<String>,
    agent_index: Option<usize>,
}

/// Flush coalesced chunks before they exceed the NIP-44 plaintext limit (65,535 bytes).
/// Leave headroom for the JSON envelope wrapping the text. This is a SOFT pre-flush
/// of raw text below the hard cap; `fit_observer_event_to_budget` (the final ceiling,
/// keyed to `OBSERVER_MAX_PLAINTEXT_LEN` in buzz-core/observer.rs:25) is what actually
/// guarantees the serialized frame fits. Edit one of these two and review the other.
const OBSERVER_CHUNK_MAX_TEXT_BYTES: usize = 60_000;

impl ObserverChunkCoalescer {
    fn ingest(&mut self, event: observer::ObserverEvent) -> Vec<observer::ObserverEvent> {
        let Some((key, text)) = observer_chunk_key_and_text(&event) else {
            let mut events = self.flush();
            events.push(event);
            return events;
        };

        if let Some(pending) = self.pending.iter_mut().find(|pending| pending.key == key) {
            // Flush before appending if this would exceed the plaintext size limit.
            if pending.text.len() + text.len() >= OBSERVER_CHUNK_MAX_TEXT_BYTES {
                let events = self.flush();
                // Start a new pending entry with the current chunk.
                self.pending.push(PendingObserverChunk { key, event, text });
                return events;
            }
            pending.text.push_str(&text);
            pending.event.seq = event.seq;
            pending.event.timestamp = event.timestamp;
            return Vec::new();
        }

        self.pending.push(PendingObserverChunk { key, event, text });
        Vec::new()
    }

    fn flush(&mut self) -> Vec<observer::ObserverEvent> {
        self.pending
            .drain(..)
            .map(|mut pending| {
                set_observer_chunk_text(&mut pending.event.payload, pending.text);
                pending.event
            })
            .collect()
    }
}

fn observer_chunk_key_and_text(
    event: &observer::ObserverEvent,
) -> Option<(ObserverChunkKey, String)> {
    let update = event.payload.get("params")?.get("update")?;
    let update_type = update.get("sessionUpdate")?.as_str()?;
    if !matches!(
        update_type,
        "agent_message_chunk" | "user_message_chunk" | "agent_thought_chunk"
    ) {
        return None;
    }

    let text = update.get("content")?.get("text")?.as_str()?.to_string();
    let message_id = update
        .get("messageId")
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned);

    Some((
        ObserverChunkKey {
            update_type: update_type.to_string(),
            message_id,
            channel_id: event.channel_id.clone(),
            session_id: event.session_id.clone(),
            turn_id: event.turn_id.clone(),
            agent_index: event.agent_index,
        },
        text,
    ))
}

fn set_observer_chunk_text(payload: &mut serde_json::Value, text: String) {
    let Some(content) = payload
        .get_mut("params")
        .and_then(|params| params.get_mut("update"))
        .and_then(|update| update.get_mut("content"))
    else {
        return;
    };

    if let Some(content_object) = content.as_object_mut() {
        content_object.insert("text".to_string(), serde_json::Value::String(text));
    }
}

/// Bytes of head and tail to retain from an elided string leaf — the value
/// shown to the renderer at each end. The ONLY tuning knob here: large enough
/// that a clipped diff/tool-result still shows real content, small enough that
/// eliding actually shrinks the frame.
const OBSERVER_LEAF_RETAIN_BYTES: usize = 3_000;

/// Trim an oversized observer telemetry frame so its SERIALIZED form fits under
/// `OBSERVER_MAX_PLAINTEXT_LEN`, instead of dropping the whole frame (silent
/// telemetry loss). The common case — a frame already under budget — is left
/// byte-identical.
///
/// The cap is measured in SERIALIZED bytes (JSON escaping makes serialized
/// length differ from raw), so the stop condition is always a full reserialize
/// of the whole frame: that counts the envelope, the variable `Option<String>`
/// IDs, and any elision markers exactly. No separate margin constant is needed.
///
/// Termination is provable: each iteration elides the largest string leaf that
/// would STRICTLY shrink the serialized frame, then reserializes. Shrinkability
/// is re-evaluated against each leaf's CURRENT value, so a leaf already at its
/// retained floor can never be re-elided — the loop strictly decreases the
/// serialized length each pass and is bounded by the leaf count. When no leaf
/// can shrink the frame and it still overflows, the payload is replaced with a
/// tiny stub, which trivially fits. Monotone decrease, bounded below by the stub.
///
/// **Signature choice (`&mut`, double-serialize accepted):** on the common
/// under-budget path this serializes the frame once to decide it fits, then
/// `encrypt_observer_payload` serializes it again — one extra `to_string` of an
/// already-small frame. Reusing that string would mean changing buzz-core's
/// `encrypt_observer_payload` signature or adding a parallel encrypt path; both
/// are out of this change's scope (buzz-core stays untouched). The clean `&mut`
/// signature with one cheap redundant serialize is the deliberate tradeoff.
fn fit_observer_event_to_budget(event: &mut observer::ObserverEvent) {
    if serialized_len(event) <= OBSERVER_MAX_PLAINTEXT_LEN {
        return;
    }

    // Raw size of the payload we are about to trim, captured before mutation so
    // the stub's `originalBytes` reports source bytes discarded, not serialized
    // overflow — consistent with the per-leaf marker's raw byte count.
    let original_payload_bytes = serde_json::to_string(&event.payload)
        .map(|s| s.len())
        .unwrap_or(0);

    // Elide the largest shrinkable leaf, reserialize, repeat. Each successful
    // elision strictly shrinks the serialized frame, and a floored leaf can
    // never be re-elided, so the loop is bounded by the leaf count.
    while let Some(leaf) = largest_shrinkable_leaf(&mut event.payload) {
        elide_leaf(leaf);
        if serialized_len(event) <= OBSERVER_MAX_PLAINTEXT_LEN {
            return;
        }
    }

    // No leaf can shrink the frame further and it still overflows: replace the
    // whole payload with a stub that is trivially under-cap.
    event.payload = serde_json::json!({
        "elided": format!("{} payload too large", event.kind),
        "originalBytes": original_payload_bytes,
    });
}

fn serialized_len(event: &observer::ObserverEvent) -> usize {
    serde_json::to_string(event).map(|s| s.len()).unwrap_or(0)
}

/// Find the longest string leaf that would STRICTLY shrink if elided, returning
/// a mutable handle to it. A leaf shrinks only if `head + marker + tail` is
/// shorter than its current value (the marker-pushback guard); a leaf already at
/// its retained floor fails this test and is skipped, which is what bounds the
/// loop. Returns `None` when no leaf can shrink.
fn largest_shrinkable_leaf(value: &mut serde_json::Value) -> Option<&mut serde_json::Value> {
    // First pass: find the byte length of the best candidate without holding a
    // borrow, then re-descend to return the matching mutable reference. Two
    // immutable-style passes keep the borrow checker happy without unsafe.
    let best_len = max_shrinkable_len(value)?;
    find_leaf_with_len(value, best_len)
}

/// Largest current length among string leaves that can strictly shrink.
fn max_shrinkable_len(value: &serde_json::Value) -> Option<usize> {
    match value {
        serde_json::Value::String(s) if leaf_shrinks(s) => Some(s.len()),
        serde_json::Value::String(_) => None,
        serde_json::Value::Array(items) => items.iter().filter_map(max_shrinkable_len).max(),
        serde_json::Value::Object(map) => map.values().filter_map(max_shrinkable_len).max(),
        _ => None,
    }
}

/// Return the first string leaf whose current length equals `target` and that
/// can strictly shrink. Used after `max_shrinkable_len` to re-acquire a mutable
/// borrow of the chosen leaf.
fn find_leaf_with_len(
    value: &mut serde_json::Value,
    target: usize,
) -> Option<&mut serde_json::Value> {
    match value {
        serde_json::Value::String(s) if s.len() == target && leaf_shrinks(s) => Some(value),
        serde_json::Value::Array(items) => items
            .iter_mut()
            .find_map(|item| find_leaf_with_len(item, target)),
        serde_json::Value::Object(map) => map
            .values_mut()
            .find_map(|item| find_leaf_with_len(item, target)),
        _ => None,
    }
}

/// True when eliding `s` to head + marker + tail yields a strictly shorter raw
/// string. The marker width grows with `N` (bytes removed), so a leaf only
/// marginally larger than the retained ends must NOT be touched.
fn leaf_shrinks(s: &str) -> bool {
    let (head_end, tail_start) = elision_boundaries(s);
    tail_start > head_end && {
        let removed = tail_start - head_end;
        let marker = elision_marker(removed);
        head_end + marker.len() + (s.len() - tail_start) < s.len()
    }
}

/// Replace the middle of a string leaf with `…[elided N bytes]…`, keeping a head
/// and tail slice on UTF-8 char boundaries. `N` is RAW bytes removed.
fn elide_leaf(leaf: &mut serde_json::Value) {
    let serde_json::Value::String(s) = leaf else {
        return;
    };
    let (head_end, tail_start) = elision_boundaries(s);
    let removed = tail_start - head_end;
    let mut elided = String::with_capacity(head_end + 32 + (s.len() - tail_start));
    elided.push_str(&s[..head_end]);
    elided.push_str(&elision_marker(removed));
    elided.push_str(&s[tail_start..]);
    *s = elided;
}

fn elision_marker(removed_bytes: usize) -> String {
    format!("…[elided {removed_bytes} bytes]…")
}

/// Byte offsets bounding the elided middle, snapped to char boundaries so we
/// never split a multi-byte char. Returns `(head_end, tail_start)` with
/// `head_end <= tail_start`.
fn elision_boundaries(s: &str) -> (usize, usize) {
    let head_end = floor_char_boundary(s, OBSERVER_LEAF_RETAIN_BYTES.min(s.len()));
    let tail_start = ceil_char_boundary(s, s.len().saturating_sub(OBSERVER_LEAF_RETAIN_BYTES));
    (head_end, tail_start.max(head_end))
}

fn floor_char_boundary(s: &str, mut i: usize) -> usize {
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn ceil_char_boundary(s: &str, mut i: usize) -> usize {
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

async fn publish_relay_observer_event(
    publisher: &RelayEventPublisher,
    keys: &nostr::Keys,
    agent_pubkey_hex: &str,
    owner_pubkey_hex: &str,
    owner_pubkey: &PublicKey,
    pacer: &mut ObserverPublishPacer,
    mut event: observer::ObserverEvent,
) {
    pacer.wait().await;
    // Trim oversized frames to fit the plaintext cap rather than letting
    // encrypt_observer_payload reject and drop them whole (silent telemetry loss).
    fit_observer_event_to_budget(&mut event);
    let encrypted = match encrypt_observer_payload(keys, owner_pubkey, &event) {
        Ok(encrypted) => encrypted,
        Err(error) => {
            tracing::warn!("failed to encrypt relay observer event: {error}");
            return;
        }
    };
    let builder = match buzz_sdk::build_agent_observer_frame(
        owner_pubkey_hex,
        agent_pubkey_hex,
        OBSERVER_FRAME_TELEMETRY,
        &encrypted,
    ) {
        Ok(builder) => builder,
        Err(error) => {
            tracing::warn!("failed to build relay observer event: {error}");
            return;
        }
    };
    let signed = match builder.sign_with_keys(keys) {
        Ok(event) => event,
        Err(error) => {
            tracing::warn!("failed to sign relay observer event: {error}");
            return;
        }
    };
    if let Err(error) = publisher.publish_event(signed).await {
        tracing::warn!("relay observer event dropped: {error}");
    }
}

/// Maximum age (seconds) for an observer control frame to be considered fresh.
const OBSERVER_CONTROL_FRESHNESS_SECS: i64 = 300;
/// Retain every authenticated control ID for the full freshness window. If an
/// owner legitimately produces more than this many controls before entries
/// expire, fail closed instead of evicting replay protection early.
const OBSERVER_CONTROL_REPLAY_CAPACITY: usize = 8_192;

#[derive(Debug, Default)]
struct ObserverControlReplayCache {
    seen: HashSet<String>,
    expiry_order: VecDeque<(String, i64)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ObserverControlAdmission {
    Accepted,
    Replay,
    Saturated,
}

impl ObserverControlReplayCache {
    fn admit(&mut self, event_id: String, event_ts: i64, now: i64) -> ObserverControlAdmission {
        while self
            .expiry_order
            .front()
            .is_some_and(|(_, expires_at)| *expires_at < now)
        {
            if let Some((expired_id, _)) = self.expiry_order.pop_front() {
                self.seen.remove(&expired_id);
            }
        }
        if self.seen.contains(&event_id) {
            return ObserverControlAdmission::Replay;
        }
        if self.seen.len() >= OBSERVER_CONTROL_REPLAY_CAPACITY {
            return ObserverControlAdmission::Saturated;
        }

        let expires_at = event_ts.saturating_add(OBSERVER_CONTROL_FRESHNESS_SECS);
        self.seen.insert(event_id.clone());
        self.expiry_order.push_back((event_id, expires_at));
        ObserverControlAdmission::Accepted
    }
}

fn handle_relay_observer_control_event(
    keys: &nostr::Keys,
    event: nostr::Event,
    pool: &mut AgentPool,
    observer: Option<&observer::ObserverHandle>,
    owner_pubkey_hex: &str,
    replay_cache: &mut ObserverControlReplayCache,
) {
    // Defense-in-depth: verify signature even though the relay already checked.
    if let Err(e) = buzz_core::verify_event(&event) {
        tracing::warn!(error = %e, "observer control frame failed signature verification");
        return;
    }

    // Defense-in-depth: verify the sender is the resolved owner.
    if event.pubkey.to_hex() != owner_pubkey_hex {
        tracing::warn!(
            sender = %event.pubkey,
            expected = %owner_pubkey_hex,
            "observer control frame from non-owner — dropping"
        );
        return;
    }

    // Freshness: reject stale/replayed frames outside ±5 minute window.
    let now = chrono::Utc::now().timestamp();
    let event_ts = event.created_at.as_secs() as i64;
    if (event_ts - now).unsigned_abs() > OBSERVER_CONTROL_FRESHNESS_SECS as u64 {
        tracing::warn!(
            event_ts,
            now,
            "observer control frame outside freshness window — dropping"
        );
        return;
    }

    let payload = match decrypt_observer_payload::<serde_json::Value>(keys, &event) {
        Ok(payload) => payload,
        Err(error) => {
            tracing::warn!("failed to decrypt observer control frame: {error}");
            return;
        }
    };

    route_authenticated_observer_control(
        event.id.to_hex(),
        event_ts,
        now,
        &payload,
        pool,
        observer,
        replay_cache,
    );
}

fn route_authenticated_observer_control(
    event_id: String,
    event_ts: i64,
    now: i64,
    payload: &serde_json::Value,
    pool: &mut AgentPool,
    observer: Option<&observer::ObserverHandle>,
    replay_cache: &mut ObserverControlReplayCache,
) {
    let command_type = payload.get("type").and_then(|value| value.as_str());
    if !matches!(command_type, Some("cancel_turn" | "switch_model")) {
        tracing::debug!(payload = %payload, "ignoring unknown observer control frame");
        return;
    }
    match replay_cache.admit(event_id.clone(), event_ts, now) {
        ObserverControlAdmission::Accepted => {}
        ObserverControlAdmission::Replay => {
            tracing::warn!(%event_id, "dropping replayed observer control frame");
            return;
        }
        ObserverControlAdmission::Saturated => {
            tracing::error!(
                %event_id,
                capacity = OBSERVER_CONTROL_REPLAY_CAPACITY,
                "observer control replay cache saturated — failing closed"
            );
            return;
        }
    }

    match command_type {
        Some("cancel_turn") => {
            handle_cancel_turn_control(payload, pool, observer);
        }
        Some("switch_model") => {
            handle_switch_model_control(payload, pool, observer);
        }
        _ => unreachable!("recognized observer control type checked above"),
    }
}

/// Handle a `cancel_turn` control frame: signal the in-flight task to cancel.
fn handle_cancel_turn_control(
    payload: &serde_json::Value,
    pool: &mut AgentPool,
    observer: Option<&observer::ObserverHandle>,
) {
    let Some(channel_id) = payload
        .get("channelId")
        .and_then(|value| value.as_str())
        .and_then(|value| value.parse::<Uuid>().ok())
    else {
        tracing::warn!("observer cancel_turn control frame missing valid channelId");
        return;
    };

    let Some(turn_id) = payload
        .get("turnId")
        .and_then(|value| value.as_str())
        .filter(|value| !value.is_empty())
    else {
        tracing::warn!(
            channel = %channel_id,
            "observer cancel_turn control frame missing turnId — dropping fail closed"
        );
        return;
    };
    let status =
        signal_observer_control(pool, channel_id, turn_id, ControlSignal::Cancel).as_status();
    if let Some(observer) = observer {
        let mut result = serde_json::json!({
            "type": "cancel_turn",
            "status": status,
        });
        result["turnId"] = serde_json::Value::String(turn_id.to_string());
        observer.emit(
            "control_result",
            None,
            &observer::ObserverContext {
                channel_id: Some(channel_id.to_string()),
                session_id: None,
                turn_id: Some(turn_id.to_owned()),
                started_at: None,
            },
            result,
        );
    }
}

/// Handle a `switch_model` control frame (Phase 3a, Option ii).
///
/// Busy path: deliver `SwitchModel` over the in-flight task's oneshot — the
/// task cancels the turn, sets `desired_model`, and requeues the batch so it
/// re-runs on a fresh session under the new model. A catalog miss surfaces
/// post-cancel via `create_session_and_apply_model` (the turn restarts on the
/// unchanged model + an `unsupported_model` result).
///
fn handle_switch_model_control(
    payload: &serde_json::Value,
    pool: &mut AgentPool,
    observer: Option<&observer::ObserverHandle>,
) {
    let Some(channel_id) = payload
        .get("channelId")
        .and_then(|value| value.as_str())
        .and_then(|value| value.parse::<Uuid>().ok())
    else {
        tracing::warn!("observer switch_model control frame missing valid channelId");
        return;
    };
    let Some(model_id) = payload.get("modelId").and_then(|value| value.as_str()) else {
        tracing::warn!("observer switch_model control frame missing modelId");
        return;
    };
    let Some(turn_id) = payload
        .get("turnId")
        .and_then(|value| value.as_str())
        .filter(|value| !value.is_empty())
    else {
        tracing::warn!(
            channel = %channel_id,
            "observer switch_model control frame missing turnId — dropping fail closed"
        );
        return;
    };

    // Controls are deliberately turn-bound: if that turn completed between
    // the desktop snapshot and this frame arriving, never fall back to a
    // potentially unrelated current or idle thread in the same channel.
    let status = signal_observer_control(
        pool,
        channel_id,
        turn_id,
        ControlSignal::SwitchModel(model_id.to_string()),
    )
    .as_status();

    if let Some(observer) = observer {
        let mut result = serde_json::json!({
            "type": "switch_model",
            "status": status,
            "modelId": model_id,
        });
        result["turnId"] = serde_json::Value::String(turn_id.to_string());
        observer.emit(
            "control_result",
            None,
            &observer::ObserverContext {
                channel_id: Some(channel_id.to_string()),
                session_id: None,
                turn_id: Some(turn_id.to_owned()),
                started_at: None,
            },
            result,
        );
    }
}

/// Maximum crashes in a 60-second window before a slot's circuit opens.
const CIRCUIT_BREAKER_THRESHOLD: usize = 3;
/// Window for circuit-breaker crash counting.
const CIRCUIT_BREAKER_WINDOW: Duration = Duration::from_secs(60);
/// Cooldown before a tripped circuit breaker allows a probe respawn.
const CIRCUIT_BREAKER_COOLDOWN: Duration = Duration::from_secs(300); // 5 minutes
/// Base backoff delay for respawn (doubles per recent crash, capped at 30s).
const RESPAWN_BASE_DELAY: Duration = Duration::from_secs(1);
/// Maximum respawn backoff delay.
const RESPAWN_MAX_DELAY: Duration = Duration::from_secs(30);

/// Per-slot circuit breaker state.
///
/// `crash_times` holds timestamps of recent crashes within `CIRCUIT_BREAKER_WINDOW`.
/// `open_until` is set when the threshold is hit; the circuit stays open until that
/// instant, then allows one probe respawn (half-open). If the probe crashes, the
/// circuit re-opens for another `CIRCUIT_BREAKER_COOLDOWN` period.
///
/// All state transitions go through methods on this struct — callers never
/// manipulate `crash_times` or `open_until` directly.
struct SlotCircuit {
    crash_times: Vec<std::time::Instant>,
    open_until: Option<std::time::Instant>,
    /// True while a background respawn/refill task is in flight for this slot.
    /// Prevents duplicate spawns from maintenance ticks that fire before the
    /// previous spawn_and_init completes.
    respawn_in_flight: bool,
}

/// Result of [`SlotCircuit::record_crash`].
enum CrashVerdict {
    /// Respawn is allowed after sleeping for this duration (jittered backoff).
    Respawn(Duration),
    /// Circuit is open — do not respawn.
    CircuitOpen,
    /// Circuit was open but cooldown has elapsed — one probe respawn is allowed
    /// (no backoff sleep). If the probe crashes, the next `record_crash` will
    /// immediately re-open the circuit.
    HalfOpenProbe,
}

impl SlotCircuit {
    /// Record a crash and decide whether to respawn.
    ///
    /// This is the **single canonical path** for all crash → respawn decisions.
    /// Called by `respawn_agent_into`, `recover_panicked_agent`, and slot refill.
    fn record_crash(&mut self) -> CrashVerdict {
        let now = std::time::Instant::now();

        // Half-open: cooldown elapsed → allow one probe.
        if let Some(open_until) = self.open_until {
            if now >= open_until {
                // Pre-seed crash_times to threshold-1 so that if the probe
                // itself crashes on the *next* call, the threshold is hit
                // immediately and the circuit re-opens. This implements a
                // "prove stability for one full window" policy.
                self.crash_times.clear();
                for _ in 0..(CIRCUIT_BREAKER_THRESHOLD - 1) {
                    self.crash_times.push(now);
                }
                self.open_until = None;
                return CrashVerdict::HalfOpenProbe;
            } else {
                return CrashVerdict::CircuitOpen;
            }
        }

        // Record this crash and prune old entries.
        self.crash_times.push(now);
        self.crash_times
            .retain(|&t| now.duration_since(t) < CIRCUIT_BREAKER_WINDOW);

        let recent = self.crash_times.len();

        if recent >= CIRCUIT_BREAKER_THRESHOLD {
            self.open_until = Some(now + CIRCUIT_BREAKER_COOLDOWN);
            return CrashVerdict::CircuitOpen;
        }

        // Exponential backoff: 1s * 2^(recent-1), capped at 30s, with ±20% jitter.
        let base = RESPAWN_BASE_DELAY.saturating_mul(1u32 << (recent - 1).min(5));
        let capped = base.min(RESPAWN_MAX_DELAY);
        let jitter = (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .subsec_nanos() as f64)
            / 1_000_000_000.0; // 0.0..1.0
        let factor = 0.8 + jitter * 0.4; // 0.8..1.2
        CrashVerdict::Respawn(capped.mul_f64(factor))
    }

    /// Mark a spawn failure — opens the circuit so the slot isn't retried
    /// on every heartbeat tick. Uses fresh `Instant::now()` so spawn latency
    /// doesn't shorten the effective cooldown.
    fn mark_spawn_failed(&mut self) {
        self.open_until = Some(std::time::Instant::now() + CIRCUIT_BREAKER_COOLDOWN);
    }

    /// Check if an empty slot can be refilled. Unlike `record_crash`, this
    /// does NOT record a new crash — it only checks whether the circuit
    /// allows a respawn attempt.
    ///
    /// Returns `true` if respawn is allowed. For half-open probes, pre-seeds
    /// crash_times so the next crash re-opens immediately. For normal refills
    /// (no circuit was ever opened), crash history is preserved so the breaker
    /// can still trip if the refilled agent crashes quickly.
    fn can_refill(&mut self) -> bool {
        let now = std::time::Instant::now();
        match self.open_until {
            Some(open_until) => {
                if now >= open_until {
                    // Half-open probe: pre-seed crash_times.
                    self.crash_times.clear();
                    for _ in 0..(CIRCUIT_BREAKER_THRESHOLD - 1) {
                        self.crash_times.push(now);
                    }
                    self.open_until = None;
                    true
                } else {
                    false // cooldown not elapsed
                }
            }
            None => true, // no circuit open — normal refill, preserve crash history
        }
    }
}

/// True if any slot has a respawn task in flight. Used to prevent premature
/// "all agents dead" exits — a respawning agent may succeed in seconds.
fn any_respawn_in_flight(crash_history: &[SlotCircuit]) -> bool {
    crash_history.iter().any(|s| s.respawn_in_flight)
}

/// Result of a background respawn task.
struct RespawnResult {
    index: usize,
    /// Tuple: (initialized client, protocol version, agent name).
    result: Result<(AcpClient, u32, String)>,
}

/// Outcome of a non-cancelling steer attempt, forwarded from a per-attempt
/// watcher task (which awaits the `SteerRequest.ack_tx` oneshot) back to
/// the main loop's `select!`. The main loop drives queue side-effects from
/// this — it cannot await the oneshot itself without blocking the relay
/// stream.
///
/// Carries enough identity to operate on the right withheld event in
/// `EventQueue::withheld_native_steer`: `channel_id` is the routing key,
/// `event_id` is the hex id of the single event the steer carried.
struct SteerAckEvent {
    channel_id: Uuid,
    conversation_key: ConversationKey,
    event_id: String,
    /// `Ok` if the read loop sent any of the locked `SteerAck` variants.
    /// `Err` if the oneshot was dropped without a send — should not happen
    /// under the current read-loop drains, but if it ever does the main
    /// loop treats it as `PromptCompletedNeutral` (release withheld, no
    /// fallback signal) to avoid leaking the withheld event.
    ack: std::result::Result<pool::SteerAck, tokio::sync::oneshot::error::RecvError>,
}

struct PendingResetConfirmation {
    event: nostr::Event,
    next_attempt_at: Instant,
    durable: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ResetAckAttemptKey {
    identity: (Uuid, String),
    event_id: String,
}

#[derive(Debug)]
struct ResetAckAttemptResult {
    key: ResetAckAttemptKey,
    accepted: bool,
}

const RESET_ACK_RETRY_INTERVAL: Duration = Duration::from_secs(5);
const RESET_HELPER_INIT_TIMEOUT: Duration = Duration::from_secs(15);
const RESET_HELPER_ATTEMPTS: usize = 2;

fn reset_confirmation(thread_sessions_enabled: bool) -> &'static str {
    if thread_sessions_enabled {
        "✨ Fresh context ready. I’ll keep this Buzz thread separate and start the next request without its prior conversation history."
    } else {
        "✨ Fresh context ready. This agent adapter does not support thread-local sessions, so the reset applies to this entire Buzz channel."
    }
}

/// Commit `/new` in the adapter's durable store before relying on any
/// in-memory queue/barrier state. An idle pool process is preferred. When the
/// only process is busy with the turn being reset, a short-lived ACP helper is
/// initialized against the same adapter state and performs the idempotent
/// logical reset by stable conversation id.
async fn commit_durable_conversation_reset(
    pool: &mut AgentPool,
    config: &Config,
    conversation_id: &str,
    reset_token: &str,
) -> Option<ConversationResetCommit> {
    commit_durable_conversation_reset_with_adapter(
        pool,
        &config.agent_command,
        &config.agent_args,
        &config.persona_env_vars,
        config.has_generated_codex_config,
        conversation_id,
        reset_token,
    )
    .await
}

async fn commit_durable_conversation_reset_with_adapter(
    pool: &mut AgentPool,
    agent_command: &str,
    agent_args: &[String],
    persona_env_vars: &[(String, String)],
    has_generated_codex_config: bool,
    conversation_id: &str,
    reset_token: &str,
) -> Option<ConversationResetCommit> {
    if let Some(commit) = pool
        .commit_conversation_reset(conversation_id, reset_token)
        .await
    {
        return Some(commit);
    }

    // A timeout/EOF after the request is ambiguous: the adapter may have
    // committed just before its response was lost. Retry the same idempotency
    // token through a fresh process so a durable commit can be confirmed.
    for attempt in 1..=RESET_HELPER_ATTEMPTS {
        let mut helper = match AcpClient::spawn(
            agent_command,
            agent_args,
            persona_env_vars,
            has_generated_codex_config,
        )
        .await
        {
            Ok(helper) => helper,
            Err(error) => {
                tracing::error!(
                    conversation_id,
                    attempt,
                    %error,
                    "could not spawn the durable reset helper"
                );
                continue;
            }
        };

        let initialized =
            tokio::time::timeout(RESET_HELPER_INIT_TIMEOUT, helper.initialize()).await;
        let supported = match initialized {
            Ok(Ok(_)) => {
                helper.thread_sessions_supported()
                    && helper.conversation_reset_supported()
                    && helper.durable_session_events_supported()
            }
            Ok(Err(error)) => {
                tracing::error!(
                    conversation_id,
                    attempt,
                    %error,
                    "durable reset helper initialization failed"
                );
                false
            }
            Err(_) => {
                tracing::error!(
                    conversation_id,
                    attempt,
                    timeout_secs = RESET_HELPER_INIT_TIMEOUT.as_secs(),
                    "durable reset helper initialization timed out"
                );
                false
            }
        };

        let committed = if supported {
            match helper
                .conversation_reset(conversation_id, reset_token)
                .await
            {
                Ok(Some(commit)) => Some(commit),
                Ok(None) => {
                    tracing::error!(
                        conversation_id,
                        attempt,
                        "durable reset helper withdrew its advertised reset capability"
                    );
                    None
                }
                Err(error) => {
                    tracing::error!(
                        conversation_id,
                        attempt,
                        %error,
                        "durable reset helper reset result was not confirmed"
                    );
                    None
                }
            }
        } else {
            tracing::error!(
                conversation_id,
                attempt,
                "durable reset helper does not advertise the required thread reset contract"
            );
            None
        };
        helper.shutdown().await;
        if committed.is_some() {
            return committed;
        }
    }
    None
}

/// Resolve crash-interrupted `/new` intents before relay intake. A prepared
/// record means the signed acknowledgement was durably stored before the Pi
/// tombstone RPC began, but the process died before recording its response.
/// Repeating the same reset token is idempotent and converts that ambiguity
/// into a confirmed committed record.
async fn recover_durable_reset_intents(
    outbox: &DurableOutbox,
    pool: &mut AgentPool,
    config: &Config,
    thread_sessions_enabled: bool,
) -> Result<Vec<DurableResetRecord>> {
    for record in outbox
        .pending_resets()
        .map_err(|error| anyhow::anyhow!("could not read durable /new intents: {error}"))?
    {
        if record.phase == DurableResetPhase::Committed {
            continue;
        }
        if !thread_sessions_enabled {
            return Err(anyhow::anyhow!(
                "a prepared durable /new intent exists, but the configured adapter no longer supports durable thread sessions"
            ));
        }
        if commit_durable_conversation_reset(
            pool,
            config,
            &record.conversation_id,
            &record.reset_token,
        )
        .await
        .is_none()
        {
            return Err(anyhow::anyhow!(
                "could not recover durable /new intent for {}:{}; refusing relay intake",
                record.channel_id,
                record.root_event_id
            ));
        }
        let identity = record.identity();
        if !outbox
            .mark_reset_committed(&identity, &record.event.id.to_hex())
            .map_err(|error| anyhow::anyhow!("could not checkpoint recovered /new: {error}"))?
        {
            return Err(anyhow::anyhow!(
                "recovered /new record disappeared before it could be committed"
            ));
        }
    }
    outbox
        .pending_resets()
        .map_err(|error| anyhow::anyhow!("could not read recovered /new intents: {error}"))
}

/// Select queue identity before any event is accepted. A required durable
/// adapter declares thread scope up front even when its pool starts lazily;
/// once a process is initialized, failure to advertise the full contract is a
/// startup error rather than a silent downgrade to channel-wide context.
fn select_thread_session_scope(
    durable_sessions_required: bool,
    pool_ready: bool,
    durable_sessions_supported: bool,
) -> Result<bool, &'static str> {
    if pool_ready && durable_sessions_required && !durable_sessions_supported {
        return Err(
            "configured adapter does not advertise required durable thread-session and reset-commit capabilities",
        );
    }
    Ok(durable_sessions_required || (pool_ready && durable_sessions_supported))
}

/// Start at most one reset acknowledgement delivery attempt without awaiting
/// the relay. The reset barrier remains closed until the signed event is
/// explicitly accepted, while a slow or black-holed HTTP bridge cannot stall
/// WebSocket ingress, prompt completion, or shutdown handling.
fn schedule_completed_reset_ack(
    queue: &EventQueue,
    pending: &HashMap<(Uuid, String), PendingResetConfirmation>,
    rest: &relay::RestClient,
    tasks: &mut tokio::task::JoinSet<ResetAckAttemptResult>,
    in_flight: &mut Option<ResetAckAttemptKey>,
) {
    if in_flight.is_some() {
        return;
    }

    for identity in queue.completed_resets() {
        let Some(confirmation) = pending.get(&identity) else {
            tracing::warn!(
                channel_id = %identity.0,
                root_event_id = %identity.1,
                "completed reset barrier has no signed acknowledgement; keeping it closed"
            );
            continue;
        };
        let next_attempt_at = confirmation.next_attempt_at;
        let event = confirmation.event.clone();
        if Instant::now() < next_attempt_at {
            continue;
        }

        let key = ResetAckAttemptKey {
            identity,
            event_id: event.id.to_string(),
        };
        let task_key = key.clone();
        let task_rest = rest.clone();
        *in_flight = Some(key);
        tasks.spawn(async move {
            let accepted = pool::submit_notice_event(&task_rest, &event).await;
            ResetAckAttemptResult {
                key: task_key,
                accepted,
            }
        });
        return;
    }
}

/// Apply one tracked delivery result. Event identity is checked before the
/// barrier is released so a stale completion can never acknowledge a newer
/// reset for the same Buzz thread.
fn finish_reset_ack_attempt(
    queue: &mut EventQueue,
    pending: &mut HashMap<(Uuid, String), PendingResetConfirmation>,
    in_flight: &mut Option<ResetAckAttemptKey>,
    durable_outbox: Option<&DurableOutbox>,
    result: std::result::Result<ResetAckAttemptResult, tokio::task::JoinError>,
) {
    let Some(expected) = in_flight.take() else {
        tracing::warn!("reset acknowledgement task completed without in-flight identity");
        return;
    };

    let (key, accepted) = match result {
        Ok(result) if result.key == expected => (result.key, result.accepted),
        Ok(result) => {
            tracing::error!(
                expected_channel_id = %expected.identity.0,
                expected_root_event_id = %expected.identity.1,
                actual_channel_id = %result.key.identity.0,
                actual_root_event_id = %result.key.identity.1,
                "reset acknowledgement task returned the wrong identity"
            );
            if let Some(confirmation) = pending.get_mut(&expected.identity) {
                confirmation.next_attempt_at = Instant::now() + RESET_ACK_RETRY_INTERVAL;
            }
            return;
        }
        Err(error) => {
            tracing::warn!(
                channel_id = %expected.identity.0,
                root_event_id = %expected.identity.1,
                %error,
                "reset acknowledgement task failed"
            );
            if let Some(confirmation) = pending.get_mut(&expected.identity) {
                confirmation.next_attempt_at = Instant::now() + RESET_ACK_RETRY_INTERVAL;
            }
            return;
        }
    };

    let is_current_event = pending
        .get(&key.identity)
        .is_some_and(|confirmation| confirmation.event.id.to_string() == key.event_id);
    if !is_current_event {
        tracing::warn!(
            channel_id = %key.identity.0,
            root_event_id = %key.identity.1,
            event_id = %key.event_id,
            "ignoring stale reset acknowledgement completion"
        );
        return;
    }

    if accepted {
        let requires_durable_completion = pending
            .get(&key.identity)
            .is_some_and(|confirmation| confirmation.durable);
        if requires_durable_completion {
            let durable_result = durable_outbox
                .ok_or_else(|| "durable outbox unavailable".to_string())
                .and_then(|outbox| {
                    outbox
                        .complete_reset(&key.identity, &key.event_id)
                        .map_err(|error| error.to_string())
                });
            match durable_result {
                Ok(true) => {}
                Ok(false) => {
                    tracing::error!(
                        channel_id = %key.identity.0,
                        root_event_id = %key.identity.1,
                        event_id = %key.event_id,
                        "accepted reset acknowledgement was absent from durable state; keeping barrier closed"
                    );
                    if let Some(confirmation) = pending.get_mut(&key.identity) {
                        confirmation.next_attempt_at = Instant::now() + RESET_ACK_RETRY_INTERVAL;
                    }
                    return;
                }
                Err(error) => {
                    tracing::error!(
                        channel_id = %key.identity.0,
                        root_event_id = %key.identity.1,
                        event_id = %key.event_id,
                        %error,
                        "relay accepted reset acknowledgement but durable completion failed; keeping barrier closed"
                    );
                    if let Some(confirmation) = pending.get_mut(&key.identity) {
                        confirmation.next_attempt_at = Instant::now() + RESET_ACK_RETRY_INTERVAL;
                    }
                    return;
                }
            }
        }
        pending.remove(&key.identity);
        queue.release_completed_reset(&key.identity);
    } else if let Some(confirmation) = pending.get_mut(&key.identity) {
        confirmation.next_attempt_at = Instant::now() + RESET_ACK_RETRY_INTERVAL;
        tracing::warn!(
            channel_id = %key.identity.0,
            root_event_id = %key.identity.1,
            event_id = %key.event_id,
            "reset acknowledgement is still pending relay delivery"
        );
    }
}

/// Earliest signed reset acknowledgement that is both ready to publish and
/// guarding a completed reset barrier. Keeping this deadline in the main
/// `select!` prevents a quiet relay from leaving `/new` blocked forever after
/// one transient HTTP failure. Confirmations whose old turn is still draining
/// are deliberately excluded to avoid a past-deadline busy loop.
fn next_completed_reset_ack_attempt(
    queue: &EventQueue,
    pending: &HashMap<(Uuid, String), PendingResetConfirmation>,
) -> Option<Instant> {
    queue
        .completed_resets()
        .into_iter()
        .filter_map(|identity| pending.get(&identity).map(|item| item.next_attempt_at))
        .min()
}

#[cfg(test)]
mod reset_ack_retry_tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind};

    fn signed_notice() -> nostr::Event {
        EventBuilder::new(Kind::Custom(KIND_STREAM_MESSAGE as u16), "reset ready")
            .sign_with_keys(&Keys::generate())
            .expect("sign reset notice")
    }

    fn completed_reset() -> (EventQueue, ConversationKey) {
        let mut queue = EventQueue::new(DedupMode::Queue).with_thread_sessions_enabled(true);
        let conversation = ConversationKey {
            channel_id: Uuid::new_v4(),
            root_event_id: "44".repeat(32),
            generation: 0,
            reset_token: None,
        };
        queue
            .reset_conversation(&conversation, "cc".repeat(32))
            .expect("idle reset completes immediately");
        (queue, conversation)
    }

    fn unreachable_rest_client() -> relay::RestClient {
        relay::RestClient {
            http: reqwest::Client::new(),
            base_url: "http://127.0.0.1:9".to_string(),
            keys: Keys::generate(),
            auth_tag_json: None,
        }
    }

    #[test]
    fn retry_deadline_tracks_only_completed_reset_barriers() {
        let mut queue = EventQueue::new(DedupMode::Queue).with_thread_sessions_enabled(true);
        let first = ConversationKey {
            channel_id: Uuid::new_v4(),
            root_event_id: "11".repeat(32),
            generation: 0,
            reset_token: None,
        };
        let second = ConversationKey {
            channel_id: Uuid::new_v4(),
            root_event_id: "22".repeat(32),
            generation: 0,
            reset_token: None,
        };
        queue
            .reset_conversation(&first, "aa".repeat(32))
            .expect("first idle reset");
        queue
            .reset_conversation(&second, "bb".repeat(32))
            .expect("second idle reset");

        let now = Instant::now();
        let first_deadline = now + Duration::from_secs(7);
        let second_deadline = now + Duration::from_secs(3);
        let unrelated_identity = (Uuid::new_v4(), "33".repeat(32));
        let pending = HashMap::from([
            (
                (first.channel_id, first.root_event_id.clone()),
                PendingResetConfirmation {
                    event: signed_notice(),
                    next_attempt_at: first_deadline,
                    durable: false,
                },
            ),
            (
                (second.channel_id, second.root_event_id.clone()),
                PendingResetConfirmation {
                    event: signed_notice(),
                    next_attempt_at: second_deadline,
                    durable: false,
                },
            ),
            (
                unrelated_identity,
                PendingResetConfirmation {
                    event: signed_notice(),
                    next_attempt_at: now,
                    durable: false,
                },
            ),
        ]);

        assert_eq!(
            next_completed_reset_ack_attempt(&queue, &pending),
            Some(second_deadline),
            "unrelated past-due confirmations must not busy-spin the loop"
        );
    }

    #[tokio::test]
    async fn reset_ack_scheduler_is_non_blocking_and_globally_bounded() {
        let (queue, conversation) = completed_reset();
        let identity = (conversation.channel_id, conversation.root_event_id.clone());
        let pending = HashMap::from([(
            identity,
            PendingResetConfirmation {
                event: signed_notice(),
                next_attempt_at: Instant::now(),
                durable: false,
            },
        )]);
        let mut tasks = tokio::task::JoinSet::new();
        let mut in_flight = None;

        schedule_completed_reset_ack(
            &queue,
            &pending,
            &unreachable_rest_client(),
            &mut tasks,
            &mut in_flight,
        );
        assert!(in_flight.is_some());
        assert_eq!(tasks.len(), 1);

        // Re-entering the scheduler while HTTP is pending cannot spawn a
        // second task, even if more barriers become ready.
        schedule_completed_reset_ack(
            &queue,
            &pending,
            &unreachable_rest_client(),
            &mut tasks,
            &mut in_flight,
        );
        assert_eq!(tasks.len(), 1);
        tasks.shutdown().await;
    }

    #[test]
    fn accepted_reset_ack_releases_only_the_matching_barrier() {
        let (mut queue, conversation) = completed_reset();
        let identity = (conversation.channel_id, conversation.root_event_id.clone());
        let event = signed_notice();
        let key = ResetAckAttemptKey {
            identity: identity.clone(),
            event_id: event.id.to_string(),
        };
        let mut pending = HashMap::from([(
            identity.clone(),
            PendingResetConfirmation {
                event,
                next_attempt_at: Instant::now(),
                durable: false,
            },
        )]);
        let mut in_flight = Some(key.clone());

        finish_reset_ack_attempt(
            &mut queue,
            &mut pending,
            &mut in_flight,
            None,
            Ok(ResetAckAttemptResult {
                key,
                accepted: true,
            }),
        );

        assert!(in_flight.is_none());
        assert!(!pending.contains_key(&identity));
        assert!(queue.completed_resets().is_empty());
    }

    #[test]
    fn accepted_durable_reset_ack_atomically_becomes_a_replay_receipt() {
        let (mut queue, conversation) = completed_reset();
        let identity = (conversation.channel_id, conversation.root_event_id.clone());
        let keys = Keys::generate();
        let event = EventBuilder::new(
            Kind::Custom(KIND_STREAM_MESSAGE as u16),
            reset_confirmation(true),
        )
        .sign_with_keys(&keys)
        .expect("sign reset acknowledgement");
        let event_id = event.id.to_hex();
        let reset_token = "dd".repeat(32);
        let dir = tempfile::tempdir().expect("durable state dir");
        let durable = DurableOutbox::open(dir.path(), &keys.public_key().to_hex()).expect("open");
        durable
            .prepare_reset(DurableResetRecord {
                channel_id: identity.0,
                root_event_id: identity.1.clone(),
                conversation_id: conversation.stable_id(),
                reset_token: reset_token.clone(),
                event: event.clone(),
                phase: DurableResetPhase::Prepared,
            })
            .expect("prepare durable reset");
        assert!(durable
            .mark_reset_committed(&identity, &event_id)
            .expect("commit durable reset"));
        let key = ResetAckAttemptKey {
            identity: identity.clone(),
            event_id,
        };
        let mut pending = HashMap::from([(
            identity.clone(),
            PendingResetConfirmation {
                event,
                next_attempt_at: Instant::now(),
                durable: true,
            },
        )]);
        let mut in_flight = Some(key.clone());

        finish_reset_ack_attempt(
            &mut queue,
            &mut pending,
            &mut in_flight,
            Some(&durable),
            Ok(ResetAckAttemptResult {
                key,
                accepted: true,
            }),
        );

        assert!(pending.is_empty());
        assert!(queue.completed_resets().is_empty());
        assert!(durable
            .pending_resets()
            .expect("read pending resets")
            .is_empty());
        assert!(durable
            .consumed_reset(&identity, &reset_token)
            .expect("read receipt")
            .is_some());
    }

    #[test]
    fn stale_reset_ack_cannot_release_a_newer_notice() {
        let (mut queue, conversation) = completed_reset();
        let identity = (conversation.channel_id, conversation.root_event_id.clone());
        let stale_event = signed_notice();
        let current_event = signed_notice();
        let stale_key = ResetAckAttemptKey {
            identity: identity.clone(),
            event_id: stale_event.id.to_string(),
        };
        let mut pending = HashMap::from([(
            identity.clone(),
            PendingResetConfirmation {
                event: current_event,
                next_attempt_at: Instant::now(),
                durable: false,
            },
        )]);
        let mut in_flight = Some(stale_key.clone());

        finish_reset_ack_attempt(
            &mut queue,
            &mut pending,
            &mut in_flight,
            None,
            Ok(ResetAckAttemptResult {
                key: stale_key,
                accepted: true,
            }),
        );

        assert!(pending.contains_key(&identity));
        assert_eq!(queue.completed_resets().len(), 1);
    }

    #[test]
    fn required_durable_scope_is_selected_before_a_lazy_pool_wakes() {
        assert_eq!(select_thread_session_scope(true, false, false), Ok(true));
    }

    #[test]
    fn required_durable_scope_fails_closed_after_an_unsupported_wake() {
        assert!(select_thread_session_scope(true, true, false).is_err());
    }

    #[test]
    fn optional_scope_uses_the_initialized_adapter_capability() {
        assert_eq!(select_thread_session_scope(false, false, false), Ok(false));
        assert_eq!(select_thread_session_scope(false, true, false), Ok(false));
        assert_eq!(select_thread_session_scope(false, true, true), Ok(true));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn durable_reset_retries_same_token_when_first_commit_response_is_dropped() {
        let marker =
            std::env::temp_dir().join(format!("buzz-acp-reset-commit-{}", uuid::Uuid::new_v4()));
        let script = format!(
            r#"
                read -r _init
                echo '{{"jsonrpc":"2.0","id":0,"result":{{"protocolVersion":2,"_meta":{{"buzz":{{"threadSessions":{{"supported":true,"persistence":true,"dispose":true,"resetToken":true,"resetCommit":true}},"sessionEvents":{{"supported":true,"schemaVersion":2,"durableReplay":true,"ack":true}}}}}}}}}}'
                read -r _reset
                if [ -f '{marker}' ]; then
                    echo '{{"jsonrpc":"2.0","id":1,"result":{{"committed":true,"alreadyCommitted":true}}}}'
                    sleep 1
                else
                    printf committed > '{marker}'
                    exit 0
                fi
            "#,
            marker = marker.display(),
        );
        let mut pool = AgentPool::from_slots(vec![None]);
        let committed = commit_durable_conversation_reset_with_adapter(
            &mut pool,
            "bash",
            &["-c".to_string(), script],
            &[],
            false,
            "buzz:channel:root",
            "signed-reset-event-id",
        )
        .await;
        assert_eq!(
            committed,
            Some(ConversationResetCommit::AlreadyCommitted),
            "fresh helper must distinguish the idempotent retry"
        );
        assert!(marker.exists(), "first helper performed the durable commit");
        std::fs::remove_file(marker).expect("remove reset test marker");
    }
}

/// RAII guard that ensures a `RespawnResult` is sent even if the task panics.
/// Without this, a panicked respawn task would leave `respawn_in_flight = true`
/// permanently, silently losing the slot forever.
struct RespawnGuard {
    index: usize,
    tx: mpsc::Sender<RespawnResult>,
    sent: bool,
}

impl RespawnGuard {
    fn new(index: usize, tx: mpsc::Sender<RespawnResult>) -> Self {
        Self {
            index,
            tx,
            sent: false,
        }
    }

    /// Send the result and disarm the guard. Uses `try_send` (sync) so there
    /// is no await boundary between marking `sent` and actually enqueueing —
    /// cancellation cannot slip between the two.
    fn send(mut self, result: Result<(AcpClient, u32, String)>) {
        // Invariant: try_send succeeds because the channel capacity equals the
        // slot count, and respawn_in_flight guarantees at most one outstanding
        // result per slot. If this ever fails, the channel sizing or the
        // respawn_in_flight guard has drifted — that's a bug, not a transient.
        match self.tx.try_send(RespawnResult {
            index: self.index,
            result,
        }) {
            Ok(()) => self.sent = true,
            Err(e) => {
                tracing::error!(
                    agent = self.index,
                    "respawn result channel full or closed: {e}"
                );
                // Drop will fire and send a failure result as fallback.
            }
        }
    }
}

impl Drop for RespawnGuard {
    fn drop(&mut self) {
        if !self.sent {
            tracing::error!(
                agent = self.index,
                "respawn task exited without sending result — sending failure"
            );
            // Best-effort: try_send in Drop (can't await).
            let _ = self.tx.try_send(RespawnResult {
                index: self.index,
                result: Err(anyhow::anyhow!("respawn task panicked or was cancelled")),
            });
        }
    }
}

//
// Sync env-var propagation must run before the tokio runtime starts so that
// any child processes inherit the correct environment. This must happen in the
// sync entry point — `std::env::set_var` is only safe before tokio spawns
// worker threads (Rust 2024 edition safety requirement).

fn inactivity_expired(
    last_activity: tokio::time::Instant,
    now: tokio::time::Instant,
    bound: Duration,
    turn_in_flight: bool,
) -> bool {
    !bound.is_zero() && !turn_in_flight && now.duration_since(last_activity) >= bound
}

#[cfg(test)]
mod inactivity_tests {
    use super::*;

    #[test]
    fn zero_disables_expiry_and_in_flight_turns_defer_it() {
        let started = tokio::time::Instant::now();
        let after_bound = started + Duration::from_secs(61);

        assert!(!inactivity_expired(
            started,
            after_bound,
            Duration::ZERO,
            false
        ));
        assert!(!inactivity_expired(
            started,
            after_bound,
            Duration::from_secs(60),
            true
        ));
        assert!(inactivity_expired(
            started,
            after_bound,
            Duration::from_secs(60),
            false
        ));
    }

    #[test]
    fn dispatched_activity_restarts_the_inactivity_bound() {
        let started = tokio::time::Instant::now();
        let dispatched = started + Duration::from_secs(50);
        let checked = started + Duration::from_secs(61);

        assert!(!inactivity_expired(
            dispatched,
            checked,
            Duration::from_secs(60),
            false
        ));
    }
}

pub fn run() -> Result<()> {
    config::propagate_legacy_env_vars();
    tokio_main()
}

/// Exercise the production Buzz↔Pi ACP contract against a built adapter.
///
/// This is intentionally a narrow public probe rather than exposing the
/// harness's internal ACP client module. It starts the real Node adapter in an
/// isolated state directory, proves a logical conversation survives an adapter
/// restart, commits an idempotent reset, and verifies the replacement reports
/// that relay history must be skipped. It never sends a model prompt or reads
/// the caller's normal Pi settings.
///
/// `adapter_entrypoint`, `state_dir`, `pi_agent_dir`, and `cwd` must be absolute
/// paths. The adapter entrypoint is run with the `node` executable on `PATH`.
#[doc(hidden)]
pub async fn verify_pi_adapter_contract(
    adapter_entrypoint: &std::path::Path,
    state_dir: &std::path::Path,
    pi_agent_dir: &std::path::Path,
    cwd: &std::path::Path,
) -> Result<()> {
    for (name, path) in [
        ("adapter entrypoint", adapter_entrypoint),
        ("state directory", state_dir),
        ("Pi agent directory", pi_agent_dir),
        ("working directory", cwd),
    ] {
        anyhow::ensure!(
            path.is_absolute(),
            "{name} must be absolute: {}",
            path.display()
        );
    }
    anyhow::ensure!(
        adapter_entrypoint.is_file(),
        "Pi adapter entrypoint does not exist: {}",
        adapter_entrypoint.display()
    );
    std::fs::create_dir_all(state_dir)?;
    std::fs::create_dir_all(pi_agent_dir)?;

    async fn spawn_adapter(
        adapter_entrypoint: &std::path::Path,
        state_dir: &std::path::Path,
        pi_agent_dir: &std::path::Path,
        namespace: &str,
    ) -> Result<AcpClient> {
        let isolated_env = [
            (
                "BUZZ_PI_STATE_DIR".to_owned(),
                state_dir.to_string_lossy().into_owned(),
            ),
            ("BUZZ_PI_NAMESPACE".to_owned(), namespace.to_owned()),
            ("BUZZ_PI_CONTEXT_LIMIT".to_owned(), "150000".to_owned()),
            (
                "PI_CODING_AGENT_DIR".to_owned(),
                pi_agent_dir.to_string_lossy().into_owned(),
            ),
            ("BUZZ_PI_TRUST_PROJECT".to_owned(), "false".to_owned()),
            ("BUZZ_PI_LOG_LEVEL".to_owned(), "error".to_owned()),
        ];

        #[cfg(unix)]
        {
            // `AcpClient::spawn` normally gives parent environment variables
            // operator precedence. The contract probe must never reuse a
            // developer's real state, so launch through `env KEY=value ...`
            // and make the isolation overrides unambiguous.
            let mut args: Vec<String> = isolated_env
                .iter()
                .map(|(key, value)| format!("{key}={value}"))
                .collect();
            args.push("node".to_owned());
            args.push(adapter_entrypoint.to_string_lossy().into_owned());
            return AcpClient::spawn("env", &args, &[], false)
                .await
                .map_err(Into::into);
        }

        #[cfg(not(unix))]
        {
            AcpClient::spawn(
                "node",
                &[adapter_entrypoint.to_string_lossy().into_owned()],
                &isolated_env,
                false,
            )
            .await
            .map_err(Into::into)
        }
    }

    fn assert_initialize_contract(client: &AcpClient, response: &serde_json::Value) -> Result<()> {
        anyhow::ensure!(
            response
                .get("protocolVersion")
                .and_then(|value| value.as_u64())
                == Some(2),
            "Pi adapter did not negotiate ACP protocol version 2"
        );
        anyhow::ensure!(
            response
                .pointer("/agentInfo/name")
                .and_then(|value| value.as_str())
                == Some("buzz-pi-agent"),
            "Pi adapter initialize response has an unexpected agent identity"
        );
        anyhow::ensure!(
            response
                .pointer("/_meta/buzz/contextLimitTokens")
                .and_then(|value| value.as_u64())
                == Some(150_000),
            "Pi adapter initialize response did not advertise the 150k context ceiling"
        );
        anyhow::ensure!(
            response
                .pointer("/_meta/buzz/sessionEvents/supported")
                .and_then(|value| value.as_bool())
                == Some(true)
                && client.durable_session_events_supported(),
            "Pi adapter did not advertise durable replay plus ACKs for typed lifecycle events"
        );
        anyhow::ensure!(
            client.thread_sessions_supported() && client.conversation_reset_supported(),
            "Pi adapter did not advertise durable thread sessions plus reset commits"
        );
        Ok(())
    }

    fn pi_session_id(response: &acp::SessionNewResponse) -> Result<&str> {
        response
            .raw
            .pointer("/_meta/buzz/piSessionId")
            .and_then(|value| value.as_str())
            .ok_or_else(|| anyhow::anyhow!("session/new response omitted _meta.buzz.piSessionId"))
    }

    fn assert_session_shape(response: &acp::SessionNewResponse) -> Result<()> {
        anyhow::ensure!(
            response
                .raw
                .get("configOptions")
                .is_some_and(|value| value.is_array()),
            "session/new response omitted stable configOptions"
        );
        anyhow::ensure!(
            response
                .raw
                .pointer("/models/availableModels")
                .is_some_and(|value| value.is_array()),
            "session/new response omitted available model metadata"
        );
        anyhow::ensure!(
            response
                .raw
                .pointer("/_meta/buzz/persistent")
                .and_then(|value| value.as_bool())
                == Some(true),
            "Pi session was not persisted"
        );
        anyhow::ensure!(
            response
                .raw
                .pointer("/_meta/buzz/contextLimitTokens")
                .and_then(|value| value.as_u64())
                == Some(150_000),
            "session/new response lost the configured context ceiling"
        );
        Ok(())
    }

    let namespace = format!("contract-{}", Uuid::new_v4());
    let conversation_id = format!("contract:{}:thread", Uuid::new_v4());
    let reset_token = format!("contract-reset:{}", Uuid::new_v4());
    let cwd = cwd.to_string_lossy();

    let mut first = spawn_adapter(adapter_entrypoint, state_dir, pi_agent_dir, &namespace).await?;
    let first_result: Result<(String, String)> = async {
        let initialize = first.initialize().await?;
        assert_initialize_contract(&first, &initialize)?;

        // Heartbeat and compatibility/admin callers do not carry a Buzz
        // conversation identity. A server that negotiated durable schema v2
        // must never emit its legacy schema-v1 lifecycle envelope on those
        // sessions: doing so is a protocol-fatal downgrade in AcpClient. The
        // adapter therefore completes local commands but suppresses their
        // undeliverable lifecycle notice.
        let unmapped = first
            .session_new_full(
                &cwd,
                Vec::new(),
                None,
                Some("Buzz Pi unmapped lifecycle smoke"),
            )
            .await?;
        let unmapped_stop = first
            .session_prompt_with_idle_timeout(
                &unmapped.session_id,
                "/context",
                Duration::from_secs(15),
                Duration::from_secs(30),
            )
            .await?;
        anyhow::ensure!(
            unmapped_stop == acp::StopReason::EndTurn,
            "Pi's local /context command failed on an unmapped heartbeat-like session"
        );
        anyhow::ensure!(
            first.take_buzz_session_events().is_empty(),
            "Pi emitted a lifecycle envelope without a durable Buzz conversation identity"
        );
        anyhow::ensure!(
            first.session_dispose(&unmapped.session_id, false).await?,
            "Pi did not dispose the unmapped contract session"
        );

        let session = first
            .session_new_full_for_conversation(
                &cwd,
                Vec::new(),
                None,
                Some("Buzz Pi contract smoke"),
                Some(&conversation_id),
                None,
            )
            .await?;
        assert_session_shape(&session)?;
        anyhow::ensure!(
            session
                .raw
                .pointer("/_meta/buzz/resumedConversation")
                .and_then(|value| value.as_bool())
                == Some(false),
            "the first logical conversation unexpectedly resumed"
        );
        anyhow::ensure!(
            session
                .raw
                .pointer("/_meta/buzz/skipRelayHistory")
                .and_then(|value| value.as_bool())
                == Some(false),
            "the first logical conversation unexpectedly suppressed relay context"
        );
        let stop_reason = first
            .session_prompt_with_idle_timeout(
                &session.session_id,
                "/context",
                Duration::from_secs(15),
                Duration::from_secs(30),
            )
            .await?;
        anyhow::ensure!(
            stop_reason == acp::StopReason::EndTurn,
            "Pi's local /context command did not complete normally"
        );
        let lifecycle_events = first.take_buzz_session_events();
        anyhow::ensure!(
            lifecycle_events.len() == 1,
            "Pi /context emitted {} typed lifecycle events instead of one",
            lifecycle_events.len()
        );
        let context_event = &lifecycle_events[0];
        anyhow::ensure!(
            context_event.schema_version == acp::BUZZ_SESSION_EVENT_SCHEMA_VERSION
                && context_event.session_id == session.session_id
                && context_event.conversation_id == conversation_id
                && Uuid::parse_str(&context_event.event_id).is_ok(),
            "Pi /context lifecycle envelope did not match the active ACP session"
        );
        anyhow::ensure!(
            matches!(
                &context_event.event,
                BuzzSessionEvent::ContextStatus {
                    limit_tokens: 150_000,
                    auto_compaction: true,
                    ..
                }
            ),
            "Pi /context lifecycle payload did not report the 150k automatic-compaction policy"
        );
        let context_event_ack = acp::BuzzSessionEventAck {
            conversation_id: context_event.conversation_id.clone(),
            event_id: context_event.event_id.clone(),
        };
        first.queue_buzz_session_event_ack(context_event_ack);
        first.flush_buzz_session_event_acks().await;
        let pi_session_id = pi_session_id(&session)?.to_owned();
        Ok((session.session_id, pi_session_id))
    }
    .await;
    first.shutdown().await;
    let (first_acp_session_id, first_pi_session_id) = first_result?;

    let mut second = spawn_adapter(adapter_entrypoint, state_dir, pi_agent_dir, &namespace).await?;
    let second_result: Result<()> = async {
        let initialize = second.initialize().await?;
        assert_initialize_contract(&second, &initialize)?;
        let resumed = second
            .session_new_full_for_conversation(
                &cwd,
                Vec::new(),
                None,
                Some("Buzz Pi contract smoke"),
                Some(&conversation_id),
                None,
            )
            .await?;
        assert_session_shape(&resumed)?;
        anyhow::ensure!(
            resumed.session_id != first_acp_session_id,
            "a restarted adapter reused an outer ACP session ID"
        );
        anyhow::ensure!(
            pi_session_id(&resumed)? == first_pi_session_id,
            "a restarted adapter failed to resume the same durable Pi session"
        );
        anyhow::ensure!(
            resumed
                .raw
                .pointer("/_meta/buzz/resumedConversation")
                .and_then(|value| value.as_bool())
                == Some(true),
            "the restarted adapter did not report a resumed conversation"
        );
        anyhow::ensure!(
            resumed
                .raw
                .pointer("/_meta/buzz/skipRelayHistory")
                .and_then(|value| value.as_bool())
                == Some(false),
            "a normal resume incorrectly suppressed Buzz relay context"
        );

        anyhow::ensure!(
            second
                .conversation_reset(&conversation_id, &reset_token)
                .await?
                == Some(ConversationResetCommit::NewlyCommitted),
            "Pi adapter did not explicitly confirm the durable reset"
        );
        anyhow::ensure!(
            second
                .conversation_reset(&conversation_id, &reset_token)
                .await?
                == Some(ConversationResetCommit::AlreadyCommitted),
            "Pi adapter did not idempotently confirm the same reset token"
        );
        let fresh = second
            .session_new_full_for_conversation(
                &cwd,
                Vec::new(),
                None,
                Some("Buzz Pi contract smoke"),
                Some(&conversation_id),
                Some(&reset_token),
            )
            .await?;
        assert_session_shape(&fresh)?;
        anyhow::ensure!(
            pi_session_id(&fresh)? != first_pi_session_id,
            "the committed reset reopened the previous Pi session"
        );
        anyhow::ensure!(
            fresh
                .raw
                .pointer("/_meta/buzz/resumedConversation")
                .and_then(|value| value.as_bool())
                == Some(false),
            "the committed reset was reported as a resume"
        );
        anyhow::ensure!(
            fresh
                .raw
                .pointer("/_meta/buzz/skipRelayHistory")
                .and_then(|value| value.as_bool())
                == Some(true),
            "the committed reset did not request relay-history suppression"
        );
        anyhow::ensure!(
            second.session_dispose(&fresh.session_id, true).await?,
            "Pi adapter did not confirm destructive session disposal"
        );
        Ok(())
    }
    .await;
    second.shutdown().await;
    second_result
}

#[tokio::main]
async fn tokio_main() -> Result<()> {
    // Install the ring crypto provider for rustls (required for wss:// connections).
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("failed to install rustls crypto provider");
    if is_subcommand("models") {
        // Strip the subcommand token so clap doesn't reject it as a positional.
        // Keeps argv[0] (binary name) and passes everything after the subcommand.
        let filtered: Vec<String> = std::env::args()
            .enumerate()
            .filter(|(i, _)| *i != 1)
            .map(|(_, a)| a)
            .collect();
        let args = ModelsArgs::parse_from(&filtered);
        return run_models(args).await;
    }

    if is_subcommand("auth-methods") {
        let filtered: Vec<String> = std::env::args()
            .enumerate()
            .filter(|(i, _)| *i != 1)
            .map(|(_, a)| a)
            .collect();
        let args = AuthMethodsArgs::parse_from(&filtered);
        return run_auth_methods(args).await;
    }

    if is_subcommand("authenticate") {
        let filtered: Vec<String> = std::env::args()
            .enumerate()
            .filter(|(i, _)| *i != 1)
            .map(|(_, a)| a)
            .collect();
        let args = AuthenticateArgs::parse_from(&filtered);
        return run_authenticate(args).await;
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("buzz_acp=info")),
        )
        .compact()
        .init();

    let mut config = Config::from_cli().map_err(|e| anyhow::anyhow!("configuration error: {e}"))?;

    // ── Setup-mode early branch ───────────────────────────────────────────────
    //
    // When the desktop determines an agent is not ready (missing credentials,
    // model, or provider), it spawns buzz-acp with BUZZ_ACP_SETUP_PAYLOAD set.
    // We enter the minimal setup-listener path and never start the agent pool.
    if let Some(payload) = setup_mode::SetupPayload::from_env()
        .map_err(|e| anyhow::anyhow!("setup payload error: {e}"))?
    {
        tracing::info!("buzz-acp: setup payload present, entering setup-listener mode");
        return setup_mode::run_setup_listener(config, payload).await;
    }

    tracing::info!("buzz-acp starting: {}", config.summary());

    let observer = config
        .relay_observer
        .then(observer::ObserverHandle::in_process);
    if let Some(handle) = &observer {
        handle.emit(
            "harness_started",
            None,
            &observer::ObserverContext::default(),
            serde_json::json!({
                "relayUrl": config.relay_url,
                "agentCommand": config.agent_command,
                "agentArgs": config.agent_args,
                "parallelism": config.agents,
                "relayObserver": config.relay_observer,
            }),
        );
    }

    let mut pool = if config.lazy_pool {
        AgentPool::from_slots((0..config.agents).map(|_| None).collect())
    } else {
        initialize_agent_pool(&PoolStartup::from_config(&config, observer.clone()), None).await?
    };
    let mut pool_ready = !config.lazy_pool;
    let mut thread_sessions_enabled = match select_thread_session_scope(
        config.require_durable_thread_sessions,
        pool_ready,
        pool.thread_sessions_supported(),
    ) {
        Ok(enabled) => enabled,
        Err(message) => {
            shutdown_agent_pool(&mut pool).await;
            return Err(anyhow::anyhow!(message));
        }
    };
    let mut pool_lifecycle: PoolLifecycle<AgentPool> = PoolLifecycle::listening();

    // Capture a startup watermark BEFORE connecting to the relay. This timestamp
    // is used for membership notification replay (via startup_watermark) and as
    // the initial subscribe_since for channels discovered at startup. The Subscribe
    // handler falls back to subscribe_since when last_seen is None, closing the
    // blind spot between "agents ready" and "first REQ sent".
    let startup_watermark: u64 = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let pubkey_hex = config.keys.public_key().to_hex();
    let durable_outbox = DurableOutbox::open(&config.state_dir, &pubkey_hex)
        .map_err(|error| anyhow::anyhow!("could not open durable harness state: {error}"))?;
    let recovered_reset_records =
        recover_durable_reset_intents(&durable_outbox, &mut pool, &config, thread_sessions_enabled)
            .await?;

    // Parse BUZZ_AUTH_TAG into a nostr::Tag for NIP-OA relay membership delegation.
    let relay_auth_tag: Option<nostr::Tag> = std::env::var("BUZZ_AUTH_TAG")
        .ok()
        .filter(|s| !s.is_empty())
        .and_then(|s| buzz_sdk::nip_oa::parse_auth_tag(&s).ok());

    let mut relay =
        HarnessRelay::connect(&config.relay_url, &config.keys, &pubkey_hex, relay_auth_tag)
            .await
            .map_err(|e| anyhow::anyhow!("relay connect error: {e}"))?;

    // Tell the relay background task the watermark so it can use
    // `since = watermark - 5s` on the first REQ instead of `since=now`.
    // Best-effort: a failure here is non-fatal (we just lose the startup window
    // protection, which is the same as the pre-fix behaviour).
    if let Err(e) = relay.set_startup_watermark(startup_watermark).await {
        tracing::warn!("failed to set startup watermark: {e}");
    }

    tracing::info!("connected to relay at {}", config.relay_url);

    relay
        .subscribe_membership_notifications()
        .await
        .map_err(|e| anyhow::anyhow!("membership notification subscribe error: {e}"))?;
    tracing::info!("subscribed to membership notifications");

    let presence_publisher = relay.event_publisher();
    let presence_keys = config.keys.clone();

    // Priority: BUZZ_AUTH_TAG (NIP-OA attestation) → --agent-owner flag.
    let startup_owner: Option<String> = resolve_agent_owner(&config);
    if let Some(ref owner) = startup_owner {
        tracing::info!("agent owner: {owner}");
    } else {
        tracing::info!("no agent owner configured");
    }
    // Warn if owner-dependent mode but no owner resolved yet.
    if startup_owner.is_none() {
        match &config.respond_to {
            RespondTo::OwnerOnly => {
                tracing::warn!(
                    "respond-to=owner-only but no owner is set — all events will be \
                     dropped. Set BUZZ_AUTH_TAG or --agent-owner, or use --respond-to=anyone."
                );
            }
            RespondTo::Allowlist => {
                tracing::warn!(
                    "respond-to=allowlist but no owner is set — allowlisted pubkeys \
                     will still be accepted, but owner-based matching is unavailable \
                     until owner is resolved."
                );
            }
            _ => {} // anyone/nobody don't depend on owner
        }
    }
    let owner_cache = OwnerCache::new(startup_owner.clone());

    let mut relay_observer_control_rx = None;
    let mut relay_observer_publisher_task = None;
    let mut relay_observer_publisher = None;
    if config.relay_observer {
        if let (Some(observer), Some(owner_pubkey_hex)) =
            (observer.clone(), owner_cache.pubkey.clone())
        {
            match PublicKey::from_hex(&owner_pubkey_hex) {
                Ok(owner_pubkey) => {
                    relay_observer_publisher = Some((
                        observer,
                        relay.event_publisher(),
                        config.keys.clone(),
                        pubkey_hex.clone(),
                        owner_pubkey_hex,
                        owner_pubkey,
                    ));
                    relay
                        .subscribe_observer_controls()
                        .await
                        .map_err(|e| anyhow::anyhow!("observer control subscribe error: {e}"))?;
                    relay_observer_control_rx = relay.take_observer_control_rx();
                    tracing::info!("relay observer enabled");
                }
                Err(error) => {
                    tracing::warn!("relay observer disabled: invalid owner pubkey: {error}");
                }
            }
        } else {
            tracing::warn!(
                "relay observer requested but no agent owner was resolved at startup; \
                 observer frames will not be published"
            );
        }
    }

    let channel_info_map = relay
        .discover_channels()
        .await
        .map_err(|e| anyhow::anyhow!("channel discovery error: {e}"))?;

    tracing::info!("discovered {} channel(s)", channel_info_map.len());
    let channel_ids: Vec<Uuid> = channel_info_map.keys().copied().collect();

    let rules: Vec<SubscriptionRule> = match config.subscribe_mode {
        SubscribeMode::Mentions => {
            vec![SubscriptionRule {
                name: "mentions".into(),
                channels: filter::ChannelScope::All("all".into()),
                kinds: config.kinds_override.clone().unwrap_or_else(|| {
                    vec![
                        KIND_STREAM_MESSAGE,
                        KIND_WORKFLOW_APPROVAL_REQUESTED,
                        KIND_STREAM_REMINDER,
                    ]
                }),
                require_mention: !config.no_mention_filter,
                filter: None,
                compiled_filter: None,
                consecutive_timeouts: std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0)),
                prompt_tag: Some("@mention".into()),
            }]
        }
        SubscribeMode::All => {
            vec![SubscriptionRule {
                name: "all".into(),
                channels: filter::ChannelScope::All("all".into()),
                kinds: config.kinds_override.clone().unwrap_or_default(),
                require_mention: false,
                filter: None,
                compiled_filter: None,
                consecutive_timeouts: std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0)),
                prompt_tag: Some("all".into()),
            }]
        }
        SubscribeMode::Config => {
            // load_rules() already warns if the config file has zero rules.
            config::load_rules(&config.config_path)?
        }
    };

    let channel_filters = config::resolve_channel_filters(&config, &channel_ids, &rules);
    if channel_filters.is_empty() {
        tracing::warn!("no channel subscriptions resolved — agent will sit idle");
    }
    let mut subscribed_channel_ids = HashSet::with_capacity(channel_filters.len());
    for (channel_id, filter) in &channel_filters {
        if let Err(e) = relay.subscribe_channel(*channel_id, filter.clone()).await {
            tracing::warn!("failed to subscribe to channel {channel_id}: {e}");
        } else {
            subscribed_channel_ids.insert(*channel_id);
            tracing::info!("subscribed to channel {channel_id}");
        }
    }

    if let Some((observer, publisher, keys, agent_pubkey, owner_pubkey, owner)) =
        relay_observer_publisher.take()
    {
        relay_observer_publisher_task = Some(spawn_relay_observer_publisher(
            observer,
            publisher,
            keys,
            agent_pubkey,
            owner_pubkey,
            owner,
        ));
    }

    let runtime_start_nonce = std::env::var("BUZZ_MANAGED_AGENT_START_NONCE").unwrap_or_default();
    let dedup_mode = config.dedup_mode;
    let mut queue = EventQueue::new(dedup_mode)
        .with_thread_sessions_enabled(thread_sessions_enabled)
        .with_in_flight_deadline(config.max_turn_duration_secs);
    let mut pending_reset_confirmations: HashMap<(Uuid, String), PendingResetConfirmation> =
        HashMap::new();
    for record in recovered_reset_records {
        let identity = record.identity();
        queue.restore_completed_reset_barrier(identity.clone());
        pending_reset_confirmations.insert(
            identity,
            PendingResetConfirmation {
                event: record.event,
                next_attempt_at: Instant::now(),
                durable: true,
            },
        );
    }
    tracing::info!(
        thread_sessions_enabled,
        "configured ACP conversation scoping"
    );

    // Online means the harness can receive work, not merely that its socket is
    // connected. Publishing after channel subscriptions gives desktop callers
    // a durable readiness boundary before they send a startup mention.
    if config.presence_enabled {
        match publish_presence(&presence_publisher, &presence_keys, "online").await {
            Ok(_) => tracing::info!("presence set to online"),
            Err(e) => tracing::warn!("failed to set initial presence: {e}"),
        }
    }

    if config.lazy_pool {
        emit_runtime_lifecycle(
            observer.as_ref(),
            &runtime_start_nonce,
            &pubkey_hex,
            &config.relay_url,
            "listening",
            None,
        );
    }

    let base_prompt_content = config.base_prompt_content.take();
    let ctx = Arc::new(PromptContext {
        mcp_servers: build_mcp_servers(&config),
        initial_message: config.initial_message.clone(),
        idle_timeout: Duration::from_secs(config.idle_timeout_secs),
        max_turn_duration: Duration::from_secs(config.max_turn_duration_secs),
        turn_liveness_interval: Duration::from_secs(config.turn_liveness_secs),
        dedup_mode: config.dedup_mode,
        system_prompt: config.system_prompt.clone(),
        session_title: config.session_title.clone(),
        team_instructions: config.team_instructions.clone(),
        base_prompt: if config.no_base_prompt {
            None
        } else if let Some(content) = base_prompt_content {
            Some(Box::leak(content.into_boxed_str()))
        } else {
            Some(include_str!("base_prompt.md"))
        },
        heartbeat_prompt: config.heartbeat_prompt.clone(),
        cwd: std::env::current_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from("/"))
            .to_string_lossy()
            .to_string(),
        rest_client: relay.rest_client(),
        channel_info: pool::ChannelInfoResolver::new(channel_info_map, relay.rest_client()),
        context_message_limit: config.context_message_limit,
        max_turns_per_session: config.max_turns_per_session,
        max_cached_conversation_sessions: config.max_cached_conversation_sessions as usize,
        conversation_session_idle_ttl: Duration::from_secs(
            config.conversation_session_idle_ttl_secs,
        ),
        permission_mode: config.permission_mode,
        agent_keys: config.keys.clone(),
        agent_owner_pubkey: startup_owner
            .as_deref()
            .and_then(|hex| nostr::PublicKey::from_hex(hex).ok()),
        memory_enabled: config.memory_enabled,
        harness_name: crate::config::normalize_agent_command_identity(&config.agent_command),
        relay_url: config.relay_url.clone(),
    });
    let mut lifecycle_notice_outbox =
        LifecycleNoticeOutbox::spawn(ctx.rest_client.clone(), durable_outbox.clone())?;

    if !config.memory_enabled {
        tracing::info!(
            target: "engram::core",
            "NIP-AE core memory injection disabled (re-enable by removing --no-memory / BUZZ_ACP_NO_MEMORY)"
        );
    }

    let mut heartbeat = if config.heartbeat_interval_secs > 0 {
        let interval = Duration::from_secs(config.heartbeat_interval_secs);
        Some(tokio::time::interval_at(
            tokio::time::Instant::now() + interval,
            interval,
        ))
    } else {
        None
    };
    let mut heartbeat_in_flight = false;

    let mut presence_heartbeat = if config.presence_enabled {
        let interval = Duration::from_secs(60);
        Some(tokio::time::interval_at(
            tokio::time::Instant::now() + interval,
            interval,
        ))
    } else {
        None
    };

    let mut typing_refresh = if config.typing_enabled {
        let interval = Duration::from_secs(3);
        Some(tokio::time::interval_at(
            tokio::time::Instant::now() + interval,
            interval,
        ))
    } else {
        None
    };
    let mut typing_channels: HashMap<ConversationKey, ThreadTags> = HashMap::new();
    let mut observer_control_replay_cache = ObserverControlReplayCache::default();
    // Reset success notices have stronger ordering than ordinary lifecycle
    // notices: their completed barrier is released only after relay
    // acceptance. Keep one tracked HTTP attempt globally so delivery never
    // blocks relay ingress and a relay outage cannot create an unbounded task
    // fan-out.
    let mut reset_ack_tasks: tokio::task::JoinSet<ResetAckAttemptResult> =
        tokio::task::JoinSet::new();
    let mut reset_ack_in_flight: Option<ResetAckAttemptKey> = None;
    let mut presence_task: Option<tokio::task::JoinHandle<()>> = None;

    // Independent of pool readiness: a never-mentioned lazy agent must still
    // self-terminate. The watch interval is capped so small configured bounds
    // remain reasonably precise without waking long-lived agents frequently.
    let inactivity_bound = Duration::from_secs(config.exit_after_inactivity_secs);
    let mut last_activity = tokio::time::Instant::now();
    let mut inactivity_reaper = if inactivity_bound.is_zero() {
        None
    } else {
        let interval = inactivity_bound.min(Duration::from_secs(30));
        Some(tokio::time::interval_at(
            tokio::time::Instant::now() + interval,
            interval,
        ))
    };

    // Runs at the TOP of every loop iteration via Instant check — cannot be
    // starved by the biased select. Slot refill spawns background tasks so
    // spawn_and_init never blocks the main loop.
    let maintenance_interval = Duration::from_secs(30);
    let mut last_maintenance = std::time::Instant::now();

    // Channel for background respawn tasks to return completed agents.
    // Bounded to agent count — at most one respawn per slot in flight.
    let (respawn_tx, mut respawn_rx) = mpsc::channel::<RespawnResult>(config.agents as usize);
    // JoinSet for respawn tasks so shutdown can abort them.
    let mut respawn_tasks: tokio::task::JoinSet<()> = tokio::task::JoinSet::new();
    let (wake_tx, mut wake_rx) = mpsc::channel::<(u32, Result<AgentPool, String>)>(1);
    let mut wake_tasks: tokio::task::JoinSet<()> = tokio::task::JoinSet::new();

    // Channel for non-cancelling steer ack watchers to forward outcomes back
    // to the main loop. Each `pool.send_steer(...) == Ok(())` spawns a
    // short-lived task that awaits the `SteerRequest.ack_tx` oneshot and
    // forwards a `SteerAckEvent`. Unbounded because:
    //   1. The producer count is bounded by in-flight goose turns
    //      (`agents` slots, capacity-1 `steer_tx` each), so the channel
    //      cannot legitimately back up under steady state.
    //   2. We must never drop a steer outcome — losing an ack would leak a
    //      withheld event in `EventQueue::withheld_native_steer` until
    //      `IN_FLIGHT_DEADLINE_SECS` expires.
    let (steer_ack_tx, mut steer_ack_rx) = mpsc::unbounded_channel::<SteerAckEvent>();

    // ── Step 7: Shutdown signal ───────────────────────────────────────────────
    let (shutdown_tx, mut shutdown_rx) = watch::channel(());

    let tx = shutdown_tx.clone();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        let _ = tx.send(());
    });

    #[cfg(unix)]
    {
        let tx = shutdown_tx.clone();
        tokio::spawn(async move {
            use tokio::signal::unix::{signal, SignalKind};
            let mut sigterm = signal(SignalKind::terminate()).expect("SIGTERM handler");
            sigterm.recv().await;
            let _ = tx.send(());
        });
    }

    // Track the newest membership notification timestamp per channel.
    // On reconnect the relay replays events newest-first, so the first event
    // per channel is authoritative. Any later event with ts < newest is stale.
    // Exact duplicates (same event ID) are caught by seen_membership_ids.
    //
    // Uses strict `<` (not `<=`) so that legitimate live events at the same
    // second are both processed. The seen_membership_ids set handles exact
    // replays that share the same timestamp.
    let mut membership_newest_ts: HashMap<Uuid, u64> = HashMap::new();
    // Two-generation dedup for membership event replays (bounded, no amnesia).
    // Rotates at 1000 entries instead of clearing the entire set at 2000.
    let mut seen_membership_current: HashSet<String> = HashSet::new();
    let mut seen_membership_previous: HashSet<String> = HashSet::new();

    // Channels the agent has been removed from. When a checked-out agent is
    // returned to the pool, its sessions for these channels are stripped, and
    // failed/panicked batches for these channels are dropped instead of requeued.
    //
    // Cleared on re-add (KIND_MEMBER_ADDED_NOTIFICATION) so re-joined channels
    // regain session affinity.
    //
    // Known limitation: if a batch is in-flight when the channel is removed AND
    // re-added before the batch returns, the stale batch may be requeued. This
    // is acceptable because: (a) the agent is a member again and has access,
    // (b) the events are from the agent's authorized history, (c) the window
    // is extremely narrow (membership changes are rare, prompt turns are seconds),
    // and (d) fixing this would require per-channel epoch tracking on TaskMeta
    // and PromptResult — significant complexity for a benign edge case. If strict
    // causal invalidation is needed, add a monotonic epoch counter per channel
    // and capture it in TaskMeta at dispatch time.
    let mut removed_channels: HashSet<Uuid> = HashSet::new();

    //
    // One SlotCircuit per agent slot. crash_times entries are pruned to the last
    // CIRCUIT_BREAKER_WINDOW on each respawn attempt. The Vec is indexed by
    // agent slot index, so it must be sized to the configured pool capacity
    // (not the live count, which may be smaller after partial startup).
    let mut crash_history: Vec<SlotCircuit> = (0..config.agents as usize)
        .map(|_| SlotCircuit {
            crash_times: Vec::new(),
            open_until: None,
            respawn_in_flight: false,
        })
        .collect();

    //
    // Branches 1 & 2 both need to borrow `pool`, but they access different
    // fields (result_rx vs join_set). We use `rx_and_join_set()` to split the
    // borrow, yielding a typed enum so the outer code can dispatch cleanly.
    enum PoolEvent {
        Result(Box<PromptResult>),
        Panic(tokio::task::JoinError),
        ResetAck(std::result::Result<ResetAckAttemptResult, tokio::task::JoinError>),
        SteerAck(SteerAckEvent),
        Wake(u32, Result<AgentPool, String>),
    }

    let mut terminal_error = None;
    loop {
        schedule_completed_reset_ack(
            &queue,
            &pending_reset_confirmations,
            &ctx.rest_client,
            &mut reset_ack_tasks,
            &mut reset_ack_in_flight,
        );

        // Whether buffered work is waiting on a lazy pool. Also gates the
        // retry-deadline sleep arm below: a `Failed` lifecycle keeps its
        // (possibly past) `retry_at` until the next wake, so sleeping on it
        // unconditionally would complete instantly on every iteration — a
        // busy spin — whenever the queued work drained after a failed wake.
        let mut lazy_wake_work_pending = false;
        if config.lazy_pool && !pool_ready {
            lazy_wake_work_pending = queue.has_flushable_work();
            if let Some(attempt) = pool_lifecycle
                .start_wake_if_due(lazy_wake_work_pending, tokio::time::Instant::now())
            {
                emit_runtime_lifecycle(
                    observer.as_ref(),
                    &runtime_start_nonce,
                    &pubkey_hex,
                    &config.relay_url,
                    "waking",
                    None,
                );
                let startup = PoolStartup::from_config(&config, observer.clone());
                let wake_tx = wake_tx.clone();
                let wake_shutdown = shutdown_rx.clone();
                wake_tasks.spawn(async move {
                    let result = initialize_agent_pool(&startup, Some(wake_shutdown))
                        .await
                        .map_err(|error| error.to_string());
                    if let Err(error) = wake_tx.send((attempt, result)).await {
                        let (_attempt, result) = error.0;
                        if let Ok(mut abandoned_pool) = result {
                            shutdown_agent_pool(&mut abandoned_pool).await;
                        }
                    }
                });
            }
        }

        if pool_ready && last_maintenance.elapsed() >= maintenance_interval {
            last_maintenance = std::time::Instant::now();
            queue.compact_expired_state();

            // Slot refill: spawn background tasks for empty slots whose
            // circuit breaker allows it. spawn_and_init runs off the main
            // loop so it never blocks event processing.
            for (idx, slot) in crash_history.iter_mut().enumerate() {
                if pool.slot_alive(idx) || slot.respawn_in_flight {
                    continue;
                }
                if !slot.can_refill() {
                    continue;
                }
                slot.respawn_in_flight = true;
                tracing::info!(agent = idx, "slot refill: spawning background respawn");
                let cmd = config.agent_command.clone();
                let args = config.agent_args.clone();
                let env = config.persona_env_vars.clone();
                let has_codex = config.has_generated_codex_config;
                let observer = observer.clone();
                let guard = RespawnGuard::new(idx, respawn_tx.clone());
                respawn_tasks.spawn(async move {
                    let result = spawn_and_init(&cmd, &args, &env, has_codex, idx, observer).await;
                    guard.send(result);
                });
            }

            // Flush requeued batches whose retry_after has expired. Without
            // this, a batch requeued during crash recovery can sit idle
            // indefinitely on quiet channels — dispatch_pending is only
            // called on relay events or pool results, neither of which
            // arrive when the channel is silent.
            if queue.has_flushable_work() {
                for (channel_id, thread_tags) in
                    dispatch_pending(&mut pool, &mut queue, &ctx, &mut last_activity)
                {
                    typing_channels.insert(channel_id, thread_tags);
                }
            }
        }

        let mut respawn_collected = false;
        while let Ok(rr) = respawn_rx.try_recv() {
            crash_history[rr.index].respawn_in_flight = false;
            match rr.result {
                Ok((mut acp, protocol_version, agent_name)) => {
                    let durable_thread_sessions_supported = acp.thread_sessions_supported()
                        && acp.conversation_reset_supported()
                        && acp.durable_session_events_supported();
                    if thread_sessions_enabled && !durable_thread_sessions_supported {
                        tracing::error!(
                            agent = rr.index,
                            "rejecting respawn whose adapter no longer advertises durable thread-session reset support"
                        );
                        acp.shutdown().await;
                        crash_history[rr.index].mark_spawn_failed();
                        continue;
                    }
                    if !thread_sessions_enabled && durable_thread_sessions_supported {
                        tracing::info!(
                            agent = rr.index,
                            "respawn supports thread sessions; retaining startup legacy channel scope"
                        );
                    }
                    let agent = OwnedAgent {
                        index: rr.index,
                        acp,
                        state: SessionState::default(),
                        model_capabilities: None,
                        desired_model: config.model.clone(),
                        model_overridden: false,
                        pending_model_switch: None,
                        agent_name,
                        goose_system_prompt_supported: None,
                        protocol_version,
                    };
                    pool.return_agent(agent);
                    tracing::info!(agent = rr.index, "respawn complete");
                    respawn_collected = true;
                }
                Err(e) => {
                    crash_history[rr.index].mark_spawn_failed();
                    tracing::warn!(agent = rr.index, "respawn failed: {e} — circuit re-opened");
                }
            }
        }
        // Flush requeued events that were waiting for a live agent. Without
        // this, batches requeued during crash recovery sit idle until the
        // next relay event arrives — which can be minutes on quiet channels.
        if respawn_collected {
            for (channel_id, thread_tags) in
                dispatch_pending(&mut pool, &mut queue, &ctx, &mut last_activity)
            {
                typing_channels.insert(channel_id, thread_tags);
            }
        }

        let reset_ack_retry_at = reset_ack_in_flight
            .is_none()
            .then(|| next_completed_reset_ack_attempt(&queue, &pending_reset_confirmations))
            .flatten();

        // Borrow result_rx and join_set simultaneously via split-borrow helper.
        let pool_event: Option<PoolEvent> = {
            let (result_rx, join_set) = pool.rx_and_join_set();
            tokio::select! {
                biased;
                // recv() returning None means all senders dropped (pool was torn down).
                // Break cleanly instead of panicking.
                r = result_rx.recv(), if pool_ready => match r {
                    Some(result) => Some(PoolEvent::Result(Box::new(result))),
                    None => {
                        tracing::info!("result channel closed — exiting main loop");
                        break;
                    }
                },
                // Guard: join_next() returns None immediately when JoinSet is
                // empty, which would cause a tight spin. Only poll when there
                // are in-flight tasks.
                Some(Err(e)) = join_set.join_next(), if !join_set.is_empty() => {
                    Some(PoolEvent::Panic(e))
                }
                Some(result) = reset_ack_tasks.join_next(), if !reset_ack_tasks.is_empty() => {
                    Some(PoolEvent::ResetAck(result))
                }
                // Goose-native steer ack from a watcher task. Outcomes drive
                // queue side-effects (drop / release withheld event) and
                // optionally the cancel+merge fallback signal. See the
                // `Some(PoolEvent::SteerAck(...))` match arm below for the
                // locked semantics (Eva + Max + Perci).
                Some(ack_event) = steer_ack_rx.recv() => {
                    Some(PoolEvent::SteerAck(ack_event))
                }
                Some((attempt, result)) = wake_rx.recv(), if config.lazy_pool && !pool_ready => {
                    Some(PoolEvent::Wake(attempt, result))
                }
                // Gated on pending work: with an empty queue there is nothing
                // for the retry to dispatch, and a past `retry_at` would
                // otherwise complete instantly on every iteration (busy spin).
                // The next accepted event re-enables the arm.
                _ = async {
                    match pool_lifecycle.retry_at() {
                        Some(retry_at) if lazy_wake_work_pending => {
                            tokio::time::sleep_until(retry_at).await
                        }
                        _ => std::future::pending().await,
                    }
                } => None,
                _ = async {
                    match reset_ack_retry_at {
                        Some(deadline) => {
                            tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await
                        }
                        None => std::future::pending().await,
                    }
                } => None,
                Some(Err(error)) = wake_tasks.join_next(), if !wake_tasks.is_empty() => {
                    if let Some(attempt) = pool_lifecycle.waking_attempt() {
                        let message = format!("pool wake task failed: {error}");
                        if pool_lifecycle.cancel_wake(
                            attempt,
                            message.clone(),
                            tokio::time::Instant::now(),
                        ) {
                            emit_runtime_lifecycle(
                                observer.as_ref(),
                                &runtime_start_nonce,
                                &pubkey_hex,
                                &config.relay_url,
                                "failed",
                                Some(&message),
                            );
                        }
                    }
                    None
                }
                control_event = async {
                    match relay_observer_control_rx.as_mut() {
                        Some(rx) => rx.recv().await,
                        None => std::future::pending().await,
                    }
                } => {
                    let _ = result_rx;
                    match control_event {
                        Some(event) => {
                            if let Some(ref owner_hex) = owner_cache.pubkey {
                                handle_relay_observer_control_event(
                                    &config.keys,
                                    event,
                                    &mut pool,
                                    observer.as_ref(),
                                    owner_hex,
                                    &mut observer_control_replay_cache,
                                );
                            } else {
                                tracing::warn!("observer control frame received but no owner resolved — dropping");
                            }
                        }
                        None => {
                            relay_observer_control_rx = None;
                            tracing::warn!("relay observer control channel closed");
                        }
                    }
                    None
                }
                // Remaining branches don't touch pool — evaluated when pool is idle.
                buzz_event = relay.next_event() => {
                    let _ = result_rx; // end split borrow before relay handling
                    match buzz_event {
                        Some(buzz_event) => {
                            let kind_u32 = buzz_event.event.kind.as_u16() as u32;

                            if kind_u32 == KIND_MEMBER_ADDED_NOTIFICATION
                                || kind_u32 == KIND_MEMBER_REMOVED_NOTIFICATION
                            {
                                let ch = buzz_event.channel_id;
                                let ts = relay::bounded_event_replay_timestamp(&buzz_event.event);
                                let eid = buzz_event.event.id.to_hex();

                                // Two-layer membership dedup:
                                //
                                // 1. Exact duplicate rejection (seen_membership_ids):
                                //    Catches the same event replayed on reconnect.
                                //
                                // 2. Timestamp watermark (membership_newest_ts):
                                //    Uses strict `<` so that older events from reconnect
                                //    replay are dropped, but legitimate live events at the
                                //    same second are both processed. This is safe because
                                //    exact duplicates are already caught by layer 1.
                                //
                                // Why not `<=`? That would suppress legitimate live
                                // add→remove (or remove→add) sequences in the same second,
                                // leaving the harness in the wrong membership state.
                                // Two-generation dedup: check both sets before inserting.
                                if seen_membership_current.contains(&eid)
                                    || seen_membership_previous.contains(&eid)
                                {
                                    tracing::debug!(
                                        channel_id = %ch,
                                        kind = kind_u32,
                                        "skipping duplicate membership notification (same event_id)"
                                    );
                                    continue;
                                }
                                seen_membership_current.insert(eid);
                                // Rotate at 1000: current → previous, no amnesia window.
                                if seen_membership_current.len() >= 1000 {
                                    seen_membership_previous =
                                        std::mem::take(&mut seen_membership_current);
                                }
                                if let Some(&newest) = membership_newest_ts.get(&ch) {
                                    if ts < newest {
                                        tracing::debug!(
                                            channel_id = %ch,
                                            kind = kind_u32,
                                            ts,
                                            newest,
                                            "skipping stale membership notification (older than newest)"
                                        );
                                        continue;
                                    }
                                }
                                membership_newest_ts.insert(ch, ts);

                                if kind_u32 == KIND_MEMBER_ADDED_NOTIFICATION {
                                    // Clear removal tracking so sessions are not
                                    // stripped for a legitimately re-added channel.
                                    removed_channels.remove(&ch);

                                    if subscribed_channel_ids.contains(&ch) {
                                        tracing::debug!(channel_id = %ch, "membership notification: channel already subscribed");
                                    } else if let Some(filter) = config::resolve_dynamic_channel_filter(&config, ch, &rules) {
                                        tracing::info!(channel_id = %ch, "membership notification: subscribing to new channel");
                                        if let Err(e) = relay.subscribe_channel_from(ch, filter, Some(ts)).await {
                                            tracing::warn!("failed to subscribe to new channel {ch}: {e}");
                                        } else {
                                            subscribed_channel_ids.insert(ch);
                                        }
                                    } else {
                                        tracing::debug!(channel_id = %ch, "membership notification: no matching rules — skipping");
                                    }
                                } else {
                                    subscribed_channel_ids.remove(&ch);
                                    tracing::info!(channel_id = %ch, "membership notification: unsubscribing from channel");
                                    if let Err(e) = relay.unsubscribe_channel(ch).await {
                                        tracing::warn!("failed to unsubscribe from channel {ch}: {e}");
                                    }
                                    // Drain queued events and invalidate sessions for the
                                    // removed channel. Events already in-flight will
                                    // complete normally (the relay may reject actions if
                                    // the agent lost access).
                                    let drained_ids = queue.drain_channel(ch);
                                    let invalidated = if pool_ready {
                                        pool.dispose_channel_sessions(ch, true).await
                                    } else {
                                        0
                                    };
                                    // Track removed channels so checked-out agents get
                                    // their sessions stripped when they return to the pool.
                                    removed_channels.insert(ch);
                                    typing_channels.retain(|key, _| key.channel_id != ch);
                                    // Best-effort: clean up 👀 on drained events.
                                    // Note: the relay revokes membership before
                                    // emitting the notification, so this DELETE may
                                    // 403 on non-open channels. Stale 👀 in that
                                    // case is a known limitation — fix belongs in
                                    // the relay (clean up bot reactions on removal).
                                    if !drained_ids.is_empty() {
                                        let rc = ctx.rest_client.clone();
                                        let ids = drained_ids.clone();
                                        tokio::spawn(async move {
                                            for eid in &ids {
                                                pool::reaction_remove(&rc, eid, "👀").await;
                                            }
                                        });
                                    }
                                    if !drained_ids.is_empty() || invalidated > 0 {
                                        tracing::info!(
                                            channel_id = %ch,
                                            drained = drained_ids.len(),
                                            invalidated,
                                            "cleaned up after membership removal"
                                        );
                                    }
                                }
                                continue;
                            }

                            if config.ignore_self && buzz_event.event.pubkey.to_hex() == pubkey_hex {
                                tracing::debug!(channel_id = %buzz_event.channel_id, "dropping self-authored event");
                                continue;
                            }

                            // Conversation identity is security-sensitive: it
                            // selects the queue, session, control target, and
                            // relay context query. Reject malformed or
                            // conflicting NIP-10 markers before interpreting
                            // any lifecycle command.
                            if !queue::has_valid_thread_identity(&buzz_event.event) {
                                tracing::warn!(
                                    channel_id = %buzz_event.channel_id,
                                    event_id = %buzz_event.event.id,
                                    "dropping event with invalid NIP-10 thread identity"
                                );
                                continue;
                            }

                            // Compute thread identity once, before any control
                            // command can consume the event. Top-level events
                            // are their own prospective roots; replies inherit
                            // their NIP-10 root.
                            let (event_is_dm, conversation_is_dm) =
                                resolve_dm_channel_scope(
                                    buzz_event.channel_id,
                                    &ctx.channel_info,
                                )
                                .await;
                            queue.set_channel_is_dm(
                                buzz_event.channel_id,
                                conversation_is_dm,
                            );
                            let event_conversation = queue.conversation_key_for_event(
                                buzz_event.channel_id,
                                &buzz_event.event,
                            );

                            // Check: kind:9, content "!shutdown", from owner, mentions THIS agent.
                            let is_shutdown = is_owner_control_command(
                                &buzz_event.event,
                                kind_u32,
                                "!shutdown",
                                &pubkey_hex,
                                config.session_title.as_deref(),
                            );
                            if is_shutdown {
                                let owner = owner_cache.get();
                                if let Some(owner) = owner {
                                    if buzz_event.event.pubkey.to_hex() == *owner {
                                        tracing::info!(
                                            channel_id = %buzz_event.channel_id,
                                            sender = %buzz_event.event.pubkey.to_hex(),
                                            "shutdown command from owner — exiting gracefully"
                                        );
                                        let _ = shutdown_tx.send(());
                                        continue;
                                    }
                                }
                                // Not from owner — fall through to normal prompt handling.
                                // Don't drop it — it's a regular message that happens to
                                // contain "!shutdown" from a non-owner.
                            }

                            // Mirrors !shutdown: kind:9, content "!cancel", from
                            // owner, mentions THIS agent. Must be BEFORE
                            // queue.push() — the event content is moved by push.
                            //
                            // Mode-independent: !cancel fires regardless of
                            // --multiple-event-handling. It is explicit user
                            // intent, not an automatic policy decision.
                            let is_cancel = is_owner_control_command(
                                &buzz_event.event,
                                kind_u32,
                                "!cancel",
                                &pubkey_hex,
                                config.session_title.as_deref(),
                            );
                            if is_cancel {
                                if let Some(owner) = owner_cache.get() {
                                    if buzz_event.event.pubkey.to_hex() == *owner {
                                        let fired = signal_in_flight_conversation(
                                            &mut pool,
                                            &event_conversation,
                                            ControlSignal::Cancel,
                                        );
                                        if !fired {
                                            tracing::warn!(
                                                channel_id = %buzz_event.channel_id,
                                                "!cancel received but no in-flight task — no-op"
                                            );
                                        }
                                        continue; // consume event — do NOT push to queue
                                    }
                                }
                                // Not from owner — fall through to normal prompt handling.
                            }

                            // Mirrors !shutdown / !cancel: kind:9, content
                            // "!rotate", from owner, mentions THIS agent.
                            //
                            // Rotation is explicit owner intent to start the
                            // next turn in this channel with a fresh ACP
                            // session. It is consumed by the harness and never
                            // forwarded to the agent. If a turn is in-flight,
                            // cancel it, drop its triggering batch, and
                            // invalidate the channel session when the task
                            // returns. If idle, invalidate the cached channel
                            // session immediately. Queued future events remain
                            // queued and will create a fresh session on dispatch.
                            let is_rotate = is_owner_control_command(
                                &buzz_event.event,
                                kind_u32,
                                "!rotate",
                                &pubkey_hex,
                                config.session_title.as_deref(),
                            );
                            if is_rotate {
                                if let Some(owner) = owner_cache.get() {
                                    if buzz_event.event.pubkey.to_hex() == *owner {
                                        let rotated = rotate_all_in_flight_channel(
                                            &mut pool,
                                            buzz_event.channel_id,
                                        );
                                        let invalidated = pool
                                            .dispose_channel_sessions(
                                                buzz_event.channel_id,
                                                true,
                                            )
                                            .await;
                                        if rotated > 0 {
                                            tracing::info!(
                                                channel_id = %buzz_event.channel_id,
                                                rotated,
                                                invalidated,
                                                "!rotate received — cancelling in-flight turn and rotating session"
                                            );
                                        } else {
                                            tracing::info!(
                                                channel_id = %buzz_event.channel_id,
                                                invalidated,
                                                "!rotate received — invalidated idle channel session(s)"
                                            );
                                        }
                                        continue; // consume event — do NOT push to queue
                                    }
                                }
                                // Not from owner — fall through to normal prompt handling.
                            }

                            // Thread-local context reset. Unlike channel-wide
                            // `!rotate`, `/new` affects only the NIP-10 thread
                            // containing the command. It is owner-authorized,
                            // consumed by the harness, and visibly acknowledged
                            // in the same thread.
                            let is_new = is_owner_control_command(
                                &buzz_event.event,
                                kind_u32,
                                "/new",
                                &pubkey_hex,
                                config.session_title.as_deref(),
                            );
                            if is_new {
                                if let Some(owner) = owner_cache.get() {
                                    if buzz_event.event.pubkey.to_hex() == *owner {
                                        let command_id = buzz_event.event.id.to_hex();
                                        let mut thread_tags =
                                            queue::parse_thread_tags(&buzz_event.event);
                                        if thread_tags.root_event_id.is_none() {
                                            thread_tags.root_event_id = Some(command_id.clone());
                                            thread_tags.parent_event_id = Some(command_id.clone());
                                        }
                                        let conversation_id = event_conversation.stable_id();
                                        let reset_identity = (
                                            event_conversation.channel_id,
                                            event_conversation.root_event_id.clone(),
                                        );
                                        if thread_sessions_enabled {
                                            match durable_outbox
                                                .consumed_reset(&reset_identity, &command_id)
                                            {
                                              Ok(Some(receipt)) => {
                                                // Relay overlap or a long-delayed duplicate must
                                                // not advance the queue generation or cancel a
                                                // newer turn. The original signed ACK was already
                                                // accepted before this receipt was checkpointed;
                                                // re-submit it idempotently for convergence.
                                                tracing::info!(
                                                    conversation = %event_conversation,
                                                    command_id,
                                                    acknowledgement_event_id = %receipt.event.id,
                                                    "replaying previously consumed /new without mutating context"
                                                );
                                                let rest = ctx.rest_client.clone();
                                                tokio::spawn(async move {
                                                    if !pool::submit_notice_event(&rest, &receipt.event).await {
                                                        tracing::warn!(
                                                            event_id = %receipt.event.id,
                                                            "relay did not accept replayed /new acknowledgement"
                                                        );
                                                    }
                                                });
                                                continue;
                                              }
                                              Ok(None) => {}
                                              Err(error) => {
                                                  // Treat loss of durable replay knowledge as
                                                  // a process-fatal safety fault. Continuing
                                                  // would make a consumed `/new` look fresh.
                                                  return Err(anyhow::anyhow!(
                                                      "could not read durable /new replay receipts: {error}"
                                                  ));
                                              }
                                            }
                                        }
                                        let reset_notice_event = match pool::build_notice_event(
                                            &ctx.rest_client,
                                            buzz_event.channel_id,
                                            &thread_tags,
                                            reset_confirmation(thread_sessions_enabled),
                                        ) {
                                            Ok(event) => event,
                                            Err(error) => {
                                                tracing::error!(
                                                    conversation = %event_conversation,
                                                    %error,
                                                    "/new rejected because its acknowledgement could not be signed"
                                                );
                                                continue;
                                            }
                                        };
                                        let prepared_reset = match queue
                                            .prepare_reset_conversation(&event_conversation)
                                        {
                                            Ok(prepared) => prepared,
                                            Err(error) => {
                                                tracing::error!(
                                                    conversation = %event_conversation,
                                                    %error,
                                                    "/new rejected before mutating the active session"
                                                );
                                                let failure = match error {
                                                    queue::ResetConversationError::ResetInProgress => {
                                                        "⚠️ A previous `/new` for this thread is still waiting for Buzz to acknowledge its signed reset notice. I left that reset intact; wait a moment, then try again."
                                                    }
                                                    queue::ResetConversationError::MetadataCapacity => {
                                                        "⚠️ I couldn't reset this thread because the agent's reset metadata is at capacity. No success was recorded; restart the agent and try `/new` again."
                                                    }
                                                };
                                                pool::post_failure_notice(
                                                    &ctx.rest_client,
                                                    buzz_event.channel_id,
                                                    &thread_tags,
                                                    failure,
                                                )
                                                .await;
                                                continue;
                                            }
                                        };
                                        if thread_sessions_enabled {
                                            let record = DurableResetRecord {
                                                channel_id: reset_identity.0,
                                                root_event_id: reset_identity.1.clone(),
                                                conversation_id: conversation_id.clone(),
                                                reset_token: command_id.clone(),
                                                event: reset_notice_event.clone(),
                                                phase: DurableResetPhase::Prepared,
                                            };
                                            if let Err(error) = durable_outbox.prepare_reset(record) {
                                                tracing::error!(
                                                    conversation = %event_conversation,
                                                    %error,
                                                    "/new rejected because its signed acknowledgement intent could not be persisted"
                                                );
                                                pool::post_failure_notice(
                                                    &ctx.rest_client,
                                                    buzz_event.channel_id,
                                                    &thread_tags,
                                                    "⚠️ I couldn't persist the reset safely, so the active context was left unchanged. Free local disk space and try `/new` again.",
                                                )
                                                .await;
                                                continue;
                                            }
                                        }
                                        let reset_commit = if thread_sessions_enabled {
                                            commit_durable_conversation_reset(
                                                &mut pool,
                                                &config,
                                                &conversation_id,
                                                &command_id,
                                            )
                                            .await
                                        } else {
                                            Some(ConversationResetCommit::NewlyCommitted)
                                        };
                                        if reset_commit != Some(ConversationResetCommit::NewlyCommitted) {
                                            // The result may be ambiguous (the
                                            // tombstone can outlive a dropped
                                            // response), or an evicted replay
                                            // receipt may have produced an
                                            // already-committed token. Never
                                            // advance a second queue generation
                                            // or continue using an old hot
                                            // handle in either state.
                                            let cancelled = signal_in_flight_conversation(
                                                &mut pool,
                                                &event_conversation,
                                                ControlSignal::Cancel,
                                            );
                                            let invalidated = pool
                                                .dispose_conversation_sessions(
                                                    &event_conversation,
                                                    false,
                                                )
                                                .await;
                                            tracing::error!(
                                                conversation = %event_conversation,
                                                cancelled,
                                                invalidated,
                                                "durable reset remained unconfirmed; old hot context was invalidated without acknowledging success"
                                            );
                                            if let Err(error) = durable_outbox.discard_reset(
                                                &reset_identity,
                                                &reset_notice_event.id.to_hex(),
                                            ) {
                                                tracing::error!(
                                                    %error,
                                                    "could not remove an unconfirmed durable reset intent; restart recovery will retry it"
                                                );
                                            }
                                            pool::post_failure_notice(
                                                &ctx.rest_client,
                                                buzz_event.channel_id,
                                                &thread_tags,
                                                "⚠️ I couldn't uniquely confirm this durable reset. I stopped the old live context and did not record success; send a new `/new` command to finish safely.",
                                            )
                                            .await;
                                            continue;
                                        }

                                        if thread_sessions_enabled {
                                            match durable_outbox.mark_reset_committed(
                                                &reset_identity,
                                                &reset_notice_event.id.to_hex(),
                                            ) {
                                                Ok(true) => {}
                                                Ok(false) => {
                                                    tracing::error!(
                                                        conversation = %event_conversation,
                                                        "durable reset intent disappeared after adapter commit; stopping before in-memory release"
                                                    );
                                                    return Err(anyhow::anyhow!(
                                                        "durable /new intent disappeared after commit"
                                                    ));
                                                }
                                                Err(error) => {
                                                    tracing::error!(
                                                        conversation = %event_conversation,
                                                        %error,
                                                        "could not checkpoint committed reset; prepared recovery record remains"
                                                    );
                                                    return Err(anyhow::anyhow!(
                                                        "could not checkpoint committed durable /new: {error}"
                                                    ));
                                                }
                                            }
                                        }

                                        // No fallible queue mutation remains after
                                        // the durable adapter commit. If the harness
                                        // dies before this in-memory step, Pi's reset
                                        // tombstone still forces fresh context and
                                        // suppresses relay-history seeding on restart.
                                        let fresh_conversation = queue
                                            .commit_prepared_reset(prepared_reset, command_id);
                                        let reset_signal = if thread_sessions_enabled {
                                            // The durable mapping is already gone;
                                            // cancellation only releases the old hot
                                            // handle and must not issue a second forget.
                                            ControlSignal::Cancel
                                        } else {
                                            ControlSignal::Rotate
                                        };
                                        let fired = signal_in_flight_conversation(
                                            &mut pool,
                                            &event_conversation,
                                            reset_signal,
                                        );
                                        let invalidated = pool
                                            .dispose_conversation_sessions(
                                                &event_conversation,
                                                !thread_sessions_enabled,
                                            )
                                            .await;
                                        tracing::info!(
                                            conversation = %event_conversation,
                                            fresh_conversation = %fresh_conversation,
                                            fired,
                                            invalidated,
                                            "/new consumed — resetting thread context"
                                        );
                                        let identity = (
                                            fresh_conversation.channel_id,
                                            fresh_conversation.root_event_id.clone(),
                                        );
                                        debug_assert_eq!(identity, reset_identity);
                                        pending_reset_confirmations.insert(
                                            identity,
                                            PendingResetConfirmation {
                                                event: reset_notice_event,
                                                next_attempt_at: Instant::now(),
                                                durable: thread_sessions_enabled,
                                            },
                                        );
                                        tracing::info!(
                                            conversation = %fresh_conversation,
                                            waiting_for_old_turn = fired,
                                            "deferring /new dispatch until its signed acknowledgement is accepted"
                                        );
                                        continue;
                                    }
                                }
                                tracing::warn!(
                                    conversation = %event_conversation,
                                    sender = %buzz_event.event.pubkey,
                                    "denying non-owner /new command"
                                );
                                // Reserved lifecycle commands are always
                                // consumed. Forwarding this to Pi could let an
                                // adapter or extension reset context despite
                                // the harness authorization failure.
                                continue;
                            }

                            // Pi lifecycle commands that mutate a thread's
                            // durable runtime are owner-only. They still pass
                            // through to the adapter for an authorized owner;
                            // an untrusted participant cannot force compaction
                            // or reload executable extension resources.
                            if let Some(command) = reserved_pi_lifecycle_command(
                                &buzz_event.event,
                                kind_u32,
                                config.session_title.as_deref(),
                            ) {
                                // An addressed /new was consumed above whether
                                // authorized or denied. Reaching this arm with
                                // /new means it lacked this agent's p tag; never
                                // forward it to an adapter whose first-line
                                // command parser could interpret it anyway.
                                let authorized = command != "/new"
                                    && is_owner_control_command(
                                        &buzz_event.event,
                                        kind_u32,
                                        command,
                                        &pubkey_hex,
                                        config.session_title.as_deref(),
                                    )
                                    && owner_cache.get().is_some_and(|owner| {
                                        buzz_event.event.pubkey.to_hex() == *owner
                                    });
                                if !authorized {
                                    tracing::warn!(
                                        conversation = %event_conversation,
                                        sender = %buzz_event.event.pubkey,
                                        command,
                                        "denying unaddressed or non-owner Pi lifecycle command"
                                    );
                                    continue;
                                }
                            }

                            // Coarse security policy: drop events from disallowed
                            // authors before they reach subscription rules or the
                            // agent. Must be AFTER !shutdown (owner can always
                            // shut down regardless of gate mode).
                            //
                            // Both OwnerOnly and Allowlist accept events from
                            // "siblings" — pubkeys whose agent_owner_pubkey
                            // matches this agent's owner (e.g. other bots
                            // launched by the same human). Allowlist adds the
                            // explicit pubkey list on top, for external people;
                            // it never revokes same-owner team bots.
                            {
                                let author = buzz_event.event.pubkey.to_hex();
                                // DM hardening: resolve channel type (fail-closed
                                // to DM) so allowlist/anyone modes cannot be
                                // exercised by non-owner authors inside DMs.
                                let allowed = author_allowed(
                                    &config.respond_to,
                                    &config.respond_to_allowlist,
                                    &author,
                                    event_is_dm,
                                    &owner_cache,
                                    &ctx.rest_client,
                                )
                                .await;
                                if !allowed {
                                    tracing::debug!(
                                        channel_id = %buzz_event.channel_id,
                                        author = %buzz_event.event.pubkey.to_hex(),
                                        mode = %config.respond_to,
                                        is_dm = event_is_dm,
                                        "inbound author gate — dropping event"
                                    );
                                    continue;
                                }
                            }

                            let matched = filter::match_event(&buzz_event.event, buzz_event.channel_id, &rules, &pubkey_hex).await;
                            let prompt_tag = match matched {
                                Some(m) => m.prompt_tag,
                                None => {
                                    tracing::debug!(channel_id = %buzz_event.channel_id, kind = buzz_event.event.kind.as_u16(), "event matched no rule — dropping");
                                    continue;
                                }
                            };
                            // Capture author pubkey before queue.push() moves
                            // buzz_event.event (needed for mode gate below).
                            let author_hex = buzz_event.event.pubkey.to_hex();
                            let event_id_hex = buzz_event.event.id.to_hex();
                            // Clone for the non-cancelling steer fork, which
                            // needs the event to render the steer body. The
                            // clone is unconditional because we don't know
                            // yet whether the mode gate will demand a steer
                            // — checking `multiple_event_handling` here
                            // would couple the queueing path to the mode
                            // and break the existing invariant that every
                            // accepted event goes through `queue.push`
                            // first. `nostr::Event::clone` is cheap (Arc-
                            // backed payload) so the cost is negligible.
                            let event_for_steer = buzz_event.event.clone();
                            let prompt_tag_for_steer = prompt_tag.clone();
                            let accepted = queue.push(QueuedEvent {
                                channel_id: buzz_event.channel_id,
                                event: buzz_event.event,
                                received_at: std::time::Instant::now(),
                                prompt_tag,
                            });
                            // 👀 — immediate "seen" reaction, only if the event
                            // was actually queued (not dropped by DedupMode::Drop).
                            // Fire-and-forget: on rare fast-failure paths the
                            // guard's cleanup may race with this add, leaving a
                            // cosmetic stale 👀. Acceptable — see ReactionGuard docs.
                            if accepted {
                                let rc = ctx.rest_client.clone();
                                let eid = event_id_hex.clone();
                                tokio::spawn(async move {
                                    pool::reaction_add(&rc, &eid, "👀").await;
                                });
                            }
                            // Event is already queued. If mode requires it AND
                            // the channel has an in-flight task, fire cancel —
                            // OR take the non-cancelling (ACP steer) fork for Steer signals.
                            if accepted
                                && queue.is_conversation_in_flight(&event_conversation)
                            {
                                // Author eligibility (owner ∪ allowlist ∪ siblings)
                                // is already enforced by the inbound author gate
                                // above, so the mid-turn signal fires for every
                                // event that reaches here.
                                let signal = mode_gate_signal(
                                    config.multiple_event_handling,
                                    &author_hex,
                                    owner_cache.get(),
                                );
                                if let Some(signal) = signal {
                                    // Non-cancelling fork: when the mode
                                    // wants a Steer, attempt the
                                    // non-cancelling path first. On accept,
                                    // withhold the queued event and spawn an
                                    // ack watcher; the main loop's
                                    // `PoolEvent::SteerAck` arm decides
                                    // success/release/fallback. On reject
                                    // (including agents that advertise no
                                    // steer transport at all), fall through
                                    // to the universal cancel+merge `Steer`
                                    // signal so the event still reaches the
                                    // agent.
                                    let native_attempted = matches!(signal, ControlSignal::Steer)
                                        && try_native_steer(
                                            &mut pool,
                                            &mut queue,
                                            event_conversation.clone(),
                                            event_for_steer,
                                            prompt_tag_for_steer,
                                            &steer_ack_tx,
                                        );
                                    if !native_attempted {
                                        signal_in_flight_conversation(
                                            &mut pool,
                                            &event_conversation,
                                            signal,
                                        );
                                    }
                                }
                            }
                            if pool_ready {
                                for (channel_id, thread_tags) in
                                    dispatch_pending(&mut pool, &mut queue, &ctx, &mut last_activity)
                                {
                                    typing_channels.insert(channel_id, thread_tags);
                                }
                            }
                        }
                        None => {
                            tracing::warn!("relay event stream ended — requesting reconnect");
                            if let Err(e) = relay.reconnect().await {
                                tracing::error!("relay background task is gone: {e} — exiting");
                                tokio::time::sleep(Duration::from_secs(1)).await;
                                break;
                            }
                        }
                    }
                    None
                }
                _ = async {
                    match inactivity_reaper.as_mut() {
                        Some(timer) => timer.tick().await,
                        None => std::future::pending().await,
                    }
                } => {
                    let _ = result_rx;
                    if inactivity_expired(
                        last_activity,
                        tokio::time::Instant::now(),
                        inactivity_bound,
                        queue.has_in_flight() || heartbeat_in_flight,
                    ) {
                        tracing::info!(
                            inactivity_seconds = config.exit_after_inactivity_secs,
                            "inactivity bound reached — exiting gracefully"
                        );
                        let _ = shutdown_tx.send(());
                    }
                    None
                }
                _ = async {
                    match heartbeat.as_mut() {
                        Some(hb) => hb.tick().await,
                        None => std::future::pending().await,
                    }
                } => {
                    let _ = result_rx;
                    if !pool_ready {
                        tracing::debug!("heartbeat_skipped_pool_not_ready");
                    } else if queue.has_flushable_work() {
                        tracing::debug!("heartbeat_skipped_events");
                        for (channel_id, thread_tags) in
                            dispatch_pending(&mut pool, &mut queue, &ctx, &mut last_activity)
                        {
                            typing_channels.insert(channel_id, thread_tags);
                        }
                    } else if pool.any_idle() {
                        dispatch_heartbeat(&mut pool, &ctx, &mut heartbeat_in_flight);
                    } else {
                        tracing::debug!("heartbeat_skipped_busy");
                    }
                    None
                }
                _ = async {
                    match presence_heartbeat.as_mut() {
                        Some(t) => t.tick().await,
                        None => std::future::pending().await,
                    }
                } => {
                    let _ = result_rx;
                    // Abort previous heartbeat if still in flight (prevents race on shutdown).
                    if let Some(h) = presence_task.take() {
                        h.abort();
                    }
                    let pp = presence_publisher.clone();
                    let pk = presence_keys.clone();
                    presence_task = Some(tokio::spawn(async move {
                        if let Err(e) = publish_presence(&pp, &pk, "online").await {
                            tracing::warn!("presence heartbeat failed: {e}");
                        }
                    }));
                    None
                }
                _ = async {
                    match typing_refresh.as_mut() {
                        Some(t) => t.tick().await,
                        None => std::future::pending().await,
                    }
                } => {
                    let _ = result_rx;
                    // Use try_publish (non-blocking) for typing indicators —
                    // they're ephemeral and must not block the main loop during
                    // relay reconnection (#35).
                    for (conversation_key, thread_tags) in &typing_channels {
                        let ch = conversation_key.channel_id;
                        if let Ok(event) = relay.build_typing_event(
                            ch,
                            thread_tags.root_event_id.as_deref(),
                            thread_tags.parent_event_id.as_deref(),
                        ) {
                            if let Err(e) = relay.try_publish_event(event) {
                                tracing::debug!("typing indicator dropped for {ch}: {e}");
                            }
                        }
                    }
                    None
                }
                _ = shutdown_rx.changed() => {
                    tracing::info!("shutting down");
                    break;
                }
            }
        };

        match pool_event {
            Some(PoolEvent::Result(result)) => {
                let mut result = *result;
                // Membership removal can race a checked-out agent that still
                // owns sibling thread sessions in the removed channel. Forget
                // those adapter runtimes before this agent becomes claimable
                // again; on failure the adapter is terminated fail-closed.
                for channel_id in &removed_channels {
                    result.agent.forget_channel_sessions(*channel_id).await;
                }
                // Stop typing indicator for the completed channel.
                if let PromptSource::Conversation(key) = &result.source {
                    typing_channels.remove(key);
                } else if let PromptSource::Channel(ch) = &result.source {
                    typing_channels.retain(|key, _| key.channel_id != *ch);
                }
                let action = handle_prompt_result(
                    &mut pool,
                    &mut queue,
                    &config,
                    result,
                    &mut heartbeat_in_flight,
                    &removed_channels,
                    &mut crash_history,
                    &respawn_tx,
                    &mut respawn_tasks,
                    observer.clone(),
                    Some(&lifecycle_notice_outbox),
                    Some(&ctx.rest_client),
                );
                if action == LoopAction::Exit {
                    break;
                }
                if drain_ready_join_results(
                    &mut pool,
                    &mut queue,
                    &config,
                    &mut heartbeat_in_flight,
                    &removed_channels,
                    &mut typing_channels,
                    &mut crash_history,
                    &respawn_tx,
                    &mut respawn_tasks,
                    observer.clone(),
                ) == LoopAction::Exit
                {
                    break;
                }
                for (channel_id, thread_tags) in
                    dispatch_pending(&mut pool, &mut queue, &ctx, &mut last_activity)
                {
                    typing_channels.insert(channel_id, thread_tags);
                }
            }
            Some(PoolEvent::Panic(join_error)) => {
                tracing::error!("agent task panicked: {join_error}");
                recover_panicked_agent(
                    &mut pool,
                    &mut queue,
                    &config,
                    join_error,
                    &mut heartbeat_in_flight,
                    &removed_channels,
                    &mut typing_channels,
                    &mut crash_history,
                    &respawn_tx,
                    &mut respawn_tasks,
                    observer.clone(),
                );
                if pool.live_count() == 0 && !any_respawn_in_flight(&crash_history) {
                    tracing::error!("all agents dead — exiting");
                    break;
                }
                for (channel_id, thread_tags) in
                    dispatch_pending(&mut pool, &mut queue, &ctx, &mut last_activity)
                {
                    typing_channels.insert(channel_id, thread_tags);
                }
            }
            Some(PoolEvent::ResetAck(result)) => {
                finish_reset_ack_attempt(
                    &mut queue,
                    &mut pending_reset_confirmations,
                    &mut reset_ack_in_flight,
                    Some(&durable_outbox),
                    result,
                );
            }
            Some(PoolEvent::SteerAck(SteerAckEvent {
                channel_id,
                conversation_key,
                event_id,
                ack,
            })) => {
                // Mid-turn steer attempt resolved (either transport:
                // `_goose/unstable/session/steer` or `_session/steering`).
                // Locked semantics (Eva + Max + Perci, unanimous on Option X):
                //
                //   Success
                //     The agent received the steer via the non-cancelling
                //     path. Drop the withheld event so normal dispatch
                //     never redelivers it.
                //
                //     Also covers `_session/steering`'s `startedNewTurn`
                //     outcome: the message was delivered, but into a fresh
                //     turn because the one being steered had already
                //     finished. Delivery is what this arm keys on, so the
                //     event is still dropped. The read loop deliberately
                //     does NOT renew its hard deadline in that case (the
                //     awaited turn is settled), while
                //     `extend_in_flight_deadline` below still applies —
                //     the agent really is running more work, so the
                //     channel's in-flight budget should reflect it.
                //
                //   Err(_) where the write never landed (Transport /
                //   ExpectedRunIdMissing):
                //     Delivery state of the underlying message is "never
                //     attempted on the wire". Release withheld back to the
                //     queue front AND issue the cancel+merge fallback so
                //     the message still reaches the agent.
                //
                //   Err(OutcomeRejected { .. })
                //     A `_session/steering` request returned a JSON-RPC
                //     success whose `outcome` was not `injected` or
                //     `startedNewTurn` (codex's `failed`, an unknown value,
                //     or a bare `{}` with no `outcome` at all). The steer
                //     did not land, so this is treated exactly like a write
                //     that never happened: release withheld AND fire the
                //     cancel+merge fallback. Handled by the catch-all
                //     `Err(_)` arm below.
                //
                //   Err(AgentError { code: -32601, .. })
                //     The agent returned method_not_found — it does not
                //     implement the steer extension. Release withheld AND
                //     fire the cancel+merge fallback so the message still
                //     reaches the agent via the universal path.
                //
                //   Err(AgentError { code: other, .. })
                //     The write landed and the agent returned a JSON-RPC
                //     error at the application level (e.g. wrong run id).
                //     The agent's turn is still running (or just completed).
                //     Release withheld for normal dispatch; do NOT fire the
                //     fallback signal — the agent already saw the steer
                //     attempt. If the turn is still running, normal dispatch
                //     re-delivers when it completes. If the turn already
                //     ended, there is nothing to cancel.
                //
                //   PromptCompletedNeutral
                //     The read loop wrote the steer (or was preparing to)
                //     but the prompt completed before the response landed.
                //     Delivery state is unknown — but the prompt completing
                //     means there is no in-flight turn to signal anymore.
                //     Release withheld for normal dispatch; do NOT fire
                //     the fallback signal (it would target a turn that
                //     just ended; normal dispatch already handles
                //     redelivery via the released queue entry).
                //
                //   Err(PromptCompleted)
                //     `SteerError::PromptCompleted` is returned synchronously
                //     by `pool::send_steer` when no task is in flight (handled
                //     in `try_native_steer`'s Err branch, which falls through
                //     to cancel+merge). It is never routed through the ack
                //     channel, so this variant never appears in `SteerAckEvent`.
                //
                //   Watcher Err (oneshot dropped)
                //     Should not happen — the read loop drains
                //     pending_steer on every return path. If it does,
                //     treat as PromptCompletedNeutral to avoid leaking
                //     the withheld event in `withheld_native_steer`.
                let (release_withheld, drop_withheld, signal_fallback) = match &ack {
                    Ok(pool::SteerAck::Success) => (false, true, false),
                    // -32601 = method_not_found: agent does not implement the
                    // steer extension. Fire cancel+merge so the message still
                    // reaches the agent.
                    Ok(pool::SteerAck::Err(pool::SteerError::AgentError { code, .. }))
                        if *code == -32601 =>
                    {
                        (true, false, true)
                    }
                    // AgentError: write landed, agent rejected it at the
                    // application level (e.g. wrong run id). Release for
                    // normal dispatch; no fallback signal (the turn is still
                    // running or just ended — either way there is nothing to
                    // cancel).
                    Ok(pool::SteerAck::Err(pool::SteerError::AgentError { .. })) => {
                        (true, false, false)
                    }
                    // Transport / ExpectedRunIdMissing / OutcomeRejected: the
                    // steer did not land. Release and fire the cancel+merge
                    // fallback so the message still reaches the agent.
                    Ok(pool::SteerAck::Err(_)) => (true, false, true),
                    Ok(pool::SteerAck::PromptCompletedNeutral) => (true, false, false),
                    Err(_recv_err) => (true, false, false),
                };
                tracing::info!(
                    channel = %channel_id,
                    event_id = %event_id,
                    ?ack,
                    release_withheld,
                    drop_withheld,
                    signal_fallback,
                    "non-cancelling steer ack received"
                );
                if matches!(ack, Ok(pool::SteerAck::Success)) {
                    queue.extend_conversation_deadline(
                        &conversation_key,
                        config.max_turn_duration_secs,
                    );
                }
                if drop_withheld {
                    queue.remove_conversation_event(&conversation_key, &event_id);
                }
                if release_withheld {
                    queue.release_conversation_native_steer(&conversation_key, &event_id);
                }
                if signal_fallback {
                    // Universal cancel+merge fallback. Note: the
                    // queued event has already been released to the
                    // front of `queues[channel_id]`, so the cancel
                    // will pick it up as part of the merged batch and
                    // re-prompt the agent.
                    signal_in_flight_conversation(
                        &mut pool,
                        &conversation_key,
                        ControlSignal::Steer,
                    );
                }
                // After releasing a withheld event, give dispatch a chance
                // to re-flush. If the prompt is still in flight, the
                // channel stays `in_flight_channels` and `flush_next`
                // skips it — but a Steer fallback signal sent above will
                // tear down the in-flight task; on its completion the
                // queue drains. We still try here in case the in-flight
                // task has already returned.
                for (channel_id, thread_tags) in
                    dispatch_pending(&mut pool, &mut queue, &ctx, &mut last_activity)
                {
                    typing_channels.insert(channel_id, thread_tags);
                }
            }
            Some(PoolEvent::Wake(attempt, result)) => {
                let completion = result.as_ref().map(|_| ()).map_err(|error| error.clone());
                if let Err(error) =
                    pool_lifecycle.complete_wake(attempt, result, tokio::time::Instant::now())
                {
                    tracing::warn!(attempt, error, "discarding stale pool wake result");
                    continue;
                }
                match completion {
                    Ok(()) => {
                        pool = pool_lifecycle
                            .take_ready()
                            .expect("successful wake stores a ready pool");
                        thread_sessions_enabled = match select_thread_session_scope(
                            config.require_durable_thread_sessions,
                            true,
                            pool.thread_sessions_supported(),
                        ) {
                            Ok(enabled) => enabled,
                            Err(message) => {
                                tracing::error!(
                                    message,
                                    "lazy adapter capability check failed closed"
                                );
                                emit_runtime_lifecycle(
                                    observer.as_ref(),
                                    &runtime_start_nonce,
                                    &pubkey_hex,
                                    &config.relay_url,
                                    "failed",
                                    Some(message),
                                );
                                terminal_error = Some(anyhow::anyhow!(message));
                                break;
                            }
                        };
                        if !queue.set_thread_sessions_enabled(thread_sessions_enabled) {
                            if config.require_durable_thread_sessions {
                                let message = "required durable thread-session scope could not be applied to the lazy event queue";
                                tracing::error!(message, "lazy adapter queue scope failed closed");
                                emit_runtime_lifecycle(
                                    observer.as_ref(),
                                    &runtime_start_nonce,
                                    &pubkey_hex,
                                    &config.relay_url,
                                    "failed",
                                    Some(message),
                                );
                                terminal_error = Some(anyhow::anyhow!(message));
                                break;
                            }
                            tracing::error!(
                                thread_sessions_enabled,
                                "lazy pool initialized after queue lifecycle state; retaining prior conversation scope"
                            );
                            thread_sessions_enabled = queue.thread_sessions_enabled();
                        }
                        pool_ready = true;
                        emit_runtime_lifecycle(
                            observer.as_ref(),
                            &runtime_start_nonce,
                            &pubkey_hex,
                            &config.relay_url,
                            "ready",
                            None,
                        );
                        for (channel_id, thread_tags) in
                            dispatch_pending(&mut pool, &mut queue, &ctx, &mut last_activity)
                        {
                            typing_channels.insert(channel_id, thread_tags);
                        }
                    }
                    Err(error) => {
                        debug_assert_eq!(pool_lifecycle.failed_error(), Some(error.as_str()));
                        emit_runtime_lifecycle(
                            observer.as_ref(),
                            &runtime_start_nonce,
                            &pubkey_hex,
                            &config.relay_url,
                            "failed",
                            Some(&error),
                        );
                    }
                }
            }
            None => {} // relay/heartbeat/shutdown branches handled inline above
        }
    }

    // A reset acknowledgement may be inside its bounded HTTP attempt when a
    // signal arrives. Give that tracked delivery one attempt window to finish
    // so a normal shutdown does not discard an already-signed `/new` success
    // message. We deliberately do not begin a fresh retry schedule while
    // shutting down.
    schedule_completed_reset_ack(
        &queue,
        &pending_reset_confirmations,
        &ctx.rest_client,
        &mut reset_ack_tasks,
        &mut reset_ack_in_flight,
    );
    let reset_ack_drain = tokio::time::timeout(Duration::from_secs(6), async {
        while let Some(result) = reset_ack_tasks.join_next().await {
            finish_reset_ack_attempt(
                &mut queue,
                &mut pending_reset_confirmations,
                &mut reset_ack_in_flight,
                Some(&durable_outbox),
                result,
            );
        }
    })
    .await;
    if reset_ack_drain.is_err() {
        tracing::warn!("reset acknowledgement task did not drain — aborting it");
        reset_ack_tasks.shutdown().await;
    }

    // Drain wake tasks gracefully rather than aborting: an in-flight
    // initialize_agent_pool observes the shutdown watch at its biased per-slot
    // select and reaps its partially-spawned agents itself. `shutdown()` here
    // would abort the task mid-init and drop those AcpClients via best-effort
    // Drop — the exact zombie class the eager path's spawn-outside-the-timeout
    // comment exists to prevent. Fire the watch first so exits that bypass the
    // signal handlers (result channel closed, LoopAction::Exit) cancel the wake
    // just as promptly. Timeout is a backstop for a slot stuck outside the
    // select (e.g. in spawn); only then do we fall back to aborting.
    let _ = shutdown_tx.send(());
    let wake_drain = tokio::time::timeout(Duration::from_secs(30), async {
        while wake_tasks.join_next().await.is_some() {}
    })
    .await;
    if wake_drain.is_err() {
        tracing::warn!("wake task did not drain within grace period — aborting");
        wake_tasks.shutdown().await;
    }
    while let Ok((_attempt, result)) = wake_rx.try_recv() {
        if let Ok(mut awakened_pool) = result {
            shutdown_agent_pool(&mut awakened_pool).await;
        }
    }

    tracing::info!("shutdown: waiting for in-flight prompts");
    // 30 s is generous for in-flight prompts to be cancelled; using
    // max_turn_duration here would cause Ctrl+C to hang for up to an hour.
    let grace = Duration::from_secs(30);
    // Best-effort drain of both join_set and result_rx during the grace period.
    // Tasks that finish normally send their OwnedAgent through result_rx — we
    // explicitly shut them down here to reap child processes. If the grace
    // period expires, remaining tasks are aborted and fall back to
    // AcpClient::Drop (start_kill + try_wait — best-effort, not guaranteed).
    let (rx_ref, js_ref) = pool.rx_and_join_set();
    let shutdown_result = tokio::time::timeout(grace, async {
        loop {
            tokio::select! {
                result = js_ref.join_next() => {
                    match result {
                        Some(Err(e)) => tracing::warn!("task error during shutdown: {e}"),
                        Some(Ok(())) => {}
                        None => break, // join_set empty
                    }
                }
                maybe_result = rx_ref.recv() => {
                    if let Some(mut pr) = maybe_result {
                        let lifecycle_scope = lifecycle_notice_scope(&pr);
                        let lifecycle_notices =
                            take_current_buzz_session_notices(&mut pr, &queue);
                        let lifecycle_outcome = enqueue_session_lifecycle_notices(
                            Some(&lifecycle_notice_outbox),
                            Some(&ctx.rest_client),
                            lifecycle_scope,
                            lifecycle_notices,
                        );
                        for acknowledgement in lifecycle_outcome.acknowledgements {
                            pr.agent
                                .acp
                                .queue_buzz_session_event_ack(acknowledgement);
                        }
                        let idx = pr.agent.index;
                        pr.agent.acp.shutdown().await;
                        tracing::debug!(agent = idx, "reaped checked-out agent on shutdown");
                    }
                    // If None, channel closed — tasks are done.
                }
            }
        }
    })
    .await;
    if shutdown_result.is_err() {
        tracing::warn!("grace period expired, aborting remaining tasks");
        pool.join_set.shutdown().await;
    }
    // Drain any remaining results that arrived after join_set drained but
    // before tasks were aborted.
    while let Ok(mut pr) = pool.result_rx_try_recv() {
        let lifecycle_scope = lifecycle_notice_scope(&pr);
        let lifecycle_notices = take_current_buzz_session_notices(&mut pr, &queue);
        let lifecycle_outcome = enqueue_session_lifecycle_notices(
            Some(&lifecycle_notice_outbox),
            Some(&ctx.rest_client),
            lifecycle_scope,
            lifecycle_notices,
        );
        for acknowledgement in lifecycle_outcome.acknowledgements {
            pr.agent.acp.queue_buzz_session_event_ack(acknowledgement);
        }
        let idx = pr.agent.index;
        pr.agent.acp.shutdown().await;
        tracing::debug!(agent = idx, "reaped late-arriving agent on shutdown");
    }
    // Explicitly shut down idle agents still sitting in their slots.
    for slot in pool.agents_mut().iter_mut() {
        if let Some(agent) = slot.take() {
            let idx = agent.index;
            let mut acp = agent.acp;
            acp.shutdown().await;
            tracing::debug!(agent = idx, "reaped idle agent on shutdown");
        }
    }
    drop(pool);

    // Abort any in-flight respawn tasks. They may be sleeping in backoff or
    // running spawn_and_init — either way, we don't want them spawning new
    // children after the main loop has exited. RespawnGuard::Drop sends a
    // failure result for aborted tasks, so respawn_in_flight is cleared.
    respawn_tasks.shutdown().await;

    // Drain any respawn results that completed before the abort. Explicitly
    // shut down returned agents instead of relying on AcpClient::Drop.
    while let Ok(rr) = respawn_rx.try_recv() {
        if let Ok((mut acp, _, _)) = rr.result {
            acp.shutdown().await;
            tracing::debug!(agent = rr.index, "reaped respawned agent on shutdown");
        }
    }

    // Cancel any in-flight presence heartbeat before sending offline.
    if let Some(h) = presence_task.take() {
        h.abort();
    }

    // Best-effort: set presence to offline before exiting.
    if config.presence_enabled {
        match tokio::time::timeout(
            Duration::from_secs(2),
            publish_presence(&presence_publisher, &presence_keys, "offline"),
        )
        .await
        {
            Ok(Ok(_)) => tracing::info!("presence set to offline"),
            Ok(Err(e)) => tracing::warn!("failed to set offline presence: {e}"),
            Err(_) => tracing::warn!("offline presence timed out"),
        }
    }

    if let Some(handle) = relay_observer_publisher_task.take() {
        handle.abort();
    }

    // Compaction/session lifecycle notices are product-visible state, not
    // cosmetic telemetry. Drain their signed, retrying outbox before tearing
    // down relay resources so a normal shutdown cannot lose an awareness
    // message that was already emitted by Pi.
    lifecycle_notice_outbox.shutdown().await;

    // Graceful relay shutdown — sends WebSocket close frame and waits up to 5s
    // for the background task to finish, rather than aborting immediately (#40).
    relay.shutdown().await;

    tracing::info!("buzz-acp stopped");
    match terminal_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

#[derive(PartialEq)]
enum LoopAction {
    Continue,
    Exit,
}

#[cfg(test)]
fn test_conversation_key(channel_id: Uuid) -> ConversationKey {
    ConversationKey {
        channel_id,
        root_event_id: format!("channel:{channel_id}"),
        generation: 0,
        reset_token: None,
    }
}

fn event_mentions_agent(event: &nostr::Event, agent_pubkey_hex: &str) -> bool {
    event.tags.iter().any(|t| {
        t.as_slice().first().map(|s| s.as_str()) == Some("p")
            && t.as_slice().get(1).map(|s| s.as_str()) == Some(agent_pubkey_hex)
    })
}

fn is_owner_control_command(
    event: &nostr::Event,
    kind_u32: u32,
    command: &str,
    agent_pubkey_hex: &str,
    agent_display_name: Option<&str>,
) -> bool {
    let content_matches = if command.starts_with('/') {
        let known_names = agent_display_name.map_or_else(Vec::new, |name| vec![name]);
        adapter_leading_slash_command(&event.content, &known_names).as_deref() == Some(command)
    } else {
        event.content.trim() == command
    };
    kind_u32 == KIND_STREAM_MESSAGE
        && content_matches
        && event_mentions_agent(event, agent_pubkey_hex)
}

/// Mirror the adapter's first-line command semantics after Buzz mention
/// normalization. This closes a policy gap where `/reload\n...` was not an
/// exact harness match but Pi still executed its first line as `/reload`.
fn adapter_leading_slash_command(content: &str, known_names: &[&str]) -> Option<String> {
    queue::extract_slash_command(content, known_names).and_then(|command| {
        command
            .split(['\r', '\n'])
            .next()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_string)
    })
}

/// Detect every Pi command that can mutate or replace durable session state,
/// independently of Nostr mention tags. Detection happens before the general
/// author/filter gates so subscribe-all and no-mention modes cannot smuggle a
/// bare command into Pi's first text block.
fn reserved_pi_lifecycle_command(
    event: &nostr::Event,
    kind_u32: u32,
    agent_display_name: Option<&str>,
) -> Option<&'static str> {
    if kind_u32 != KIND_STREAM_MESSAGE {
        return None;
    }
    let known_names = agent_display_name.map_or_else(Vec::new, |name| vec![name]);
    let command = adapter_leading_slash_command(&event.content, &known_names)?;
    ["/new", "/compact", "/reload"]
        .into_iter()
        .find(|reserved| command == *reserved)
}

// ── signal_in_flight_task ─────────────────────────────────────────────────────

/// Decide which [`ControlSignal`] (if any) to send to an in-flight turn when a
/// new, already-author-gated event arrives for that channel.
///
/// Returns `None` to leave the in-flight turn untouched (the event waits in the
/// queue and is delivered when the turn completes). Author eligibility — owner
/// ∪ allowlist ∪ siblings — is enforced upstream by the inbound author gate, so
/// `Steer`/`Interrupt` apply to every event that reaches this point; only
/// `OwnerInterrupt` re-checks authorship (owner-only) here.
///
/// `owner` is the resolved owner pubkey hex, if known.
fn mode_gate_signal(
    handling: MultipleEventHandling,
    author_hex: &str,
    owner: Option<&str>,
) -> Option<ControlSignal> {
    match handling {
        MultipleEventHandling::Queue => None,
        MultipleEventHandling::Steer => Some(ControlSignal::Steer),
        MultipleEventHandling::Interrupt => Some(ControlSignal::Interrupt),
        MultipleEventHandling::OwnerInterrupt => match owner {
            Some(o) if author_hex == o => Some(ControlSignal::Interrupt),
            _ => None,
        },
    }
}

/// Send a control signal to the in-flight task for `channel_id`.
/// Returns `true` if a signal was sent, `false` if no in-flight task was found.
#[cfg(test)]
fn signal_in_flight_task(
    pool: &mut AgentPool,
    channel_id: uuid::Uuid,
    mode: ControlSignal,
) -> bool {
    let matches = pool
        .task_map()
        .values()
        .filter(|meta| meta.channel_id == Some(channel_id))
        .count();
    if matches != 1 {
        if matches > 1 {
            tracing::warn!(
                channel = %channel_id,
                active_conversations = matches,
                "refusing ambiguous channel-only control signal"
            );
        }
        return false;
    }
    let entry = pool
        .task_map_mut()
        .values_mut()
        .find(|m| m.channel_id == Some(channel_id));

    if let Some(meta) = entry {
        if let Some(tx) = meta.control_tx.take() {
            tracing::info!(channel = %channel_id, ?mode, "control signal sent to in-flight task");
            let _ = tx.send(mode);
            return true;
        }
    }
    false
}

/// Routing outcome for an owner-authenticated desktop control frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ObserverControlResult {
    Sent,
    TurnEnding,
    NoActiveTurn,
    Ambiguous,
}

impl ObserverControlResult {
    fn as_status(self) -> &'static str {
        match self {
            Self::Sent => "sent",
            Self::TurnEnding => "turn_ending",
            Self::NoActiveTurn => "no_active_turn",
            Self::Ambiguous => "ambiguous",
        }
    }
}

/// Route a desktop control to one in-flight turn.
///
/// `turn_id` must match both the exact turn and its channel. Channel-only relay
/// controls are rejected before this function so a retained signed frame can
/// never select a later turn after a process restart.
fn signal_observer_control(
    pool: &mut AgentPool,
    channel_id: Uuid,
    turn_id: &str,
    signal: ControlSignal,
) -> ObserverControlResult {
    let matching_task_ids: Vec<_> = pool
        .task_map()
        .iter()
        .filter(|(_, meta)| meta.channel_id == Some(channel_id) && meta.turn_id == turn_id)
        .map(|(task_id, _)| *task_id)
        .collect();

    let task_id = match matching_task_ids.as_slice() {
        [] => return ObserverControlResult::NoActiveTurn,
        [task_id] => *task_id,
        _ => {
            tracing::warn!(
                channel = %channel_id,
                %turn_id,
                matches = matching_task_ids.len(),
                "refusing ambiguous observer control signal"
            );
            return ObserverControlResult::Ambiguous;
        }
    };

    let Some(meta) = pool.task_map_mut().get_mut(&task_id) else {
        return ObserverControlResult::NoActiveTurn;
    };
    let Some(tx) = meta.control_tx.take() else {
        return ObserverControlResult::TurnEnding;
    };

    match tx.send(signal) {
        Ok(()) => {
            tracing::info!(
                channel = %channel_id,
                turn_id = %meta.turn_id,
                "observer control signal sent to exact in-flight turn"
            );
            ObserverControlResult::Sent
        }
        Err(signal) => {
            tracing::warn!(
                channel = %channel_id,
                turn_id = %meta.turn_id,
                ?signal,
                "observer control receiver closed before delivery"
            );
            ObserverControlResult::TurnEnding
        }
    }
}

/// Broadcast the intentionally channel-wide `!rotate` command to every active
/// thread. Unlike legacy channel-only desktop controls, this fan-out is
/// explicit product semantics rather than an ambiguous first-match lookup.
fn rotate_all_in_flight_channel(pool: &mut AgentPool, channel_id: Uuid) -> usize {
    let mut sent = 0;
    for meta in pool
        .task_map_mut()
        .values_mut()
        .filter(|meta| meta.channel_id == Some(channel_id))
    {
        if let Some(tx) = meta.control_tx.take() {
            let _ = tx.send(ControlSignal::Rotate);
            sent += 1;
        }
    }
    sent
}

/// Send a control signal to one exact in-flight thread conversation.
fn signal_in_flight_conversation(
    pool: &mut AgentPool,
    conversation_key: &ConversationKey,
    mode: ControlSignal,
) -> bool {
    let entry = pool
        .task_map_mut()
        .values_mut()
        .find(|meta| meta.conversation_key.as_ref() == Some(conversation_key));

    if let Some(meta) = entry {
        if let Some(tx) = meta.control_tx.take() {
            tracing::info!(
                conversation = %conversation_key,
                ?mode,
                "control signal sent to in-flight conversation"
            );
            let _ = tx.send(mode);
            return true;
        }
    }
    false
}

/// Attempt the non-cancelling (ACP) steer for a freshly-queued event.
///
/// Caller invariants:
/// - `event` has already been pushed into `EventQueue::queues[channel_id]`
///   via [`EventQueue::push`] — its `event.id` must still be locatable
///   there so [`EventQueue::mark_native_steer_pending`] can move it to the
///   side table.
/// - `multiple_event_handling` resolved to `ControlSignal::Steer`; this
///   function is the non-cancelling fork of that signal.
///
/// Returns `true` if the native attempt was accepted by the read loop
/// (capacity-1 mpsc `try_send` succeeded, event withheld synchronously,
/// ack watcher spawned). On `true` the caller MUST NOT issue the
/// universal cancel+merge `ControlSignal::Steer` fallback — the watcher
/// will issue it from the ack arm if the native attempt fails.
///
/// Returns `false` if `pool.send_steer` failed (no in-flight task,
/// `steer_tx` already full from a prior in-flight steer, or read loop
/// torn down). The caller MUST fall through to
/// `signal_in_flight_task(channel_id, ControlSignal::Steer)` so the
/// event still reaches the agent via the universal path.
///
/// The withheld event is NOT released here on `false` because no withhold
/// was established: `mark_native_steer_pending` only runs on `Ok(())`.
fn try_native_steer(
    pool: &mut AgentPool,
    queue: &mut EventQueue,
    conversation_key: ConversationKey,
    event: nostr::Event,
    prompt_tag: String,
    steer_ack_tx: &mpsc::UnboundedSender<SteerAckEvent>,
) -> bool {
    let channel_id = conversation_key.channel_id;
    // Build the steer body: framing strings come from
    // `queue::native_steer_framing()` (Eva's drift-proof requirement —
    // native and cancel+merge fallback share these so the agent gets the
    // same orientation regardless of transport). The single event block
    // is rendered by `queue::format_event_block`, the same function
    // `queue::format_prompt` uses internally for `[Buzz event: …]`
    // sections, so the rendering also cannot drift.
    //
    // Passing `None` for `channel_info` / `profile_lookup` is intentional:
    // native steer is a *delta* into a live turn — the agent already saw
    // channel context and the actor's profile in the original prompt,
    // duplicating it here would defeat the point of non-cancelling
    // steering (which is to inject only what's new).
    let (header, closing) = queue::native_steer_framing();
    let event_id_hex = event.id.to_hex();
    let be = queue::BatchEvent {
        event,
        prompt_tag: prompt_tag.clone(),
        received_at: std::time::Instant::now(),
    };
    let event_block = queue::format_event_block(channel_id, None, &be, None);
    let body = format!("{header}\n\n[Buzz event: {prompt_tag}]\n{event_block}\n\n{closing}");

    let (ack_tx, ack_rx) = tokio::sync::oneshot::channel::<pool::SteerAck>();
    let request = pool::SteerRequest {
        prompt_blocks: vec![body],
        ack_tx,
    };

    match pool.send_conversation_steer(&conversation_key, request) {
        Ok(()) => {
            // Withhold the queued event synchronously BEFORE spawning
            // the watcher: this closes the race where `mark_complete`
            // clears `in_flight_channels` and a stray `flush_next` could
            // re-deliver the event via normal dispatch. See
            // `EventQueue::mark_native_steer_pending` docs at queue.rs:606.
            let withheld =
                queue.mark_conversation_native_steer_pending(&conversation_key, &event_id_hex);
            if !withheld {
                // Race: the event was already drained out of the queue
                // before we got here (e.g. a concurrent flush picked it
                // up). The steer is on the wire; if it succeeds the
                // agent gets it via the native path AND normal
                // dispatch — duplicate delivery is benign (agent gets
                // the same message twice). Log so this is visible if it
                // ever happens in production.
                tracing::warn!(
                    channel = %channel_id,
                    event_id = %event_id_hex,
                    "native steer accepted by read loop but event was not in queue to withhold \
                     — possible duplicate delivery if steer succeeds"
                );
            }
            let ack_tx_clone = steer_ack_tx.clone();
            let event_id_for_watcher = event_id_hex.clone();
            tokio::spawn(async move {
                let ack = ack_rx.await;
                let _ = ack_tx_clone.send(SteerAckEvent {
                    channel_id,
                    conversation_key,
                    event_id: event_id_for_watcher,
                    ack,
                });
            });
            true
        }
        Err(e) => {
            tracing::info!(
                channel = %channel_id,
                error = ?e,
                "non-cancelling steer not accepted — falling back to cancel+merge"
            );
            false
        }
    }
}

// ── dispatch_pending ──────────────────────────────────────────────────────────

/// Flush queued work to available agents.
fn dispatch_pending(
    pool: &mut AgentPool,
    queue: &mut EventQueue,
    ctx: &Arc<PromptContext>,
    last_activity: &mut tokio::time::Instant,
) -> Vec<(ConversationKey, ThreadTags)> {
    let mut dispatched_channels = Vec::new();
    let mut affinity_blocked = HashSet::new();
    loop {
        let batch = match queue.flush_next_excluding(&affinity_blocked) {
            Some(b) => b,
            None => break,
        };
        let channel_id = batch.channel_id;
        let conversation_key = batch.conversation_key.clone();
        let typing_scope = batch
            .events
            .last()
            .map(|event| queue::parse_thread_tags(&event.event))
            .unwrap_or_default();
        let affinity_hit = pool.has_session_for_conversation(&conversation_key);
        let mut agent = match pool.try_claim_conversation(&conversation_key) {
            Some(a) => a,
            None => {
                let pending = queue.pending_channels();
                tracing::debug!(pending_channels = pending, "pool_exhausted");
                queue.requeue_preserve_timestamps(batch);
                queue.mark_conversation_complete(&conversation_key);
                if pool.any_idle() {
                    affinity_blocked.insert(conversation_key);
                    continue;
                }
                break;
            }
        };
        tracing::debug!(agent = agent.index, channel = %channel_id, affinity_hit, "agent_claimed");

        let recoverable_batch = match ctx.dedup_mode {
            DedupMode::Queue => Some(batch.clone()),
            DedupMode::Drop => None,
        };

        let result_tx = pool.result_tx();
        let ctx_clone = Arc::clone(ctx);
        let agent_index = agent.index;

        // Mid-turn non-cancelling steer seam: install the per-turn steer
        // receiver on the read loop so the main loop's mode-gate fork
        // (see the `if accepted && queue.is_channel_in_flight(...)` block
        // in the relay event branch of the main `select!` loop) can drive
        // it via the matching sender stored in `TaskMeta.steer_tx`.
        // Installed for every prompt task: the read loop picks the steer
        // transport at write time from `active_run_id` and the agent's
        // advertised `_session/steering` capability, and acks
        // `ExpectedRunIdMissing` (→ cancel+merge) when it has neither.
        let (tx, rx) = tokio::sync::mpsc::channel::<pool::SteerRequest>(1);
        agent.acp.install_steer_rx(rx);
        let steer_tx = Some(tx);

        // Prompt text is now built inside run_prompt_task (needs async for
        // context fetching). Pass None for prompt_text; batch carries the data.
        let (control_tx, control_rx) = tokio::sync::oneshot::channel::<ControlSignal>();
        let turn_id = Uuid::new_v4().to_string();
        let task_turn_id = turn_id.clone();

        let abort_handle = pool.join_set.spawn(async move {
            pool::run_prompt_task(
                agent,
                Some(batch),
                None,
                ctx_clone,
                result_tx,
                Some(control_rx),
                task_turn_id,
            )
            .await;
        });

        pool.task_map_mut().insert(
            abort_handle.id(),
            pool::TaskMeta {
                agent_index,
                channel_id: Some(channel_id),
                conversation_key: Some(conversation_key.clone()),
                turn_id,
                recoverable_batch,
                control_tx: Some(control_tx),
                steer_tx,
            },
        );
        dispatched_channels.push((conversation_key, typing_scope));
        *last_activity = tokio::time::Instant::now();
    }
    tracing::debug!(
        dispatched = dispatched_channels.len(),
        queue_depth = queue.pending_channels(),
        "dispatch_pending"
    );
    dispatched_channels
}

/// Returns `true` when `error` is a non-retryable authentication failure.
///
/// Retrying auth errors is harmful: the token won't self-repair between
/// attempts, so each retry wastes an attempt slot, delays the visible failure,
/// and burns the user's context window. Dead-letter immediately and surface a
/// re-authentication hint instead.
///
/// # Classification rationale
///
/// Auth failures arrive as [`acp::AcpError::AgentError`] with a message
/// surfaced from the upstream CLI. Two narrow patterns reliably identify
/// non-transient auth failures observed in the field:
///
/// - `"Re-authenticate"` — emitted by the Claude CLI when an OAuth token has
///   expired ("OAuth access token has expired. Re-authenticate to continue.").
///   Specific to the auth-expiry flow; does not appear in unrelated errors.
/// - `"API Error: 401"` — present in Claude/Codex HTTP-401 responses; 401 is
///   the standard auth-failure status and does not arise from network blips.
///
/// False positives (misclassifying a transient error as non-retryable) silently
/// drop a user message, which is worse than a false negative (extra retries on
/// an auth error). Both patterns are therefore chosen for high precision.
fn is_auth_error(error: &acp::AcpError) -> bool {
    let acp::AcpError::AgentError { message, .. } = error else {
        return false;
    };
    message.contains("Re-authenticate") || message.contains("API Error: 401")
}

/// Stable adapter error used when Pi's final provider-payload guard cannot
/// compact enough to stay under the configured context ceiling.
const PI_CONTEXT_LIMIT_ERROR_CODE: i64 = -32042;
/// Pi session transcript/quota exhaustion. This is distinct from the context
/// window: retrying appends more rollback/control entries and cannot recover.
const PI_SESSION_STORAGE_LIMIT_ERROR_CODE: i64 = -32044;
/// Pi has deliberately invalidated the inner runtime after a safety-boundary
/// failure (for example, a project-resource reload whose old runner was
/// disposed). The outer ACP process must be replaced; reusing its session ID
/// can only repeat against the invalid registry proxy.
const PI_SESSION_INVALIDATED_ERROR_CODE: i64 = -32045;
/// Disk/outbox repair is an operator prerequisite, not a prompt failure. Keep
/// the original request crash-replayable in Buzz and retry locally at a calm
/// fixed cadence without burning the normal poison-batch budget.
const DURABLE_NOTICE_BLOCKED_RETRY_DELAY: Duration = Duration::from_secs(30);

fn is_pi_context_limit_error(error: &acp::AcpError) -> bool {
    matches!(
        error,
        acp::AcpError::AgentError {
            code: PI_CONTEXT_LIMIT_ERROR_CODE,
            ..
        }
    )
}

fn is_pi_session_storage_limit_error(error: &acp::AcpError) -> bool {
    matches!(
        error,
        acp::AcpError::AgentError {
            code: PI_SESSION_STORAGE_LIMIT_ERROR_CODE,
            ..
        }
    )
}

fn is_pi_session_invalidated_error(error: &acp::AcpError) -> bool {
    matches!(
        error,
        acp::AcpError::AgentError {
            code: PI_SESSION_INVALIDATED_ERROR_CODE,
            ..
        }
    )
}

fn pi_session_storage_limit_notice() -> &'static str {
    "⚠️ This thread's durable Pi session reached its local storage limit, so I stopped without retrying or changing its context. Use `/new` in this thread to start a fresh session, then resend the request."
}

fn persist_pi_session_storage_limit_notice(
    outbox: Option<&LifecycleNoticeOutbox>,
    rest: Option<&relay::RestClient>,
    batch: &FlushBatch,
) -> bool {
    let (Some(outbox), Some(rest)) = (outbox, rest) else {
        return false;
    };
    let mut thread_tags = batch
        .events
        .last()
        .map(|event| queue::parse_thread_tags(&event.event))
        .unwrap_or_default();
    if thread_tags.root_event_id.is_none()
        && !batch.conversation_key.root_event_id.starts_with("dm:")
        && nostr::EventId::from_hex(&batch.conversation_key.root_event_id).is_ok()
    {
        thread_tags.root_event_id = Some(batch.conversation_key.root_event_id.clone());
        thread_tags.parent_event_id = Some(batch.conversation_key.root_event_id.clone());
    }
    let source_event = batch
        .events
        .last()
        .map(|event| event.event.id.to_hex())
        .unwrap_or_else(|| batch.conversation_key.stable_id());
    outbox
        .enqueue(
            batch.channel_id,
            &thread_tags,
            vec![LifecycleNotice {
                dedupe_key: Some(format!("pi-storage-limit:{source_event}")),
                content: pi_session_storage_limit_notice().to_owned(),
                ack: None,
            }],
            rest,
        )
        .persisted
        == 1
}

/// Spawn a task that posts a user-visible failure notice to the relay.
///
/// Shared by the hard-cap immediate dead-letter path and the retries-exhausted
/// dead-letter path so neither duplicates the tokio::spawn block.
fn spawn_failure_notice(
    rest_client: Option<&relay::RestClient>,
    batch: &FlushBatch,
    content: String,
) {
    if let Some(rest) = rest_client {
        let thread_tags = batch
            .events
            .last()
            .map(|be| queue::parse_thread_tags(&be.event))
            .unwrap_or_default();
        let rest = rest.clone();
        let channel_id = batch.channel_id;
        tokio::spawn(async move {
            pool::post_failure_notice(&rest, channel_id, &thread_tags, &content).await;
        });
    }
}

fn format_token_count(value: u64) -> String {
    let digits = value.to_string();
    let mut formatted = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            formatted.push(',');
        }
        formatted.push(digit);
    }
    formatted
}

fn context_limit_summary(limit: u64, effective: u64, threshold: u64) -> String {
    if limit == effective {
        format!(
            "Configured cap {} tokens; automatic compaction threshold {}.",
            format_token_count(limit),
            format_token_count(threshold)
        )
    } else {
        format!(
            "Configured cap {} tokens; this model's effective limit is {}; automatic compaction threshold {}.",
            format_token_count(limit),
            format_token_count(effective),
            format_token_count(threshold)
        )
    }
}

fn safe_notice_model(model: &str) -> Option<String> {
    let model = model.trim();
    (!model.is_empty()
        && model.len() <= 128
        && model
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "-._/:+".contains(character)))
    .then(|| model.to_string())
}

fn should_publish_buzz_session_notice(event: &BuzzSessionEvent) -> bool {
    match event {
        // Pi retries this internally. Publishing the transient failure before
        // the eventual completion would make one successful compaction look
        // like an error to the user.
        BuzzSessionEvent::CompactionFailed {
            will_retry: true, ..
        } => false,
        // The harness acknowledges `/new` only after its reset barrier commits.
        // Repeating Pi's confirmation on the next prompt is noisy and can be
        // mistaken for a second reset.
        BuzzSessionEvent::SessionReset { .. } => false,
        _ => true,
    }
}

/// Render a lifecycle notice exclusively from bounded typed fields. Adapter
/// `message`, `error`, paths, and resource diagnostics are intentionally never
/// copied into Buzz content.
fn render_buzz_session_notice(event: &BuzzSessionEvent) -> String {
    match event {
        BuzzSessionEvent::CompactionCompleted {
            reason,
            before_tokens,
            after_tokens,
            limit_tokens,
            effective_limit_tokens,
            compaction_threshold_tokens,
            ..
        } => {
            let mode = if matches!(reason, acp::CompactionReason::Manual) {
                "on request"
            } else {
                "automatically"
            };
            let change = match (before_tokens, after_tokens) {
                (Some(before), Some(after)) => format!(
                    ": {} → ~{} tokens",
                    format_token_count(*before),
                    format_token_count(*after)
                ),
                (Some(before), None) => {
                    format!(" from about {} tokens", format_token_count(*before))
                }
                _ => String::new(),
            };
            format!(
                "🧠 Pi compacted this thread's context {mode}{change}. It will continue from a summary. {}",
                context_limit_summary(
                    *limit_tokens,
                    *effective_limit_tokens,
                    *compaction_threshold_tokens,
                )
            )
        }
        BuzzSessionEvent::CompactionFailed {
            limit_tokens,
            effective_limit_tokens,
            compaction_threshold_tokens,
            aborted,
            ..
        } => {
            let failure = if *aborted {
                "Context compaction was cancelled"
            } else {
                "Context compaction failed"
            };
            format!(
                "⚠️ {failure}, so this request was stopped before Pi could exceed its context limit. Your thread is preserved; try again or use `/new` for a fresh session. {}",
                context_limit_summary(
                    *limit_tokens,
                    *effective_limit_tokens,
                    *compaction_threshold_tokens,
                )
            )
        }
        BuzzSessionEvent::ContextStatus {
            used_tokens,
            limit_tokens,
            effective_limit_tokens,
            compaction_threshold_tokens,
            auto_compaction,
            compacting,
            model,
            ..
        } => {
            let usage = match used_tokens {
                Some(used) => {
                    let percent = (*used as f64 / *effective_limit_tokens as f64) * 100.0;
                    format!(
                        "{} / {} tokens ({percent:.1}%)",
                        format_token_count(*used),
                        format_token_count(*effective_limit_tokens)
                    )
                }
                None => format!(
                    "recalculating after compaction (effective limit {})",
                    format_token_count(*effective_limit_tokens)
                ),
            };
            let state = if *compacting {
                " Compaction is running now."
            } else if *auto_compaction {
                " Auto-compaction is enabled."
            } else {
                " Auto-compaction is disabled."
            };
            let model = model
                .as_deref()
                .and_then(safe_notice_model)
                .map(|model| format!(" Model: `{model}`."))
                .unwrap_or_default();
            format!(
                "🧭 Pi context: {usage}. Automatic compaction threshold {}.{}{model} Configured cap {} tokens.",
                format_token_count(*compaction_threshold_tokens),
                state,
                format_token_count(*limit_tokens),
            )
        }
        BuzzSessionEvent::SessionReset {
            limit_tokens,
            effective_limit_tokens,
            compaction_threshold_tokens,
            ..
        } => format!(
            "✅ Pi confirmed a fresh persistent session for this Buzz thread. Its previous conversation context will not be reused. {}",
            context_limit_summary(
                *limit_tokens,
                *effective_limit_tokens,
                *compaction_threshold_tokens,
            )
        ),
        BuzzSessionEvent::ExtensionsReloaded {
            extensions,
            skills,
            prompts,
            context_files,
            errors,
            project_trusted,
            ..
        } => {
            let trust = if *project_trusted {
                "Trusted project-local Pi resources are enabled."
            } else {
                "Untrusted project-local Pi resources stayed disabled."
            };
            let diagnostics = if errors.is_empty() {
                String::new()
            } else {
                format!(
                    " {} resource diagnostic(s) were reported; see the agent logs for safe details.",
                    errors.len()
                )
            };
            format!(
                "🧩 Reloaded Pi resources: {extensions} extension(s), {skills} skill(s), {prompts} prompt template(s), and {context_files} context file(s). {trust}{diagnostics}"
            )
        }
    }
}

fn lifecycle_notice_scope(result: &PromptResult) -> Option<(Uuid, ThreadTags)> {
    let PromptSource::Conversation(key) = &result.source else {
        return None;
    };
    let mut tags = result
        .batch
        .as_ref()
        .and_then(|batch| batch.events.last())
        .map(|event| queue::parse_thread_tags(&event.event))
        .unwrap_or_default();
    if tags.root_event_id.is_none()
        && !key.root_event_id.starts_with("dm:")
        && nostr::EventId::from_hex(&key.root_event_id).is_ok()
    {
        tags.root_event_id = Some(key.root_event_id.clone());
        tags.parent_event_id = Some(key.root_event_id.clone());
    }
    Some((key.channel_id, tags))
}

/// A rendered lifecycle transition plus its adapter-stable identity. Pi emits
/// a UUID for every compaction attempt; retaining it here lets the delivery
/// outbox collapse retransmissions without trusting adapter-provided prose.
#[derive(Debug)]
struct LifecycleNotice {
    dedupe_key: Option<String>,
    content: String,
    ack: Option<acp::BuzzSessionEventAck>,
}

#[derive(Default)]
struct LifecycleEnqueueOutcome {
    acknowledgements: Vec<acp::BuzzSessionEventAck>,
    persisted: usize,
    failed: usize,
}

fn lifecycle_persistence_boundary_error(failed: usize) -> Option<acp::AcpError> {
    (failed > 0).then(|| {
        acp::AcpError::Protocol(format!(
            "failed to durably persist {failed} adapter lifecycle notice(s)"
        ))
    })
}

type LifecycleNoticeEnvelope = DurableLifecycleEnvelope;

#[derive(Debug, Default)]
struct LifecycleNoticeDedupe {
    pending: HashSet<String>,
    delivered: HashSet<String>,
    delivered_order: VecDeque<String>,
}

impl LifecycleNoticeDedupe {
    fn reserve(&mut self, key: Option<&str>) -> bool {
        let Some(key) = key else {
            return true;
        };
        if self.pending.contains(key) || self.delivered.contains(key) {
            return false;
        }
        self.pending.insert(key.to_owned());
        true
    }

    fn finish(&mut self, key: Option<&str>, delivered: bool) {
        let Some(key) = key else {
            return;
        };
        self.pending.remove(key);
        if !delivered || !self.delivered.insert(key.to_owned()) {
            return;
        }
        self.delivered_order.push_back(key.to_owned());
        while self.delivered_order.len() > LIFECYCLE_NOTICE_DEDUPE_CAPACITY {
            if let Some(expired) = self.delivered_order.pop_front() {
                self.delivered.remove(&expired);
            }
        }
    }
}

const LIFECYCLE_NOTICE_DEDUPE_CAPACITY: usize = 1_024;
const LIFECYCLE_NOTICE_SHUTDOWN_GRACE: Duration = Duration::from_secs(60);
/// Bound relay pressure while allowing unrelated threads to make progress
/// around a permanently rejected lifecycle notice.
const LIFECYCLE_NOTICE_MAX_IN_FLIGHT_ATTEMPTS: usize = 8;
const LIFECYCLE_NOTICE_RETRY_DELAYS: [Duration; 6] = [
    Duration::ZERO,
    Duration::from_millis(250),
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(5),
    Duration::from_secs(10),
];

/// Bounded, tracked delivery for user-visible Pi lifecycle messages.
///
/// Events are signed and persisted before entering the worker queue, so every
/// retry has the same Nostr event ID. Pending events and delivered compaction
/// identities survive a hard process death. The worker retries until the relay
/// accepts the event *and* that completion is durably checkpointed.
struct LifecycleNoticeOutbox {
    tx: Option<mpsc::UnboundedSender<LifecycleNoticeEnvelope>>,
    worker: Option<tokio::task::JoinHandle<()>>,
    dedupe: Arc<Mutex<LifecycleNoticeDedupe>>,
    durable: DurableOutbox,
}

impl LifecycleNoticeOutbox {
    fn spawn(rest: relay::RestClient, durable: DurableOutbox) -> Result<Self> {
        let (tx, mut rx) = mpsc::unbounded_channel::<LifecycleNoticeEnvelope>();
        let restored = durable.pending_lifecycle()?;
        let mut initial_dedupe = LifecycleNoticeDedupe::default();
        for envelope in &restored {
            if let Some(key) = envelope.dedupe_key.as_ref() {
                initial_dedupe.pending.insert(key.clone());
            }
        }
        for key in durable.delivered_keys()? {
            initial_dedupe.delivered.insert(key.clone());
            initial_dedupe.delivered_order.push_back(key);
        }
        let dedupe = Arc::new(Mutex::new(initial_dedupe));
        let worker_dedupe = Arc::clone(&dedupe);
        let worker_durable = durable.clone();
        let worker = tokio::spawn(async move {
            let attempt_slots = Arc::new(Semaphore::new(LIFECYCLE_NOTICE_MAX_IN_FLIGHT_ATTEMPTS));
            let mut deliveries = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    envelope = rx.recv() => {
                        let Some(envelope) = envelope else {
                            break;
                        };
                        let delivery_rest = rest.clone();
                        let delivery_durable = worker_durable.clone();
                        let delivery_dedupe = Arc::clone(&worker_dedupe);
                        let delivery_slots = Arc::clone(&attempt_slots);
                        deliveries.spawn(async move {
                            let delivered = submit_lifecycle_notice_until_persisted(
                                &delivery_rest,
                                &delivery_durable,
                                &envelope,
                                &delivery_slots,
                            )
                            .await;
                            if let Ok(mut state) = delivery_dedupe.lock() {
                                state.finish(envelope.dedupe_key.as_deref(), delivered);
                            } else {
                                tracing::error!(
                                    event_id = %envelope.event.id,
                                    "lifecycle notice dedupe state was poisoned"
                                );
                            }
                        });
                    }
                    completed = deliveries.join_next(), if !deliveries.is_empty() => {
                        if let Some(Err(error)) = completed {
                            tracing::error!(%error, "lifecycle notice delivery task failed");
                        }
                    }
                }
            }

            // Normal shutdown keeps its existing bounded grace period: accepted
            // deliveries drain, while a permanently rejected envelope is
            // aborted with this worker and remains in the durable snapshot for
            // exact-event replay on the next start.
            while let Some(completed) = deliveries.join_next().await {
                if let Err(error) = completed {
                    tracing::error!(%error, "lifecycle notice delivery task failed while draining");
                }
            }
        });
        for envelope in restored {
            tx.send(envelope).map_err(|error| {
                anyhow::anyhow!("could not restore durable lifecycle notice: {error}")
            })?;
        }
        Ok(Self {
            tx: Some(tx),
            worker: Some(worker),
            dedupe,
            durable,
        })
    }

    fn enqueue(
        &self,
        channel_id: Uuid,
        thread_tags: &ThreadTags,
        notices: Vec<LifecycleNotice>,
        rest: &relay::RestClient,
    ) -> LifecycleEnqueueOutcome {
        let mut outcome = LifecycleEnqueueOutcome::default();
        let Some(tx) = self.tx.as_ref() else {
            tracing::warn!("lifecycle notice outbox is already closed");
            outcome.failed = notices.len();
            return outcome;
        };
        for notice in notices {
            let event = match pool::build_notice_event(
                rest,
                channel_id,
                thread_tags,
                &notice.content,
            ) {
                Ok(event) => event,
                Err(error) => {
                    tracing::error!(channel = %channel_id, %error, "could not construct lifecycle notice");
                    outcome.failed += 1;
                    continue;
                }
            };
            let dedupe_key = notice.dedupe_key.map(|key| {
                let thread_scope = thread_tags
                    .root_event_id
                    .as_deref()
                    .unwrap_or("channel-root");
                format!("{channel_id}:{thread_scope}:{key}")
            });
            let reserved = match self.dedupe.lock() {
                Ok(mut state) => Some(state.reserve(dedupe_key.as_deref())),
                Err(_) => {
                    tracing::error!(
                        event_id = %event.id,
                        "lifecycle notice dedupe state was poisoned"
                    );
                    None
                }
            };
            let Some(reserved) = reserved else {
                // A poisoned in-memory dedupe index is not evidence of an
                // already-persisted duplicate. Do not ACK the adapter or claim
                // the user-visible notice is safe; its durable record remains
                // available for retransmission after harness restart.
                outcome.failed += 1;
                continue;
            };
            if !reserved {
                tracing::debug!(
                    event_id = %event.id,
                    lifecycle_key = dedupe_key.as_deref(),
                    "suppressing duplicate lifecycle notice"
                );
                outcome.persisted += 1;
                outcome.acknowledgements.extend(notice.ack);
                continue;
            }

            let envelope = LifecycleNoticeEnvelope {
                dedupe_key: dedupe_key.clone(),
                event,
            };
            match self.durable.enqueue_lifecycle(envelope.clone()) {
                Ok(true) => {}
                Ok(false) => {
                    if let Ok(mut state) = self.dedupe.lock() {
                        state.finish(dedupe_key.as_deref(), true);
                    }
                    outcome.persisted += 1;
                    outcome.acknowledgements.extend(notice.ack);
                    continue;
                }
                Err(error) => {
                    if let Ok(mut state) = self.dedupe.lock() {
                        state.finish(dedupe_key.as_deref(), false);
                    }
                    tracing::error!(
                        channel = %channel_id,
                        lifecycle_key = dedupe_key.as_deref(),
                        %error,
                        "could not durably enqueue lifecycle notice"
                    );
                    outcome.failed += 1;
                    continue;
                }
            }
            if let Err(error) = tx.send(envelope) {
                tracing::error!(
                    channel = %channel_id,
                    lifecycle_key = dedupe_key.as_deref(),
                    %error,
                    "lifecycle notice outbox worker is closed"
                );
            }
            // Durability, not worker queue admission, is the ACK boundary. A
            // closed worker leaves this exact signed event for startup replay.
            outcome.persisted += 1;
            outcome.acknowledgements.extend(notice.ack);
        }
        outcome
    }

    async fn shutdown(&mut self) {
        self.tx.take();
        let Some(mut worker) = self.worker.take() else {
            return;
        };
        match tokio::time::timeout(LIFECYCLE_NOTICE_SHUTDOWN_GRACE, &mut worker).await {
            Ok(Ok(())) => tracing::debug!("lifecycle notice outbox drained"),
            Ok(Err(error)) => tracing::warn!(%error, "lifecycle notice outbox worker failed"),
            Err(_) => {
                tracing::warn!("lifecycle notice outbox did not drain within shutdown grace");
                worker.abort();
                let _ = worker.await;
            }
        }
    }
}

async fn submit_lifecycle_notice_until_persisted(
    rest: &relay::RestClient,
    durable: &DurableOutbox,
    envelope: &LifecycleNoticeEnvelope,
    attempt_slots: &Semaphore,
) -> bool {
    let event = &envelope.event;
    let mut attempt = 0usize;
    loop {
        let delay = LIFECYCLE_NOTICE_RETRY_DELAYS
            .get(attempt)
            .copied()
            .unwrap_or(Duration::from_secs(30));
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }
        let attempt_permit = match attempt_slots.acquire().await {
            Ok(permit) => permit,
            Err(_) => return false,
        };
        let accepted = pool::submit_notice_event(rest, event).await;
        drop(attempt_permit);
        if accepted {
            match durable.mark_lifecycle_delivered(&event.id.to_hex()) {
                Ok(true) => return true,
                Ok(false) => {
                    // Absence is success only when the same stable key is
                    // durably recorded as delivered. Otherwise we cannot
                    // distinguish a logic/corruption bug from completion and
                    // must keep retrying the exact accepted event ID.
                    let durably_delivered = envelope.dedupe_key.as_ref().is_some_and(|key| {
                        durable
                            .delivered_keys()
                            .is_ok_and(|keys| keys.iter().any(|item| item == key))
                    });
                    if durably_delivered {
                        return true;
                    }
                    tracing::error!(
                        event_id = %event.id,
                        "accepted lifecycle notice was absent without a durable delivered receipt; retrying"
                    );
                }
                Err(error) => {
                    tracing::error!(
                        event_id = %event.id,
                        %error,
                        "relay accepted lifecycle notice but durable completion failed; retrying the same event ID"
                    );
                }
            }
        }
        attempt = attempt.saturating_add(1);
    }
}

/// Drain lifecycle events at the terminal boundary of one exact turn. The ACP
/// session envelope and conversation generation must both still match.
fn take_current_buzz_session_notices(
    result: &mut PromptResult,
    queue: &EventQueue,
) -> Vec<LifecycleNotice> {
    let expected_session_id = result
        .agent
        .acp
        .current_turn_session_id()
        .map(ToOwned::to_owned);
    let expected_pi_session_id = expected_session_id
        .as_deref()
        .and_then(|session_id| result.agent.acp.pi_session_id_for(session_id))
        .map(ToOwned::to_owned);
    let events = result.agent.acp.take_buzz_session_events();
    if events.is_empty() {
        return Vec::new();
    }
    let (is_current_generation, expected_conversation_id) = match &result.source {
        PromptSource::Conversation(key) => {
            (queue.is_current_conversation(key), Some(key.stable_id()))
        }
        PromptSource::Channel(_) | PromptSource::Heartbeat => (false, None),
    };
    let Some(expected_session_id) = expected_session_id.filter(|_| is_current_generation) else {
        tracing::debug!(
            count = events.len(),
            "discarding lifecycle events without a current conversation session"
        );
        return Vec::new();
    };

    let Some(expected_conversation_id) = expected_conversation_id else {
        return Vec::new();
    };
    let mut notices = Vec::new();
    for notification in events {
        if notification.session_id != expected_session_id {
            tracing::debug!(
                event_kind = notification.event.kind(),
                "discarding lifecycle event from a stale ACP session"
            );
            continue;
        }
        if notification.conversation_id != expected_conversation_id {
            tracing::warn!(
                event_id = %notification.event_id,
                "discarding lifecycle event whose durable conversation identity does not match the active turn"
            );
            continue;
        }
        let ack = acp::BuzzSessionEventAck {
            conversation_id: notification.conversation_id,
            event_id: notification.event_id,
        };
        if expected_pi_session_id.as_deref() != Some(notification.event.pi_session_id()) {
            tracing::warn!(
                event_id = %ack.event_id,
                "suppressing stale lifecycle event from a superseded Pi session"
            );
            // The outer ACP session and durable conversation are current, so
            // the mismatched inner Pi identity is definitively stale. ACKing
            // deletes it instead of replaying it forever into this generation.
            result.agent.acp.queue_buzz_session_event_ack(ack);
            continue;
        }
        if !should_publish_buzz_session_notice(&notification.event) {
            tracing::debug!(
                event_kind = notification.event.kind(),
                "suppressing non-terminal compaction failure while Pi retries"
            );
            result.agent.acp.queue_buzz_session_event_ack(ack);
            continue;
        }
        notices.push(LifecycleNotice {
            dedupe_key: Some(format!("adapter-event:{}", ack.event_id)),
            content: render_buzz_session_notice(&notification.event),
            ack: Some(ack),
        });
    }
    notices
}

fn enqueue_session_lifecycle_notices(
    outbox: Option<&LifecycleNoticeOutbox>,
    rest_client: Option<&relay::RestClient>,
    scope: Option<(Uuid, ThreadTags)>,
    notices: Vec<LifecycleNotice>,
) -> LifecycleEnqueueOutcome {
    if notices.is_empty() {
        return LifecycleEnqueueOutcome::default();
    }
    let notice_count = notices.len();
    let (Some(outbox), Some(rest), Some((channel_id, thread_tags))) = (outbox, rest_client, scope)
    else {
        tracing::error!(
            notice_count,
            "durable lifecycle notices had no delivery scope or outbox"
        );
        return LifecycleEnqueueOutcome {
            failed: notice_count,
            ..LifecycleEnqueueOutcome::default()
        };
    };
    outbox.enqueue(channel_id, &thread_tags, notices, rest)
}

#[allow(clippy::too_many_arguments)]
fn handle_prompt_result(
    pool: &mut AgentPool,
    queue: &mut EventQueue,
    config: &Config,
    mut result: PromptResult,
    heartbeat_in_flight: &mut bool,
    removed_channels: &HashSet<Uuid>,
    crash_history: &mut [SlotCircuit],
    respawn_tx: &mpsc::Sender<RespawnResult>,
    respawn_tasks: &mut tokio::task::JoinSet<()>,
    observer: Option<observer::ObserverHandle>,
    lifecycle_notice_outbox: Option<&LifecycleNoticeOutbox>,
    rest_client: Option<&relay::RestClient>,
) -> LoopAction {
    let before = pool.task_map().len();
    let agent_index = result.agent.index;
    pool.task_map_mut()
        .retain(|_, meta| meta.agent_index != agent_index);
    debug_assert_eq!(before, pool.task_map().len() + 1);

    // Drain before any outcome arm moves or respawns the agent. The scope is
    // captured while the original batch is still present, including successful
    // turns whose batch is consumed below.
    let lifecycle_scope = lifecycle_notice_scope(&result);
    let lifecycle_notices = take_current_buzz_session_notices(&mut result, queue);
    let lifecycle_outcome = enqueue_session_lifecycle_notices(
        lifecycle_notice_outbox,
        rest_client,
        lifecycle_scope,
        lifecycle_notices,
    );
    for acknowledgement in lifecycle_outcome.acknowledgements {
        result
            .agent
            .acp
            .queue_buzz_session_event_ack(acknowledgement);
    }
    if let Some(error) = lifecycle_persistence_boundary_error(lifecycle_outcome.failed) {
        // The Pi adapter retains every durable lifecycle record until it gets
        // an explicit ACK. Continuing to reuse this ACP process after draining
        // a record that Buzz could not persist would suppress the adapter's
        // replay behind this client's in-memory seen-ID set. Treat the boundary
        // as a transport failure: the child is replaced and replays the exact
        // event ID from its own durable outbox on startup.
        result.outcome = PromptOutcome::Error(error);
    }

    // The hard-timeout death_message (below) must describe the batch's
    // *actual* fate, not just the `recently_active` eligibility flag — a
    // recently-active batch that exhausts the retry budget in queue.requeue()
    // is dead-lettered same as an immediate one, and both differ from a
    // channel-removed drop or a heartbeat call with no batch at all. Each
    // branch below records what actually happened; only the hard-timeout
    // match arm in the death_message construction reads it.
    let mut hard_timeout_fate_suffix: Option<&'static str> = None;

    // Requeue BEFORE mark_complete: requeue() sets retry_after with a future
    // deadline, and mark_complete() checks for it to decide whether to preserve
    // retry_counts. If mark_complete runs first, retry_counts is cleared and
    // every retry starts at attempt 1 — defeating exponential backoff and
    // dead-letter protection.
    if let Some(batch) = result.batch.take() {
        if !queue.is_current_conversation(&batch.conversation_key) {
            tracing::info!(
                conversation = %batch.conversation_key,
                events = batch.events.len(),
                "discarding late batch from superseded /new generation"
            );
        // Don't requeue batches for channels the agent was removed from —
        // those events are stale and should be silently dropped.
        } else if !removed_channels.contains(&batch.channel_id) {
            if matches!(
                result.outcome,
                PromptOutcome::Cancelled | PromptOutcome::CancelDrainTimeout(_)
            ) {
                // Cancel re-prompt: store as cancelled events so flush_next()
                // merges them into the next FlushBatch.cancelled_events,
                // enabling the annotated merged-prompt format. The batch's
                // cancel_reason (set by the pool task per the control signal)
                // selects steer vs interrupt framing. It is always set on this
                // path; if somehow unset, fall back to the gentler Steer framing
                // — consistent with MergeFraming::for_reason(None) and the
                // system default — rather than telling the agent to supersede.
                //
                // CancelDrainTimeout shares this path with Cancelled: a failed
                // 5s drain after a control-signal cancel is a cleanup-deadline
                // problem, not the deterministic hard-cap death below — the
                // original batch must survive with no retry/dead-letter
                // accounting, same as a clean cancel.
                let reason = batch.cancel_reason.unwrap_or(CancelReason::Steer);
                queue.requeue_as_cancelled(batch, reason);
            } else if matches!(
                result.outcome,
                PromptOutcome::Timeout(TimeoutKind::Hard {
                    recently_active: false
                })
            ) {
                tracing::error!(
                    channel_id = %batch.channel_id,
                    events = batch.events.len(),
                    "dead-lettering batch after hard-cap timeout (no recent activity) — discarding {} events",
                    batch.events.len(),
                );
                let content = format!(
                    "⚠️ I couldn't process the last request (the turn exceeded the maximum duration ({}s)). Please re-send if it's still needed.",
                    config.max_turn_duration_secs
                );
                spawn_failure_notice(rest_client, &batch, content);
                hard_timeout_fate_suffix = Some(" — dead-lettered (no recent activity)");
            } else if matches!(
                result.outcome,
                PromptOutcome::Timeout(TimeoutKind::Hard {
                    recently_active: true
                })
            ) {
                tracing::warn!(
                    channel_id = %batch.channel_id,
                    events = batch.events.len(),
                    "hard-cap timeout with recent activity — requeueing for retry"
                );
                if let Some(dead) = queue.requeue(batch) {
                    let content = format!(
                        "⚠️ I couldn't process the last request after multiple retries (the turn exceeded the maximum duration ({}s)). Please re-send if it's still needed.",
                        config.max_turn_duration_secs
                    );
                    spawn_failure_notice(rest_client, &dead, content);
                    hard_timeout_fate_suffix = Some(" — dead-lettered (retry budget exhausted)");
                } else {
                    hard_timeout_fate_suffix = Some(" — requeued for retry (recently active)");
                }
            } else if matches!(&result.outcome, PromptOutcome::Error(e) if is_pi_context_limit_error(e))
            {
                // Pi already emitted one typed, safely-rendered lifecycle
                // notice for this failure. Retrying would append duplicate
                // error turns to its durable context and publish duplicate
                // notices, while the same oversized provider payload remains
                // deterministic. Preserve the session and dead-letter once.
                tracing::warn!(
                    channel_id = %batch.channel_id,
                    events = batch.events.len(),
                    "dead-lettering batch immediately — Pi context ceiling could not be satisfied"
                );
            } else if matches!(&result.outcome, PromptOutcome::Error(e) if is_pi_session_storage_limit_error(e))
            {
                tracing::warn!(
                    channel_id = %batch.channel_id,
                    events = batch.events.len(),
                    "dead-lettering batch immediately — Pi durable session storage is exhausted"
                );
                if !persist_pi_session_storage_limit_notice(
                    lifecycle_notice_outbox,
                    rest_client,
                    &batch,
                ) {
                    tracing::error!(
                        channel_id = %batch.channel_id,
                        "could not persist the storage-limit recovery notice; retaining the user batch"
                    );
                    queue.requeue_blocked(batch, DURABLE_NOTICE_BLOCKED_RETRY_DELAY);
                }
            } else if matches!(&result.outcome, PromptOutcome::Error(e) if is_auth_error(e)) {
                // Auth errors are non-retryable: the token won't self-repair
                // between retries, so requeueing only wastes attempt slots and
                // delays the visible failure. Dead-letter immediately and tell
                // the user to re-authenticate the CLI.
                tracing::warn!(
                    channel_id = %batch.channel_id,
                    events = batch.events.len(),
                    "dead-lettering batch immediately — non-retryable auth error"
                );
                let content = "⚠️ I couldn't process the last request: authentication failed. \
                    Please re-authenticate the CLI (e.g. run `claude /login` or `codex login`) \
                    and then re-send."
                    .to_string();
                spawn_failure_notice(rest_client, &batch, content);
            } else if let Some(dead) = queue.requeue(batch) {
                let reason = match &result.outcome {
                    PromptOutcome::Timeout(TimeoutKind::Idle) => "the turn timed out".to_string(),
                    PromptOutcome::Timeout(TimeoutKind::Hard { .. }) => {
                        "the turn exceeded the maximum duration".to_string()
                    }
                    PromptOutcome::AgentExited => "the agent process exited".to_string(),
                    PromptOutcome::Error(e) => format!("{e}"),
                    _ => "repeated failures".to_string(),
                };
                let content = format!(
                    "⚠️ I couldn't process the last request after multiple retries ({reason}). Please re-send if it's still needed."
                );
                spawn_failure_notice(rest_client, &dead, content);
            }
        } else {
            tracing::debug!(
                channel_id = %batch.channel_id,
                events = batch.events.len(),
                "dropping failed batch for removed channel"
            );
            hard_timeout_fate_suffix = Some(" — batch dropped (channel removed)");
        }
    }

    match &result.source {
        PromptSource::Conversation(key) => queue.mark_conversation_complete(key),
        PromptSource::Channel(ch) => queue.mark_complete(*ch),
        PromptSource::Heartbeat => *heartbeat_in_flight = false,
    }

    // Strip sessions for channels the agent was removed from while this
    // agent was checked out. This covers the gap where invalidate_channel_sessions
    // only touches idle agents.
    for ch in removed_channels {
        result.agent.state.invalidate_channel(ch);
    }

    let outcome_label = match &result.outcome {
        PromptOutcome::Ok(_) => "ok",
        PromptOutcome::Error(_) => "error",
        PromptOutcome::Timeout(TimeoutKind::Idle) => "idle_timeout",
        PromptOutcome::Timeout(TimeoutKind::Hard { .. }) => "hard_timeout",
        PromptOutcome::AgentExited => "exited",
        PromptOutcome::Cancelled => "cancelled",
        PromptOutcome::CancelDrainTimeout(_) => "cancel_drain_timeout",
    };
    let agent_index = result.agent.index;
    // Capture the spawn-time configured model and our PID before the agent is
    // moved into match arms below. `desired_model` reflects the config/persona
    // model at spawn time — it does NOT reflect `session/set_model` overrides,
    // which live in buzz-agent's session state and are what `llm: (model) …`
    // errors carry. The two can legitimately differ; `configured_model=` is
    // still valuable for identifying a stale orphan running an old model.
    let harness_configured_model = result
        .agent
        .desired_model
        .as_deref()
        .unwrap_or("<none>")
        .to_string();
    let harness_pid = std::process::id();

    let channel_id = match &result.source {
        PromptSource::Conversation(key) => Some(key.channel_id),
        PromptSource::Channel(ch) => Some(*ch),
        PromptSource::Heartbeat => None,
    };
    let turn_id = result.turn_id.clone();
    let emit_turn_error = |error_msg: &str, error_code: Option<i64>| {
        if let Some(ref observer) = observer {
            let mut payload = serde_json::json!({
                "outcome": outcome_label,
                "error": error_msg,
            });
            if let Some(code) = error_code {
                payload["code"] = serde_json::json!(code);
            }
            observer.emit(
                "turn_error",
                Some(agent_index),
                &observer::context_for(channel_id, None, Some(turn_id.clone())),
                payload,
            );
        }
    };

    match result.outcome {
        // Successful prompt — return agent to pool.
        PromptOutcome::Ok(_) => {
            tracing::debug!(
                agent = agent_index,
                outcome = outcome_label,
                "agent_returned"
            );
            pool.return_agent(result.agent);
        }
        // Fatal outcomes: the agent subprocess is dead or poisoned — respawn it.
        PromptOutcome::AgentExited | PromptOutcome::Timeout(_) => {
            tracing::warn!(
                agent = agent_index,
                outcome = outcome_label,
                configured_model = %harness_configured_model,
                pid = harness_pid,
                "agent_returned — respawning"
            );
            let death_message: String = match outcome_label {
                "exited" => "Agent process exited unexpectedly".to_string(),
                "hard_timeout" => {
                    // Neutral wording when no fate was recorded above: a
                    // heartbeat hard timeout carries no batch at all, so
                    // nothing was requeued or dead-lettered.
                    let suffix = hard_timeout_fate_suffix.unwrap_or(" (no batch to retry)");
                    format!(
                        "Agent turn exceeded the maximum duration ({}s){}",
                        config.max_turn_duration_secs, suffix
                    )
                }
                _ => "Agent session timed out due to inactivity".to_string(),
            };
            emit_turn_error(&death_message, None);

            let index = result.agent.index;
            let slot_history = &mut crash_history[index];
            if !spawn_respawn_task(
                result.agent,
                config,
                slot_history,
                respawn_tx,
                respawn_tasks,
                observer.clone(),
            ) {
                // Circuit open — slot stays empty until maintenance refill.
                if pool.live_count() == 0 && !any_respawn_in_flight(crash_history) {
                    tracing::error!("all agents dead — exiting");
                    return LoopAction::Exit;
                }
            }
        }
        // Cancel-drain expiry: a control-signal cancel (steer fallback,
        // interrupt, or explicit stop) did not drain within its bounded
        // grace window. The process is poisoned/uncertain like a hard
        // timeout — respawn it — but this is NOT the configured max-turn
        // cap, so the message must name the actual grace, not
        // `max_turn_duration_secs`. The triggering batch's fate (preserved
        // for Steer/Interrupt, dropped for explicit Cancel/Rotate or a
        // removed channel) is decided above — the message stays fate-neutral
        // since it must be true in every case.
        PromptOutcome::CancelDrainTimeout(grace) => {
            tracing::warn!(
                agent = agent_index,
                outcome = outcome_label,
                configured_model = %harness_configured_model,
                pid = harness_pid,
                grace = ?grace,
                "agent_returned — respawning (cancel-drain timeout)"
            );
            let death_message = format!(
                "Agent did not stop within {grace:?} after cancellation; the agent process is being replaced."
            );
            emit_turn_error(&death_message, None);

            let index = result.agent.index;
            let slot_history = &mut crash_history[index];
            if !spawn_respawn_task(
                result.agent,
                config,
                slot_history,
                respawn_tx,
                respawn_tasks,
                observer.clone(),
            ) {
                // Circuit open — slot stays empty until maintenance refill.
                if pool.live_count() == 0 && !any_respawn_in_flight(crash_history) {
                    tracing::error!("all agents dead — exiting");
                    return LoopAction::Exit;
                }
            }
        }
        // Errors fall into two categories:
        //
        // 1. Transport-class (Io, WriteTimeout, Timeout, Protocol): the stdio
        //    pipe may be corrupted or the agent desynchronized. These are fatal
        //    to the agent regardless of whether they occurred during session
        //    creation or an active prompt — respawn unconditionally.
        //
        // 2. Application-class (IdleTimeout, HardTimeout, Json): the pipe is
        //    intact but the prompt failed. Return the agent to the pool so it
        //    can be reused for the next event.

        // Intentional cancel — agent is healthy, return it to the pool.
        // No respawn, no retry penalty. The cancelled batch was already stored
        // via requeue_as_cancelled() above and will be merged into the next
        // FlushBatch by flush_next().
        PromptOutcome::Cancelled => {
            tracing::debug!(
                agent = agent_index,
                outcome = outcome_label,
                configured_model = %harness_configured_model,
                pid = harness_pid,
                "agent_returned (cancelled)"
            );
            pool.return_agent(result.agent);
        }
        PromptOutcome::Error(ref e) => {
            let is_transport_error = matches!(
                e,
                acp::AcpError::Io(_)
                    | acp::AcpError::WriteTimeout(_)
                    | acp::AcpError::Timeout(_)
                    | acp::AcpError::Protocol(_)
            ) || is_pi_session_invalidated_error(e);
            let error_code = match &e {
                acp::AcpError::AgentError { code, .. } => Some(*code),
                _ => None,
            };
            if is_transport_error {
                tracing::warn!(
                    agent = agent_index,
                    outcome = outcome_label,
                    configured_model = %harness_configured_model,
                    pid = harness_pid,
                    error = %e,
                    "transport/protocol error — respawning agent"
                );
                emit_turn_error(&e.to_string(), error_code);

                let index = result.agent.index;
                let slot_history = &mut crash_history[index];
                if !spawn_respawn_task(
                    result.agent,
                    config,
                    slot_history,
                    respawn_tx,
                    respawn_tasks,
                    observer,
                ) && pool.live_count() == 0
                    && !any_respawn_in_flight(crash_history)
                {
                    tracing::error!("all agents dead — exiting");
                    return LoopAction::Exit;
                }
            } else {
                tracing::warn!(
                    agent = agent_index,
                    outcome = outcome_label,
                    configured_model = %harness_configured_model,
                    pid = harness_pid,
                    error = %e,
                    "agent_returned (application error — pipe intact)"
                );
                emit_turn_error(&e.to_string(), error_code);
                pool.return_agent(result.agent);
            }
        }
    }
    LoopAction::Continue
}

#[allow(clippy::too_many_arguments)]
fn recover_panicked_agent(
    pool: &mut AgentPool,
    queue: &mut EventQueue,
    config: &Config,
    join_error: tokio::task::JoinError,
    heartbeat_in_flight: &mut bool,
    removed_channels: &HashSet<Uuid>,
    typing_channels: &mut HashMap<ConversationKey, ThreadTags>,
    crash_history: &mut [SlotCircuit],
    respawn_tx: &mpsc::Sender<RespawnResult>,
    respawn_tasks: &mut tokio::task::JoinSet<()>,
    observer: Option<observer::ObserverHandle>,
) {
    let task_id = join_error.id();
    let Some(meta) = pool.task_map_mut().remove(&task_id) else {
        tracing::error!("panic for unknown task {task_id:?} — bug");
        return;
    };
    let i = meta.agent_index;

    // Requeue BEFORE mark_complete (same rationale as handle_prompt_result).
    if let Some(batch) = meta.recoverable_batch {
        if let Some(ch) = meta.channel_id {
            if !queue.is_current_conversation(&batch.conversation_key) {
                tracing::info!(
                    conversation = %batch.conversation_key,
                    "discarding panicked batch from superseded /new generation"
                );
            } else if !removed_channels.contains(&ch) {
                // Dead-letter on exhaustion is logged inside requeue(); a
                // panic path has no outcome to report, so no notice here.
                let _ = queue.requeue(batch);
                tracing::warn!("requeued batch for panicked agent {i}");
            } else {
                tracing::debug!(
                    channel_id = %ch,
                    "dropping panicked batch for removed channel"
                );
            }
        }
    }

    if let Some(key) = meta.conversation_key.as_ref() {
        queue.mark_conversation_complete(key);
        typing_channels.remove(key);
        tracing::warn!("cleared wedged in-flight conversation {key} from panicked agent {i}");
    } else if let Some(ch) = meta.channel_id {
        queue.mark_complete(ch);
        typing_channels.retain(|key, _| key.channel_id != ch);
        tracing::warn!("cleared wedged legacy in-flight channel {ch} from panicked agent {i}");
    } else {
        *heartbeat_in_flight = false;
        tracing::warn!("cleared wedged heartbeat_in_flight from panicked agent {i}");
    }

    if let Some(ref observer) = observer {
        observer.emit(
            "agent_panic",
            Some(i),
            &observer::context_for(meta.channel_id, None, Some(meta.turn_id)),
            serde_json::json!({
                "outcome": "panic",
                "error": format!("Agent task panicked: {join_error}"),
            }),
        );
    }

    // Panics count as crashes for the circuit breaker.
    // The panicked task already dropped the AcpClient, so we just need to
    // check the circuit and spawn a fresh agent in the background.
    let slot = &mut crash_history[i];

    let delay = match slot.record_crash() {
        CrashVerdict::CircuitOpen => {
            tracing::error!(agent = i, "circuit open after panic — not respawning");
            return;
        }
        CrashVerdict::HalfOpenProbe => {
            tracing::info!(agent = i, "circuit half-open — probe respawn after panic");
            Duration::ZERO
        }
        CrashVerdict::Respawn(d) => {
            tracing::info!(
                agent = i,
                delay_ms = d.as_millis(),
                "respawn backoff after panic"
            );
            d
        }
    };

    // Spawn respawn work off the main loop.
    slot.respawn_in_flight = true;
    let cmd = config.agent_command.clone();
    let args = config.agent_args.clone();
    let env = config.persona_env_vars.clone();
    let has_codex = config.has_generated_codex_config;
    let guard = RespawnGuard::new(i, respawn_tx.clone());
    respawn_tasks.spawn(async move {
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }
        let result = spawn_and_init(&cmd, &args, &env, has_codex, i, observer).await;
        guard.send(result);
    });
}

#[allow(clippy::too_many_arguments)]
fn drain_ready_join_results(
    pool: &mut AgentPool,
    queue: &mut EventQueue,
    config: &Config,
    heartbeat_in_flight: &mut bool,
    removed_channels: &HashSet<Uuid>,
    typing_channels: &mut HashMap<ConversationKey, ThreadTags>,
    crash_history: &mut [SlotCircuit],
    respawn_tx: &mpsc::Sender<RespawnResult>,
    respawn_tasks: &mut tokio::task::JoinSet<()>,
    observer: Option<observer::ObserverHandle>,
) -> LoopAction {
    while let Some(Some(join_result)) = pool.join_set.join_next().now_or_never() {
        if let Err(join_error) = join_result {
            tracing::error!("agent task panicked: {join_error}");
            recover_panicked_agent(
                pool,
                queue,
                config,
                join_error,
                heartbeat_in_flight,
                removed_channels,
                typing_channels,
                crash_history,
                respawn_tx,
                respawn_tasks,
                observer.clone(),
            );
            if pool.live_count() == 0 && !any_respawn_in_flight(crash_history) {
                return LoopAction::Exit;
            }
        }
    }
    LoopAction::Continue
}

fn dispatch_heartbeat(
    pool: &mut AgentPool,
    ctx: &Arc<PromptContext>,
    heartbeat_in_flight: &mut bool,
) {
    if *heartbeat_in_flight {
        return;
    }
    let agent = match pool.try_claim(None) {
        Some(a) => a,
        None => return,
    };

    let prompt_text = ctx
        .heartbeat_prompt
        .clone()
        .unwrap_or_else(default_heartbeat_prompt);
    let result_tx = pool.result_tx();
    let ctx_clone = Arc::clone(ctx);
    let agent_index = agent.index;
    let turn_id = Uuid::new_v4().to_string();
    let task_turn_id = turn_id.clone();

    let abort_handle = pool.join_set.spawn(async move {
        pool::run_prompt_task(
            agent,
            None,
            Some(prompt_text),
            ctx_clone,
            result_tx,
            None,
            task_turn_id,
        )
        .await;
    });

    pool.task_map_mut().insert(
        abort_handle.id(),
        pool::TaskMeta {
            agent_index,
            channel_id: None,
            conversation_key: None,
            turn_id,
            recoverable_batch: None,
            control_tx: None,
            steer_tx: None,
        },
    );
    *heartbeat_in_flight = true;
    tracing::info!(agent = agent_index, "heartbeat_fired");
}

#[cfg(test)]
mod agent_draft_prompt_tests {
    #[test]
    fn shared_base_prompt_teaches_portable_agent_drafts() {
        let prompt = include_str!("base_prompt.md");
        assert!(prompt.contains("buzz agents draft-create"));
        assert!(prompt.contains("ask for at most two things"));
        assert!(prompt.contains("what it should do day-to-day"));
        assert!(prompt.contains("owner saves it"));
        assert!(prompt.contains("Do not ask about runtime, provider, model, credentials"));
    }

    #[test]
    fn shared_base_prompt_teaches_real_newlines_for_multiline_messages() {
        let prompt = include_str!("base_prompt.md");
        assert!(prompt.contains("pass real newline bytes through stdin"));
        assert!(prompt.contains("single-quoted shell strings preserve `\\n` literally"));
        assert!(prompt.contains("buzz messages send ... --content -"));
    }

    #[test]
    fn shared_base_prompt_teaches_single_command_mentions_and_preflight() {
        let prompt = include_str!("base_prompt.md");
        assert!(prompt.contains("--mention <hex-or-npub>"));
        assert!(prompt.contains("every presentation-only name that should notify"));
        assert!(
            prompt.contains("permits unresolved or ambiguous `@Name` text as presentation-only")
        );
        assert!(prompt.contains("success JSON's `mention_pubkeys`"));
        assert!(prompt.contains("no follow-up verification command is needed"));
        assert!(prompt.contains("stops before sending"));
        assert!(prompt
            .contains("add them explicitly with `buzz channels add-member` only when authorized"));
        assert!(prompt.contains("never changes membership automatically"));
    }
}

fn default_heartbeat_prompt() -> String {
    let now = chrono::Utc::now().to_rfc3339();
    format!(
        "[System: Heartbeat]\nTime: {now}\n\n\
         You have been awakened for a routine heartbeat. You have NO incoming messages or\n\
         active channel context for this turn.\n\n\
         Your tasks:\n\
         1. Run `buzz feed get --types needs_action` to check for pending workflow approvals or\n\
            high-priority requests addressed to you.\n\
         2. Run `buzz feed get --types mentions` to check for unanswered @mentions.\n\
         3. If you find actionable items, address them using the appropriate CLI commands\n\
            (e.g., `buzz workflows approve --token <UUID>`, `buzz messages send`,\n\
            `buzz messages send --reply-to <event-id>`).\n\
         4. If there are no pending actions or mentions, end your turn immediately.\n\n\
         Do not run `buzz channels list` or `buzz messages search` unless you have a specific reason.\n\
         Do not invent work — only act on items surfaced by the feed commands."
    )
}

/// Spawn a background respawn task for a crashed agent slot.
///
/// Does the circuit breaker check synchronously (non-blocking), then spawns
/// the actual shutdown + backoff + spawn_and_init work into a background task.
/// The result comes back through `respawn_tx` so the main loop stays responsive.
///
/// Returns `true` if a respawn task was spawned, `false` if the circuit is open.
fn spawn_respawn_task(
    old_agent: OwnedAgent,
    config: &Config,
    slot: &mut SlotCircuit,
    respawn_tx: &mpsc::Sender<RespawnResult>,
    respawn_tasks: &mut tokio::task::JoinSet<()>,
    observer: Option<observer::ObserverHandle>,
) -> bool {
    let index = old_agent.index;

    // Circuit breaker: record crash, decide whether to respawn.
    let delay = match slot.record_crash() {
        CrashVerdict::CircuitOpen => {
            tracing::error!(agent = index, "circuit open — not respawning");
            return false;
        }
        CrashVerdict::HalfOpenProbe => {
            tracing::info!(agent = index, "circuit half-open — probe respawn");
            Duration::ZERO
        }
        CrashVerdict::Respawn(d) => {
            tracing::info!(agent = index, delay_ms = d.as_millis(), "respawn backoff");
            d
        }
    };

    slot.respawn_in_flight = true;

    // Spawn the actual work (shutdown + sleep + spawn + init) off the main loop.
    let cmd = config.agent_command.clone();
    let args = config.agent_args.clone();
    let env = config.persona_env_vars.clone();
    let has_codex = config.has_generated_codex_config;
    let guard = RespawnGuard::new(index, respawn_tx.clone());
    respawn_tasks.spawn(async move {
        // Shutdown old agent (reap child, prevent zombie).
        let mut agent = old_agent;
        agent.acp.shutdown().await;
        drop(agent);

        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }

        let result = spawn_and_init(&cmd, &args, &env, has_codex, index, observer).await;
        guard.send(result);
    });

    true
}

fn normalized_agent_name(init_result: &serde_json::Value) -> String {
    init_result
        .get("agentInfo")
        .or_else(|| init_result.get("serverInfo"))
        .and_then(|info| info.get("name"))
        .and_then(|value| value.as_str())
        .unwrap_or("unknown")
        .trim()
        .to_ascii_lowercase()
}

async fn shutdown_agent_slots(slots: &mut [Option<OwnedAgent>]) {
    for slot in slots {
        if let Some(mut agent) = slot.take() {
            agent.acp.shutdown().await;
        }
    }
}

async fn shutdown_agent_pool(pool: &mut AgentPool) {
    pool.join_set.shutdown().await;
    while let Ok(mut result) = pool.result_rx_try_recv() {
        result.agent.acp.shutdown().await;
    }
    for slot in pool.agents_mut() {
        if let Some(mut agent) = slot.take() {
            agent.acp.shutdown().await;
        }
    }
}

struct PoolStartup {
    agents: u32,
    command: String,
    args: Vec<String>,
    extra_env: Vec<(String, String)>,
    has_generated_codex_config: bool,
    model: Option<String>,
    observer: Option<observer::ObserverHandle>,
}

impl PoolStartup {
    fn from_config(config: &Config, observer: Option<observer::ObserverHandle>) -> Self {
        Self {
            agents: config.agents,
            command: config.agent_command.clone(),
            args: config.agent_args.clone(),
            extra_env: config.persona_env_vars.clone(),
            has_generated_codex_config: config.has_generated_codex_config,
            model: config.model.clone(),
            observer,
        }
    }
}

async fn initialize_agent_pool(
    startup: &PoolStartup,
    mut shutdown: Option<watch::Receiver<()>>,
) -> Result<AgentPool> {
    // One agent failing to start must not kill the whole pool.
    // Attempt each spawn under a 60-second timeout; a partial pool is valid.
    let mut agent_slots: Vec<Option<OwnedAgent>> = Vec::with_capacity(startup.agents as usize);
    for i in 0..startup.agents as usize {
        let spawn_result = AcpClient::spawn(
            &startup.command,
            &startup.args,
            &startup.extra_env,
            startup.has_generated_codex_config,
        )
        .await;
        match spawn_result {
            Ok(mut acp) => {
                acp.set_observer(startup.observer.clone(), i);
                let initialize = tokio::time::timeout(Duration::from_secs(60), acp.initialize());
                let initialize_result = match shutdown.as_mut() {
                    Some(shutdown) => tokio::select! {
                        biased;
                        _ = shutdown.changed() => {
                            acp.shutdown().await;
                            shutdown_agent_slots(&mut agent_slots).await;
                            return Err(anyhow::anyhow!("pool initialization cancelled by shutdown"));
                        }
                        result = initialize => result,
                    },
                    None => initialize.await,
                };
                match initialize_result {
                    Ok(Ok(init_result)) => {
                        tracing::info!(agent = i, "agent initialized: {init_result}");
                        let protocol_version =
                            init_result["protocolVersion"].as_u64().unwrap_or(1) as u32;
                        tracing::info!(
                            agent = i,
                            name = init_result
                                .get("agentInfo")
                                .or_else(|| init_result.get("serverInfo"))
                                .and_then(|info| info.get("name"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("unknown"),
                            steering_supported = acp.steering_supported(),
                            "agent initialized"
                        );
                        acp.observe(
                            "agent_initialized",
                            serde_json::json!({
                                "agentIndex": i,
                                "initializeResult": init_result,
                            }),
                        );
                        let agent_name = normalized_agent_name(&init_result);
                        agent_slots.push(Some(OwnedAgent {
                            index: i,
                            acp,
                            state: SessionState::default(),
                            model_capabilities: None,
                            desired_model: startup.model.clone(),
                            model_overridden: false,
                            pending_model_switch: None,
                            agent_name,
                            goose_system_prompt_supported: None,
                            protocol_version,
                        }));
                    }
                    Ok(Err(e)) => {
                        tracing::error!(agent = i, "agent initialize failed: {e}");
                        acp.shutdown().await;
                        agent_slots.push(None);
                    }
                    Err(_) => {
                        tracing::error!(agent = i, "agent timed out during init (60s)");
                        acp.shutdown().await;
                        agent_slots.push(None);
                    }
                }
            }
            Err(e) => {
                tracing::error!(agent = i, "agent failed to spawn: {e}");
                agent_slots.push(None);
            }
        }
    }
    let live_count = agent_slots.iter().filter(|slot| slot.is_some()).count();
    if live_count == 0 {
        return Err(anyhow::anyhow!(
            "all {} agents failed to start — cannot continue",
            startup.agents
        ));
    }
    if live_count < startup.agents as usize {
        tracing::warn!(
            "started {}/{} agents — continuing with reduced pool",
            live_count,
            startup.agents
        );
    }
    tracing::info!("agent_pool_ready agents={}", live_count);
    Ok(AgentPool::from_slots(agent_slots))
}

// ── spawn_and_init ────────────────────────────────────────────────────────────
/// Spawn an agent subprocess and run the MCP `initialize` handshake.
///
/// Takes owned args so it can run in a background `tokio::spawn` task without
/// borrowing `Config`. All respawn/refill paths use this.
async fn spawn_and_init(
    command: &str,
    args: &[String],
    extra_env: &[(String, String)],
    has_generated_codex_config: bool,
    agent_index: usize,
    observer: Option<observer::ObserverHandle>,
) -> Result<(AcpClient, u32, String)> {
    let mut acp = AcpClient::spawn(command, args, extra_env, has_generated_codex_config)
        .await
        .map_err(|e| anyhow::anyhow!("failed to spawn agent: {e}"))?;
    acp.set_observer(observer, agent_index);

    match acp.initialize().await {
        Ok(init_result) => {
            tracing::info!("agent initialized: {init_result}");
            let protocol_version = init_result["protocolVersion"].as_u64().unwrap_or(1) as u32;
            acp.observe(
                "agent_initialized",
                serde_json::json!({
                    "agentIndex": agent_index,
                    "initializeResult": init_result,
                }),
            );
            let agent_name = normalized_agent_name(&init_result);
            Ok((acp, protocol_version, agent_name))
        }
        Err(e) => {
            // Explicitly shut down the spawned child to prevent zombie/leak.
            // Drop only does start_kill + try_wait (best-effort); shutdown()
            // does start_kill + bounded wait (guaranteed reap).
            acp.shutdown().await;
            Err(anyhow::anyhow!("agent initialize failed: {e}"))
        }
    }
}

async fn spawn_auth_client(agent: &AuthAgentArgs) -> Result<AcpClient, acp::AcpError> {
    let agent_args = config::normalize_agent_args(&agent.agent_command, agent.agent_args.clone());
    AcpClient::spawn(&agent.agent_command, &agent_args, &[], false).await
}

fn extract_auth_methods(init_result: &serde_json::Value) -> Vec<serde_json::Value> {
    init_result
        .get("authMethods")
        .and_then(|methods| methods.as_array())
        .cloned()
        .unwrap_or_default()
}

/// `buzz-acp auth-methods` — spawn an adapter, initialize it, print authMethods.
async fn run_auth_methods(args: AuthMethodsArgs) -> Result<()> {
    let mut client = match spawn_auth_client(&args.agent).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: failed to spawn agent: {e}");
            std::process::exit(1);
        }
    };

    let init_result = match tokio::time::timeout(MODELS_TIMEOUT, client.initialize()).await {
        Ok(Ok(result)) => result,
        Ok(Err(e)) => {
            client.shutdown().await;
            eprintln!("error: agent initialize failed: {e}");
            std::process::exit(1);
        }
        Err(_) => {
            client.shutdown().await;
            eprintln!("error: agent timed out ({MODELS_TIMEOUT:?})");
            std::process::exit(1);
        }
    };

    let methods = extract_auth_methods(&init_result);
    client.shutdown().await;

    if args.json {
        let output = serde_json::json!({ "methods": methods });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else if methods.is_empty() {
        println!("No auth methods advertised.");
    } else {
        for method in methods {
            let id = method
                .get("id")
                .and_then(|value| value.as_str())
                .unwrap_or("unknown");
            let name = method
                .get("name")
                .and_then(|value| value.as_str())
                .unwrap_or(id);
            println!("{id}\t{name}");
        }
    }
    Ok(())
}

/// `buzz-acp authenticate` — invoke one adapter-owned auth method.
async fn run_authenticate(args: AuthenticateArgs) -> Result<()> {
    let mut client = match spawn_auth_client(&args.agent).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: failed to spawn agent: {e}");
            std::process::exit(1);
        }
    };

    let init_result = match tokio::time::timeout(MODELS_TIMEOUT, client.initialize()).await {
        Ok(Ok(result)) => result,
        Ok(Err(e)) => {
            client.shutdown().await;
            eprintln!("error: agent initialize failed: {e}");
            std::process::exit(1);
        }
        Err(_) => {
            client.shutdown().await;
            eprintln!("error: agent initialize timed out ({MODELS_TIMEOUT:?})");
            std::process::exit(1);
        }
    };

    let supports_method = extract_auth_methods(&init_result)
        .iter()
        .any(|method| method.get("id").and_then(|id| id.as_str()) == Some(args.method_id.as_str()));
    if !supports_method {
        client.shutdown().await;
        eprintln!(
            "error: auth method '{}' is not advertised by this adapter",
            args.method_id
        );
        std::process::exit(1);
    }

    let result =
        tokio::time::timeout(AUTHENTICATE_TIMEOUT, client.authenticate(&args.method_id)).await;

    match result {
        Ok(Ok(_)) => {
            client.shutdown().await;
            Ok(())
        }
        Ok(Err(e)) => {
            client.shutdown().await;
            eprintln!("error: authenticate failed: {e}");
            std::process::exit(1);
        }
        Err(_) => {
            client.shutdown().await;
            eprintln!("error: authenticate timed out ({AUTHENTICATE_TIMEOUT:?})");
            std::process::exit(1);
        }
    }
}

/// Flow: spawn → initialize → session/new → print models → shutdown.
/// No relay connection, no MCP servers, no subscriptions. ~2-5s total.
async fn run_models(args: ModelsArgs) -> Result<()> {
    use acp::{extract_model_config_options, extract_model_state};

    let agent_args = config::normalize_agent_args(&args.agent.agent_command, args.agent.agent_args);
    let cwd = std::env::current_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("/"))
        .to_string_lossy()
        .to_string();

    // Spawn outside the timeout so we always own the child for cleanup.
    // `models` subcommand doesn't use persona packs — no extra env, no codex config.
    let mut client =
        match AcpClient::spawn(&args.agent.agent_command, &agent_args, &[], false).await {
            Ok(c) => c,
            Err(e) => {
                eprintln!("error: failed to spawn agent: {e}");
                std::process::exit(1);
            }
        };

    // Initialize + session/new under a timeout. Client is owned above,
    // so shutdown() runs on all paths (success, error, timeout).
    let protocol_result = tokio::time::timeout(MODELS_TIMEOUT, async {
        let init = client.initialize().await?;
        let session = client.session_new_full(&cwd, vec![], None, None).await?;
        Ok::<_, acp::AcpError>((init, session))
    })
    .await;

    let (init_result, session_resp) = match protocol_result {
        Ok(Ok(tuple)) => tuple,
        Ok(Err(e)) => {
            client.shutdown().await;
            eprintln!("error: agent communication failed: {e}");
            std::process::exit(1);
        }
        Err(_) => {
            client.shutdown().await;
            eprintln!("error: agent timed out ({MODELS_TIMEOUT:?})");
            std::process::exit(1);
        }
    };

    // Extract agent info from initialize response.
    // ACP spec uses "serverInfo" (MCP heritage); some agents may use "agentInfo".
    let info_obj = init_result
        .get("serverInfo")
        .or_else(|| init_result.get("agentInfo"));
    let agent_name = info_obj
        .and_then(|ai| ai.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let agent_version = info_obj
        .and_then(|ai| ai.get("version"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    // Extract model info from session/new response.
    let config_options = extract_model_config_options(&session_resp.raw);
    let model_state = extract_model_state(&session_resp.raw);

    if args.json {
        // Structured JSON output — consumed by Phase 3 `get_agent_models`.
        let output = serde_json::json!({
            "agent": {
                "name": agent_name,
                "version": agent_version,
            },
            "stable": {
                "configOptions": config_options,
            },
            "unstable": model_state.as_ref().map(|ms| serde_json::json!({
                "currentModelId": ms.get("currentModelId"),
                "availableModels": ms.get("availableModels"),
            })),
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        // Human-readable output.
        println!("Agent: {} v{}", agent_name, agent_version);
        println!();

        let mut has_models = false;

        if !config_options.is_empty() {
            println!("Models (stable configOptions):");
            for opt in &config_options {
                let config_id = opt.get("configId").and_then(|v| v.as_str()).unwrap_or("?");
                let display = opt
                    .get("displayName")
                    .and_then(|v| v.as_str())
                    .unwrap_or(config_id);
                println!("  {display} (configId: {config_id})");
                if let Some(options) = opt.get("options").and_then(|v| v.as_array()) {
                    for o in options {
                        let val = o.get("value").and_then(|v| v.as_str()).unwrap_or("?");
                        let name = o.get("displayName").and_then(|v| v.as_str()).unwrap_or(val);
                        println!("    - {name} (value: {val})");
                    }
                }
            }
            has_models = true;
        }

        if let Some(ref ms) = model_state {
            let current = ms
                .get("currentModelId")
                .and_then(|v| v.as_str())
                .unwrap_or("(none)");
            println!("Models (unstable SessionModelState):");
            println!("  Current: {current}");
            if let Some(available) = ms.get("availableModels").and_then(|v| v.as_array()) {
                println!("  Available:");
                for m in available {
                    let id = m.get("modelId").and_then(|v| v.as_str()).unwrap_or("?");
                    let name = m.get("name").and_then(|v| v.as_str()).unwrap_or(id);
                    let desc = m.get("description").and_then(|v| v.as_str()).unwrap_or("");
                    if desc.is_empty() {
                        println!("    - {name} (id: {id})");
                    } else {
                        println!("    - {name} (id: {id}) — {desc}");
                    }
                }
            }
            has_models = true;
        }

        if !has_models {
            println!("No model information available from this agent.");
        }
    }

    client.shutdown().await;
    Ok(())
}

fn build_mcp_servers(config: &Config) -> Vec<McpServer> {
    if config.mcp_command.is_empty() {
        return vec![];
    }
    vec![McpServer {
        name: std::path::Path::new(&config.mcp_command)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("mcp")
            .to_string(),
        command: config.mcp_command.clone(),
        args: vec![],
        env: {
            let mut env = vec![
                EnvVar {
                    name: "BUZZ_RELAY_URL".into(),
                    value: config.relay_url.clone(),
                },
                EnvVar {
                    name: "BUZZ_PRIVATE_KEY".into(),
                    // bech32 encoding of a valid secret key is infallible.
                    // Panic here is correct: injecting a bogus secret would cause
                    // delayed, hard-to-diagnose agent failures downstream.
                    value: config
                        .keys
                        .secret_key()
                        .to_bech32()
                        .expect("secret key bech32 encoding should never fail"),
                },
            ];
            // Forward BUZZ_AUTH_TAG (NIP-OA owner attestation credential)
            // so the MCP server can attach it to every signed event.
            if let Ok(auth_tag) = std::env::var("BUZZ_AUTH_TAG") {
                if !auth_tag.is_empty() {
                    env.push(EnvVar {
                        name: "BUZZ_AUTH_TAG".into(),
                        value: auth_tag,
                    });
                }
            }
            // Forward the agent's display name so dev-mcp can use it as the git
            // author name instead of the raw npub. Read from the process env
            // rather than Config: this is a pass-through of a contract owned
            // upstream, and absent simply means dev-mcp falls back to the npub.
            if let Ok(display_name) = std::env::var("BUZZ_ACP_DISPLAY_NAME") {
                if !display_name.is_empty() {
                    env.push(EnvVar {
                        name: "BUZZ_ACP_DISPLAY_NAME".into(),
                        value: display_name,
                    });
                }
            }
            env
        },
    }]
}

#[cfg(test)]
mod heartbeat_base_prompt_tests {
    use super::*;

    // Pins the heartbeat dispatch path (dispatch_heartbeat, ~line 2359): a
    // legacy agent WITH a base_prompt must get [Base] prepended to the
    // heartbeat user message, composed as `[Base]\n{bp}\n\n{prompt}`. This is
    // the second half of the round-2 regression (the first being initial_message).

    #[test]
    fn test_heartbeat_legacy_agent_gets_base_prepended() {
        // protocol_version 1 + Some(base_prompt): heartbeat prompt is prefixed
        // with the [Base] section exactly as the legacy session/new path would.
        let prompt = "[System: Heartbeat]\nrun feed get";
        let composed = pool::prepend_base_for_legacy(1, Some("you are a helpful agent"), prompt);
        assert_eq!(
            composed,
            "[Base]\nyou are a helpful agent\n\n[System: Heartbeat]\nrun feed get"
        );
        assert!(composed.starts_with("[Base]\nyou are a helpful agent\n\n"));
    }

    #[test]
    fn test_heartbeat_modern_agent_omits_base() {
        // protocol_version 2 gets base_prompt via session/new; the heartbeat
        // prompt is sent verbatim.
        let prompt = "[System: Heartbeat]\nrun feed get";
        let composed = pool::prepend_base_for_legacy(2, Some("you are a helpful agent"), prompt);
        assert_eq!(composed, prompt);
    }
}

#[cfg(test)]
mod buzz_session_notice_tests {
    use super::*;

    #[test]
    fn lifecycle_persistence_failure_poison_is_transport_fatal() {
        let error = lifecycle_persistence_boundary_error(1)
            .expect("one unpersisted durable notice must poison the live ACP process");
        assert!(matches!(error, acp::AcpError::Protocol(_)));
        assert!(lifecycle_persistence_boundary_error(0).is_none());
    }

    #[test]
    fn lifecycle_notice_dedupe_covers_pending_and_delivered_compactions() {
        let mut state = LifecycleNoticeDedupe::default();
        assert!(state.reserve(Some("compaction:one")));
        assert!(!state.reserve(Some("compaction:one")));
        state.finish(Some("compaction:one"), true);
        assert!(!state.reserve(Some("compaction:one")));

        assert!(state.reserve(Some("compaction:retryable")));
        state.finish(Some("compaction:retryable"), false);
        assert!(
            state.reserve(Some("compaction:retryable")),
            "a notice that exhausted delivery must remain retryable if Pi retransmits it"
        );
    }

    #[tokio::test]
    async fn lifecycle_notice_outbox_dedupes_and_drains_on_shutdown() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test HTTP server");
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(AtomicUsize::new(0));
        let server_requests = Arc::clone(&requests);
        let server = tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                let mut request = vec![0; 16_384];
                let _ = socket.read(&mut request).await;
                server_requests.fetch_add(1, Ordering::SeqCst);
                let body = r#"{"event_id":"accepted","accepted":true,"message":"ok"}"#;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = socket.write_all(response.as_bytes()).await;
            }
        });
        let keys = nostr::Keys::generate();
        let rest = relay::RestClient {
            http: reqwest::Client::new(),
            base_url,
            keys: keys.clone(),
            auth_tag_json: None,
        };
        let state_dir = tempfile::tempdir().expect("outbox tempdir");
        let durable = DurableOutbox::open(state_dir.path(), &keys.public_key().to_hex())
            .expect("open durable outbox");
        let mut outbox =
            LifecycleNoticeOutbox::spawn(rest.clone(), durable).expect("spawn lifecycle outbox");
        let channel_id = Uuid::new_v4();
        let notice = || LifecycleNotice {
            dedupe_key: Some("compaction:stable-id".to_owned()),
            content: "🧹 Context compacted safely.".to_owned(),
            ack: Some(acp::BuzzSessionEventAck {
                conversation_id: "buzz:test:conversation".to_owned(),
                event_id: Uuid::new_v4().to_string(),
            }),
        };

        outbox.enqueue(channel_id, &ThreadTags::default(), vec![notice()], &rest);
        outbox.enqueue(channel_id, &ThreadTags::default(), vec![notice()], &rest);
        outbox.enqueue(
            Uuid::new_v4(),
            &ThreadTags::default(),
            vec![notice()],
            &rest,
        );
        outbox.shutdown().await;

        assert_eq!(
            requests.load(Ordering::SeqCst),
            2,
            "one thread must dedupe its compaction ID without suppressing another thread"
        );
        server.abort();
    }

    #[tokio::test]
    async fn rejected_lifecycle_notice_does_not_block_another_thread() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test HTTP server");
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let blocked_event_id = Arc::new(Mutex::new(None::<String>));
        let server_blocked_event_id = Arc::clone(&blocked_event_id);
        let accepted_requests = Arc::new(AtomicUsize::new(0));
        let server_accepted_requests = Arc::clone(&accepted_requests);
        let server = tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                let mut request = Vec::new();
                let mut buffer = [0u8; 4_096];
                while let Ok(read) = socket.read(&mut buffer).await {
                    if read == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..read]);
                    let Some(header_end) = request.windows(4).position(|item| item == b"\r\n\r\n")
                    else {
                        continue;
                    };
                    let headers = String::from_utf8_lossy(&request[..header_end]);
                    let content_length = headers
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().ok())
                                .flatten()
                        })
                        .unwrap_or(0);
                    if request.len() >= header_end + 4 + content_length {
                        break;
                    }
                }

                let blocked = loop {
                    if let Ok(guard) = server_blocked_event_id.lock() {
                        if let Some(value) = guard.clone() {
                            break value;
                        }
                    }
                    tokio::task::yield_now().await;
                };
                let is_blocked = String::from_utf8_lossy(&request).contains(&blocked);
                if !is_blocked {
                    server_accepted_requests.fetch_add(1, Ordering::SeqCst);
                }
                let body = format!(
                    r#"{{"event_id":"fixture","accepted":{},"message":"fixture"}}"#,
                    !is_blocked
                );
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = socket.write_all(response.as_bytes()).await;
            }
        });

        let keys = nostr::Keys::generate();
        let rest = relay::RestClient {
            http: reqwest::Client::new(),
            base_url,
            keys: keys.clone(),
            auth_tag_json: None,
        };
        let state_dir = tempfile::tempdir().expect("outbox tempdir");
        let durable = DurableOutbox::open(state_dir.path(), &keys.public_key().to_hex())
            .expect("open durable outbox");
        let mut outbox = LifecycleNoticeOutbox::spawn(rest.clone(), durable.clone())
            .expect("spawn lifecycle outbox");
        let blocked_channel = Uuid::new_v4();
        let healthy_channel = Uuid::new_v4();
        let notice = |key: &str, content: &str| LifecycleNotice {
            dedupe_key: Some(key.to_owned()),
            content: content.to_owned(),
            ack: None,
        };

        let blocked_outcome = outbox.enqueue(
            blocked_channel,
            &ThreadTags::default(),
            vec![notice("blocked", "blocked lifecycle notice")],
            &rest,
        );
        assert_eq!(blocked_outcome.persisted, 1);
        let pending = durable
            .pending_lifecycle()
            .expect("read blocked durable lifecycle event");
        assert_eq!(pending.len(), 1);
        let rejected_event_id = pending[0].event.id.to_hex();
        *blocked_event_id.lock().expect("set blocked event id") = Some(rejected_event_id.clone());

        let healthy_outcome = outbox.enqueue(
            healthy_channel,
            &ThreadTags::default(),
            vec![notice("healthy", "healthy lifecycle notice")],
            &rest,
        );
        assert_eq!(healthy_outcome.persisted, 1);

        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let pending = durable
                    .pending_lifecycle()
                    .expect("read lifecycle delivery progress");
                if accepted_requests.load(Ordering::SeqCst) >= 1
                    && pending.len() == 1
                    && pending[0].event.id.to_hex() == rejected_event_id
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("healthy lifecycle notice was head-of-line blocked");

        let healthy_key = format!("{healthy_channel}:channel-root:healthy");
        assert!(
            durable
                .delivered_keys()
                .expect("read durable delivery receipts")
                .contains(&healthy_key),
            "healthy notice must be durably retired while the rejected notice remains pending"
        );

        outbox.tx.take();
        if let Some(worker) = outbox.worker.take() {
            worker.abort();
            let _ = worker.await;
        }
        server.abort();
    }

    #[tokio::test]
    async fn poisoned_lifecycle_dedupe_never_acks_without_durable_persistence() {
        let keys = nostr::Keys::generate();
        let rest = relay::RestClient {
            http: reqwest::Client::new(),
            base_url: "http://127.0.0.1:9".to_owned(),
            keys: keys.clone(),
            auth_tag_json: None,
        };
        let state_dir = tempfile::tempdir().expect("outbox tempdir");
        let durable = DurableOutbox::open(state_dir.path(), &keys.public_key().to_hex())
            .expect("open durable outbox");
        let mut outbox =
            LifecycleNoticeOutbox::spawn(rest.clone(), durable.clone()).expect("spawn outbox");
        let poisoned = Arc::clone(&outbox.dedupe);
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = poisoned.lock().expect("lock before poison");
            panic!("intentional lifecycle dedupe poison");
        }));

        let ack = acp::BuzzSessionEventAck {
            conversation_id: "buzz:test:poison".to_owned(),
            event_id: Uuid::new_v4().to_string(),
        };
        let adapter_event_id = ack.event_id.clone();
        let outcome = outbox.enqueue(
            Uuid::new_v4(),
            &ThreadTags::default(),
            vec![LifecycleNotice {
                dedupe_key: Some("poisoned-dedupe-event".to_owned()),
                content: "must not be acknowledged".to_owned(),
                ack: Some(ack.clone()),
            }],
            &rest,
        );
        assert_eq!(outcome.persisted, 0);
        assert_eq!(outcome.failed, 1);
        assert!(outcome.acknowledgements.is_empty());
        assert!(durable
            .pending_lifecycle()
            .expect("read pending lifecycle")
            .is_empty());
        outbox.shutdown().await;

        // A poisoned live boundary is fatal to that ACP process. After a
        // replacement process replays the same adapter record, the exact
        // durable event identity can be persisted and acknowledged.
        let mut recovered = LifecycleNoticeOutbox::spawn(rest.clone(), durable.clone())
            .expect("restart lifecycle outbox");
        let recovery_channel_id = Uuid::new_v4();
        let recovered_outcome = recovered.enqueue(
            recovery_channel_id,
            &ThreadTags::default(),
            vec![LifecycleNotice {
                dedupe_key: Some(format!("adapter-event:{adapter_event_id}")),
                content: "replayed safely".to_owned(),
                ack: Some(ack.clone()),
            }],
            &rest,
        );
        assert_eq!(recovered_outcome.failed, 0);
        assert_eq!(recovered_outcome.persisted, 1);
        assert_eq!(recovered_outcome.acknowledgements, vec![ack]);
        let pending = durable
            .pending_lifecycle()
            .expect("read recovered durable lifecycle");
        assert_eq!(pending.len(), 1);
        let expected_key =
            format!("{recovery_channel_id}:channel-root:adapter-event:{adapter_event_id}");
        assert_eq!(
            pending[0].dedupe_key.as_deref(),
            Some(expected_key.as_str())
        );
        recovered.tx.take();
        if let Some(worker) = recovered.worker.take() {
            worker.abort();
        }
    }

    #[test]
    fn compaction_notice_reports_configured_cap_and_effective_threshold() {
        let notice = render_buzz_session_notice(&BuzzSessionEvent::CompactionCompleted {
            compaction_id: Uuid::new_v4().to_string(),
            timestamp: "2026-08-02T12:00:00Z".to_string(),
            message: "do not publish adapter prose SECRET".to_string(),
            pi_session_id: "pi-session".to_string(),
            reason: acp::CompactionReason::Threshold,
            before_tokens: Some(147_820),
            after_tokens: Some(31_400),
            limit_tokens: 150_000,
            effective_limit_tokens: 150_000,
            compaction_threshold_tokens: 133_616,
            will_retry: false,
            from_extension: false,
        });

        assert!(notice.contains("147,820 → ~31,400"));
        assert!(notice.contains("Configured cap 150,000 tokens"));
        assert!(notice.contains("automatic compaction threshold 133,616"));
        assert!(!notice.contains("SECRET"));
    }

    #[test]
    fn compaction_failure_never_exposes_adapter_error_details() {
        let notice = render_buzz_session_notice(&BuzzSessionEvent::CompactionFailed {
            compaction_id: Uuid::new_v4().to_string(),
            timestamp: "2026-08-02T12:00:00Z".to_string(),
            message: "secret message".to_string(),
            pi_session_id: "pi-session".to_string(),
            reason: acp::CompactionReason::Preflight,
            before_tokens: Some(140_000),
            limit_tokens: 150_000,
            effective_limit_tokens: 150_000,
            compaction_threshold_tokens: 133_616,
            error: "/Users/example/.pi/extensions/private.ts: API_KEY=secret".to_string(),
            aborted: false,
            will_retry: false,
            from_extension: true,
        });

        assert!(notice.contains("request was stopped"));
        assert!(notice.contains("`/new`"));
        assert!(!notice.contains("API_KEY"));
        assert!(!notice.contains("/Users"));
        assert!(!notice.contains("secret message"));
    }

    #[test]
    fn context_and_reload_notices_sanitize_untrusted_strings() {
        let context = render_buzz_session_notice(&BuzzSessionEvent::ContextStatus {
            timestamp: "2026-08-02T12:00:00Z".to_string(),
            message: "ignored".to_string(),
            pi_session_id: "pi-session".to_string(),
            used_tokens: Some(75_000),
            remaining_tokens: Some(75_000),
            percent: Some(1.0),
            limit_tokens: 150_000,
            effective_limit_tokens: 150_000,
            compaction_threshold_tokens: 133_616,
            auto_compaction: true,
            compacting: false,
            model: Some("provider/model`\n@everyone SECRET".to_string()),
        });
        assert!(
            context.contains("50.0%"),
            "percentage is recomputed from counts"
        );
        assert!(!context.contains("@everyone"));
        assert!(!context.contains("SECRET"));

        let reload = render_buzz_session_notice(&BuzzSessionEvent::ExtensionsReloaded {
            timestamp: "2026-08-02T12:00:00Z".to_string(),
            message: "ignored".to_string(),
            pi_session_id: "pi-session".to_string(),
            extensions: 2,
            skills: 3,
            prompts: 4,
            context_files: 1,
            errors: vec!["/private/path: token=SECRET".to_string()],
            project_trusted: false,
        });
        assert!(reload.contains("1 resource diagnostic(s)"));
        assert!(reload.contains("stayed disabled"));
        assert!(!reload.contains("SECRET"));
        assert!(!reload.contains("/private/path"));
    }

    #[test]
    fn transient_compaction_failures_and_pi_reset_echoes_are_not_published() {
        let retrying_failure = BuzzSessionEvent::CompactionFailed {
            compaction_id: Uuid::new_v4().to_string(),
            timestamp: "2026-08-02T12:00:00Z".to_string(),
            message: "ignored".to_string(),
            pi_session_id: "pi-session".to_string(),
            reason: acp::CompactionReason::Preflight,
            before_tokens: Some(140_000),
            limit_tokens: 150_000,
            effective_limit_tokens: 150_000,
            compaction_threshold_tokens: 133_616,
            error: "ignored".to_string(),
            aborted: false,
            will_retry: true,
            from_extension: false,
        };
        let reset_echo = BuzzSessionEvent::SessionReset {
            timestamp: "2026-08-02T12:00:00Z".to_string(),
            message: "ignored".to_string(),
            previous_pi_session_id: "old-pi-session".to_string(),
            pi_session_id: "pi-session".to_string(),
            limit_tokens: 150_000,
            effective_limit_tokens: 150_000,
            compaction_threshold_tokens: 133_616,
        };

        assert!(!should_publish_buzz_session_notice(&retrying_failure));
        assert!(!should_publish_buzz_session_notice(&reset_echo));
    }
}

#[cfg(test)]
mod owner_control_command_tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag};

    fn make_event(kind: u32, content: &str, p_hex: Option<&str>) -> nostr::Event {
        let keys = Keys::generate();
        let tags = match p_hex {
            Some(hex) => vec![Tag::parse(["p", hex]).expect("p tag")],
            None => vec![],
        };
        EventBuilder::new(Kind::Custom(kind as u16), content)
            .tags(tags)
            .sign_with_keys(&keys)
            .unwrap()
    }

    #[test]
    fn owner_control_command_requires_kind_content_and_agent_mention() {
        let agent = "ab".repeat(32);

        let event = make_event(KIND_STREAM_MESSAGE, " !rotate ", Some(&agent));
        assert!(is_owner_control_command(
            &event,
            KIND_STREAM_MESSAGE,
            "!rotate",
            &agent,
            None,
        ));

        let wrong_kind = make_event(1, "!rotate", Some(&agent));
        assert!(!is_owner_control_command(
            &wrong_kind,
            1,
            "!rotate",
            &agent,
            None,
        ));

        let wrong_content = make_event(KIND_STREAM_MESSAGE, "!cancel", Some(&agent));
        assert!(!is_owner_control_command(
            &wrong_content,
            KIND_STREAM_MESSAGE,
            "!rotate",
            &agent,
            None,
        ));

        let no_mention = make_event(KIND_STREAM_MESSAGE, "!rotate", None);
        assert!(!is_owner_control_command(
            &no_mention,
            KIND_STREAM_MESSAGE,
            "!rotate",
            &agent,
            None,
        ));
    }

    #[test]
    fn reserved_new_uses_the_same_leading_mention_normalization_as_slash_commands() {
        let agent = "ab".repeat(32);
        let npub = "nostr:npub1xhqc4cnnln86lqxk983qulu8yxusfxfhntwl75es2jkvy5zvz26qzr0685";

        for content in ["/new", "@Agent /new", &format!("{npub} /new")] {
            let event = make_event(KIND_STREAM_MESSAGE, content, Some(&agent));
            assert!(
                is_owner_control_command(&event, KIND_STREAM_MESSAGE, "/new", &agent, None),
                "reserved /new should be recognized after mention stripping: {content}"
            );
        }

        for content in [
            "@Agent see /new",
            "@Agent /new please",
            "@Agent /newer",
            "@Agent /tmp/new",
            "look at /new",
        ] {
            let event = make_event(KIND_STREAM_MESSAGE, content, Some(&agent));
            assert!(
                !is_owner_control_command(&event, KIND_STREAM_MESSAGE, "/new", &agent, None),
                "non-exact or non-leading content must not be consumed: {content}"
            );
        }
    }

    #[test]
    fn reserved_new_recognizes_the_configured_multi_word_agent_name() {
        let agent = "ab".repeat(32);
        let event = make_event(KIND_STREAM_MESSAGE, "@Buzz Agent /new", Some(&agent));

        assert!(is_owner_control_command(
            &event,
            KIND_STREAM_MESSAGE,
            "/new",
            &agent,
            Some("Buzz Agent"),
        ));
        assert!(!is_owner_control_command(
            &event,
            KIND_STREAM_MESSAGE,
            "/new",
            &agent,
            None,
        ));
    }

    #[test]
    fn reserved_new_detection_is_sender_independent_so_non_owners_are_consumed() {
        let agent = "ab".repeat(32);
        let owner_event = make_event(KIND_STREAM_MESSAGE, "@Agent /new", Some(&agent));
        let non_owner_event = make_event(KIND_STREAM_MESSAGE, "@Agent /new", Some(&agent));

        assert_ne!(owner_event.pubkey, non_owner_event.pubkey);
        assert!(is_owner_control_command(
            &owner_event,
            KIND_STREAM_MESSAGE,
            "/new",
            &agent,
            None,
        ));
        assert!(is_owner_control_command(
            &non_owner_event,
            KIND_STREAM_MESSAGE,
            "/new",
            &agent,
            None,
        ));
        // The main intake authorizes the first only when its pubkey equals the
        // configured owner, but consumes both once this reserved shape matches.
    }

    #[test]
    fn bare_pi_mutations_are_reserved_even_without_a_nostr_mention() {
        let agent = "ab".repeat(32);
        for (content, expected) in [
            ("/new", "/new"),
            ("/compact", "/compact"),
            ("/reload\nignore this trailing prompt", "/reload"),
            ("@Buzz Agent /compact", "/compact"),
        ] {
            let event = make_event(KIND_STREAM_MESSAGE, content, None);
            assert_eq!(
                reserved_pi_lifecycle_command(&event, KIND_STREAM_MESSAGE, Some("Buzz Agent")),
                Some(expected),
                "subscribe-all/no-mention intake must reserve {content:?}"
            );
            assert!(
                !is_owner_control_command(
                    &event,
                    KIND_STREAM_MESSAGE,
                    expected,
                    &agent,
                    Some("Buzz Agent"),
                ),
                "an unaddressed command may be consumed but never authorized"
            );
        }
    }

    #[test]
    fn pi_mutation_reservation_matches_only_the_adapter_command_line() {
        for content in ["/compact now", "/reloader", "please /reload", "/context"] {
            let event = make_event(KIND_STREAM_MESSAGE, content, None);
            assert_eq!(
                reserved_pi_lifecycle_command(&event, KIND_STREAM_MESSAGE, None),
                None,
                "non-mutating or non-exact command must not be reserved: {content:?}"
            );
        }
        let wrong_kind = make_event(1, "/reload", None);
        assert_eq!(reserved_pi_lifecycle_command(&wrong_kind, 1, None), None);
    }

    #[test]
    fn addressed_owner_command_uses_the_same_first_line_semantics_as_pi() {
        let agent = "ab".repeat(32);
        let event = make_event(
            KIND_STREAM_MESSAGE,
            "@Buzz Agent /reload\ntrailing context",
            Some(&agent),
        );
        assert!(is_owner_control_command(
            &event,
            KIND_STREAM_MESSAGE,
            "/reload",
            &agent,
            Some("Buzz Agent"),
        ));
    }

    #[test]
    fn mode_gate_signal_maps_handling_to_control_signal() {
        let owner = "a".repeat(64);
        let other = "b".repeat(64);

        // Queue: never signals — events wait for the turn to finish.
        assert!(mode_gate_signal(MultipleEventHandling::Queue, &owner, Some(&owner)).is_none());

        // Steer: always steers (eligibility already enforced upstream).
        assert!(matches!(
            mode_gate_signal(MultipleEventHandling::Steer, &other, Some(&owner)),
            Some(ControlSignal::Steer)
        ));
        // Steer even when owner is unknown — gate doesn't re-check authorship.
        assert!(matches!(
            mode_gate_signal(MultipleEventHandling::Steer, &other, None),
            Some(ControlSignal::Steer)
        ));

        // Interrupt: always interrupts for any eligible author.
        assert!(matches!(
            mode_gate_signal(MultipleEventHandling::Interrupt, &other, Some(&owner)),
            Some(ControlSignal::Interrupt)
        ));

        // OwnerInterrupt: interrupts only for the owner.
        assert!(matches!(
            mode_gate_signal(MultipleEventHandling::OwnerInterrupt, &owner, Some(&owner)),
            Some(ControlSignal::Interrupt)
        ));
        assert!(
            mode_gate_signal(MultipleEventHandling::OwnerInterrupt, &other, Some(&owner)).is_none(),
            "owner-interrupt must not fire for a non-owner author"
        );
        assert!(
            mode_gate_signal(MultipleEventHandling::OwnerInterrupt, &owner, None).is_none(),
            "owner-interrupt must not fire when the owner is unknown"
        );
    }

    #[tokio::test]
    async fn signal_in_flight_task_sends_rotate_once() {
        let mut pool = AgentPool::from_slots(vec![]);
        let channel_id = Uuid::new_v4();
        let other_channel_id = Uuid::new_v4();
        let (control_tx, control_rx) = tokio::sync::oneshot::channel();

        let abort_handle = pool.join_set.spawn(async {});
        pool.task_map_mut().insert(
            abort_handle.id(),
            pool::TaskMeta {
                agent_index: 0,
                channel_id: Some(channel_id),
                conversation_key: None,
                turn_id: "test-turn-id".to_string(),
                recoverable_batch: None,
                control_tx: Some(control_tx),
                steer_tx: None,
            },
        );

        assert!(!signal_in_flight_task(
            &mut pool,
            other_channel_id,
            ControlSignal::Rotate
        ));
        assert!(signal_in_flight_task(
            &mut pool,
            channel_id,
            ControlSignal::Rotate
        ));
        assert_eq!(control_rx.await.unwrap(), ControlSignal::Rotate);
        assert!(!signal_in_flight_task(
            &mut pool,
            channel_id,
            ControlSignal::Rotate
        ));
    }

    #[tokio::test]
    async fn observer_control_routes_exact_turn_and_rejects_ambiguous_legacy_frames() {
        let mut pool = AgentPool::from_slots(vec![]);
        let channel_id = Uuid::new_v4();
        let wrong_channel_id = Uuid::new_v4();
        let (first_tx, mut first_rx) = tokio::sync::oneshot::channel();
        let (second_tx, second_rx) = tokio::sync::oneshot::channel();

        let first_task = pool.join_set.spawn(std::future::pending::<()>());
        pool.task_map_mut().insert(
            first_task.id(),
            pool::TaskMeta {
                agent_index: 0,
                channel_id: Some(channel_id),
                conversation_key: None,
                turn_id: "turn-one".to_string(),
                recoverable_batch: None,
                control_tx: Some(first_tx),
                steer_tx: None,
            },
        );
        let second_task = pool.join_set.spawn(std::future::pending::<()>());
        pool.task_map_mut().insert(
            second_task.id(),
            pool::TaskMeta {
                agent_index: 1,
                channel_id: Some(channel_id),
                conversation_key: None,
                turn_id: "turn-two".to_string(),
                recoverable_batch: None,
                control_tx: Some(second_tx),
                steer_tx: None,
            },
        );

        assert_eq!(
            signal_observer_control(&mut pool, channel_id, "turn-two", ControlSignal::Cancel,),
            ObserverControlResult::Sent
        );
        assert_eq!(second_rx.await.unwrap(), ControlSignal::Cancel);
        assert!(
            first_rx.try_recv().is_err(),
            "sibling turn must be untouched"
        );

        assert_eq!(
            signal_observer_control(
                &mut pool,
                wrong_channel_id,
                "turn-one",
                ControlSignal::Cancel,
            ),
            ObserverControlResult::NoActiveTurn,
            "an exact turn id cannot be replayed against another channel"
        );
        assert_eq!(
            signal_observer_control(&mut pool, channel_id, "turn-two", ControlSignal::Cancel,),
            ObserverControlResult::TurnEnding,
            "a second signal for the same turn reports that it is already ending"
        );

        let (closed_tx, closed_rx) = tokio::sync::oneshot::channel();
        drop(closed_rx);
        let closed_task = pool.join_set.spawn(std::future::pending::<()>());
        pool.task_map_mut().insert(
            closed_task.id(),
            pool::TaskMeta {
                agent_index: 2,
                channel_id: Some(channel_id),
                conversation_key: None,
                turn_id: "turn-closed".to_string(),
                recoverable_batch: None,
                control_tx: Some(closed_tx),
                steer_tx: None,
            },
        );
        assert_eq!(
            signal_observer_control(
                &mut pool,
                channel_id,
                "turn-closed",
                ControlSignal::SwitchModel("provider/model".to_string()),
            ),
            ObserverControlResult::TurnEnding,
            "a closed receiver must never be reported to Desktop as sent"
        );
    }

    #[tokio::test]
    async fn legacy_observer_control_cannot_mutate_turns_across_fresh_replay_caches() {
        let mut pool = AgentPool::from_slots(vec![]);
        let mut replay_cache = ObserverControlReplayCache::default();
        let channel_id = Uuid::new_v4();
        let payload = serde_json::json!({
            "type": "cancel_turn",
            "channelId": channel_id,
        });
        let now = chrono::Utc::now().timestamp();

        let (first_tx, mut first_rx) = tokio::sync::oneshot::channel();
        let first_task = pool.join_set.spawn(std::future::pending::<()>());
        let first_task_id = first_task.id();
        pool.task_map_mut().insert(
            first_task_id,
            pool::TaskMeta {
                agent_index: 0,
                channel_id: Some(channel_id),
                conversation_key: None,
                turn_id: "turn-one".to_string(),
                recoverable_batch: None,
                control_tx: Some(first_tx),
                steer_tx: None,
            },
        );

        route_authenticated_observer_control(
            "aa".repeat(32),
            now,
            now,
            &payload,
            &mut pool,
            None,
            &mut replay_cache,
        );
        assert!(
            first_rx.try_recv().is_err(),
            "a channel-only control must not mutate even the first matching turn"
        );
        pool.task_map_mut().remove(&first_task_id);
        first_task.abort();

        let (second_tx, mut second_rx) = tokio::sync::oneshot::channel();
        let second_task = pool.join_set.spawn(std::future::pending::<()>());
        pool.task_map_mut().insert(
            second_task.id(),
            pool::TaskMeta {
                agent_index: 0,
                channel_id: Some(channel_id),
                conversation_key: None,
                turn_id: "turn-two".to_string(),
                recoverable_batch: None,
                control_tx: Some(second_tx),
                steer_tx: None,
            },
        );

        // A restart has an empty in-memory replay cache. The missing turnId is
        // still enough to reject the retained signed frame fail closed.
        let mut restarted_replay_cache = ObserverControlReplayCache::default();
        route_authenticated_observer_control(
            "aa".repeat(32),
            now,
            now,
            &payload,
            &mut pool,
            None,
            &mut restarted_replay_cache,
        );
        assert!(
            second_rx.try_recv().is_err(),
            "the same signed legacy control must not target a later turn"
        );
    }

    #[test]
    fn observer_control_replay_cache_fails_closed_at_capacity_until_expiry() {
        let mut cache = ObserverControlReplayCache::default();
        let now = 10_000;
        for index in 0..OBSERVER_CONTROL_REPLAY_CAPACITY {
            assert_eq!(
                cache.admit(format!("event-{index}"), now, now),
                ObserverControlAdmission::Accepted
            );
        }
        assert_eq!(
            cache.admit("overflow".into(), now, now),
            ObserverControlAdmission::Saturated
        );
        assert_eq!(
            cache.admit(
                "after-expiry".into(),
                now + OBSERVER_CONTROL_FRESHNESS_SECS + 1,
                now + OBSERVER_CONTROL_FRESHNESS_SECS + 1,
            ),
            ObserverControlAdmission::Accepted
        );
    }
}

#[cfg(test)]
mod owner_cache_tests {
    use super::*;

    #[test]
    fn new_with_some_caches_immediately() {
        let cache = OwnerCache::new(Some("abcd".into()));
        assert_eq!(cache.get(), Some("abcd"));
    }

    #[test]
    fn new_with_none_returns_none() {
        let cache = OwnerCache::new(None);
        assert!(cache.get().is_none());
    }

    #[test]
    fn get_returns_cached_value() {
        let cache = OwnerCache::new(Some("ab".repeat(32)));
        assert_eq!(cache.get(), Some("ab".repeat(32)).as_deref());
    }
}

#[cfg(test)]
mod author_gate_tests {
    use super::*;

    /// A `RestClient` for tests. The author-gate decisions exercised here all
    /// resolve from the owner pubkey or sibling cache before any HTTP call, so
    /// this client is never actually used to make a request.
    fn dummy_rest_client() -> relay::RestClient {
        relay::RestClient {
            http: reqwest::Client::new(),
            base_url: "http://localhost:0".into(),
            keys: nostr::Keys::generate(),
            auth_tag_json: None,
        }
    }

    const OWNER: &str = "00";
    const SIBLING: &str = "11";
    const EXTERNAL: &str = "22";
    const STRANGER: &str = "33";

    /// Owner + a known sibling, none of them on the explicit allowlist.
    fn cache_with_sibling() -> OwnerCache {
        let cache = OwnerCache::new(Some(OWNER.into()));
        cache.cache_sibling(SIBLING.into(), true);
        cache.cache_sibling(STRANGER.into(), false);
        cache.cache_sibling(EXTERNAL.into(), false);
        cache
    }

    #[tokio::test]
    async fn test_allowlist_accepts_sibling_not_in_allowlist() {
        let cache = cache_with_sibling();
        let allowlist = HashSet::from([EXTERNAL.to_string()]);
        assert!(
            author_allowed(
                &RespondTo::Allowlist,
                &allowlist,
                SIBLING,
                false,
                &cache,
                &dummy_rest_client()
            )
            .await,
            "a same-owner sibling must fire a turn under Allowlist even when not listed"
        );
    }

    #[tokio::test]
    async fn test_allowlist_accepts_explicit_external_pubkey() {
        let cache = cache_with_sibling();
        let allowlist = HashSet::from([EXTERNAL.to_string()]);
        assert!(
            author_allowed(
                &RespondTo::Allowlist,
                &allowlist,
                EXTERNAL,
                false,
                &cache,
                &dummy_rest_client()
            )
            .await,
            "an explicitly allowlisted external pubkey must still be accepted"
        );
    }

    #[tokio::test]
    async fn test_allowlist_rejects_non_sibling_not_in_allowlist() {
        let cache = cache_with_sibling();
        let allowlist = HashSet::from([EXTERNAL.to_string()]);
        assert!(
            !author_allowed(
                &RespondTo::Allowlist,
                &allowlist,
                STRANGER,
                false,
                &cache,
                &dummy_rest_client()
            )
            .await,
            "a non-sibling absent from the allowlist must be dropped"
        );
    }

    #[tokio::test]
    async fn test_allowlist_accepts_owner() {
        let cache = cache_with_sibling();
        let allowlist = HashSet::new();
        assert!(
            author_allowed(
                &RespondTo::Allowlist,
                &allowlist,
                OWNER,
                false,
                &cache,
                &dummy_rest_client()
            )
            .await,
            "the owner must always be accepted under Allowlist"
        );
    }

    // The default `respond-to` is OwnerOnly. Under steering, "an ineligible
    // author must NOT steer" is enforced *here* — author_allowed drops the
    // event before it reaches the mode gate — not in the gate itself. These
    // pin that invariant against the default mode.
    #[tokio::test]
    async fn test_owner_only_rejects_stranger_so_no_steer() {
        let cache = cache_with_sibling();
        assert!(
            !author_allowed(
                &RespondTo::OwnerOnly,
                &HashSet::new(),
                STRANGER,
                false,
                &cache,
                &dummy_rest_client()
            )
            .await,
            "under the default OwnerOnly, a stranger must be dropped — so it can never reach the mode gate to steer"
        );
    }

    #[tokio::test]
    async fn test_owner_only_admits_owner_and_sibling_to_steer() {
        let cache = cache_with_sibling();
        for (who, label) in [(OWNER, "owner"), (SIBLING, "sibling")] {
            assert!(
                author_allowed(
                    &RespondTo::OwnerOnly,
                    &HashSet::new(),
                    who,
                    false,
                    &cache,
                    &dummy_rest_client()
                )
                .await,
                "under default OwnerOnly, the {label} must be admitted so steering can fire"
            );
        }
    }

    // ── DM hardening ──────────────────────────────────────────────────────
    //
    // In a DM, clients auto-p-tag every participant, and an agent can be
    // asked to open a DM with a third party. The gate must therefore ignore
    // the allowlist and `anyone` mode inside DMs: only owner + verified
    // siblings fire turns.

    #[tokio::test]
    async fn test_dm_rejects_allowlisted_external_pubkey() {
        let cache = cache_with_sibling();
        let allowlist = HashSet::from([EXTERNAL.to_string()]);
        assert!(
            !author_allowed(
                &RespondTo::Allowlist,
                &allowlist,
                EXTERNAL,
                true,
                &cache,
                &dummy_rest_client()
            )
            .await,
            "an allowlisted external pubkey must NOT fire a turn inside a DM"
        );
    }

    #[tokio::test]
    async fn test_dm_rejects_stranger_under_anyone() {
        let cache = cache_with_sibling();
        assert!(
            !author_allowed(
                &RespondTo::Anyone,
                &HashSet::new(),
                STRANGER,
                true,
                &cache,
                &dummy_rest_client()
            )
            .await,
            "respond_to=anyone must still drop non-owner authors inside a DM"
        );
    }

    #[tokio::test]
    async fn test_dm_admits_owner_and_sibling_in_every_responding_mode() {
        let cache = cache_with_sibling();
        for mode in [
            RespondTo::OwnerOnly,
            RespondTo::Allowlist,
            RespondTo::Anyone,
        ] {
            for (who, label) in [(OWNER, "owner"), (SIBLING, "sibling")] {
                assert!(
                    author_allowed(
                        &mode,
                        &HashSet::new(),
                        who,
                        true,
                        &cache,
                        &dummy_rest_client()
                    )
                    .await,
                    "in a DM under {mode}, the {label} must still be admitted"
                );
            }
        }
    }

    #[tokio::test]
    async fn test_dm_nobody_rejects_even_owner() {
        let cache = cache_with_sibling();
        assert!(
            !author_allowed(
                &RespondTo::Nobody,
                &HashSet::new(),
                OWNER,
                true,
                &cache,
                &dummy_rest_client()
            )
            .await,
            "respond_to=nobody must drop everything, DMs included"
        );
    }

    // ── is_dm_channel resolution ──────────────────────────────────────────

    fn resolver(startup: HashMap<Uuid, relay::ChannelInfo>) -> pool::ChannelInfoResolver {
        pool::ChannelInfoResolver::new(startup, dummy_rest_client())
    }

    #[tokio::test]
    async fn test_is_dm_channel_uses_definitive_startup_metadata() {
        let dm_id = Uuid::new_v4();
        let stream_id = Uuid::new_v4();
        let startup = HashMap::from([
            (
                dm_id,
                relay::ChannelInfo {
                    name: "dm".into(),
                    channel_type: "dm".into(),
                },
            ),
            (
                stream_id,
                relay::ChannelInfo {
                    name: "stream".into(),
                    channel_type: "stream".into(),
                },
            ),
        ]);
        let resolver = resolver(startup);
        assert!(is_dm_channel(dm_id, &resolver).await);
        assert!(!is_dm_channel(stream_id, &resolver).await);
    }

    #[tokio::test]
    async fn test_is_dm_channel_fails_closed_for_unknown_startup_metadata() {
        let id = Uuid::new_v4();
        let startup = HashMap::from([(
            id,
            relay::ChannelInfo {
                name: "unknown".into(),
                channel_type: "unknown".into(),
            },
        )]);
        assert!(
            is_dm_channel(id, &resolver(startup)).await,
            "missing startup metadata must not be trusted as a stream"
        );
    }

    #[tokio::test]
    async fn unknown_channel_is_dm_for_auth_but_not_for_conversation_scope() {
        let id = Uuid::new_v4();
        let classification = resolve_dm_channel_scope(id, &resolver(HashMap::new())).await;
        assert_eq!(classification, (true, false));
    }

    async fn lazy_resolver_with_response(
        response: serde_json::Value,
    ) -> (
        pool::ChannelInfoResolver,
        std::sync::Arc<std::sync::atomic::AtomicUsize>,
        tokio::task::JoinHandle<()>,
    ) {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test HTTP server");
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let requests = std::sync::Arc::new(AtomicUsize::new(0));
        let server_requests = requests.clone();
        let body = response.to_string();
        let server = tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                let mut request = vec![0; 8192];
                let _ = socket.read(&mut request).await;
                server_requests.fetch_add(1, Ordering::SeqCst);
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = socket.write_all(response.as_bytes()).await;
            }
        });
        let rest = relay::RestClient {
            http: reqwest::Client::new(),
            base_url,
            keys: nostr::Keys::generate(),
            auth_tag_json: None,
        };
        (
            pool::ChannelInfoResolver::new(HashMap::new(), rest),
            requests,
            server,
        )
    }

    #[tokio::test]
    async fn test_is_dm_channel_lazy_resolves_declared_dm_and_caches_it() {
        use std::sync::atomic::Ordering;

        let id = Uuid::new_v4();
        let response = serde_json::json!([{
            "tags": [["d", id.to_string()], ["name", "DM"], ["t", "dm"]]
        }]);
        let (resolver, requests, server) = lazy_resolver_with_response(response).await;

        assert!(is_dm_channel(id, &resolver).await);
        assert!(is_dm_channel(id, &resolver).await);
        assert_eq!(
            requests.load(Ordering::SeqCst),
            1,
            "second resolution uses cache"
        );
        server.abort();
    }

    #[tokio::test]
    async fn test_discovery_without_metadata_stays_fail_closed_at_author_gate() {
        let id = Uuid::new_v4();
        let discovered = relay::merge_discovered_channels(vec![id], &serde_json::json!([]));
        let channel_info = resolver(discovered);
        let owner_cache = cache_with_sibling();
        let allowlist = HashSet::from([EXTERNAL.to_string()]);

        let is_dm = is_dm_channel(id, &channel_info).await;
        assert!(is_dm, "unknown startup metadata must fail closed as DM");
        assert!(
            !author_allowed(
                &RespondTo::Allowlist,
                &allowlist,
                EXTERNAL,
                is_dm,
                &owner_cache,
                &dummy_rest_client(),
            )
            .await,
            "an external author must not pass when startup discovery omitted metadata"
        );
    }

    #[tokio::test]
    async fn test_is_dm_channel_fails_closed_when_lazy_resolution_fails() {
        assert!(
            is_dm_channel(Uuid::new_v4(), &resolver(HashMap::new())).await,
            "an unresolvable channel type must be treated as a DM"
        );
    }
}

#[cfg(test)]
mod observer_snapshot_race_tests {
    use super::*;
    use nostr::Keys;

    fn emit_marker(observer: &observer::ObserverHandle, marker: &str) {
        observer.emit(
            "test_event",
            None,
            &observer::context_for(None, None, None),
            serde_json::json!({ "marker": marker }),
        );
    }

    /// An event emitted between `subscribe()` and `snapshot()` lands in BOTH
    /// the snapshot and the live receiver; the seq high-water dedupe must
    /// deliver it exactly once — and never lose events on either side of it.
    #[tokio::test(start_paused = true)]
    async fn overlap_between_subscribe_and_snapshot_publishes_exactly_once() {
        let observer = observer::ObserverHandle::in_process();
        let agent_keys = Keys::generate();
        let owner_keys = Keys::generate();
        let (publisher, mut published_rx) = RelayEventPublisher::test_pair();

        // Before the publisher starts: replay-buffer only.
        emit_marker(&observer, "before");
        // The race window: emitted after subscribe() but before snapshot(),
        // so it is present in the snapshot AND queued on the receiver.
        let rx = observer.subscribe();
        emit_marker(&observer, "overlap");
        let snapshot = observer.snapshot();
        assert_eq!(snapshot.len(), 2, "overlap event must be in the snapshot");
        // After the snapshot: live receiver only.
        emit_marker(&observer, "after");
        // Close the broadcast channel so the run loop drains and exits.
        drop(observer);

        run_relay_observer_publisher(
            snapshot,
            rx,
            publisher,
            agent_keys.clone(),
            agent_keys.public_key().to_hex(),
            owner_keys.public_key().to_hex(),
            owner_keys.public_key(),
        )
        .await;

        // The run loop has exited, dropping the publisher; drain the forwarded
        // events until the channel closes (deterministic — no try_recv race
        // with the test_pair forwarding task).
        let mut markers = Vec::new();
        while let Some(event) = published_rx.recv().await {
            let payload: serde_json::Value =
                decrypt_observer_payload(&owner_keys, &event).expect("decrypt published frame");
            markers.push(payload["payload"]["marker"].as_str().unwrap().to_string());
        }
        assert_eq!(
            markers,
            ["before", "overlap", "after"],
            "each event must be published exactly once, in order"
        );
    }
}

#[cfg(test)]
mod observer_publish_pacer_tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn starts_without_a_burst_and_spaces_frames() {
        let started = tokio::time::Instant::now();
        let mut pacer = ObserverPublishPacer::new();

        pacer.wait().await;
        let first = tokio::time::Instant::now();
        pacer.wait().await;
        let second = tokio::time::Instant::now();

        assert_eq!(first.duration_since(started), OBSERVER_PUBLISH_INTERVAL);
        assert_eq!(second.duration_since(first), OBSERVER_PUBLISH_INTERVAL);
    }

    #[tokio::test(start_paused = true)]
    async fn limits_frames_in_each_rolling_minute() {
        let mut pacer = ObserverPublishPacer::new();
        pacer.wait().await;
        let first = tokio::time::Instant::now();
        for _ in 1..OBSERVER_PUBLISH_LIMIT_PER_MINUTE {
            pacer.wait().await;
        }

        pacer.wait().await;
        let ninety_first = tokio::time::Instant::now();

        assert_eq!(ninety_first.duration_since(first), Duration::from_secs(60));
    }
}

#[cfg(test)]
mod observer_chunk_coalescer_tests {
    use super::*;

    fn chunk_event(
        seq: u64,
        update_type: &str,
        message_id: &str,
        text: &str,
    ) -> observer::ObserverEvent {
        observer::ObserverEvent {
            seq,
            timestamp: format!("2026-04-29T04:00:0{seq}Z"),
            kind: "acp_read".to_string(),
            agent_index: Some(0),
            channel_id: Some("channel-1".to_string()),
            session_id: Some("session-1".to_string()),
            turn_id: Some("turn-1".to_string()),
            started_at: None,
            payload: serde_json::json!({
                "jsonrpc": "2.0",
                "method": "session/update",
                "params": {
                    "sessionId": "session-1",
                    "update": {
                        "sessionUpdate": update_type,
                        "messageId": message_id,
                        "content": {
                            "type": "text",
                            "text": text,
                        },
                    },
                },
            }),
        }
    }

    fn non_chunk_event(seq: u64) -> observer::ObserverEvent {
        observer::ObserverEvent {
            seq,
            timestamp: format!("2026-04-29T04:00:0{seq}Z"),
            kind: "turn_started".to_string(),
            agent_index: Some(0),
            channel_id: Some("channel-1".to_string()),
            session_id: Some("session-1".to_string()),
            turn_id: Some("turn-1".to_string()),
            started_at: None,
            payload: serde_json::json!({ "type": "turn_started" }),
        }
    }

    fn chunk_text(event: &observer::ObserverEvent) -> &str {
        event.payload["params"]["update"]["content"]["text"]
            .as_str()
            .expect("chunk text")
    }

    #[test]
    fn coalesces_chunks_until_non_chunk_event() {
        let mut coalescer = ObserverChunkCoalescer::default();

        assert!(coalescer
            .ingest(chunk_event(1, "agent_message_chunk", "message-1", "hello "))
            .is_empty());
        assert!(coalescer
            .ingest(chunk_event(2, "agent_message_chunk", "message-1", "world"))
            .is_empty());

        let events = coalescer.ingest(non_chunk_event(3));
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].seq, 2);
        assert_eq!(chunk_text(&events[0]), "hello world");
        assert_eq!(events[1].kind, "turn_started");
    }

    #[test]
    fn keeps_independent_chunk_streams_separate() {
        let mut coalescer = ObserverChunkCoalescer::default();

        assert!(coalescer
            .ingest(chunk_event(1, "agent_message_chunk", "message-1", "answer"))
            .is_empty());
        assert!(coalescer
            .ingest(chunk_event(
                2,
                "agent_thought_chunk",
                "thought-1",
                "thinking"
            ))
            .is_empty());

        let events = coalescer.flush();
        assert_eq!(events.len(), 2);
        assert_eq!(chunk_text(&events[0]), "answer");
        assert_eq!(chunk_text(&events[1]), "thinking");
    }
}

#[cfg(test)]
mod build_mcp_servers_tests {
    use super::*;
    use std::sync::Mutex;

    /// Env-var-touching tests must run serially — env vars are process-global.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn test_config() -> Config {
        Config {
            keys: nostr::Keys::generate(),
            relay_url: "ws://localhost:3000".into(),
            agent_command: "goose".into(),
            agent_args: vec!["acp".into()],
            mcp_command: "test-mcp-server".into(),
            idle_timeout_secs: config::DEFAULT_IDLE_TIMEOUT_SECS,
            max_turn_duration_secs: config::DEFAULT_MAX_TURN_DURATION_SECS,
            agents: 1,
            heartbeat_interval_secs: 0,
            turn_liveness_secs: 10,
            heartbeat_prompt: None,
            system_prompt: None,
            team_instructions: None,
            initial_message: None,
            subscribe_mode: config::SubscribeMode::All,
            dedup_mode: config::DedupMode::Queue,
            multiple_event_handling: config::MultipleEventHandling::Queue,
            ignore_self: true,
            kinds_override: None,
            channels_override: None,
            no_mention_filter: false,
            config_path: std::path::PathBuf::from("./buzz-acp.toml"),
            state_dir: std::env::temp_dir().join("buzz-acp-lib-test"),
            context_message_limit: 12,
            max_turns_per_session: 0,
            max_cached_conversation_sessions: 8,
            conversation_session_idle_ttl_secs: 1_800,
            presence_enabled: true,
            typing_enabled: true,
            memory_enabled: false,
            model: None,
            session_title: None,
            permission_mode: config::PermissionMode::BypassPermissions,
            respond_to: config::RespondTo::Anyone,
            respond_to_allowlist: std::collections::HashSet::new(),
            allowed_respond_to: vec![],
            persona_env_vars: vec![],
            has_generated_codex_config: false,
            relay_observer: false,
            exit_after_inactivity_secs: 0,
            lazy_pool: false,
            require_durable_thread_sessions: false,
            agent_owner: None,
            no_base_prompt: false,
            base_prompt_content: None,
        }
    }

    #[test]
    fn session_new_mcp_server_has_required_fields() {
        let config = test_config();
        let servers = build_mcp_servers(&config);
        assert_eq!(servers.len(), 1);
        let server = &servers[0];
        assert_eq!(server.name, "test-mcp-server");

        let names: Vec<&str> = server.env.iter().map(|e| e.name.as_str()).collect();
        assert!(
            names.contains(&"BUZZ_RELAY_URL"),
            "missing BUZZ_RELAY_URL; got {names:?}"
        );
        assert!(
            names.contains(&"BUZZ_PRIVATE_KEY"),
            "missing BUZZ_PRIVATE_KEY; got {names:?}"
        );
    }

    #[test]
    fn session_new_mcp_server_forwards_buzz_auth_tag() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("BUZZ_AUTH_TAG", "test-attestation-tag");
        let config = test_config();
        let servers = build_mcp_servers(&config);
        std::env::remove_var("BUZZ_AUTH_TAG");

        let server = &servers[0];
        let auth_tag_env = server.env.iter().find(|e| e.name == "BUZZ_AUTH_TAG");
        assert!(
            auth_tag_env.is_some(),
            "BUZZ_AUTH_TAG should be forwarded when set"
        );
        assert_eq!(auth_tag_env.unwrap().value, "test-attestation-tag");
    }

    #[test]
    fn session_new_mcp_server_skips_empty_buzz_auth_tag() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("BUZZ_AUTH_TAG", "");
        let config = test_config();
        let servers = build_mcp_servers(&config);
        std::env::remove_var("BUZZ_AUTH_TAG");

        let server = &servers[0];
        let has_auth_tag = server.env.iter().any(|e| e.name == "BUZZ_AUTH_TAG");
        assert!(!has_auth_tag, "empty BUZZ_AUTH_TAG should not be forwarded");
    }

    #[test]
    fn test_display_name_set_is_forwarded_to_mcp_server() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("BUZZ_ACP_DISPLAY_NAME", "Duncan");
        let config = test_config();
        let servers = build_mcp_servers(&config);
        std::env::remove_var("BUZZ_ACP_DISPLAY_NAME");

        let entry = servers[0]
            .env
            .iter()
            .find(|e| e.name == "BUZZ_ACP_DISPLAY_NAME");
        assert_eq!(
            entry.map(|e| e.value.as_str()),
            Some("Duncan"),
            "a set display name should reach the MCP server verbatim"
        );
    }

    #[test]
    fn test_display_name_unset_omits_the_key_entirely() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("BUZZ_ACP_DISPLAY_NAME");
        let config = test_config();
        let servers = build_mcp_servers(&config);

        // Absent, not empty-valued: dev-mcp distinguishes the two and only
        // falls back to the npub when the key is missing or blank.
        assert!(
            !servers[0]
                .env
                .iter()
                .any(|e| e.name == "BUZZ_ACP_DISPLAY_NAME"),
            "unset display name should not add the key"
        );
    }

    #[test]
    fn test_display_name_empty_omits_the_key_entirely() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("BUZZ_ACP_DISPLAY_NAME", "");
        let config = test_config();
        let servers = build_mcp_servers(&config);
        std::env::remove_var("BUZZ_ACP_DISPLAY_NAME");

        assert!(
            !servers[0]
                .env
                .iter()
                .any(|e| e.name == "BUZZ_ACP_DISPLAY_NAME"),
            "empty display name should not be forwarded"
        );
    }

    #[test]
    fn empty_mcp_command_returns_no_servers() {
        let mut config = test_config();
        config.mcp_command = "".into();
        let servers = build_mcp_servers(&config);
        assert!(
            servers.is_empty(),
            "empty mcp_command should produce no MCP servers"
        );
    }

    #[test]
    fn absolute_path_mcp_command_uses_file_stem_as_name() {
        let mut config = test_config();
        config.mcp_command = "/opt/bin/my-mcp-server".into();
        let servers = build_mcp_servers(&config);
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].name, "my-mcp-server");
    }

    #[test]
    fn mcp_command_with_no_stem_falls_back_to_mcp() {
        // Path::new("").file_stem() returns None — exercises the unwrap_or("mcp") path.
        let mut config = test_config();
        config.mcp_command = "".into();
        // Empty command returns no servers; test the stem logic directly.
        assert_eq!(
            std::path::Path::new("")
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("mcp"),
            "mcp"
        );

        // Confirm a non-empty command with no stem (e.g. just a dot) also falls back.
        config.mcp_command = ".".into();
        let servers = build_mcp_servers(&config);
        assert_eq!(servers.len(), 1);
        assert_eq!(
            servers[0].name, "mcp",
            "Path::new(\".\").file_stem() is None — should fall back to \"mcp\""
        );
    }
}

#[cfg(test)]
mod error_outcome_emission_tests {
    //! Pins the policy that error-class outcomes surface to the activity feed
    //! and never to the channel:
    //!
    //! - Channel silence is enforced *structurally* — `handle_prompt_result`
    //!   takes no relay handle, so it has no way to post a channel message. A
    //!   future re-introduction of channel notices would have to add the relay
    //!   parameter back, which these tests' construction would then refuse to
    //!   compile against.
    //! - Feed coverage is the regression-prone half and is asserted at runtime:
    //!   each error outcome must emit exactly one `turn_error` observer event.
    //!   If any branch drops its `emit_turn_error` call, the matching test goes
    //!   red.

    use super::*;
    use crate::acp::{AcpClient, AcpError};
    use crate::observer::ObserverHandle;
    use crate::pool::{
        AgentPool, OwnedAgent, PromptOutcome, PromptResult, PromptSource, TimeoutKind,
    };
    use crate::queue::{BatchEvent, FlushBatch};
    use nostr::{EventBuilder, Keys, Kind};
    use std::collections::HashSet;

    fn test_config() -> Config {
        Config {
            keys: nostr::Keys::generate(),
            relay_url: "ws://localhost:3000".into(),
            // `true` exits cleanly, so the async respawn fails fast and
            // harmlessly off the JoinSet — irrelevant to the synchronous
            // feed emission under test.
            agent_command: "true".into(),
            agent_args: vec![],
            mcp_command: "test-mcp-server".into(),
            idle_timeout_secs: config::DEFAULT_IDLE_TIMEOUT_SECS,
            max_turn_duration_secs: config::DEFAULT_MAX_TURN_DURATION_SECS,
            agents: 1,
            heartbeat_interval_secs: 0,
            turn_liveness_secs: 10,
            heartbeat_prompt: None,
            system_prompt: None,
            team_instructions: None,
            initial_message: None,
            subscribe_mode: config::SubscribeMode::All,
            dedup_mode: config::DedupMode::Queue,
            multiple_event_handling: config::MultipleEventHandling::Queue,
            ignore_self: true,
            kinds_override: None,
            channels_override: None,
            no_mention_filter: false,
            config_path: std::path::PathBuf::from("./buzz-acp.toml"),
            state_dir: std::env::temp_dir().join("buzz-acp-observer-test"),
            context_message_limit: 12,
            max_turns_per_session: 0,
            max_cached_conversation_sessions: 8,
            conversation_session_idle_ttl_secs: 1_800,
            presence_enabled: true,
            typing_enabled: true,
            memory_enabled: false,
            model: None,
            session_title: None,
            permission_mode: config::PermissionMode::BypassPermissions,
            respond_to: config::RespondTo::Anyone,
            respond_to_allowlist: HashSet::new(),
            allowed_respond_to: vec![],
            persona_env_vars: vec![],
            has_generated_codex_config: false,
            relay_observer: false,
            exit_after_inactivity_secs: 0,
            lazy_pool: false,
            require_durable_thread_sessions: false,
            agent_owner: None,
            no_base_prompt: false,
            base_prompt_content: None,
        }
    }

    #[test]
    fn normalizes_agent_name_from_initialize_result() {
        assert_eq!(
            normalized_agent_name(&serde_json::json!({
                "agentInfo": { "name": " Goose ", "version": "1.43.0" }
            })),
            "goose"
        );
        assert_eq!(
            normalized_agent_name(&serde_json::json!({
                "serverInfo": { "name": "buzz-agent" }
            })),
            "buzz-agent"
        );
    }

    /// Spawn a real but inert agent subprocess (`cat`) so the error paths have
    /// an `OwnedAgent` to move into respawn or return to the pool. The error
    /// branches never talk to the subprocess.
    async fn dummy_agent(index: usize) -> OwnedAgent {
        OwnedAgent {
            index,
            acp: AcpClient::spawn("cat", &[], &[], false)
                .await
                .expect("spawn cat as inert agent"),
            state: Default::default(),
            model_capabilities: None,
            desired_model: None,
            model_overridden: false,
            pending_model_switch: None,
            agent_name: "unknown".into(),
            goose_system_prompt_supported: None,
            // Error branches under test never read this; 1 is the legacy
            // non-systemPrompt path, the simplest valid value.
            protocol_version: 1,
        }
    }

    /// Drive one error outcome through `handle_prompt_result` and return how
    /// many `turn_error` events it emitted to the observer feed.
    async fn turn_errors_emitted_for(outcome: PromptOutcome) -> usize {
        let agent = dummy_agent(0).await;
        let mut pool = AgentPool::from_slots(vec![None]);

        // `handle_prompt_result` asserts it removes exactly one in-flight task
        // for the completing agent (the slot was checked out, not idle). Mirror
        // the real dispatch path by registering a TaskMeta keyed on a genuine
        // `task::Id` — only obtainable from inside a spawned task.
        let task_id = pool.join_set.spawn(async {}).id();
        pool.task_map_mut().insert(
            task_id,
            crate::pool::TaskMeta {
                agent_index: 0,
                channel_id: None,
                conversation_key: None,
                turn_id: "test-turn-id".to_string(),
                recoverable_batch: None,
                control_tx: None,
                steer_tx: None,
            },
        );

        let mut queue = EventQueue::new(config::DedupMode::Queue);
        let config = test_config();
        let mut heartbeat_in_flight = false;
        let removed_channels = HashSet::new();
        let mut crash_history = vec![SlotCircuit {
            crash_times: Vec::new(),
            open_until: None,
            respawn_in_flight: false,
        }];
        let (respawn_tx, _respawn_rx) = mpsc::channel(8);
        let mut respawn_tasks = tokio::task::JoinSet::new();
        let observer = ObserverHandle::in_process();

        let result = PromptResult {
            agent,
            source: PromptSource::Channel(Uuid::new_v4()),
            turn_id: "test-turn-id".to_string(),
            outcome,
            batch: None,
        };

        handle_prompt_result(
            &mut pool,
            &mut queue,
            &config,
            result,
            &mut heartbeat_in_flight,
            &removed_channels,
            &mut crash_history,
            &respawn_tx,
            &mut respawn_tasks,
            Some(observer.clone()),
            None,
            None,
        );

        let turn_errors: Vec<_> = observer
            .snapshot()
            .into_iter()
            .filter(|e| e.kind == "turn_error")
            .collect();
        assert!(
            turn_errors
                .iter()
                .all(|event| event.turn_id.as_deref() == Some("test-turn-id")),
            "turn_error must retain the completed turn id"
        );
        turn_errors.len()
    }

    #[tokio::test]
    async fn agent_exited_emits_exactly_one_feed_event() {
        assert_eq!(turn_errors_emitted_for(PromptOutcome::AgentExited).await, 1);
    }

    #[tokio::test]
    async fn panic_event_retains_task_turn_id() {
        let mut pool = AgentPool::from_slots(vec![]);
        let channel_id = Uuid::new_v4();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let abort_handle = pool.join_set.spawn(async move {
            let _ = started_tx.send(());
            std::future::pending::<()>().await;
        });
        let task_id = abort_handle.id();
        pool.task_map_mut().insert(
            task_id,
            crate::pool::TaskMeta {
                agent_index: 0,
                channel_id: Some(channel_id),
                conversation_key: None,
                turn_id: "panic-turn-id".to_string(),
                recoverable_batch: None,
                control_tx: None,
                steer_tx: None,
            },
        );
        started_rx.await.unwrap();
        abort_handle.abort();
        let join_error = pool.join_set.join_next().await.unwrap().unwrap_err();

        let mut queue = EventQueue::new(config::DedupMode::Queue);
        let config = test_config();
        let mut heartbeat_in_flight = false;
        let removed_channels = HashSet::new();
        let mut typing_channels = HashMap::new();
        let mut crash_history = vec![SlotCircuit {
            crash_times: Vec::new(),
            open_until: None,
            respawn_in_flight: false,
        }];
        let (respawn_tx, _respawn_rx) = mpsc::channel(8);
        let mut respawn_tasks = tokio::task::JoinSet::new();
        let observer = ObserverHandle::in_process();

        recover_panicked_agent(
            &mut pool,
            &mut queue,
            &config,
            join_error,
            &mut heartbeat_in_flight,
            &removed_channels,
            &mut typing_channels,
            &mut crash_history,
            &respawn_tx,
            &mut respawn_tasks,
            Some(observer.clone()),
        );

        let panic = observer
            .snapshot()
            .into_iter()
            .find(|event| event.kind == "agent_panic")
            .expect("panic recovery emits an observer event");
        assert_eq!(
            panic.channel_id.as_deref(),
            Some(channel_id.to_string().as_str())
        );
        assert_eq!(panic.turn_id.as_deref(), Some("panic-turn-id"));
    }

    #[tokio::test]
    async fn idle_timeout_emits_exactly_one_feed_event() {
        assert_eq!(
            turn_errors_emitted_for(PromptOutcome::Timeout(TimeoutKind::Idle)).await,
            1
        );
    }

    #[tokio::test]
    async fn hard_timeout_emits_exactly_one_feed_event() {
        assert_eq!(
            turn_errors_emitted_for(PromptOutcome::Timeout(TimeoutKind::Hard {
                recently_active: false
            }))
            .await,
            1
        );
    }

    #[tokio::test]
    async fn cancel_drain_timeout_emits_exactly_one_feed_event() {
        assert_eq!(
            turn_errors_emitted_for(PromptOutcome::CancelDrainTimeout(
                std::time::Duration::from_secs(5)
            ))
            .await,
            1
        );
    }

    /// idle_timeout outcome_label is "idle_timeout"; hard_timeout is "hard_timeout".
    #[tokio::test]
    async fn timeout_outcome_labels_differ() {
        let check_label = |outcome: PromptOutcome, expected_label: &'static str| async move {
            let agent = dummy_agent(0).await;
            let mut pool = AgentPool::from_slots(vec![None]);
            let task_id = pool.join_set.spawn(async {}).id();
            pool.task_map_mut().insert(
                task_id,
                crate::pool::TaskMeta {
                    agent_index: 0,
                    channel_id: None,
                    conversation_key: None,
                    turn_id: "test-turn-id".to_string(),
                    recoverable_batch: None,
                    control_tx: None,
                    steer_tx: None,
                },
            );
            let mut queue = EventQueue::new(config::DedupMode::Queue);
            let config = test_config();
            let mut heartbeat_in_flight = false;
            let removed_channels = HashSet::new();
            let mut crash_history = vec![SlotCircuit {
                crash_times: Vec::new(),
                open_until: None,
                respawn_in_flight: false,
            }];
            let (respawn_tx, _respawn_rx) = mpsc::channel(8);
            let mut respawn_tasks = tokio::task::JoinSet::new();
            let observer = ObserverHandle::in_process();
            let result = PromptResult {
                agent,
                source: PromptSource::Channel(Uuid::new_v4()),
                turn_id: "test-turn-id".to_string(),
                outcome,
                batch: None,
            };
            handle_prompt_result(
                &mut pool,
                &mut queue,
                &config,
                result,
                &mut heartbeat_in_flight,
                &removed_channels,
                &mut crash_history,
                &respawn_tx,
                &mut respawn_tasks,
                Some(observer.clone()),
                None,
                None,
            );
            let events = observer.snapshot();
            let turn_error = events.iter().find(|e| e.kind == "turn_error").unwrap();
            assert_eq!(
                turn_error.payload["outcome"].as_str().unwrap(),
                expected_label
            );
        };
        check_label(PromptOutcome::Timeout(TimeoutKind::Idle), "idle_timeout").await;
        check_label(
            PromptOutcome::Timeout(TimeoutKind::Hard {
                recently_active: false,
            }),
            "hard_timeout",
        )
        .await;
        check_label(
            PromptOutcome::CancelDrainTimeout(std::time::Duration::from_secs(5)),
            "cancel_drain_timeout",
        )
        .await;
    }

    /// hard-cap timeout dead-letters immediately (no requeue); idle timeout is requeued.
    #[tokio::test]
    async fn hard_timeout_not_requeued_idle_timeout_is_requeued() {
        let make_batch = || {
            let keys = Keys::generate();
            let event = EventBuilder::new(Kind::Custom(9), "test")
                .sign_with_keys(&keys)
                .unwrap();
            let channel_id = Uuid::new_v4();
            FlushBatch {
                channel_id,
                conversation_key: test_conversation_key(channel_id),
                events: vec![BatchEvent {
                    event,
                    prompt_tag: "test".into(),
                    received_at: std::time::Instant::now(),
                }],
                cancelled_events: vec![],
                cancel_reason: None,
            }
        };

        // Returns (pending_channels, queued_event_count_for_channel).
        let run = |outcome: PromptOutcome, batch: FlushBatch| async move {
            let channel_id = batch.channel_id;
            let agent = dummy_agent(0).await;
            let mut pool = AgentPool::from_slots(vec![None]);
            let task_id = pool.join_set.spawn(async {}).id();
            pool.task_map_mut().insert(
                task_id,
                crate::pool::TaskMeta {
                    agent_index: 0,
                    channel_id: None,
                    conversation_key: None,
                    turn_id: "test-turn-id".to_string(),
                    recoverable_batch: None,
                    control_tx: None,
                    steer_tx: None,
                },
            );
            let mut queue = EventQueue::new(config::DedupMode::Queue);
            let config = test_config();
            let mut heartbeat_in_flight = false;
            let removed_channels = HashSet::new();
            let mut crash_history = vec![SlotCircuit {
                crash_times: Vec::new(),
                open_until: None,
                respawn_in_flight: false,
            }];
            let (respawn_tx, _respawn_rx) = mpsc::channel(8);
            let mut respawn_tasks = tokio::task::JoinSet::new();
            let result = PromptResult {
                agent,
                source: PromptSource::Channel(channel_id),
                turn_id: "test-turn-id".to_string(),
                outcome,
                batch: Some(batch),
            };
            handle_prompt_result(
                &mut pool,
                &mut queue,
                &config,
                result,
                &mut heartbeat_in_flight,
                &removed_channels,
                &mut crash_history,
                &respawn_tx,
                &mut respawn_tasks,
                None,
                None,
                None,
            );
            (
                queue.pending_channels(),
                queue.queued_event_count(&channel_id),
            )
        };

        // Hard timeout (not recently active): dead-lettered immediately.
        let hard_batch = make_batch();
        let (hard_channels, hard_events) = run(
            PromptOutcome::Timeout(TimeoutKind::Hard {
                recently_active: false,
            }),
            hard_batch,
        )
        .await;
        assert_eq!(
            hard_channels, 0,
            "hard-cap timeout (not recently active) must not requeue the batch"
        );
        assert_eq!(
            hard_events, 0,
            "hard-cap timeout (not recently active) must drop all events"
        );

        // Idle timeout: batch IS requeued (first attempt, not yet dead-lettered).
        let idle_batch = make_batch();
        let (idle_channels, idle_events) =
            run(PromptOutcome::Timeout(TimeoutKind::Idle), idle_batch).await;
        assert_eq!(
            idle_channels, 1,
            "idle timeout must requeue the batch for retry"
        );
        assert_eq!(
            idle_events, 1,
            "idle timeout must preserve the event for retry"
        );
    }

    #[tokio::test]
    async fn hard_timeout_recently_active_requeues_batch() {
        let channel_id = Uuid::new_v4();
        let make_batch = || {
            let keys = Keys::generate();
            let event = EventBuilder::new(Kind::Custom(9), "test")
                .sign_with_keys(&keys)
                .unwrap();
            FlushBatch {
                channel_id,
                conversation_key: test_conversation_key(channel_id),
                events: vec![BatchEvent {
                    event,
                    prompt_tag: "test".into(),
                    received_at: std::time::Instant::now(),
                }],
                cancelled_events: vec![],
                cancel_reason: None,
            }
        };

        let run = |outcome: PromptOutcome, batch: FlushBatch| async move {
            let channel_id = batch.channel_id;
            let agent = dummy_agent(0).await;
            let mut pool = AgentPool::from_slots(vec![None]);
            let task_id = pool.join_set.spawn(async {}).id();
            pool.task_map_mut().insert(
                task_id,
                crate::pool::TaskMeta {
                    agent_index: 0,
                    channel_id: None,
                    conversation_key: None,
                    turn_id: "test-turn-id".to_string(),
                    recoverable_batch: None,
                    control_tx: None,
                    steer_tx: None,
                },
            );
            let mut queue = EventQueue::new(config::DedupMode::Queue);
            let config = test_config();
            let mut heartbeat_in_flight = false;
            let removed_channels = HashSet::new();
            let mut crash_history = vec![SlotCircuit {
                crash_times: Vec::new(),
                open_until: None,
                respawn_in_flight: false,
            }];
            let (respawn_tx, _respawn_rx) = mpsc::channel(8);
            let mut respawn_tasks = tokio::task::JoinSet::new();
            let result = PromptResult {
                agent,
                source: PromptSource::Channel(channel_id),
                turn_id: "test-turn-id".to_string(),
                outcome,
                batch: Some(batch),
            };
            handle_prompt_result(
                &mut pool,
                &mut queue,
                &config,
                result,
                &mut heartbeat_in_flight,
                &removed_channels,
                &mut crash_history,
                &respawn_tx,
                &mut respawn_tasks,
                None,
                None,
                None,
            );
            (
                queue.pending_channels(),
                queue.queued_event_count(&channel_id),
            )
        };

        let batch = make_batch();
        let (channels, events) = run(
            PromptOutcome::Timeout(TimeoutKind::Hard {
                recently_active: true,
            }),
            batch,
        )
        .await;
        assert_eq!(
            channels, 1,
            "hard-cap timeout with recent activity must requeue the batch"
        );
        assert_eq!(
            events, 1,
            "hard-cap timeout with recent activity must preserve the event"
        );
    }

    /// The hard-timeout `death_message` must report what actually happened to
    /// the batch, not just the `recently_active` eligibility flag: a
    /// recently-active batch within its retry budget is requeued, so the
    /// observer payload must say so.
    #[tokio::test]
    async fn hard_timeout_recently_active_requeue_success_reports_requeued_for_retry() {
        let channel_id = Uuid::new_v4();
        let agent = dummy_agent(0).await;
        let mut pool = AgentPool::from_slots(vec![None]);
        let task_id = pool.join_set.spawn(async {}).id();
        pool.task_map_mut().insert(
            task_id,
            crate::pool::TaskMeta {
                agent_index: 0,
                channel_id: None,
                conversation_key: None,
                turn_id: "test-turn-id".to_string(),
                recoverable_batch: None,
                control_tx: None,
                steer_tx: None,
            },
        );
        let mut queue = EventQueue::new(config::DedupMode::Queue);
        let config = test_config();
        let mut heartbeat_in_flight = false;
        let removed_channels = HashSet::new();
        let mut crash_history = vec![SlotCircuit {
            crash_times: Vec::new(),
            open_until: None,
            respawn_in_flight: false,
        }];
        let (respawn_tx, _respawn_rx) = mpsc::channel(8);
        let mut respawn_tasks = tokio::task::JoinSet::new();
        let observer = ObserverHandle::in_process();
        let batch = FlushBatch {
            channel_id,
            conversation_key: test_conversation_key(channel_id),
            events: vec![BatchEvent {
                event: EventBuilder::new(Kind::Custom(9), "test")
                    .sign_with_keys(&Keys::generate())
                    .unwrap(),
                prompt_tag: "test".into(),
                received_at: std::time::Instant::now(),
            }],
            cancelled_events: vec![],
            cancel_reason: None,
        };
        let result = PromptResult {
            agent,
            source: PromptSource::Channel(channel_id),
            turn_id: "test-turn-id".to_string(),
            outcome: PromptOutcome::Timeout(TimeoutKind::Hard {
                recently_active: true,
            }),
            batch: Some(batch),
        };
        handle_prompt_result(
            &mut pool,
            &mut queue,
            &config,
            result,
            &mut heartbeat_in_flight,
            &removed_channels,
            &mut crash_history,
            &respawn_tx,
            &mut respawn_tasks,
            Some(observer.clone()),
            None,
            None,
        );

        let events = observer.snapshot();
        let turn_error = events
            .iter()
            .find(|e| e.kind == "turn_error")
            .expect("exactly one turn_error event must be emitted");
        assert_eq!(
            turn_error.payload["error"].as_str().unwrap(),
            format!(
                "Agent turn exceeded the maximum duration ({}s) — requeued for retry (recently active)",
                config.max_turn_duration_secs
            ),
        );
        assert_eq!(
            queue.pending_channels(),
            1,
            "batch must be requeued, not dead-lettered, while within the retry budget"
        );
    }

    /// Same recently-active hard timeout, but the channel has already
    /// exhausted its retry budget ([`crate::queue::MAX_RETRIES`] prior
    /// attempts) — `queue.requeue()` dead-letters instead of requeueing, and
    /// the observer payload must report that fate, not the requeue wording
    /// above.
    #[tokio::test]
    async fn hard_timeout_recently_active_budget_exhausted_reports_dead_lettered() {
        let channel_id = Uuid::new_v4();
        let mut queue = EventQueue::new(config::DedupMode::Queue);
        // Simulate MAX_RETRIES prior failed attempts on this channel so the
        // upcoming requeue() call in handle_prompt_result crosses the
        // dead-letter threshold.
        queue.set_retry_count_for_test(channel_id, crate::queue::MAX_RETRIES);

        let agent = dummy_agent(0).await;
        let mut pool = AgentPool::from_slots(vec![None]);
        let task_id = pool.join_set.spawn(async {}).id();
        pool.task_map_mut().insert(
            task_id,
            crate::pool::TaskMeta {
                agent_index: 0,
                channel_id: None,
                conversation_key: None,
                turn_id: "test-turn-id".to_string(),
                recoverable_batch: None,
                control_tx: None,
                steer_tx: None,
            },
        );
        let config = test_config();
        let mut heartbeat_in_flight = false;
        let removed_channels = HashSet::new();
        let mut crash_history = vec![SlotCircuit {
            crash_times: Vec::new(),
            open_until: None,
            respawn_in_flight: false,
        }];
        let (respawn_tx, _respawn_rx) = mpsc::channel(8);
        let mut respawn_tasks = tokio::task::JoinSet::new();
        let observer = ObserverHandle::in_process();
        let batch = FlushBatch {
            channel_id,
            conversation_key: test_conversation_key(channel_id),
            events: vec![BatchEvent {
                event: EventBuilder::new(Kind::Custom(9), "final-attempt")
                    .sign_with_keys(&Keys::generate())
                    .unwrap(),
                prompt_tag: "test".into(),
                received_at: std::time::Instant::now(),
            }],
            cancelled_events: vec![],
            cancel_reason: None,
        };
        let result = PromptResult {
            agent,
            source: PromptSource::Channel(channel_id),
            turn_id: "test-turn-id".to_string(),
            outcome: PromptOutcome::Timeout(TimeoutKind::Hard {
                recently_active: true,
            }),
            batch: Some(batch),
        };
        handle_prompt_result(
            &mut pool,
            &mut queue,
            &config,
            result,
            &mut heartbeat_in_flight,
            &removed_channels,
            &mut crash_history,
            &respawn_tx,
            &mut respawn_tasks,
            Some(observer.clone()),
            None,
            None,
        );

        let events = observer.snapshot();
        let turn_error = events
            .iter()
            .find(|e| e.kind == "turn_error")
            .expect("exactly one turn_error event must be emitted");
        assert_eq!(
            turn_error.payload["error"].as_str().unwrap(),
            format!(
                "Agent turn exceeded the maximum duration ({}s) — dead-lettered (retry budget exhausted)",
                config.max_turn_duration_secs
            ),
        );
        assert_eq!(
            queue.queued_event_count(&channel_id),
            0,
            "batch with an exhausted retry budget must be dead-lettered, not requeued"
        );
    }

    /// Cancel-drain-timeout batches are requeued as cancelled (merge into the
    /// next flush, `CancelReason` preserved) — never dead-lettered like a real
    /// hard-cap. The agent itself is NOT returned to the idle pool: it is
    /// handed to `spawn_respawn_task` instead, mirroring a fatal `Timeout`.
    ///
    /// This reproduces the full steer-fallback incident, not just the
    /// original batch in isolation: the steer ack handler already released
    /// the new triggering event back to `queue` (`lib.rs`'s
    /// `ExpectedRunIdMissing` path) before the cancel-drain expiry fires. The
    /// next `flush_next()` must merge the surviving original event (via
    /// `cancelled_events`) with that already-queued new event (via `events`)
    /// exactly once each — proving no loss and no duplication.
    #[tokio::test]
    async fn cancel_drain_timeout_requeues_batch_and_does_not_return_agent() {
        let keys = Keys::generate();
        let original_event = EventBuilder::new(Kind::Custom(9), "original")
            .sign_with_keys(&keys)
            .unwrap();
        let new_event = EventBuilder::new(Kind::Custom(9), "new")
            .sign_with_keys(&keys)
            .unwrap();
        assert_ne!(
            original_event.id, new_event.id,
            "test fixture must use two distinct events"
        );
        let channel_id = Uuid::new_v4();
        let batch = FlushBatch {
            channel_id,
            conversation_key: test_conversation_key(channel_id),
            events: vec![BatchEvent {
                event: original_event.clone(),
                prompt_tag: "test".into(),
                received_at: std::time::Instant::now(),
            }],
            cancelled_events: vec![],
            cancel_reason: Some(CancelReason::Steer),
        };

        let agent = dummy_agent(0).await;
        let mut pool = AgentPool::from_slots(vec![None]);
        let task_id = pool.join_set.spawn(async {}).id();
        pool.task_map_mut().insert(
            task_id,
            crate::pool::TaskMeta {
                agent_index: 0,
                channel_id: None,
                conversation_key: None,
                turn_id: "test-turn-id".to_string(),
                recoverable_batch: None,
                control_tx: None,
                steer_tx: None,
            },
        );
        let mut queue = EventQueue::new(config::DedupMode::Queue);
        // The steer ack handler releases the new event to the queue BEFORE
        // signaling the fallback ControlSignal::Steer that ultimately times
        // out on drain — so it is already queued by the time
        // handle_prompt_result runs.
        queue.push(QueuedEvent {
            channel_id,
            event: new_event.clone(),
            received_at: std::time::Instant::now(),
            prompt_tag: "test".into(),
        });
        let config = test_config();
        let mut heartbeat_in_flight = false;
        let removed_channels = HashSet::new();
        let mut crash_history = vec![SlotCircuit {
            crash_times: Vec::new(),
            open_until: None,
            respawn_in_flight: false,
        }];
        let (respawn_tx, _respawn_rx) = mpsc::channel(8);
        let mut respawn_tasks = tokio::task::JoinSet::new();
        let observer = ObserverHandle::in_process();
        let grace = std::time::Duration::from_secs(5);
        let result = PromptResult {
            agent,
            source: PromptSource::Channel(channel_id),
            turn_id: "test-turn-id".to_string(),
            outcome: PromptOutcome::CancelDrainTimeout(grace),
            batch: Some(batch),
        };

        handle_prompt_result(
            &mut pool,
            &mut queue,
            &config,
            result,
            &mut heartbeat_in_flight,
            &removed_channels,
            &mut crash_history,
            &respawn_tx,
            &mut respawn_tasks,
            Some(observer.clone()),
            None,
            None,
        );

        // Batch preserved as a cancelled merge, not dead-lettered — same
        // treatment as a normal `Cancelled` outcome. `handle_prompt_result`
        // already called `mark_complete` internally, releasing the channel.
        // `flush_next()` must merge the already-queued new event with the
        // preserved original: each exactly once, in the correct bucket.
        let requeued = queue.flush_next().expect("batch must be requeued");
        assert_eq!(
            requeued.events.len(),
            1,
            "exactly one new event must be in the regular events bucket"
        );
        assert_eq!(
            requeued.events[0].event.id, new_event.id,
            "the regular events bucket must hold the new (already-queued) event"
        );
        assert_eq!(
            requeued.cancelled_events.len(),
            1,
            "exactly one original event must be in the cancelled_events bucket"
        );
        assert_eq!(
            requeued.cancelled_events[0].event.id, original_event.id,
            "the cancelled_events bucket must hold the original (interrupted) event"
        );
        assert_ne!(
            requeued.events[0].event.id, requeued.cancelled_events[0].event.id,
            "the new and original events must not be the same event"
        );
        assert_eq!(
            requeued.cancel_reason,
            Some(CancelReason::Steer),
            "CancelReason must ride through to the requeued batch"
        );

        // Agent must NOT be back in the idle pool — it was handed to respawn.
        assert_eq!(
            pool.live_count(),
            0,
            "agent must not be returned to the pool after a cancel-drain timeout"
        );
        assert_eq!(
            respawn_tasks.len(),
            1,
            "a respawn task must be spawned for the poisoned agent"
        );

        // The observer payload must be fate-neutral: it names the grace and
        // the process replacement, and must NOT claim the batch was
        // preserved — that claim is false for explicit Stop/removed-channel
        // drops (see the sibling dropped-Stop test below), so the same
        // wording is used regardless of fate.
        let events = observer.snapshot();
        let turn_error = events
            .iter()
            .find(|e| e.kind == "turn_error")
            .expect("exactly one turn_error event must be emitted");
        assert_eq!(
            turn_error.payload["outcome"].as_str().unwrap(),
            "cancel_drain_timeout"
        );
        assert_eq!(
            turn_error.payload["error"].as_str().unwrap(),
            format!("Agent did not stop within {grace:?} after cancellation; the agent process is being replaced."),
            "observer message must name the actual grace and must not claim preservation"
        );
        assert_eq!(
            events.iter().filter(|e| e.kind == "turn_error").count(),
            1,
            "exactly one turn_error event must be emitted"
        );
    }

    /// Explicit Stop (`ControlSignal::Cancel`) on cancel-drain expiry drops
    /// the triggering batch — `requeue_cancelled_batch` returns `None` for
    /// `Cancel`/`Rotate`. The observer payload must be the SAME fate-neutral
    /// text as the preserved-Steer case above: it must never claim work was
    /// preserved when it was intentionally discarded. The poisoned agent is
    /// still respawned exactly as in the preserved case.
    #[tokio::test]
    async fn cancel_drain_timeout_dropped_stop_batch_none_same_neutral_payload() {
        let agent = dummy_agent(0).await;
        let mut pool = AgentPool::from_slots(vec![None]);
        let task_id = pool.join_set.spawn(async {}).id();
        pool.task_map_mut().insert(
            task_id,
            crate::pool::TaskMeta {
                agent_index: 0,
                channel_id: None,
                conversation_key: None,
                turn_id: "test-turn-id".to_string(),
                recoverable_batch: None,
                control_tx: None,
                steer_tx: None,
            },
        );
        let mut queue = EventQueue::new(config::DedupMode::Queue);
        let config = test_config();
        let mut heartbeat_in_flight = false;
        let removed_channels = HashSet::new();
        let mut crash_history = vec![SlotCircuit {
            crash_times: Vec::new(),
            open_until: None,
            respawn_in_flight: false,
        }];
        let (respawn_tx, _respawn_rx) = mpsc::channel(8);
        let mut respawn_tasks = tokio::task::JoinSet::new();
        let observer = ObserverHandle::in_process();
        let grace = std::time::Duration::from_secs(5);
        let result = PromptResult {
            agent,
            source: PromptSource::Channel(Uuid::new_v4()),
            turn_id: "test-turn-id".to_string(),
            outcome: PromptOutcome::CancelDrainTimeout(grace),
            // Explicit Stop already dropped the batch upstream in
            // `classify_control_cancel_failure` — `handle_prompt_result`
            // never sees one to requeue.
            batch: None,
        };

        handle_prompt_result(
            &mut pool,
            &mut queue,
            &config,
            result,
            &mut heartbeat_in_flight,
            &removed_channels,
            &mut crash_history,
            &respawn_tx,
            &mut respawn_tasks,
            Some(observer.clone()),
            None,
            None,
        );

        // No batch to merge — the queue has nothing pending for any channel.
        assert_eq!(
            queue.pending_channels(),
            0,
            "a dropped Stop batch must not leave anything queued"
        );

        // Same respawn treatment as the preserved case: never returned idle.
        assert_eq!(
            pool.live_count(),
            0,
            "agent must not be returned to the pool after a cancel-drain timeout"
        );
        assert_eq!(
            respawn_tasks.len(),
            1,
            "a respawn task must be spawned for the poisoned agent"
        );

        // The observer payload is byte-identical to the preserved-Steer case:
        // fate-neutral, naming the grace, with no preservation claim.
        let events = observer.snapshot();
        let turn_error = events
            .iter()
            .find(|e| e.kind == "turn_error")
            .expect("exactly one turn_error event must be emitted");
        assert_eq!(
            turn_error.payload["outcome"].as_str().unwrap(),
            "cancel_drain_timeout"
        );
        assert_eq!(
            turn_error.payload["error"].as_str().unwrap(),
            format!("Agent did not stop within {grace:?} after cancellation; the agent process is being replaced."),
            "observer message must be fate-neutral even though the batch was dropped"
        );
        assert_eq!(
            events.iter().filter(|e| e.kind == "turn_error").count(),
            1,
            "exactly one turn_error event must be emitted"
        );
    }

    #[tokio::test]
    async fn transport_error_emits_exactly_one_feed_event() {
        let io = AcpError::Io(std::io::Error::other("pipe broke"));
        assert_eq!(turn_errors_emitted_for(PromptOutcome::Error(io)).await, 1);
    }

    #[tokio::test]
    async fn application_error_emits_exactly_one_feed_event() {
        let app = AcpError::IdleTimeout(std::time::Duration::from_secs(1));
        assert_eq!(turn_errors_emitted_for(PromptOutcome::Error(app)).await, 1);
    }

    // ── is_auth_error classification ───────────────────────────────────────

    #[test]
    fn is_auth_error_matches_reauthenticate_message() {
        let e = acp::AcpError::AgentError {
            code: -32000,
            message: "API Error: OAuth access token has expired. Re-authenticate to continue."
                .to_string(),
        };
        assert!(
            is_auth_error(&e),
            "Re-authenticate variant must be classified as auth error"
        );
    }

    #[test]
    fn is_auth_error_matches_401_message() {
        let e = acp::AcpError::AgentError {
            code: -32000,
            message: "Internal error: API Error: 401 OAuth access token has expired.".to_string(),
        };
        assert!(
            is_auth_error(&e),
            "API Error: 401 variant must be classified as auth error"
        );
    }

    #[test]
    fn is_auth_error_rejects_other_agent_error_message() {
        let e = acp::AcpError::AgentError {
            code: -32601,
            message: "Usage credits required for 1M context — turn on usage credits".to_string(),
        };
        assert!(
            !is_auth_error(&e),
            "usage-credit error must NOT be classified as auth error"
        );
    }

    #[test]
    fn is_auth_error_rejects_transport_errors() {
        let io = acp::AcpError::Io(std::io::Error::other("pipe broke"));
        assert!(
            !is_auth_error(&io),
            "I/O error must not be classified as auth error"
        );
        let timeout = acp::AcpError::WriteTimeout(std::time::Duration::from_secs(5));
        assert!(
            !is_auth_error(&timeout),
            "WriteTimeout must not be classified as auth error"
        );
    }

    #[test]
    fn pi_context_limit_code_is_stable_and_non_retryable() {
        let limit = acp::AcpError::AgentError {
            code: PI_CONTEXT_LIMIT_ERROR_CODE,
            message: "untrusted adapter prose is not part of classification".to_string(),
        };
        let other = acp::AcpError::AgentError {
            code: -32000,
            message: "context words alone must not trigger dead-lettering".to_string(),
        };

        assert!(is_pi_context_limit_error(&limit));
        assert!(!is_pi_context_limit_error(&other));
    }

    #[test]
    fn pi_storage_limit_code_and_recovery_notice_are_stable() {
        let limit = acp::AcpError::AgentError {
            code: PI_SESSION_STORAGE_LIMIT_ERROR_CODE,
            message: "adapter detail must not drive classification".to_string(),
        };
        let other = acp::AcpError::AgentError {
            code: -32602,
            message: "BUZZ_SESSION_STORAGE_LIMIT text alone is untrusted".to_string(),
        };
        assert!(is_pi_session_storage_limit_error(&limit));
        assert!(!is_pi_session_storage_limit_error(&other));
        assert!(pi_session_storage_limit_notice().contains("`/new`"));
        assert!(pi_session_storage_limit_notice().contains("without retrying"));
    }

    #[test]
    fn pi_session_invalidation_code_is_stable_and_exact() {
        let invalidated = acp::AcpError::AgentError {
            code: PI_SESSION_INVALIDATED_ERROR_CODE,
            message: "safe adapter detail does not drive classification".to_string(),
        };
        let text_only = acp::AcpError::AgentError {
            code: -32603,
            message: "BUZZ_PI_SESSION_INVALIDATED".to_string(),
        };
        assert!(is_pi_session_invalidated_error(&invalidated));
        assert!(!is_pi_session_invalidated_error(&text_only));
    }

    #[tokio::test]
    async fn pi_session_invalidation_requeues_exact_batch_and_replaces_agent() {
        let channel_id = Uuid::new_v4();
        let event = nostr::EventBuilder::new(nostr::Kind::Custom(9), "reload safely")
            .sign_with_keys(&nostr::Keys::generate())
            .expect("sign invalidation fixture");
        let event_id = event.id.to_hex();
        let batch = FlushBatch {
            channel_id,
            conversation_key: test_conversation_key(channel_id),
            events: vec![BatchEvent {
                event,
                prompt_tag: "test".into(),
                received_at: std::time::Instant::now(),
            }],
            cancelled_events: vec![],
            cancel_reason: None,
        };
        let agent = dummy_agent(0).await;
        let mut pool = AgentPool::from_slots(vec![None]);
        let task_id = pool.join_set.spawn(async {}).id();
        pool.task_map_mut().insert(
            task_id,
            crate::pool::TaskMeta {
                agent_index: 0,
                channel_id: Some(channel_id),
                conversation_key: None,
                turn_id: "reload-invalidated-turn".to_string(),
                recoverable_batch: None,
                control_tx: None,
                steer_tx: None,
            },
        );
        let mut queue = EventQueue::new(config::DedupMode::Queue);
        let config = test_config();
        let mut heartbeat_in_flight = false;
        let removed_channels = HashSet::new();
        let mut crash_history = vec![SlotCircuit {
            crash_times: Vec::new(),
            open_until: None,
            respawn_in_flight: false,
        }];
        let (respawn_tx, _respawn_rx) = mpsc::channel(8);
        let mut respawn_tasks = tokio::task::JoinSet::new();

        handle_prompt_result(
            &mut pool,
            &mut queue,
            &config,
            PromptResult {
                agent,
                source: PromptSource::Channel(channel_id),
                turn_id: "reload-invalidated-turn".to_string(),
                outcome: PromptOutcome::Error(acp::AcpError::AgentError {
                    code: PI_SESSION_INVALIDATED_ERROR_CODE,
                    message: "Pi session was invalidated after reload failure".to_string(),
                }),
                batch: Some(batch),
            },
            &mut heartbeat_in_flight,
            &removed_channels,
            &mut crash_history,
            &respawn_tx,
            &mut respawn_tasks,
            None,
            None,
            None,
        );

        assert_eq!(pool.live_count(), 0, "invalid proxy must never be reused");
        assert_eq!(respawn_tasks.len(), 1, "ACP process must be replaced");
        assert_eq!(queue.queued_event_count(&channel_id), 1);
        assert_eq!(
            queue.drain_channel(channel_id),
            vec![event_id],
            "the exact triggering event must survive for the fresh process"
        );
    }

    // ── auth error dead-letter behavior ────────────────────────────────────

    /// An auth-class `PromptOutcome::Error` must dead-letter immediately
    /// (the batch is never requeued) so the user sees a re-auth hint at once
    /// rather than after 10 futile retries.
    #[tokio::test]
    async fn auth_error_dead_letters_immediately_without_requeueing() {
        let keys = nostr::Keys::generate();
        let event = nostr::EventBuilder::new(nostr::Kind::Custom(9), "test")
            .sign_with_keys(&keys)
            .unwrap();
        let channel_id = uuid::Uuid::new_v4();
        let batch = FlushBatch {
            channel_id,
            conversation_key: test_conversation_key(channel_id),
            events: vec![BatchEvent {
                event,
                prompt_tag: "test".into(),
                received_at: std::time::Instant::now(),
            }],
            cancelled_events: vec![],
            cancel_reason: None,
        };

        let auth_error = acp::AcpError::AgentError {
            code: -32000,
            message: "API Error: 401 OAuth access token has expired. Re-authenticate to continue."
                .to_string(),
        };

        let agent = dummy_agent(0).await;
        let mut pool = AgentPool::from_slots(vec![None]);
        let task_id = pool.join_set.spawn(async {}).id();
        pool.task_map_mut().insert(
            task_id,
            crate::pool::TaskMeta {
                agent_index: 0,
                channel_id: None,
                conversation_key: None,
                turn_id: "test-turn-id".to_string(),
                recoverable_batch: None,
                control_tx: None,
                steer_tx: None,
            },
        );
        let mut queue = EventQueue::new(config::DedupMode::Queue);
        let config = test_config();
        let mut heartbeat_in_flight = false;
        let removed_channels = std::collections::HashSet::new();
        let mut crash_history = vec![SlotCircuit {
            crash_times: Vec::new(),
            open_until: None,
            respawn_in_flight: false,
        }];
        let (respawn_tx, _respawn_rx) = mpsc::channel(8);
        let mut respawn_tasks = tokio::task::JoinSet::new();
        let result = PromptResult {
            agent,
            source: PromptSource::Channel(channel_id),
            turn_id: "test-turn-id".to_string(),
            outcome: PromptOutcome::Error(auth_error),
            batch: Some(batch),
        };
        handle_prompt_result(
            &mut pool,
            &mut queue,
            &config,
            result,
            &mut heartbeat_in_flight,
            &removed_channels,
            &mut crash_history,
            &respawn_tx,
            &mut respawn_tasks,
            None,
            None,
            None,
        );

        // The batch must not be requeued: pending_channels returns 0.
        assert_eq!(
            queue.pending_channels(),
            0,
            "auth error must dead-letter immediately — batch must not be requeued"
        );
        assert_eq!(
            queue.queued_event_count(&channel_id),
            0,
            "auth error must dead-letter immediately — no events should be pending"
        );
    }

    #[tokio::test]
    async fn pi_storage_exhaustion_dead_letters_immediately_without_requeueing() {
        let channel_id = uuid::Uuid::new_v4();
        let event = nostr::EventBuilder::new(nostr::Kind::Custom(9), "test")
            .sign_with_keys(&nostr::Keys::generate())
            .expect("sign test event");
        let batch = FlushBatch {
            channel_id,
            conversation_key: test_conversation_key(channel_id),
            events: vec![BatchEvent {
                event,
                prompt_tag: "test".into(),
                received_at: std::time::Instant::now(),
            }],
            cancelled_events: vec![],
            cancel_reason: None,
        };
        let agent = dummy_agent(0).await;
        let mut pool = AgentPool::from_slots(vec![None]);
        let task_id = pool.join_set.spawn(async {}).id();
        pool.task_map_mut().insert(
            task_id,
            crate::pool::TaskMeta {
                agent_index: 0,
                channel_id: Some(channel_id),
                conversation_key: None,
                turn_id: "storage-limit-turn".to_string(),
                recoverable_batch: None,
                control_tx: None,
                steer_tx: None,
            },
        );
        let mut queue = EventQueue::new(config::DedupMode::Queue);
        let config = test_config();
        let mut heartbeat_in_flight = false;
        let removed_channels = HashSet::new();
        let mut crash_history = vec![SlotCircuit {
            crash_times: Vec::new(),
            open_until: None,
            respawn_in_flight: false,
        }];
        let (respawn_tx, _respawn_rx) = mpsc::channel(8);
        let mut respawn_tasks = tokio::task::JoinSet::new();
        let notice_keys = nostr::Keys::generate();
        let rest = relay::RestClient {
            http: reqwest::Client::new(),
            base_url: "http://127.0.0.1:9".to_owned(),
            keys: notice_keys.clone(),
            auth_tag_json: None,
        };
        let state_dir = tempfile::tempdir().expect("durable state tempdir");
        let durable = DurableOutbox::open(state_dir.path(), &notice_keys.public_key().to_hex())
            .expect("open durable outbox");
        let outbox = LifecycleNoticeOutbox::spawn(rest.clone(), durable.clone())
            .expect("spawn durable notice worker");
        handle_prompt_result(
            &mut pool,
            &mut queue,
            &config,
            PromptResult {
                agent,
                source: PromptSource::Channel(channel_id),
                turn_id: "storage-limit-turn".to_string(),
                outcome: PromptOutcome::Error(acp::AcpError::AgentError {
                    code: PI_SESSION_STORAGE_LIMIT_ERROR_CODE,
                    message: "quota exhausted".to_string(),
                }),
                batch: Some(batch),
            },
            &mut heartbeat_in_flight,
            &removed_channels,
            &mut crash_history,
            &respawn_tx,
            &mut respawn_tasks,
            None,
            Some(&outbox),
            Some(&rest),
        );
        assert_eq!(queue.pending_channels(), 0);
        assert_eq!(queue.queued_event_count(&channel_id), 0);
        assert_eq!(
            durable
                .pending_lifecycle()
                .expect("read pending lifecycle")
                .len(),
            1,
            "dead-lettering is allowed only after the recovery notice is crash-durable"
        );
    }

    /// A non-auth application error (e.g. usage credits) must still follow the
    /// standard requeue path so today's behavior is unchanged.
    #[tokio::test]
    async fn non_auth_application_error_is_requeued() {
        let keys = nostr::Keys::generate();
        let event = nostr::EventBuilder::new(nostr::Kind::Custom(9), "test")
            .sign_with_keys(&keys)
            .unwrap();
        let channel_id = uuid::Uuid::new_v4();
        let batch = FlushBatch {
            channel_id,
            conversation_key: test_conversation_key(channel_id),
            events: vec![BatchEvent {
                event,
                prompt_tag: "test".into(),
                received_at: std::time::Instant::now(),
            }],
            cancelled_events: vec![],
            cancel_reason: None,
        };

        // Usage-credits error — AgentError but NOT an auth error.
        let usage_error = acp::AcpError::AgentError {
            code: -32000,
            message: "Usage credits required for 1M context".to_string(),
        };

        let agent = dummy_agent(0).await;
        let mut pool = AgentPool::from_slots(vec![None]);
        let task_id = pool.join_set.spawn(async {}).id();
        pool.task_map_mut().insert(
            task_id,
            crate::pool::TaskMeta {
                agent_index: 0,
                channel_id: None,
                conversation_key: None,
                turn_id: "test-turn-id".to_string(),
                recoverable_batch: None,
                control_tx: None,
                steer_tx: None,
            },
        );
        let mut queue = EventQueue::new(config::DedupMode::Queue);
        let config = test_config();
        let mut heartbeat_in_flight = false;
        let removed_channels = std::collections::HashSet::new();
        let mut crash_history = vec![SlotCircuit {
            crash_times: Vec::new(),
            open_until: None,
            respawn_in_flight: false,
        }];
        let (respawn_tx, _respawn_rx) = mpsc::channel(8);
        let mut respawn_tasks = tokio::task::JoinSet::new();
        let result = PromptResult {
            agent,
            source: PromptSource::Channel(channel_id),
            turn_id: "test-turn-id".to_string(),
            outcome: PromptOutcome::Error(usage_error),
            batch: Some(batch),
        };
        handle_prompt_result(
            &mut pool,
            &mut queue,
            &config,
            result,
            &mut heartbeat_in_flight,
            &removed_channels,
            &mut crash_history,
            &respawn_tx,
            &mut respawn_tasks,
            None,
            None,
            None,
        );

        // Non-auth application error: batch IS requeued (first attempt, retry budget > 0).
        assert_eq!(
            queue.pending_channels(),
            1,
            "non-auth application error must requeue the batch for retry"
        );
        assert_eq!(
            queue.queued_event_count(&channel_id),
            1,
            "non-auth application error must preserve the event for retry"
        );
    }
}

#[cfg(test)]
mod observer_payload_trim_tests {
    use super::*;

    fn event_with_payload(kind: &str, payload: serde_json::Value) -> observer::ObserverEvent {
        observer::ObserverEvent {
            seq: 1,
            timestamp: "2026-06-16T00:00:00Z".to_string(),
            kind: kind.to_string(),
            agent_index: Some(0),
            channel_id: Some("11111111-1111-1111-1111-111111111111".to_string()),
            session_id: Some("sess-1".to_string()),
            turn_id: Some("turn-1".to_string()),
            started_at: None,
            payload,
        }
    }

    fn serialized(event: &observer::ObserverEvent) -> String {
        serde_json::to_string(event).unwrap()
    }

    #[test]
    fn test_under_budget_frame_passes_through_byte_identical() {
        let mut event = event_with_payload("acp_read", serde_json::json!({ "body": "small" }));
        let before = serialized(&event);
        fit_observer_event_to_budget(&mut event);
        assert_eq!(
            serialized(&event),
            before,
            "under-budget frame must not be mutated"
        );
    }

    #[test]
    fn test_single_giant_leaf_is_elided_to_fit_with_envelope_intact() {
        let big = "x".repeat(100_000);
        let mut event = event_with_payload("acp_read", serde_json::json!({ "body": big }));
        fit_observer_event_to_budget(&mut event);

        assert!(
            serialized(&event).len() <= OBSERVER_MAX_PLAINTEXT_LEN,
            "frame must fit after trimming"
        );
        // Envelope intact.
        assert_eq!(event.kind, "acp_read");
        assert_eq!(event.turn_id.as_deref(), Some("turn-1"));
        assert_eq!(
            event.channel_id.as_deref(),
            Some("11111111-1111-1111-1111-111111111111")
        );
        assert_eq!(event.seq, 1);

        let leaf = event.payload["body"].as_str().unwrap();
        assert!(
            leaf.starts_with(&"x".repeat(OBSERVER_LEAF_RETAIN_BYTES)),
            "head retained"
        );
        assert!(
            leaf.ends_with(&"x".repeat(OBSERVER_LEAF_RETAIN_BYTES)),
            "tail retained"
        );
        // N in the marker is RAW bytes removed: original len minus retained len.
        let removed = 100_000 - leaf.chars().filter(|c| *c == 'x').count();
        assert!(
            leaf.contains(&format!("…[elided {removed} bytes]…")),
            "marker reports raw bytes removed"
        );
    }

    #[test]
    fn test_multi_block_prompt_retains_every_section_header_after_elision() {
        // The real session/prompt fix: format_prompt now emits one block per
        // section, so the observer payload is params.prompt = [{text: "[Base]…"},
        // {text: "[Agent Memory — core]…"}, … {text: "[Buzz event: …]…<huge>"}].
        // An oversized section is its own leaf, so eliding its body keeps the
        // leaf's head-3000 (which begins with the section's [Header] line) — every
        // header survives, so the desktop "Prompt context" panel counts them all.
        // This is the regression the single-fat-leaf shape caused (the trailing
        // [Buzz event] header fell into the elided middle and the count collapsed
        // to 1).
        let sections = [
            "[Base]\nyou are a helpful agent".to_string(),
            "[System]\npersona text".to_string(),
            "[Agent Memory — core]\nremember this".to_string(),
            "[Context]\nScope: thread".to_string(),
            // The triggering event body, oversized on its own.
            format!("[Buzz event: @mention]\nContent: {}", "E".repeat(90_000)),
        ];
        let block_refs: Vec<&str> = sections.iter().map(String::as_str).collect();
        // Mirror the wire shape build_prompt_params produces: each block is its
        // own {type:"text", text} leaf under params.prompt.
        let prompt_blocks: Vec<serde_json::Value> = block_refs
            .iter()
            .map(|text| serde_json::json!({ "type": "text", "text": text }))
            .collect();
        let mut event = event_with_payload(
            "acp_write",
            serde_json::json!({
                "method": "session/prompt",
                "params": { "sessionId": "sess-1", "prompt": prompt_blocks },
            }),
        );
        assert!(
            serialized(&event).len() > OBSERVER_MAX_PLAINTEXT_LEN,
            "precondition: oversized event body pushes the frame over the cap"
        );

        fit_observer_event_to_budget(&mut event);

        assert!(
            serialized(&event).len() <= OBSERVER_MAX_PLAINTEXT_LEN,
            "frame must fit after trimming"
        );
        let blocks = event.payload["params"]["prompt"]
            .as_array()
            .expect("prompt array survives");
        let texts: Vec<&str> = blocks.iter().map(|b| b["text"].as_str().unwrap()).collect();
        for header in [
            "[Base]",
            "[System]",
            "[Agent Memory — core]",
            "[Context]",
            "[Buzz event: @mention]",
        ] {
            assert!(
                texts.iter().any(|t| t.starts_with(header)),
                "section header {header} must survive at the head of its own block"
            );
        }
        // The oversized event body was elided in place (header kept, middle cut).
        let event_block = texts
            .iter()
            .find(|t| t.starts_with("[Buzz event: @mention]"))
            .unwrap();
        assert!(
            event_block.contains("…[elided"),
            "the oversized event body is elided, not dropped"
        );
    }

    #[test]
    fn test_multi_leaf_elides_largest_shrinkable_first_and_stops_when_it_fits() {
        // One leaf alone over the cap; a second smaller-but-still-large leaf.
        // Eliding the biggest should suffice, leaving the smaller intact.
        let mut event = event_with_payload(
            "acp_write",
            serde_json::json!({
                "huge": "a".repeat(90_000),
                "medium": "b".repeat(20_000),
            }),
        );
        fit_observer_event_to_budget(&mut event);

        assert!(serialized(&event).len() <= OBSERVER_MAX_PLAINTEXT_LEN);
        assert!(
            event.payload["huge"].as_str().unwrap().contains("…[elided"),
            "the largest leaf is elided"
        );
        assert_eq!(
            event.payload["medium"].as_str().unwrap().len(),
            20_000,
            "the smaller leaf is left untouched once the frame fits"
        );
    }

    #[test]
    fn test_coalesced_chunk_nested_leaf_is_reached_by_recursive_walk() {
        // The coalesced-chunk big leaf lives at params.update.content.text,
        // not a top-level field — the walk must recurse to reach it.
        let big = "z".repeat(80_000);
        let mut event = event_with_payload(
            "session_update",
            serde_json::json!({
                "params": {
                    "update": {
                        "sessionUpdate": "agent_message_chunk",
                        "content": { "text": big }
                    }
                }
            }),
        );
        fit_observer_event_to_budget(&mut event);

        assert!(serialized(&event).len() <= OBSERVER_MAX_PLAINTEXT_LEN);
        let text = event.payload["params"]["update"]["content"]["text"]
            .as_str()
            .unwrap();
        assert!(text.contains("…[elided"), "nested leaf was elided");
    }

    #[test]
    fn test_many_medium_leaves_terminate_via_stub() {
        // Many leaves each too small to shrink on their own (below 2x retain),
        // collectively over the cap. No leaf can strictly shrink, so the trimmer
        // must terminate via the stub rather than loop forever.
        let leaf = "m".repeat(OBSERVER_LEAF_RETAIN_BYTES); // shorter than head+tail → cannot shrink
        let items: Vec<serde_json::Value> = (0..40)
            .map(|_| serde_json::Value::String(leaf.clone()))
            .collect();
        let mut event = event_with_payload("acp_read", serde_json::json!({ "items": items }));
        assert!(
            serialized(&event).len() > OBSERVER_MAX_PLAINTEXT_LEN,
            "precondition: frame is over the cap"
        );

        fit_observer_event_to_budget(&mut event);

        assert!(serialized(&event).len() <= OBSERVER_MAX_PLAINTEXT_LEN);
        assert_eq!(
            event.payload["elided"].as_str().unwrap(),
            "acp_read payload too large",
            "fell back to the stub"
        );
        assert!(event.payload.get("originalBytes").is_some());
    }

    #[test]
    fn test_leaf_too_small_to_shrink_is_not_mutated() {
        // A frame already under budget whose only leaf is below the shrink floor:
        // nothing should change. (Under-budget short-circuits, and even if forced,
        // leaf_shrinks would reject it.)
        let short = "s".repeat(OBSERVER_LEAF_RETAIN_BYTES); // == head; cannot strictly shrink
        assert!(
            !leaf_shrinks(&short),
            "a leaf at the retain floor must not shrink"
        );
        let longer = "L".repeat(OBSERVER_LEAF_RETAIN_BYTES * 2 + 100);
        assert!(leaf_shrinks(&longer), "a clearly larger leaf must shrink");
    }

    #[test]
    fn test_utf8_multibyte_leaf_elides_on_char_boundary() {
        // A leaf of 3-byte chars (… = U+2026) — eliding must land on char
        // boundaries and never panic or produce invalid UTF-8.
        let big: String = "…".repeat(40_000); // 120_000 bytes
        let mut event = event_with_payload("acp_read", serde_json::json!({ "body": big }));
        fit_observer_event_to_budget(&mut event);

        assert!(serialized(&event).len() <= OBSERVER_MAX_PLAINTEXT_LEN);
        let leaf = event.payload["body"].as_str().unwrap();
        // Valid UTF-8 by construction (it's a &str); confirm head/tail are whole
        // multi-byte chars and the marker is present.
        assert!(leaf.starts_with('…'));
        assert!(leaf.ends_with('…'));
        assert!(leaf.contains("[elided"));
    }
}
